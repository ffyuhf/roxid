//! HuggingFace GGUF 直拉（辅源，来源：用户确认 2026-08-24 18:24 Q3）。
//!
//! 语法：hf.co/{user}/{repo}[:{quant}]
//! - repo 内 GGUF 目录：GET {基址}/api/models/{repo}/tree/main/GGUF
//! - 直链下载：{基址}/{repo}/resolve/main/GGUF/{file}
//! - quant 缺省时选择 Q4_K_M 变体；指定时按文件名词边界匹配
//! - 注册名（迭代11 Q3 裁决 2026-09-06 22:35）：hf.co/{user}/{repo}:{quant}
//! - 落位目录（迭代11 F1）：models/HF/{user}/{repo}/{quant}/model.gguf
//! - 镜像支持（M16）：基址经 hf_base() 构造——ROXID_HF_PROXY 优先回退
//!   HF_ENDPOINT，官方 https://huggingface.co 整体替换为镜像地址（直连不可达
//!   环境经镜像可用，2026-08-24 20:06 实测 hf-mirror 302→CDN 200）
//!
//! 元数据来源：M14 起解析 GGUF header（见 gguf.rs），
//! 解析失败降级为 repo 名/文件名保守推断（不阻断 pull）。
//! 完整性校验（M16）：HEAD 取 LFS x-linked-etag（sha256），复用分块下载
//! 校验管线；摘要不可得或形态不符降级跳过（warn 不阻断）。
//!
//! 修改历史：M5 新增 2026-08-24 19:15；M14 元数据真实化 2026-08-24 20-04；
//! M16 镜像变量与 sha256 校验 2026-08-24 20-14；M20 层摘要索引 2026-08-24 22:44；
//! M22 license 字段 2026-08-24 23:20；
//! M28 十八碴清偿三碴（迭代8）：量化词边界精确匹配（碴5：Q4_K_M 不再误中
//! IQ4_K_M）、hf_base 文件段 mtime 缓存（碴11：消除每次 HF 请求的重复读盘）、
//! 下载进度回调直连 on_event（碴12：HF 拉取进度不再静默）2026-08-30 02-10
//! M29 碴5（迭代9）：ETag 摘要获取按 insecure 参数化（原全局安全 Client 在
//! 自签镜像场景 HEAD 失败 → sha256 校验静默降级，R4-A 下载链漏 HEAD 环节）
//! 2026-09-05 12-52
//! M30 两碴（迭代10）：碴3 pull 落位 dest 级互斥+就位快路径（hf.co 别名
//! 在 api 层键回退原名绕过 PullGate，pick 后同 dest 并发互踩 .parts）、
//! 碴4 resolve_etag_sha256 网络失败/缺头 warn 留痕（原静默降级无从排查）
//! 2026-09-06 22-15
//! M31（迭代11 F1/F3）：落位改 models/HF/{user}/{repo}/{quant}/ 三级路径
//! （废弃 -- 转义）、注册名改 hf.co/{user}/{repo}:{quant} 对齐原版
//! （原因：存储三分离与 HF 直引需求变更）2026-09-06 22-52

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::error::{RoxidError, RoxidResult};
use crate::repo::{ModelFiles, ModelMeta, ModelRef};

use super::downloader::ChunkedDownloader;
use super::downloader::DownloadPhase;
use super::PullEvent;

/// HuggingFace 官方基址（未设镜像变量时的默认值）
pub const HF_BASE: &str = "https://huggingface.co";

/// 镜像环境变量：本项目命名惯例（优先）
const ENV_HF_PROXY: &str = "ROXID_HF_PROXY";
/// 镜像环境变量：HF 生态工具惯例（回退）
const ENV_HF_ENDPOINT: &str = "HF_ENDPOINT";

