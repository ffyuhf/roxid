//! 分块并发下载引擎：Range 请求 + 块文件断点续传 + sha256 校验 + 进度回调。
//!
//! 断点续传策略：每块独立落盘于 {dest}.parts/ 目录，
//! 已存在且大小正确的块直接跳过；全部完成后顺序拼接并流式计算 sha256。
//!
//! 修改历史：M5 新增 2026-08-24 19:14
//!
//! M26 坏块自愈与 3 次重试（迭代6 P0碴3，R2/R3 裁决）：download 整轮重试
//! 3 次；拼接/校验失败清理 .parts 下轮全量重下 2026-08-26 06-54
//! M28 碴10 严格 206（迭代8）：探测与分块均要求 206 响应——原实现容忍
//! 2xx，服务器忽略 Range 返回 200 全文件时整个 body 被误写入单块导致
//! 拼接错乱、重试 3 轮全败 2026-08-30 02-10
//! M28 碴16 insecure 参数化（迭代8）：R4-A 裁决——仅 pull 下载链生效，
//! 显式请求时跳过 TLS 证书校验（对齐原版语义）2026-08-30 06-25
//! M34 O-1（迭代14）：download_chunks 流式进度上报——块任务 bytes_stream
//! 逐段累计共享计数器，主循环 200ms ticker 周期回调（原整块缓冲仅块完成
//! 回调，并发块同进同出时进度事件长时间空窗，258MB 实测全程近乎静默）；
//! 断点续传/严格 206/重试语义零变化 2026-09-07 19-50

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use sha2::{Digest, Sha256};

use crate::error::{RoxidError, RoxidResult};

/// 默认分块大小（8MB：兼顾 Range 开销与并发吞吐）
pub const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024;
/// 默认并发下载数
pub const DEFAULT_CONCURRENCY: usize = 4;

/// 下载阶段事件（M110，迭代33 碴3+碴12）：进度回调由 (完成, 总量)
/// 二元组扩展为阶段枚举——校验/重试阶段也上回调链（原校验耗时发生在
/// 层 100% 之后的静默期且事件在校验完成后才补发、重试仅 tracing 日志，
/// CLI 端全程不可见）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    /// 下载进度（已完成字节, 总字节）
    Progress(u64, u64),
    /// 完整性校验开始（拼接 + sha256 计算前——此段是 100% 后的主要耗时）
    Verifying,
    /// 重试进行中（第 N 次, 上限 M 次）——退避等待前发出，CLI 在等待
    /// 期即可见
    Retrying(u32, u32),
}

/// 分块并发下载器
pub struct ChunkedDownloader {
    http: reqwest::Client,
    chunk_size: u64,
    concurrency: usize,
}

impl Default for ChunkedDownloader {
    fn default() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE, DEFAULT_CONCURRENCY, false)
    }
}

