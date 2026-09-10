//! Registry 客户端：Ollama registry v2 协议（主源）与 pull 编排。
//!
//! 协议实测（2026-08-24 19:12，registry.ollama.ai 直连可达）：
//! - manifest 匿名可读（无需 token）
//! - blob 307 重定向至 R2 预签名 URL（HTTP 客户端自动跟随即可）
//! - 层 mediaType 语义见 manifest::media_types
//!
//! 落位遵循 repo 模块的三目录分离布局（Q8/Q9 裁决 + 迭代11 F1）：
//! 主源产物落 models/ollama/{model}/{tag}/。
//!
//! 修改历史：占位 2026-08-24 18:35；M5 实装 2026-08-24 19:15；
//! M20 层摘要索引 2026-08-24 22:44；M22 license 收集 2026-08-24 23:22；
//! M25 移除未使用 import self（迭代5 M12 遗留警告清偿）2026-08-26 05:48
//! M32 碴3/碴4/碴9a（迭代12）：ignored e2e 断言对齐 M31 新布局
//! （ollama/ 前缀，原旧两级路径使挂账手动验收必炸）、头注释与 pull_model
//! 文档同步三目录语义、清除 download_big_blob 死代码 2026-09-07 01-15
//! M33 碴5（迭代13）：license 层拉取失败 warn 留痕（原 if let Ok 静默
//! 吞错，/api/show license 静默缺失无从排查）2026-09-07 01-58
//! M53（迭代19 碴A）：重复 pull 大层 digest 级跳过——既有 model.json
//! 同层摘要与本次 manifest 一致且文件在位时不再全量重下（.parts 完成即
//! 清，重复 pull 无断点可续；HF 侧为大小级快路径，主源用更强的 digest
//! 级，对齐官方「已存在层跳过」语义）2026-09-09 20-20

pub mod downloader;
pub mod gguf;
pub mod hf;
pub mod manifest;

pub use downloader::ChunkedDownloader;
pub use downloader::DownloadPhase;
pub use hf::HuggingFaceSource;
pub use manifest::{ImageConfig, LayerDescriptor, Manifest};

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{RoxidError, RoxidResult};
use crate::repo::{ChatMessage, ModelFiles, ModelMeta, ModelRef};

/// Ollama registry 基址
pub const REGISTRY_BASE: &str = "https://registry.ollama.ai";

/// manifest 请求的 Accept 头（v2 schema）
const MANIFEST_ACCEPT: &str = "application/vnd.docker.distribution.manifest.v2+json";

/// /api/pull 的 NDJSON 进度事件（字段与原版 Ollama 对齐）
#[derive(Debug, Clone, Serialize)]
pub struct PullEvent {
    /// 阶段描述：pulling manifest / verifying sha256 digest / writing manifest / success
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// 当前层摘要
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// 层总字节
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// 已完成字节
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
    /// 错误信息（发生即终止）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PullEvent {
    /// 纯状态事件（无进度数字）
    pub fn status(text: impl Into<String>) -> Self {
        Self {
            status: Some(text.into()),
            digest: None,
            total: None,
            completed: None,
            error: None,
        }
    }

    /// 错误事件
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            status: None,
            digest: None,
            total: None,
            completed: None,
            error: Some(text.into()),
        }
    }
}

/// 大层 digest 级跳过判定（M53 碴A）：既有元数据同层摘要与本次
/// manifest 一致且目标文件仍在位 → 已拉取过可跳过下载。远端 tag
/// 更新（digest 变化）或文件被手删时自然失效走重下，无误跳过面。
///
/// - 参数 existing：既有 model.json 元数据（None 恒 false）
/// - 参数 layer_key：层摘要索引键（model / projection / adapter-{i}）
/// - 参数 digest：本次 manifest 层摘要
/// - 参数 dest：层目标文件路径
/// - 返回：true 表示可跳过下载
fn big_layer_already_pulled(
    existing: &Option<ModelMeta>,
    layer_key: &str,
    digest: &str,
    dest: &Path,
) -> bool {
    existing
        .as_ref()
        .and_then(|m| m.layer_digests.get(layer_key))
        .is_some_and(|d| d == digest)
        && dest.is_file()
}

/// Ollama registry 客户端
pub struct OllamaRegistry {
    http: reqwest::Client,
    downloader: ChunkedDownloader,
}

impl Default for OllamaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaRegistry {
    /// 构造客户端（默认安全校验路径）。
    pub fn new() -> Self {
        Self::with_insecure(false)
    }