/// 镜像优先级与尾斜杠规整（纯函数，便于单测）
fn resolve_base(proxy: Option<String>, endpoint: Option<String>) -> String {
    proxy
        .or(endpoint)
        .unwrap_or_else(|| HF_BASE.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// HF 请求基址：官方域名整体替换为镜像地址（路径拼接语义）。
/// 优先级：ROXID_HF_PROXY > HF_ENDPOINT > config.toml [proxy].hf > 官方
///（读取链 env 优先 → 文件回退：迭代4 裁决 Q2 2026-08-26 05:24）。
/// 代码不含任何默认镜像地址（沿用 Q11「只要变量不要默认网址」裁决模式）。
/// 来源：用户确认 R2-0a 2026-08-24 19:57 / R2-3C 2026-08-24 19:59
/// M28 碴11：文件段经 mtime 缓存读取（hf_url 随每次 HF 请求构造，
/// 原实现每次读盘解析 config.toml）——env 两级仍每次实时读取。
pub fn hf_base() -> String {
    let file_hf = cached_file_proxy_hf();
    resolve_base(
        std::env::var(ENV_HF_PROXY).ok().filter(|s| !s.is_empty()),
        std::env::var(ENV_HF_ENDPOINT)
            .ok()
            .filter(|s| !s.is_empty())
            .or(file_hf),
    )
}

/// config.toml [proxy].hf 的 mtime 缓存读取（M28 碴11）。
/// 缓存键 =（文件路径, mtime）：路径变化（ROXID_HOME 切换）或 mtime 变更
/// 才重读文件；文件不存在（mtime 取不到）不缓存、每次走 load 的快速默认路径。
/// env 部分不缓存（进程内读取开销可忽略），保持 Q2 动态语义与既有单测兼容。
///
/// - 返回：文件段镜像基址（未配置或空为 None）
fn cached_file_proxy_hf() -> Option<String> {
    static CACHE: std::sync::Mutex<
        Option<((std::path::PathBuf, std::time::SystemTime), Option<String>)>,
    > = std::sync::Mutex::new(None);
    let path = crate::config::config_file_path();
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let (Some((key, value)), Some(current)) = (&*guard, mtime) {
        if *key == (path.clone(), current) {
            return value.clone();
        }
    }
    let value = crate::config::load_persist_config()
        .proxy
        .hf
        .filter(|s| !s.is_empty());
    if let Some(current) = mtime {
        *guard = Some(((path, current), value.clone()));
    }
    value
}

/// 基址 + 路径 → 完整 URL（path 以 / 开头，基址已规整无尾斜杠）
fn hf_url(path: &str) -> String {
    format!("{}{path}", hf_base())
}

/// 文件路径尾段的量化词（小写）：GGUF/m-IQ4_K_M.gguf → "iq4_k_m"；
/// 无连字符时以整个 stem 充当（model.gguf → "model"，天然不命中量化 tag）。
/// M28 碴5：量化匹配的词边界提取基础。
fn quant_segment_lower(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let stem = base.trim_end_matches(".gguf");
    stem.rsplit('-').next().unwrap_or(stem).to_ascii_lowercase()
}

/// 量化词边界匹配：段与 tag 全等，或段以 tag 起始且紧随下划线边界
/// （"q8" 命中 "q8_0"；"q4_k_m" 不命中 "iq4_k_m"——M28 碴5 根除前缀误选）。
fn quant_segment_matches(segment: &str, tag: &str) -> bool {
    segment == tag || (segment.starts_with(tag) && segment[tag.len()..].starts_with('_'))
}

/// ETag 值 → sha256 hex（HF LFS 语义）：剥引号与 sha256: 前缀后
/// 恰 64 个 hex 字符才有效；旧文件 md5（32hex）等形态返回 None。
fn etag_to_sha256(etag: &str) -> Option<String> {
    let hex = etag.trim().trim_matches('"');
    let hex = hex.strip_prefix("sha256:").unwrap_or(hex);
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

/// GGUF 目录文件条目（tree API 返回）
#[derive(Debug, Clone, Deserialize)]
pub struct HfTreeFile {
    /// 文件路径（GGUF/xxx.gguf）
    pub path: String,
    /// 字节大小
    pub size: u64,
}

/// HuggingFace GGUF 直拉客户端
pub struct HuggingFaceSource {
    http: reqwest::Client,
    downloader: ChunkedDownloader,
    /// HEAD 摘要获取的证书校验开关（M29 碴5：与下载链同 insecure 参数化）
    insecure: bool,
}

impl Default for HuggingFaceSource {
    fn default() -> Self {
        Self::new()
    }
}

impl HuggingFaceSource {
    /// 构造客户端（默认安全校验路径）。
    pub fn new() -> Self {
        Self::with_insecure(false)
    }

    /// 构造客户端。
    /// M28 碴16（R4-A）：insecure=true 时下载链客户端跳过 TLS 证书校验
    ///（对齐原版 pull insecure 字段语义：显式请求时生效，默认路径不变）
    ///
    /// - 参数 insecure：是否跳过 TLS 证书校验
    pub fn with_insecure(insecure: bool) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(120))
                .danger_accept_invalid_certs(insecure)
                .build()
                .expect("构建 HF 客户端失败"),
            downloader: ChunkedDownloader::new(
                super::downloader::DEFAULT_CHUNK_SIZE,
                super::downloader::DEFAULT_CONCURRENCY,
                insecure,
            ),
            insecure,
        }
    }

    /// 列出 repo 全部 GGUF 文件。
    /// 布局兼容（2026-08-24 20:17 实测）：部分仓库用 GGUF/ 子目录，
    /// bartowski 系等将 GGUF 平铺在仓库根目录——先查子目录，空或 404 回退根目录。
    ///
    /// - 参数 repo：形如 user/model
    /// - 返回：GGUF 文件条目列表（两处均无则空，由调用方报 ModelNotFound）
    pub async fn list_gguf_files(&self, repo: &str) -> RoxidResult<Vec<HfTreeFile>> {
        let subdir = self.list_tree_path(repo, "/GGUF").await?;
        if !subdir.is_empty() {
            return Ok(subdir);
        }
        self.list_tree_path(repo, "").await
    }

    /// tree API 查询单一路径（sub 为 "" 或 "/GGUF"）。
    /// 404 视为该路径无文件（返回空，支撑布局回退）；其他非 2xx 报错。
    async fn list_tree_path(&self, repo: &str, sub: &str) -> RoxidResult<Vec<HfTreeFile>> {
        let url = hf_url(&format!("/api/models/{repo}/tree/main{sub}"));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        if !resp.status().is_success() {
            return Err(RoxidError::RegistryRequest(format!(
                "HTTP {}：{url}",
                resp.status()
            )));
        }
        let files: Vec<HfTreeFile> = resp
            .json()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("解析 GGUF 列表失败：{e}")))?;
        Ok(files
            .into_iter()
            .filter(|f| f.path.ends_with(".gguf"))
            .collect())
    }

    /// 从文件名提取 quant tag：model-Q4_K_M.gguf → Q4_K_M
    fn quant_of(file_name: &str) -> String {
        let stem = file_name.trim_end_matches(".gguf");
        match stem.rsplit_once('-') {
            Some((_, q)) if q.len() >= 4 && q.as_bytes()[0].is_ascii_alphabetic() => q.to_string(),
            _ => "default".to_string(),
        }
    }

    /// 按 tag 选择文件（M28 碴5：量化词边界匹配——tag 命中文件名尾段量化词的
    /// 全等或「tag + 下划线边界」；"Q4_K_M" 不再误中 "IQ4_K_M"）；
    /// 缺省 tag 优先 Q4_K_M，其次取列表第一个（语义不变）。
    fn pick_file<'a>(files: &'a [HfTreeFile], tag: Option<&str>) -> Option<&'a HfTreeFile> {
        if files.is_empty() {
            return None;
        }
        if let Some(tag) = tag {
            let needle = tag.to_ascii_lowercase();
            return files
                .iter()
                .find(|f| quant_segment_matches(&quant_segment_lower(&f.path), &needle));
        }
        files
            .iter()
            .find(|f| quant_segment_matches(&quant_segment_lower(&f.path), "q4_k_m"))
            .or_else(|| files.first())
    }

    /// 无匹配 GGUF 的报错文案（迭代18 BUG-14）：人类可读、无 Debug 泄漏。
    ///
    /// - 参数 repo：形如 user/model
    /// - 参数 tag：量化 tag（None 表示缺省首选 Q4_K_M）
    /// - 返回：错误载荷文本
    fn no_gguf_message(repo: &str, tag: Option<&str>) -> String {
        let quant_desc = match tag {
            Some(q) => format!("量化 {q}"),
            None => "量化 Q4_K_M（缺省首选）".to_string(),
        };
        format!("hf.co/{repo} 未找到{quant_desc}的 GGUF 文件（仓库不含 GGUF 或无此变体）")
    }

    /// 拉取 HF 模型并落位。
    ///
    /// - 参数 repo：user/model 仓库路径
    /// - 参数 tag：量化变体（None → Q4_K_M 优先）
    /// - 参数 models_root：模型仓库根目录
    /// - 参数 on_event：进度事件回调
    /// - 返回：落位后的模型引用（{dir_name}:{quant}）
    pub async fn pull(
        &self,
        repo: &str,
        tag: Option<&str>,
        models_root: &Path,
        mut on_event: impl FnMut(PullEvent) + Send,
    ) -> RoxidResult<ModelRef> {
        on_event(PullEvent::status(format!("pulling hf.co/{repo} GGUF 列表")));
        let files = self.list_gguf_files(repo).await?;
        // 迭代18 BUG-14（M50）：文案整形——原 `tag={tag:?}` 泄漏 Rust
        // Debug 格式（Some("Q4_K_M")）；改人类可读（外层 ModelNotFound
        // Display 的官方 404 模板形态保持——官方对齐锚定，见 ops.rs 测试）
        let picked = Self::pick_file(&files, tag)
            .ok_or_else(|| RoxidError::ModelNotFound(Self::no_gguf_message(repo, tag)))?;
        let file_name = picked
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&picked.path)
            .to_string();
        let quant = Self::quant_of(&file_name);

        // 迭代11 F1/F3：注册名 hf.co/{user}/{repo}:{quant}（quant_of 产物
        // 仅含字母数字/点/下划线/连字符，parse 必过）；落位 HF 三级路径
        let r = ModelRef::parse(&format!("hf.co/{repo}:{quant}"))?;
        let dir = r.dir_in(models_root, crate::repo::ModelSource::Hf);
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join("model.gguf");
        on_event(PullEvent {
            status: Some(format!("pulling {file_name}")),
            digest: None,
            total: Some(picked.size),
            completed: Some(0),
            error: None,
        });
        let url = hf_url(&format!("/{repo}/resolve/main/{}", picked.path));
        // HEAD 取 LFS 摘要（x-linked-etag 即文件 sha256）：镜像/直连同语义；
        // 不可得或形态不符降级跳过校验（warn 不阻断下载；M30 碴4 全路径留痕）
        let expected = self.resolve_etag_sha256(&url).await;
        // M30 碴3（R1-A）：落位 dest 级互斥——hf.co 别名（"hf.co/user/repo"
        // 缺省 tag 与 "hf.co/user/repo:Q4_K_M"）在 api 层 pull_key 回退原名
        // 无法去重，但经 pick_file 解析后落位同一 dest——并发写同名 .parts
        // 互踩（M28 碴2 修复在 HF 别名场景的遗留面）。等锁者先走就位快路径
        // （dest 完整且大小与 tree 条目一致即前序任务产物，downloader 原子
        // rename 保证完整文件才会出现），否则自行下载。
        // M28 碴12：下载进度直连外层 on_event；digest 携带 ETag sha（可得时），
        // 对齐主源 download_big_blob 的事件形态。借用作用域收敛：门守卫与
        // progress 借用在块结束时释放，后续 gguf 解析与落位事件可继续使用。
        {
            let gate = dest_gate(&dest);
            let _guard = gate.lock().await;
            if dest_already_complete(&dest, picked.size) {
                tracing::info!(
                    "HF 目标已就位（别名并发快路径跳过下载）：{}",
                    dest.display()
                );
            } else {
                // M110（迭代33）：回调改阶段枚举——Progress 转进度事件、
                // Verifying/Retrying 转状态事件（对齐主源事件形态）
                let mut progress = |ph: DownloadPhase| match ph {
                    DownloadPhase::Progress(done, tot) => on_event(PullEvent {
                        status: Some("pulling".into()),
                        digest: expected.as_deref().map(|s| format!("sha256:{s}")),
                        total: Some(tot),
                        completed: Some(done),
                        error: None,
                    }),
                    DownloadPhase::Verifying => {
                        on_event(PullEvent::status("verifying sha256 digest"))
                    }
                    DownloadPhase::Retrying(n, max) => on_event(PullEvent::status(format!(
                        "retrying download (attempt {n}/{max})"
                    ))),
                };
                self.downloader
                    .download(
                        &url,
                        &dest,
                        expected.as_deref(), // 下载完成后流式 sha256 校验（M16 增强）
                        progress,
                    )
                    .await?;
            }
        }

        on_event(PullEvent::status("reading gguf header"));
        // GGUF header 解析：失败降级保守推断（warn 不阻断）
        let hdr = super::gguf::parse_metadata(&dest).unwrap_or_else(|e| {
            tracing::warn!("GGUF header 解析失败，元数据降级推断：{e}");
            super::gguf::GgufMetadata::default()
        });

        on_event(PullEvent::status("writing manifest"));
        let family = hdr
            .architecture
            .clone()
            .unwrap_or_else(|| repo.rsplit('/').next().unwrap_or(repo).to_string());
        let meta = ModelMeta {
            runtime: None,
            name: r.full_name(),
            family: family.clone(),
            families: if family.is_empty() {
                vec![]
            } else {
                vec![family]
            },
            parameter_size: hdr.size_label.clone().unwrap_or_default(),
            quantization_level: hdr
                .file_type
                .map(super::gguf::file_type_to_quant_name)
                .unwrap_or_else(|| r.tag.clone()),
            system: String::new(),
            template: None,
            parameters: Default::default(),
            messages: vec![],
            // M20：ETag sha256（可得时）作为 model 层摘要索引
            layer_digests: expected
                .map(|sha| {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("model".to_string(), format!("sha256:{sha}"));
                    m
                })
                .unwrap_or_default(),
            adapters: vec![],
            files: ModelFiles {
                model: "model.gguf".into(),
                mmproj: None,
            },
            digest: String::new(),
            license: String::new(), // HF 直拉无 license 层
            source: format!("huggingface:{repo}"),
            created_at: super::now_rfc3339(),
        };
        crate::repo::model_json::save_meta(&dir, &meta)?;
        on_event(PullEvent::status("success"));
        Ok(r)
    }

    /// HEAD 请求取 resolve 的 LFS 摘要（sha256 hex）。
    /// 禁止跟随重定向：302 层响应头才是 HF 元数据的文件 sha256
    /// （CDN 层 etag 为 xet hash，与内容 sha256 不一定一致，不可用于校验）。
    /// M29 碴5：按 self.insecure 选危险证书变体——自签镜像 HEAD 不再因证书
    /// 校验失败而使完整性校验静默降级。
    /// M30 碴4：全部降级路径 warn 留痕——原网络失败/缺头 `.ok()?` 全静默，
    /// sha256 校验悄悄消失无从排查（与 M29 碴11 同类可观测性缺口）。
    ///
    /// - 参数 url：resolve 直链（官方或镜像基址）
    /// - 返回：合法形态的 sha256 hex；任何失败返回 None（跳过校验，留 warn）
    async fn resolve_etag_sha256(&self, url: &str) -> Option<String> {
        let no_redirect = no_redirect_client(self.insecure);
        let resp = match no_redirect.head(url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("HF HEAD 摘要获取失败，跳过完整性校验：{url}：{e}");
                return None;
            }
        };
        let etag = match resp
            .headers()
            .get("x-linked-etag")
            .or_else(|| resp.headers().get("etag"))
        {
            Some(v) => match v.to_str() {
                Ok(s) => s.to_string(),
                Err(e) => {
                    tracing::warn!("HF ETag 头非 ASCII，跳过完整性校验：{url}：{e}");
                    return None;
                }
            },
            None => {
                tracing::warn!(
                    "HF 响应缺少 ETag 头，跳过完整性校验：{url}（HTTP {}）",
                    resp.status()
                );
                return None;
            }
        };
        let sha = etag_to_sha256(&etag);
        if sha.is_none() {
            tracing::warn!("HF ETag 非 sha256 形态，跳过完整性校验：{etag}");
        }
        sha
    }
}