impl ChunkedDownloader {
    /// 构造下载器。
    ///
    /// - 参数 chunk_size：单块字节数
    /// - 参数 concurrency：并发块数
    /// - 参数 insecure：true 时客户端跳过 TLS 证书校验（M28 碴16，R4-A：
    ///   仅 pull 下载链，显式请求时生效）
    pub fn new(chunk_size: u64, concurrency: usize, insecure: bool) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .danger_accept_invalid_certs(insecure)
            .build()
            .expect("构建下载客户端失败");
        Self {
            http,
            chunk_size: chunk_size.max(1),
            concurrency: concurrency.max(1),
        }
    }

    /// 下载完整文件到 dest（经 .parts 临时目录组装）。
    /// M26 碴3（R2/R3 裁决 2026-08-26 06:42）：整轮最多重试 3 次（1s/2s 退避，
    /// 对齐项目计划书第四章失败判定第 4 条与 runtime/download.rs 先例节奏）——
    /// 网络失败保留 .parts 断点续传重试；拼接/校验失败已清理 .parts，
    /// 下轮全量重下覆盖坏块（自愈，根除长度恰好匹配的坏块死循环）。
    ///
    /// - 参数 url：直链（可重定向）
    /// - 参数 dest：目标文件路径
    /// - 参数 expected_sha256：期望摘要（十六进制，不带前缀）；不匹配即删除产物报错
    /// - 参数 on_progress：进度回调 (已完成字节, 总字节)，供 NDJSON/进度条消费
    /// - 返回：Ok(()) 表示文件就位且校验通过
    pub async fn download(
        &self,
        url: &str,
        dest: &Path,
        expected_sha256: Option<&str>,
        mut on_event: impl FnMut(DownloadPhase) + Send,
    ) -> RoxidResult<()> {
        let mut last_error = String::new();
        for attempt in 1..=3u32 {
            match self
                .download_once(url, dest, expected_sha256, &mut on_event)
                .await
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::warn!("下载失败（第 {attempt}/3 次）{url}：{e}");
                    last_error = e.to_string();
                    if attempt < 3 {
                        // M110：重试事件上回调链（退避前发出——等待期
                        // CLI 即可见；原仅 serve 日志可见）
                        on_event(DownloadPhase::Retrying(attempt, 3));
                        // 1s/2s 退避
                        tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await;
                    }
                }
            }
        }
        Err(RoxidError::RegistryRequest(format!(
            "重试 3 次仍失败 {url}：{last_error}"
        )))
    }

    /// 单轮下载（重试循环体）：探测大小 → 分块并发 → 拼接校验。
    ///
    /// - 参数：语义同 download
    /// - 返回：Ok(()) 表示本轮成功
    async fn download_once(
        &self,
        url: &str,
        dest: &Path,
        expected_sha256: Option<&str>,
        mut on_event: impl FnMut(DownloadPhase) + Send,
    ) -> RoxidResult<()> {
        let total = self.probe_total_size(url).await?;
        on_event(DownloadPhase::Progress(0, total));
        let parts_dir = parts_dir_of(dest);
        self.download_chunks(url, &parts_dir, total, &mut on_event)
            .await?;
        // M110：校验阶段事件前移——拼接 + sha256 是 100% 后的主要耗时
        //（原 registry 尾部 verifying 事件在校验完成后才发，CLI 在
        // 「100% 停留期」空转无阶段区分）
        on_event(DownloadPhase::Verifying);
        self.assemble_and_verify(&parts_dir, dest, total, expected_sha256)
            .await?;
        on_event(DownloadPhase::Progress(total, total));
        Ok(())
    }

    /// HEAD/Range 探测资源总大小。
    ///
    /// - 参数 url：直链
    /// - 返回：资源字节总数
    async fn probe_total_size(&self, url: &str) -> RoxidResult<u64> {
        // Range: bytes=0-0 探测，Content-Range: bytes 0-0/TOTAL
        let resp = self
            .http
            .get(url)
            .header("Range", "bytes=0-0")
            .send()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
        // M28 碴10：Range 探测必须得到 206——非 206（如 200 全文件）说明源
        // 不支持 Range，分块下载前提不成立，直接报错而非静默错乱
        if resp.status().as_u16() != 206 {
            return Err(RoxidError::RegistryRequest(format!(
                "探测大小失败：HTTP {}（非 206，源不支持 Range）：{url}",
                resp.status()
            )));
        }
        let content_range = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| RoxidError::RegistryRequest(format!("缺少 Content-Range：{url}")))?;
        // 形如 "bytes 0-0/91727296"
        let total = content_range
            .rsplit('/')
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| {
                RoxidError::RegistryRequest(format!("解析 Content-Range 失败：{content_range}"))
            })?;
        Ok(total)
    }

    /// 并发下载全部分块（已完成的块跳过 → 断点续传）。
    /// M34 O-1：块任务流式接收（bytes_stream 逐段累计共享计数器）+ 主循环
    /// 200ms ticker 周期上报——原整块 resp.bytes() 缓冲、仅块完成回调，
    /// 并发块同进同出时进度事件长时间空窗；ticker 只在主任务调用回调
    ///（无共享闭包，块任务零锁；断点续传/严格 206/重试语义零变化）。
    async fn download_chunks(
        &self,
        url: &str,
        parts_dir: &Path,
        total: u64,
        on_event: &mut (impl FnMut(DownloadPhase) + Send),
    ) -> RoxidResult<()> {
        std::fs::create_dir_all(parts_dir)?;
        let chunk_count = total.div_ceil(self.chunk_size);
        let mut done_bytes = 0u64;

        // 先统计已完成块（断点续传基线）
        let mut pending = Vec::new();
        for index in 0..chunk_count {
            let (start, end) = self.chunk_range(index, total);
            let path = parts_dir.join(format!("{index:06}.part"));
            let expect_len = end - start + 1;
            match std::fs::metadata(&path) {
                Ok(m) if m.len() == expect_len => done_bytes += expect_len,
                _ => pending.push(index),
            }
        }
        on_event(DownloadPhase::Progress(done_bytes, total));

        // 信号量限制并发；进度经共享计数器流式累计
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(self.concurrency));
        let done_counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(done_bytes));
        let mut handles = Vec::new();
        for index in pending {
            let sem = semaphore.clone();
            let counter = done_counter.clone();
            let url = url.to_string();
            let parts_dir = parts_dir.to_path_buf();
            let http = self.http.clone();
            let chunk_size = self.chunk_size;
            handles.push(tokio::spawn(async move {
                let _permit = sem
                    .acquire()
                    .await
                    .map_err(|e| RoxidError::RunnerFailure(e.to_string()))?;
                let (start, end) = chunk_range_static(index, total, chunk_size);
                let resp = http
                    .get(&url)
                    .header("Range", format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
                // M28 碴10：分块必须 206——200 全文件若被误当单块字节写入，
                // 拼接产物错乱且重试必然全败（每轮同样收到 200）
                if resp.status().as_u16() != 206 {
                    return Err(RoxidError::RegistryRequest(format!(
                        "分块下载失败：HTTP {}（非 206，源忽略 Range）：#{index}",
                        resp.status()
                    )));
                }
                // M34 O-1：流式接收逐段累计（原 resp.bytes() 整块缓冲期间零进度）
                let mut stream = resp.bytes_stream();
                let mut buf: Vec<u8> = Vec::with_capacity(chunk_size as usize);
                while let Some(seg) = stream.next().await {
                    let seg =
                        seg.map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
                    counter.fetch_add(seg.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    buf.extend_from_slice(&seg);
                }
                let tmp = parts_dir.join(format!("{index:06}.tmp"));
                tokio::fs::write(&tmp, &buf).await?;
                tokio::fs::rename(&tmp, parts_dir.join(format!("{index:06}.part"))).await?;
                Ok::<(), RoxidError>(())
            }));
        }
        // M34 O-1：join 与 200ms ticker 交替——周期上报流式进度，全部完成收敛
        let mut jobs: futures::stream::FuturesUnordered<_> = handles.into_iter().collect();
        let mut ticker = tokio::time::interval(Duration::from_millis(200));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if jobs.is_empty() {
                break;
            }
            tokio::select! {
                _ = ticker.tick() => {
                    on_event(DownloadPhase::Progress(
                        done_counter.load(std::sync::atomic::Ordering::Relaxed),
                        total,
                    ));
                }
                res = jobs.next() => {
                    match res {
                        Some(r) => r.map_err(|e| RoxidError::RunnerFailure(format!("下载任务异常：{e}")))??,
                        None => break,
                    }
                }
            }
        }
        // 终值兜底（断点全命中 jobs 为空时直接至此；保证收敛到 total）
        on_event(DownloadPhase::Progress(
            done_counter.load(std::sync::atomic::Ordering::Relaxed),
            total,
        ));
        Ok(())
    }

    /// 顺序拼接分块为目标文件，同时流式计算 sha256 并校验。
    async fn assemble_and_verify(
        &self,
        parts_dir: &Path,
        dest: &Path,
        total: u64,
        expected_sha256: Option<&str>,
    ) -> RoxidResult<()> {
        use tokio::io::AsyncWriteExt;

        let chunk_count = total.div_ceil(self.chunk_size);
        let tmp = dest.with_extension("assembling");
        let mut writer = tokio::io::BufWriter::new(tokio::fs::File::create(&tmp).await?);
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        for index in 0..chunk_count {
            let bytes = tokio::fs::read(parts_dir.join(format!("{index:06}.part"))).await?;
            hasher.update(&bytes);
            writer.write_all(&bytes).await?;
            written += bytes.len() as u64;
        }
        writer.flush().await?;
        drop(writer);
        if written != total {
            std::fs::remove_file(&tmp).ok();
            // M26 碴3：大小不符即含坏块，清空分块目录（下轮全量重下自愈）
            std::fs::remove_dir_all(parts_dir).ok();
            return Err(RoxidError::RegistryRequest(format!(
                "拼接大小不符：{written} != {total}"
            )));
        }
        if let Some(expected) = expected_sha256 {
            let actual = format!("{:x}", hasher.finalize());
            if actual != expected {
                std::fs::remove_file(&tmp).ok();
                // M26 碴3：坏块自愈核心——校验失败清空分块目录，
                // 根除「长度恰好匹配的坏块被续传跳过 → sha256 永远失败」死循环
                std::fs::remove_dir_all(parts_dir).ok();
                return Err(RoxidError::RegistryRequest(format!(
                    "sha256 校验失败：期望 {expected}，实际 {actual}"
                )));
            }
        }
        tokio::fs::rename(&tmp, dest).await?;
        std::fs::remove_dir_all(parts_dir).ok();
        Ok(())
    }

    /// 计算第 index 块的字节区间 [start, end]（闭区间）
    fn chunk_range(&self, index: u64, total: u64) -> (u64, u64) {
        chunk_range_static(index, total, self.chunk_size)
    }
}