    /// 构造客户端（默认 8MB 分块 × 4 并发）。
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
                .expect("构建 registry 客户端失败"),
            downloader: ChunkedDownloader::new(
                downloader::DEFAULT_CHUNK_SIZE,
                downloader::DEFAULT_CONCURRENCY,
                insecure,
            ),
        }
    }

    /// 仓库名归一化：不含命名空间时补 library/（与原版一致）
    ///
    /// - 参数 name：用户输入模型名
    /// - 返回：完整仓库路径（如 library/smollm 或 user/repo）
    pub fn repo_path(name: &str) -> String {
        if name.contains('/') {
            name.to_string()
        } else {
            format!("library/{name}")
        }
    }

    /// blob 下载 URL
    fn blob_url(repo: &str, digest: &str) -> String {
        format!("{REGISTRY_BASE}/v2/{repo}/blobs/{digest}")
    }

    /// 拉取 manifest（匿名）。
    ///
    /// - 参数 model：模型名（不含 tag）
    /// - 参数 tag：变体 tag
    /// - 返回：manifest 结构；404 时 ModelNotFound
    pub async fn fetch_manifest(&self, model: &str, tag: &str) -> RoxidResult<Manifest> {
        let repo = Self::repo_path(model);
        let url = format!("{REGISTRY_BASE}/v2/{repo}/manifests/{tag}");
        let resp = self
            .http
            .get(&url)
            .header("Accept", MANIFEST_ACCEPT)
            .send()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
        if resp.status().as_u16() == 404 {
            return Err(RoxidError::ModelNotFound(format!("{model}:{tag}")));
        }
        if !resp.status().is_success() {
            return Err(RoxidError::RegistryRequest(format!(
                "HTTP {}：{url}",
                resp.status()
            )));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
        serde_json::from_str(&text)
            .map_err(|e| RoxidError::RegistryRequest(format!("manifest 解析失败：{e}")))
    }

    /// 整取小 blob（config / params / template / system / messages 等文本层），
    /// 带 sha256 校验。
    ///
    /// - 参数 repo：完整仓库路径
    /// - 参数 layer：层描述
    /// - 返回：层原始字节
    async fn fetch_small_blob(&self, repo: &str, layer: &LayerDescriptor) -> RoxidResult<Vec<u8>> {
        let url = Self::blob_url(repo, &layer.digest);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(RoxidError::RegistryRequest(format!(
                "HTTP {}：{url}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RoxidError::RegistryRequest(format!("{url}: {e}")))?;
        let expected = layer
            .digest
            .strip_prefix("sha256:")
            .unwrap_or(&layer.digest);
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != expected {
            return Err(RoxidError::RegistryRequest(format!(
                "小层 sha256 不符：{expected} != {actual}"
            )));
        }
        Ok(bytes.to_vec())
    }

    /// 拉取模型并落位 {models_root}/ollama/{model}/{tag}/（迭代11 F1 主源落位）。
    ///
    /// - 参数 name：用户输入模型名（可不含 tag，缺省 latest）
    /// - 参数 models_root：模型仓库根目录
    /// - 参数 on_event：进度事件回调（NDJSON 事件流来源）
    /// - 返回：落位后的模型引用
    pub async fn pull_model(
        &self,
        name: &str,
        models_root: &Path,
        mut on_event: impl FnMut(PullEvent) + Send,
    ) -> RoxidResult<ModelRef> {
        let r = ModelRef::parse(name)?;
        on_event(PullEvent::status("pulling manifest"));
        let manifest = self.fetch_manifest(&r.model, &r.tag).await?;
        let repo = Self::repo_path(&r.model);

        // config 层：family 等元数据
        let config: ImageConfig =
            match serde_json::from_slice(&self.fetch_small_blob(&repo, &manifest.config).await?) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("config 层解析失败（忽略）：{e}");
                    ImageConfig {
                        model_family: String::new(),
                        model_families: None,
                        model_type: String::new(),
                        file_type: String::new(),
                        rootfs: None,
                    }
                }
            };

        // 分类处理各层（迭代11 F1：主源产物落 models/ollama/）
        let dir = r.dir_in(models_root, crate::repo::ModelSource::Ollama);
        std::fs::create_dir_all(&dir)?;
        // M53：重复 pull 快路径输入——既有元数据（首拉无 model.json 时
        // load_meta 报错归 None，视为全新拉取走原下载路径）
        let existing_meta = crate::repo::model_json::load_meta(&dir).ok();
        let mut template: Option<String> = None;
        let mut system = String::new();
        let mut parameters: std::collections::BTreeMap<String, serde_json::Value> =
            Default::default();
        let mut messages: Vec<ChatMessage> = Vec::new();
        let mut adapters: Vec<String> = Vec::new();
        let mut model_file: Option<String> = None;
        let mut mmproj_file: Option<String> = None;
        // 许可证文本（M22：/api/show 返回）
        let mut license = String::new();
        // 各层摘要索引（M20 /api/blobs 定位用：role → digest）
        let mut layer_digests: std::collections::BTreeMap<String, String> = Default::default();
        layer_digests.insert("config".into(), manifest.config.digest.clone());

        for (i, layer) in manifest.layers.iter().enumerate() {
            use manifest::media_types as mt;
            match layer.mediaType.as_str() {
                mt::MODEL => {
                    let file = "model.gguf";
                    let dest = dir.join(file);
                    layer_digests.insert("model".into(), layer.digest.clone());
                    on_event(PullEvent {
                        status: Some(format!("pulling {file}")),
                        digest: Some(layer.digest.clone()),
                        total: Some(layer.size),
                        completed: Some(0),
                        error: None,
                    });
                    // M53：digest 级跳过——已就位同层补发完成事件收尾进度条，
                    // 不进入下载（碴A：原无脑重下数 GB）
                    if big_layer_already_pulled(&existing_meta, "model", &layer.digest, &dest) {
                        tracing::info!("层已就位（digest 一致跳过下载）：{file} {}", layer.digest);
                        on_event(PullEvent {
                            status: Some(format!("pulling {file}")),
                            digest: Some(layer.digest.clone()),
                            total: Some(layer.size),
                            completed: Some(layer.size),
                            error: None,
                        });
                    } else {
                        self.download_big_blob(&repo, layer, &dest, &mut on_event)
                            .await?;
                    }
                    model_file = Some(file.to_string());
                }
                mt::PROJECTION => {
                    let file = "mmproj.gguf";
                    let dest = dir.join(file);
                    layer_digests.insert("projection".into(), layer.digest.clone());
                    on_event(PullEvent {
                        status: Some(format!("pulling {file}")),
                        digest: Some(layer.digest.clone()),
                        total: Some(layer.size),
                        completed: Some(0),
                        error: None,
                    });
                    // M53：digest 级跳过（同 MODEL 分支语义）
                    if big_layer_already_pulled(&existing_meta, "projection", &layer.digest, &dest)
                    {
                        tracing::info!("层已就位（digest 一致跳过下载）：{file} {}", layer.digest);
                        on_event(PullEvent {
                            status: Some(format!("pulling {file}")),
                            digest: Some(layer.digest.clone()),
                            total: Some(layer.size),
                            completed: Some(layer.size),
                            error: None,
                        });
                    } else {
                        self.download_big_blob(&repo, layer, &dest, &mut on_event)
                            .await?;
                    }
                    mmproj_file = Some(file.to_string());
                }
                mt::ADAPTER => {
                    let file = format!("lora-{i}.gguf");
                    let layer_key = format!("adapter-{i}");
                    layer_digests.insert(layer_key.clone(), layer.digest.clone());
                    // M53：digest 级跳过（原分支无前置事件，跳过时同样零事件）
                    if !big_layer_already_pulled(
                        &existing_meta,
                        &layer_key,
                        &layer.digest,
                        &dir.join(&file),
                    ) {
                        self.download_big_blob(&repo, layer, &dir.join(&file), &mut on_event)
                            .await?;
                    }
                    adapters.push(file);
                }
                mt::TEMPLATE => {
                    layer_digests.insert("template".into(), layer.digest.clone());
                    let bytes = self.fetch_small_blob(&repo, layer).await?;
                    template = Some(String::from_utf8_lossy(&bytes).to_string());
                }
                mt::SYSTEM => {
                    layer_digests.insert("system".into(), layer.digest.clone());
                    let bytes = self.fetch_small_blob(&repo, layer).await?;
                    system = String::from_utf8_lossy(&bytes).to_string();
                }
                mt::PARAMS => {
                    layer_digests.insert("params".into(), layer.digest.clone());
                    let bytes = self.fetch_small_blob(&repo, layer).await?;
                    if let Ok(map) =
                        serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
                    {
                        for (k, v) in map {
                            parameters.insert(k, v);
                        }
                    }
                }
                mt::MESSAGES => {
                    layer_digests.insert("messages".into(), layer.digest.clone());
                    let bytes = self.fetch_small_blob(&repo, layer).await?;
                    if let Ok(list) = serde_json::from_slice::<Vec<ChatMessage>>(&bytes) {
                        messages = list;
                    }
                }
                mt::LICENSE => {
                    // M22：许可证文本入元数据（不落独立文件，维持直观布局）
                    // M33 碴5：拉取失败 warn 留痕（原 if let Ok 静默吞错，
                    // license 空串落盘无从排查；对齐同函数 config 层与 M30
                    // 碴4 全路径留痕原则）——摘要键照写、不阻断 pull
                    layer_digests.insert("license".into(), layer.digest.clone());
                    match self.fetch_small_blob(&repo, layer).await {
                        Ok(bytes) => license = String::from_utf8_lossy(&bytes).to_string(),
                        Err(e) => tracing::warn!(
                            "license 层拉取失败（元数据降级空串，不阻断）：{}：{e}",
                            layer.digest
                        ),
                    }
                }
                // 其余文本层不落盘（与直观布局目标一致：只留可运行文件）
                _ => tracing::debug!("跳过层 {}", layer.mediaType),
            }
        }

        let model_file = model_file.ok_or_else(|| {
            RoxidError::RegistryRequest(format!("manifest 无 model 层：{}", r.full_name()))
        })?;

        // M110（迭代33 碴3）：verifying 事件已前移至每层真实校验时刻
        //（download_big_blob 内 DownloadPhase::Verifying——大 GGUF 的
        // sha256 计算是 100% 后主要耗时，原尾部补发使 CLI 空转无阶段
        // 区分）；此处仅保留写入清单事件
        on_event(PullEvent::status("writing manifest"));
        let meta = ModelMeta {
            runtime: None,
            name: r.full_name(),
            family: config.model_family,
            families: config.model_families.clone().unwrap_or_default(),
            parameter_size: config.model_type,
            quantization_level: config.file_type,
            system,
            template,
            parameters,
            messages,
            adapters,
            files: ModelFiles {
                model: model_file,
                mmproj: mmproj_file,
            },
            digest: manifest.config.digest.clone(),
            layer_digests,
            license,
            source: "ollama-registry".into(),
            created_at: now_rfc3339(),
        };
        crate::repo::model_json::save_meta(&dir, &meta)?;
        on_event(PullEvent::status("success"));
        Ok(r)
    }

    /// 大 blob 分块下载（8MB×4 并发 + 断点续传 + sha256 + 进度事件）
    async fn download_big_blob(
        &self,
        repo: &str,
        layer: &LayerDescriptor,
        dest: &Path,
        on_event: &mut (impl FnMut(PullEvent) + Send),
    ) -> RoxidResult<()> {
        let url = Self::blob_url(repo, &layer.digest);
        let expected = layer
            .digest
            .strip_prefix("sha256:")
            .unwrap_or(&layer.digest)
            .to_string();
        let digest = layer.digest.clone();
        // M32 碴9a：事件 total 取 downloader 探测的真实资源大小（tot），
        // 移除未使用的 layer.size 死代码绑定
        // M110（迭代33 碴3+碴12）：回调改阶段枚举——Progress 转进度事件、
        // Verifying/Retrying 转状态事件（官方英文文案，CLI 渲染层中文映射）
        self.downloader
            .download(&url, dest, Some(&expected), move |ph| match ph {
                DownloadPhase::Progress(done, tot) => on_event(PullEvent {
                    status: Some("pulling".into()),
                    digest: Some(digest.clone()),
                    total: Some(tot),
                    completed: Some(done),
                    error: None,
                }),
                DownloadPhase::Verifying => on_event(PullEvent::status("verifying sha256 digest")),
                DownloadPhase::Retrying(n, max) => on_event(PullEvent::status(format!(
                    "retrying download (attempt {n}/{max})"
                ))),
            })
            .await
    }
}

/// 当前时刻 RFC3339（UTC，秒级）；registry 与 hf/scheduler 模块共用
pub(crate) fn now_rfc3339() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now_rfc3339_with(d)
}