/// HF 落位 dest 级互斥门表（M30 碴3，R1-A）：dest 由 repo 转义名 + 量化 tag
/// 决定，同 dest 必为同一目标文件——门条目为小对象常驻（模型数量级可忽略，
/// 对齐 scheduler loading 锁表先例）。
static DEST_GATES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// 取 dest 对应的互斥门（同 dest 同门、不同 dest 各持各门）。
///
/// - 参数 dest：落位目标文件路径（model.gguf 全路径）
/// - 返回：该 dest 的互斥门句柄
fn dest_gate(dest: &Path) -> Arc<tokio::sync::Mutex<()>> {
    let mut gates = DEST_GATES.lock().unwrap_or_else(|p| p.into_inner());
    gates.entry(dest.to_path_buf()).or_default().clone()
}

/// 就位快路径判定（M30 碴3）：dest 存在且大小与 tree 条目一致——downloader
/// 仅在完整拼接并通过校验后才 rename 落位，完整文件出现即前序任务产物；
/// 大小不符（前次异常残留的半文件）走重下自愈。
///
/// - 参数 dest：落位目标文件路径
/// - 参数 expected_size：HF tree API 报告的文件字节数
/// - 返回：true 表示文件已就位可跳过下载
fn dest_already_complete(dest: &Path, expected_size: u64) -> bool {
    dest.is_file() && std::fs::metadata(dest).is_ok_and(|m| m.len() == expected_size)
}