/// 独立函数形式的块区间计算（并发任务内使用）
fn chunk_range_static(index: u64, total: u64, chunk_size: u64) -> (u64, u64) {
    let start = index * chunk_size;
    let end = (start + chunk_size - 1).min(total - 1);
    (start, end)
}

/// 分块临时目录路径：{dest}.parts/
fn parts_dir_of(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".parts");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 块区间：整除、不整除、单块小文件三种形态
    #[test]
    fn chunk_range_math() {
        assert_eq!(chunk_range_static(0, 100, 40), (0, 39));
        assert_eq!(chunk_range_static(1, 100, 40), (40, 79));
        assert_eq!(
            chunk_range_static(2, 100, 40),
            (80, 99),
            "末块必须收在 total-1"
        );
        assert_eq!(chunk_range_static(0, 30, 40), (0, 29), "单块小文件区间完整");
    }

    /// parts 目录命名必须紧贴目标文件
    #[test]
    fn parts_dir_naming() {
        assert_eq!(
            parts_dir_of(Path::new("/a/b/model.gguf")),
            PathBuf::from("/a/b/model.gguf.parts")
        );
    }

    /// M26 碴3：坏块自愈（纯逻辑，无网络）——拼接校验失败必须清理 parts 目录
    /// 与临时文件，否则长度恰好匹配的坏块会被续传跳过导致 sha256 永久失败
    #[tokio::test]
    async fn corrupt_chunk_triggers_parts_cleanup() {
        let dir = std::env::temp_dir().join(format!("roxid-dl-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let payload: Vec<u8> = (0..16u8).collect(); // 16 字节真身
        let parts = dir.join("dest.parts");
        std::fs::create_dir_all(&parts).unwrap();
        // 两块（8B 分块）：第二块内容损坏但长度与期望一致（坏块最险形态）
        std::fs::write(parts.join("000000.part"), &payload[..8]).unwrap();
        std::fs::write(parts.join("000001.part"), b"XXXXXXXX").unwrap();
        let dest = dir.join("dest.bin");
        let expected = format!("{:x}", Sha256::digest(&payload));
        let dl = ChunkedDownloader::new(8, 2, false);
        let result = dl
            .assemble_and_verify(&parts, &dest, 16, Some(&expected))
            .await;
        assert!(result.is_err(), "坏块场景校验必须失败");
        assert!(!parts.exists(), "校验失败必须清理 parts 目录（自愈前提）");
        assert!(
            !dest.with_extension("assembling").exists(),
            "临时文件必须清理"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// M26 碴3：坏块经 3 次重试自愈（python 替身）——预置坏块后 download
    /// 应在第 2 轮全量重下后成功（第一轮校验失败已清 parts）
    #[tokio::test]
    #[ignore = "需要 python3 替身服务，验收时手动执行"]
    async fn corrupt_chunk_download_recovers_on_retry() {
        let payload: Vec<u8> = (0..32768u32).map(|i| (i * 7 % 251) as u8).collect();
        let port = 38213u16;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(format!(
            r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
data = bytes((i * 7 % 251) for i in range(32768))
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        rng = self.headers.get('Range')
        if rng:
            a, b = rng.replace('bytes=', '').split('-')
            chunk = data[int(a):int(b)+1]
            self.send_response(206)
            self.send_header('Content-Range', f'bytes {{a}}-{{b}}/{{len(data)}}')
        else:
            chunk = data
            self.send_response(200)
        self.send_header('Content-Length', str(len(chunk)))
        self.end_headers(); self.wfile.write(chunk)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', {port}), H).serve_forever()
"#
        ));
        let _stub =
            crate::scheduler::Runner::spawn_with("stub", port, Duration::from_secs(300), &mut cmd)
                .await
                .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let dest = std::env::temp_dir().join("roxid-dl-retry-e2e.bin");
        std::fs::remove_file(&dest).ok();
        let parts = parts_dir_of(&dest);
        std::fs::remove_dir_all(&parts).ok();
        std::fs::create_dir_all(&parts).unwrap();
        // 预置坏块：块 0 内容损坏但长度与期望一致（8KB 分块 → 4 块）
        std::fs::write(parts.join("000000.part"), vec![0u8; 8192]).unwrap();
        let expected = format!("{:x}", Sha256::digest(&payload));
        let dl = ChunkedDownloader::new(8192, 4, false);
        dl.download(
            &format!("http://127.0.0.1:{port}/file"),
            &dest,
            Some(&expected),
            |_| {},
        )
        .await
        .expect("坏块必须经重试自愈（第 2 轮全量重下成功）");
        assert_eq!(std::fs::read(&dest).unwrap(), payload, "最终产物必须正确");
        assert!(!parts.exists(), "完成后 parts 目录必须清理");
    }

    /// M28 碴10：分块下载必须严格 206——坏源忽略 Range 恒返 200 全文件时
    /// 必须报错且信息指明（原实现误将整个 body 写入单块导致拼接错乱）
    #[tokio::test]
    #[ignore = "需要 python3 替身服务，验收时手动执行"]
    async fn non_206_full_body_rejected() {
        let port = 38215u16;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(format!(
            r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
data = b"x" * 65536
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        # 忽略 Range 头恒返 200 全文件（坏源行为）
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers(); self.wfile.write(data)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', {port}), H).serve_forever()
"#
        ));
        let _stub =
            crate::scheduler::Runner::spawn_with("stub", port, Duration::from_secs(300), &mut cmd)
                .await
                .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let dl = ChunkedDownloader::new(8192, 2, false);
        let dest = std::env::temp_dir().join("roxid-dl-non206.bin");
        std::fs::remove_file(&dest).ok();
        let err = dl
            .download(
                &format!("http://127.0.0.1:{port}/file"),
                &dest,
                None,
                |_| {},
            )
            .await
            .expect_err("非 206 响应必须报错（不得误作单块写入）");
        assert!(
            err.to_string().contains("非 206"),
            "错误信息必须指明非 206：{err}"
        );
        assert!(!dest.exists(), "失败产物不得落位");
    }

    /// 真实分块下载验收（经本地 python http 替身提供 Range 响应）：
    /// 验证并发下载、拼接、sha256 校验与断点续传跳过
    #[tokio::test]
    #[ignore = "需要 python3 替身服务，验收时手动执行"]
    async fn chunked_download_end_to_end() {
        // 32KB 随机数据，8KB 分块 → 4 块
        let payload: Vec<u8> = (0..32768u32).map(|i| (i * 7 % 251) as u8).collect();
        let port = 38211u16;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(format!(
            r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys
data = bytes((i * 7 % 251) for i in range(32768))
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        rng = self.headers.get('Range')
        if rng:
            a, b = rng.replace('bytes=', '').split('-')
            chunk = data[int(a):int(b)+1]
            self.send_response(206)
            self.send_header('Content-Range', f'bytes {{a}}-{{b}}/{{len(data)}}')
        else:
            chunk = data
            self.send_response(200)
        self.send_header('Content-Length', str(len(chunk)))
        self.end_headers(); self.wfile.write(chunk)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', {port}), H).serve_forever()
"#
        ));
        let _stub =
            crate::scheduler::Runner::spawn_with("stub", port, Duration::from_secs(300), &mut cmd)
                .await
                .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let expected = format!("{:x}", Sha256::digest(&payload));
        let dest = std::env::temp_dir().join("roxid-dl-e2e.bin");
        std::fs::remove_file(&dest).ok();
        let dl = ChunkedDownloader::new(8192, 4, false);
        let mut events = 0usize;
        dl.download(
            &format!("http://127.0.0.1:{port}/file"),
            &dest,
            Some(&expected),
            |_| events += 1,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            payload,
            "拼接结果必须逐字节一致"
        );
        assert!(events >= 4, "进度回调至少每块一次");
        assert!(!parts_dir_of(&dest).exists(), "完成后 parts 目录必须清理");
    }
}