/// 指定 epoch 时长后的 RFC3339（UTC，秒级）；keep_alive 到期时刻换算用
pub(crate) fn now_rfc3339_with(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    // 简易格式化：以 1970-01-01 为基线的 Civil 日期换算（避免引入 chrono）
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let rem = secs % 86400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// 天数 → 公历日期（Howard Hinnant 算法）
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库名归一化：裸名补 library/，含命名空间原样
    #[test]
    fn repo_path_normalization() {
        assert_eq!(OllamaRegistry::repo_path("smollm"), "library/smollm");
        assert_eq!(OllamaRegistry::repo_path("firmwire/nous"), "firmwire/nous");
    }

    /// RFC3339 时间：2026-08-24 已知时刻换算正确性
    /// （epoch-days 权威值 20689，来源：date -d '2026-08-24 UTC' +%s 换算）
    #[test]
    fn rfc3339_formatting() {
        assert_eq!(civil_from_days(20689), (2026, 8, 24));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(
            civil_from_days(10957),
            (2000, 1, 1),
            "python3 timedelta 权威核验"
        );
        let s = now_rfc3339();
        assert!(
            s.ends_with('Z') && s.len() == 20,
            "格式必须为 YYYY-MM-DDTHH:MM:SSZ：{s}"
        );
    }

    /// M28 碴16：insecure 变体构造冒烟（危险证书客户端构建成功；
    /// 网络行为等价性由 reqwest builder 选项语义保证）
    #[test]
    fn insecure_variants_construct() {
        let _r = OllamaRegistry::with_insecure(true);
        let _h = crate::registry::HuggingFaceSource::with_insecure(true);
        let _d = downloader::ChunkedDownloader::new(8192, 2, true);
    }

    /// 真实拉取验收（registry 直连可达，实测 19:12）：
    /// ROXID_HOME=/tmp/roxid-pull-e2e cargo test -p roxid-server registry -- --ignored --nocapture
    /// M32 碴3：断言对齐 M31 三目录布局（主源产物落 models/ollama/）
    #[tokio::test]
    #[ignore = "真实网络拉取 smollm:135m（约 91MB），验收时手动执行"]
    async fn pull_smollm_end_to_end() {
        let root = std::path::PathBuf::from("/tmp/roxid-pull-e2e/models");
        let _ = std::fs::remove_dir_all("/tmp/roxid-pull-e2e");
        let reg = OllamaRegistry::new();
        let mut events = 0usize;
        let r = reg
            .pull_model("smollm:135m", &root, |_| events += 1)
            .await
            .unwrap();
        assert_eq!(r.full_name(), "smollm:135m");
        let gguf = root.join("ollama/smollm/135m/model.gguf");
        assert!(
            gguf.is_file(),
            "GGUF 必须落位 ollama 源：{}",
            gguf.display()
        );
        assert_eq!(
            std::fs::metadata(&gguf).unwrap().len(),
            91727296,
            "大小必须与 manifest 一致"
        );
        let meta = crate::repo::model_json::load_meta(&root.join("ollama/smollm/135m")).unwrap();
        assert_eq!(meta.source, "ollama-registry");
        assert!(events > 5, "进度事件必须多次回调（实际 {events}）");
        assert!(meta.template.is_some(), "template 层必须落入元数据");
        assert!(!meta.license.is_empty(), "M22：license 层必须保存");
        assert!(
            meta.layer_digests.contains_key("model"),
            "M20：层摘要索引必须写入"
        );
    }

    /// M53：重复 pull 大层 digest 级跳过判定——digest 一致且文件在位才跳过；
    /// digest 变化 / 文件缺失 / 无既有元数据均走重下
    #[test]
    fn big_layer_already_pulled_judgement() {
        let dir = std::env::temp_dir().join("roxid-m53-judge");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"x").unwrap();

        let mut layer_digests = std::collections::BTreeMap::new();
        layer_digests.insert("model".to_string(), "sha256:aa".to_string());
        let existing = Some(ModelMeta {
            name: "m:t".into(),
            family: String::new(),
            families: vec![],
            parameter_size: String::new(),
            quantization_level: String::new(),
            system: String::new(),
            template: None,
            parameters: Default::default(),
            messages: vec![],
            layer_digests,
            adapters: vec![],
            files: ModelFiles {
                model: "model.gguf".into(),
                mmproj: None,
            },
            digest: String::new(),
            license: String::new(),
            runtime: None,
            source: "ollama-registry".into(),
            created_at: String::new(),
        });
        let gguf = dir.join("model.gguf");

        // ① digest 一致 + 文件在位 → 跳过
        assert!(big_layer_already_pulled(
            &existing,
            "model",
            "sha256:aa",
            &gguf
        ));
        // ② digest 变化（远端 tag 更新）→ 重下
        assert!(!big_layer_already_pulled(
            &existing,
            "model",
            "sha256:bb",
            &gguf
        ));
        // ③ 文件被手删 → 重下
        std::fs::remove_file(&gguf).unwrap();
        assert!(!big_layer_already_pulled(
            &existing,
            "model",
            "sha256:aa",
            &gguf
        ));
        // ④ 无既有元数据（首拉）→ 重下
        assert!(!big_layer_already_pulled(
            &None,
            "model",
            "sha256:aa",
            &gguf
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