/// no_redirect 客户端（安全/危险证书双变体，进程内各自共享）。
/// M28 碴3：OnceLock 消除每次 HEAD 重建（原无任何超时，补 connect_timeout）。
/// M29 碴5：insecure=true 变体跳过证书校验（R4-A 裁决的 HEAD 环节补齐）。
///
/// - 参数 insecure：是否跳过 TLS 证书校验
/// - 返回：对应变体的共享客户端
fn no_redirect_client(insecure: bool) -> reqwest::Client {
    static SAFE: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    static UNSAFE: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let cell = if insecure { &UNSAFE } else { &SAFE };
    cell.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .danger_accept_invalid_certs(insecure)
            .build()
            .expect("构建 no_redirect 客户端失败")
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文件名 → quant 提取
    #[test]
    fn quant_extraction() {
        assert_eq!(
            HuggingFaceSource::quant_of("Qwen3-0.6B-Q4_K_M.gguf"),
            "Q4_K_M"
        );
        assert_eq!(HuggingFaceSource::quant_of("model.gguf"), "default");
    }

    /// tag 选择：精确子串、缺省 Q4_K_M 优先、兜底第一个
    #[test]
    fn pick_file_strategy() {
        let mk = |name: &str| HfTreeFile {
            path: format!("GGUF/{name}"),
            size: 1,
        };
        let files = vec![mk("m-Q2_K.gguf"), mk("m-Q4_K_M.gguf"), mk("m-Q8_0.gguf")];
        assert!(HuggingFaceSource::pick_file(&files, Some("Q8"))
            .unwrap()
            .path
            .contains("Q8_0"));
        assert!(HuggingFaceSource::pick_file(&files, None)
            .unwrap()
            .path
            .contains("Q4_K_M"));
        let only = vec![mk("m-Q2_K.gguf")];
        assert!(HuggingFaceSource::pick_file(&only, None)
            .unwrap()
            .path
            .contains("Q2_K"));
        assert!(HuggingFaceSource::pick_file(&files, Some("nope")).is_none());
    }

    /// M28 碴5：量化词边界——IQ 前缀变体排在首位时不得被 "Q4_K_M" 误选
    ///（子串匹配时代的缺陷行为：contains("q4_k_m") 命中 "iq4_k_m"）
    #[test]
    fn pick_file_rejects_i_prefix_mismatch() {
        let mk = |name: &str| HfTreeFile {
            path: format!("GGUF/{name}"),
            size: 1,
        };
        let files = vec![mk("m-IQ4_K_M.gguf"), mk("m-Q4_K_M.gguf")];
        assert_eq!(
            HuggingFaceSource::pick_file(&files, Some("Q4_K_M"))
                .unwrap()
                .path,
            "GGUF/m-Q4_K_M.gguf",
            "tag Q4_K_M 必须跳过 IQ4_K_M"
        );
        assert_eq!(
            HuggingFaceSource::pick_file(&files, Some("IQ4_K_M"))
                .unwrap()
                .path,
            "GGUF/m-IQ4_K_M.gguf",
            "IQ 前缀量化可被显式 tag 精确选中"
        );
        assert_eq!(
            HuggingFaceSource::pick_file(&files, None).unwrap().path,
            "GGUF/m-Q4_K_M.gguf",
            "缺省 Q4_K_M 优先同样不得误中 IQ 前缀"
        );
    }

    /// 迭代18 BUG-14：无匹配 GGUF 报错文案——无 Rust Debug 泄漏、量化名
    /// 人类可读（原形态 `无匹配 GGUF（tag=Some("Q4_K_M")）`）
    #[test]
    fn no_gguf_message_readable_no_debug_leak() {
        let with_tag = HuggingFaceSource::no_gguf_message("u/m", Some("Q4_K_M"));
        assert!(with_tag.contains("Q4_K_M"), "量化名必须可读：{with_tag}");
        assert!(
            !with_tag.contains("Some(") && !with_tag.contains("\\\""),
            "不得泄漏 Debug 格式：{with_tag}"
        );
        let default_tag = HuggingFaceSource::no_gguf_message("u/m", None);
        assert!(
            default_tag.contains("缺省首选"),
            "缺省形态必须说明默认策略：{default_tag}"
        );
    }

    /// M31：HF 注册名解析与三级落位路径（迭代11 F1/F3：废弃 -- 转义）
    #[test]
    fn hf_ref_parse_and_dir_in() {
        let r = ModelRef::parse("hf.co/unsloth/Qwen3-0.6B:Q4_K_M").unwrap();
        assert_eq!(r.full_name(), "hf.co/unsloth/Qwen3-0.6B:Q4_K_M");
        let root = std::path::Path::new("/tmp/roxid-test-root");
        assert_eq!(
            r.dir_in(root, crate::repo::ModelSource::Hf),
            root.join("HF/unsloth/Qwen3-0.6B/Q4_K_M"),
            "HF 名必须落位三级自然路径"
        );
        // 非法形态拒绝：空段、普通名带斜杠
        assert!(ModelRef::parse("hf.co//repo:Q4").is_err());
        assert!(ModelRef::parse("a/b:Q4").is_err(), "普通名斜杠仍拒绝");
    }

    /// 镜像优先级：ROXID_HF_PROXY 优先、回退 HF_ENDPOINT、缺省官方、尾斜杠规整
    #[test]
    fn mirror_base_priority() {
        assert_eq!(
            resolve_base(
                Some("https://hf-mirror.com/".into()),
                Some("https://other".into())
            ),
            "https://hf-mirror.com"
        );
        assert_eq!(
            resolve_base(None, Some("https://other/".into())),
            "https://other"
        );
        assert_eq!(resolve_base(None, None), HF_BASE);
    }

    /// hf_base() 读取链：env 两级均未设置时回退 config.toml [proxy].hf；
    /// 任一 env 设置时优先于文件；全无时官方直连（迭代4 Q2 裁决）。
    /// ROXID_HOME 隔离排除开发机真实 config.toml 干扰。
    #[test]
    fn hf_base_falls_back_to_config_file() {
        use crate::config::{
            save_persist_config, PersistConfig, ProxySection, ROXID_HOME_TEST_LOCK,
        };
        let _guard = ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-hf-fb-{}", std::process::id()));
        std::env::set_var("ROXID_HOME", &dir);
        std::env::remove_var(ENV_HF_PROXY);
        std::env::remove_var(ENV_HF_ENDPOINT);
        save_persist_config(&PersistConfig {
            setup_done: true,
            proxy: ProxySection {
                gh: None,
                hf: Some("https://file-hf.mirror/".into()),
            },
            runtime: Default::default(),
        })
        .expect("写盘必须成功");
        assert_eq!(
            hf_base(),
            "https://file-hf.mirror",
            "文件回退必须生效且规整尾斜杠"
        );
        std::env::set_var(ENV_HF_ENDPOINT, "https://endpoint-hf.mirror/");
        assert_eq!(
            hf_base(),
            "https://endpoint-hf.mirror",
            "HF_ENDPOINT 必须优先于文件"
        );
        std::env::set_var(ENV_HF_PROXY, "https://proxy-hf.mirror/");
        assert_eq!(
            hf_base(),
            "https://proxy-hf.mirror",
            "ROXID_HF_PROXY 必须最优先"
        );
        std::env::remove_var(ENV_HF_PROXY);
        std::env::remove_var(ENV_HF_ENDPOINT);
        assert_eq!(
            hf_base(),
            "https://file-hf.mirror",
            "env 清除后文件回退恢复"
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// ETag → sha256 形态判断：64hex 合法（含引号与前缀形态）、md5/乱串拒绝
    #[test]
    fn etag_form_validation() {
        let sha64 = "6eb923e7d26e9cea28811e1a8e852009b21242fb157b26149d3b188f3a8c8653";
        assert_eq!(
            etag_to_sha256(&format!("\"{sha64}\"")).as_deref(),
            Some(sha64)
        );
        assert_eq!(
            etag_to_sha256(&format!("\"sha256:{sha64}\"")).as_deref(),
            Some(sha64)
        );
        assert_eq!(
            etag_to_sha256("\"d41d8cd98f00b204e9800998ecf8427e\""),
            None,
            "md5 必须拒绝"
        );
        assert_eq!(etag_to_sha256("W/\"weak-tag\""), None, "弱 ETag 必须拒绝");
        assert_eq!(etag_to_sha256(""), None);
    }

    /// 真实镜像 HEAD 摘要（ignored：依赖网络与镜像可达性，手动验收）。
    /// 直链 hf-mirror 302 层 x-linked-etag 应为合法 64hex sha256。
    #[tokio::test]
    #[ignore = "真实网络验收：hf-mirror HEAD x-linked-etag"]
    async fn real_mirror_etag() {
        let url = "https://hf-mirror.com/bartowski/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/Qwen2.5-0.5B-Instruct-Q4_K_M.gguf";
        let sha = HuggingFaceSource::new().resolve_etag_sha256(url).await;
        assert!(sha.is_some(), "镜像 302 层必须提供 LFS sha256");
        assert_eq!(sha.unwrap().len(), 64);
    }

    /// 镜像全链路真实拉取（ignored：网络 + ~313MB 下载，手动验收）。
    /// 覆盖：hf_base 镜像替换 → tree API（根目录布局回退）→ HEAD ETag →
    /// 分块下载 → sha256 校验（ETag 与内容一致才可通过）→ GGUF header
    /// 元数据 → model.json。
    #[tokio::test]
    #[ignore = "真实网络验收：ROXID_HF_PROXY=hf-mirror 全链路 Qwen2.5-0.5B IQ2_M"]
    async fn real_hf_pull_via_mirror() {
        std::env::set_var("ROXID_HF_PROXY", "https://hf-mirror.com/");
        let root = std::env::temp_dir().join("roxid-m16-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // M28 碴12：进度事件计数（原缺陷行为全程静默，仅首尾两条状态事件）
        let mut events = 0usize;
        let r = HuggingFaceSource::new()
            .pull(
                "bartowski/Qwen2.5-0.5B-Instruct-GGUF",
                Some("IQ2_M"),
                &root,
                |_| events += 1,
            )
            .await
            .expect("镜像全链路拉取必须成功（含 sha256 校验）");
        assert!(
            events > 5,
            "M28 碴12：HF 拉取进度事件必须多次回调（实际 {events}）"
        );
        // M31：三级路径落位 + hf.co 注册名断言（迭代11 F1/F3）
        let dir = r.dir(&root);
        assert_eq!(
            dir,
            root.join("HF/bartowski/Qwen2.5-0.5B-Instruct-GGUF/IQ2_M"),
            "HF 产物必须落位三级自然路径"
        );
        assert!(dir.join("model.gguf").is_file(), "GGUF 必须落位");
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("model.json")).unwrap())
                .unwrap();
        assert_eq!(
            meta["name"], "hf.co/bartowski/Qwen2.5-0.5B-Instruct-GGUF:IQ2_M",
            "注册名必须为 hf.co 形态（迭代11 Q3）"
        );
        eprintln!(
            "family={} parameter_size={} quantization_level={}",
            meta["family"], meta["parameter_size"], meta["quantization_level"]
        );
        assert!(
            !meta["family"].as_str().unwrap_or("").is_empty(),
            "GGUF header 解析的 family 必须非空"
        );
        assert_eq!(meta["quantization_level"], "IQ2_M", "header file_type 映射");
        assert_eq!(meta["parameter_size"], "0.5B", "header size_label");
        std::env::remove_var("ROXID_HF_PROXY");
    }

    /// M30 碴3：dest 门互斥——同 dest 同门（并发串行）、不同 dest 互不阻塞
    #[tokio::test]
    async fn dest_gates_same_path_serialized() {
        let g1 = dest_gate(Path::new("/x/y/model.gguf"));
        let g2 = dest_gate(Path::new("/x/y/model.gguf"));
        let l1 = g1.lock().await;
        let waiter = tokio::spawn(async move {
            let _ = g2.lock().await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(
            !waiter.is_finished(),
            "同 dest 门必须互斥（等锁者不得进入）"
        );
        drop(l1);
        waiter.await.unwrap();
        // 不同 dest 各持各门：立即获得不阻塞
        let g3 = dest_gate(Path::new("/x/z/model.gguf"));
        let _l3 = g3.lock().await;
    }

    /// M30 碴3：就位快路径判定——存在且大小一致才跳过（半文件/缺失走重下）
    #[test]
    fn dest_already_complete_judgement() {
        let dir = std::env::temp_dir().join(format!("roxid-hf-fastpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("model.gguf");
        std::fs::write(&dest, b"12345").unwrap();
        assert!(dest_already_complete(&dest, 5), "存在且大小一致 → 跳过下载");
        assert!(!dest_already_complete(&dest, 6), "大小不符 → 重下自愈");
        assert!(
            !dest_already_complete(&dir.join("absent.gguf"), 5),
            "不存在 → 重下"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
