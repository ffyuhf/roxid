//! API 网关层：axum 路由总装与服务启动。
//!
//! 三套 API：
//! - ollama：/api/*（NDJSON 流式）
//! - openai：/v1/*（SSE 流式，M10）
//! - responses：/v1/responses（M11）
//! - llamacpp：llama.cpp 原生端点直通（/tokenize 等 8 端点，M18）
//!
//! CORS（M21 D4d）：默认放行 localhost 系来源（对齐原版 Ollama 默认），
//! ROXID_ORIGINS / OLLAMA_ORIGINS 环境变量覆盖（支持逗号分隔与 * 通配）。
//!
//! 修改历史：占位 2026-08-24 18:35；M7 实装 2026-08-24 19:31；
//! M18 llama.cpp 直通层 2026-08-24 22:17；M20 /api/blobs 2026-08-24 22:42；
//! M21 CORS 中间件 2026-08-24 22:58；
//! M25 移除未使用 import DEFAULT_BIND_ADDR（迭代5 M12 遗留警告清偿）2026-08-26 05-43；
//! M28 碴2 pull 去重（迭代8）：AppState 增进行中任务表 + PullGate 完成门
//!（R1-A 裁决：并发同模型 pull 后到请求等待终态并转发，根除重复下载
//! 与 .parts 目录互踩）2026-08-30 02-15
//! M32 碴2（迭代12）：PullGate::wait 改先创建 notified future 再查终态
//!（原「先查终态、后订阅」窗口内 finish 的 notify_waiters 无订阅者且不存
//! 许可，唤醒丢失使等待者永久挂起）2026-09-07 01-05
//! M34 两项（迭代14 BUG-2/BUG-5）：BUG-2 LenientJson 宽松提取器（不校验
//! Content-Type 直读 body 解析，对齐官方 Go/gin 直读 body 行为与官方文档
//! 裸 `curl -d` 示例；全部 JSON POST 端点替换 axum Json<T>，原无头/异头
//! POST 全部 415）；BUG-5 serve 启动错误新变体 ServeStartup（端口绑定失败
//! 不再误报 llama-server 分类）2026-09-07 19-20
//! M93 碴B（迭代29）：PullGate 增共享进度快照（completed/total 原子量，
//! 首任务发事件时顺带写入）+ 等待者 ticker 转发——原「后到请求等待终态
//! 并转发，无中途进度」使 CLI 中断后二次 run 复用后台任务时全程静默
//!（方案 A 用户裁决 2026-09-10 06:32）2026-09-10 06-40

pub mod llamacpp;
pub mod ollama;
pub mod openai;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::FromRequest;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};

use crate::config::{models_root, ROXID_VERSION};
use crate::error::RoxidResult;
use crate::registry::OllamaRegistry;
use crate::scheduler::RunnerRegistry;

/// CORS 来源环境变量：本项目命名惯例（优先）
pub const ENV_ORIGINS: &str = "ROXID_ORIGINS";
/// CORS 来源环境变量：原版 Ollama 惯例（回退）
pub const ENV_ORIGINS_FALLBACK: &str = "OLLAMA_ORIGINS";

/// 默认放行来源（对齐原版 Ollama 默认集：localhost 系 + 桌面壳协议）
pub const DEFAULT_ORIGINS: &[&str] = &[
    "http://localhost",
    "http://localhost:*",
    "https://localhost",
    "https://localhost:*",
    "http://127.0.0.1",
    "http://127.0.0.1:*",
    "https://127.0.0.1",
    "https://127.0.0.1:*",
    "app://*",
    "file://*",
    "tauri://*",
];

/// 解析 CORS 来源集：变量优先（逗号分隔），缺省用默认 localhost 系。
///
/// - 参数 roxid / ollama：两变量取值（纯函数便于单测）
/// - 返回：来源模式列表（支持 * 通配）
pub fn resolve_origins(roxid: Option<String>, ollama: Option<String>) -> Vec<String> {
    match roxid.or(ollama) {
        Some(s) => s
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
        None => DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect(),
    }
}

/// 简易通配匹配：pattern 按 * 分段，依次在 origin 中定位（顺序匹配，支持多 *）。
///
/// - 参数 pattern：含 * 的模式（* 匹配任意串，含空）
/// - 参数 origin：实际 Origin 头值
/// - 返回：true 表示匹配
pub fn wildcard_match(pattern: &str, origin: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == origin;
    }
    // 标准 glob：首段前缀锚定、末段后缀锚定、中间段按序定位
    let mut parts: Vec<&str> = pattern.split('*').collect();
    let mut s = origin;
    let first = parts.remove(0);
    if !s.starts_with(first) {
        return false;
    }
    s = &s[first.len()..];
    if let Some(last) = parts.pop() {
        if !s.ends_with(last) {
            return false;
        }
        s = &s[..s.len() - last.len()];
    }
    for mid in parts {
        if mid.is_empty() {
            continue;
        }
        match s.find(mid) {
            Some(pos) => s = &s[pos + mid.len()..],
            None => return false,
        }
    }
    true
}

/// 构造 CORS 中间件：放行来源按模式匹配（精确或 * 通配），
/// 方法/头放开（本地推理网关语义，与原版一致）。
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{Any, CorsLayer};
    let patterns = resolve_origins(
        std::env::var(ENV_ORIGINS).ok().filter(|s| !s.is_empty()),
        std::env::var(ENV_ORIGINS_FALLBACK)
            .ok()
            .filter(|s| !s.is_empty()),
    );
    CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(
            move |origin: &HeaderValue, _| {
                let o = origin.to_str().unwrap_or_default();
                patterns
                    .iter()
                    .any(|p| p == "*" || p == o || wildcard_match(p, o))
            },
        ))
        .allow_methods(Any)
        .allow_headers(Any)
}

/// 全局应用状态（全部 handler 共享）
pub struct AppState {
    /// 推理调度注册表
    pub scheduler: Arc<RunnerRegistry>,
    /// 模型仓库根目录
    pub models_root: PathBuf,
    /// Ollama registry 客户端（pull）
    pub registry: Arc<OllamaRegistry>,
    /// /api/pull 进行中任务表（模型名 → 完成门；M28 碴2）：
    /// 首个请求注册，后到请求等待终态并转发——同模型并发 pull 去重
    pub pulls: tokio::sync::Mutex<HashMap<String, Arc<PullGate>>>,
}

/// pull 任务完成门（M28 碴2，R1-A 裁决 2026-08-30 02:08）：
/// 首个请求注册于 AppState.pulls；任务结束 finish 唤醒全部等待者并携带终态。
/// 等待语义：先查终态再注册通知（notify 先例模式），终态设置后新等待者立即返回。
pub struct PullGate {
    done: tokio::sync::Notify,
    /// None=进行中；Some(Ok(()))=拉取成功；Some(Err(错误描述))=拉取失败
    result: std::sync::Mutex<Option<Result<(), String>>>,
    /// M93 碴B：已完成字节快照（首任务写者唯一，等待者多并发读）
    progress_completed: std::sync::atomic::AtomicU64,
    /// M93 碴B：总量快照（0=尚无字节级进度，等待者据此跳过进度帧防 0/0 误显）
    progress_total: std::sync::atomic::AtomicU64,
}

impl PullGate {
    /// 构造进行中状态的门。
    pub fn new() -> Self {
        Self {
            done: tokio::sync::Notify::new(),
            result: std::sync::Mutex::new(None),
            progress_completed: std::sync::atomic::AtomicU64::new(0),
            progress_total: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// M93 碴B：写入进度快照（首任务每下发一条进度事件即调用一次）。
    ///
    /// - 参数 completed / total：已完成字节数与该层总量
    pub fn update_progress(&self, completed: u64, total: u64) {
        use std::sync::atomic::Ordering;
        self.progress_completed.store(completed, Ordering::Release);
        self.progress_total.store(total, Ordering::Release);
    }

    /// M93 碴B：读取进度快照（等待者 ticker 周期调用转发真实进度）。
    ///
    /// - 返回：(completed, total)；total 为 0 表示首任务尚无字节级进度
    pub fn snapshot_progress(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering;
        let total = self.progress_total.load(Ordering::Acquire);
        (self.progress_completed.load(Ordering::Acquire), total)
    }

    /// 等待任务终态（可多等待者并发等待）。
    ///
    /// - 返回：Ok(()) 拉取成功；Err(错误描述) 拉取失败
    pub async fn wait(&self) -> Result<(), String> {
        loop {
            // M32 碴2：先创建 notified future 再查终态——订阅先于检查，
            // 「查完 None、尚未订阅」的窗口内 finish 的唤醒不再丢失
            //（notify_waiters 不存许可，无订阅者的唤醒即丢弃；tokio 官方
            // 先例模式：enabled pin 订阅 → 检查状态 → await）
            let notified = self.done.notified();
            tokio::pin!(notified);
            {
                let guard = self.result.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(r) = guard.as_ref() {
                    return r.clone();
                }
            }
            notified.await;
        }
    }

    /// 登记终态并唤醒全部等待者（幂等：仅首次生效）。
    ///
    /// - 参数 r：任务终态
    pub fn finish(&self, r: Result<(), String>) {
        let mut guard = self.result.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_some() {
            return; // 已有终态（panic 兜底先于正常路径等极端时序）：保持首个
        }
        *guard = Some(r);
        drop(guard);
        self.done.notify_waiters();
    }
}

impl Default for PullGate {
    fn default() -> Self {
        Self::new()
    }
}

/// 宽松 JSON 提取器（M34 BUG-2）：不校验 Content-Type，直接读 body 解析。
/// 官方 Ollama（Go/gin 实现）直读 body 不校验头——官方文档示例即裸
/// `curl -d`（curl 默认 x-www-form-urlencoded 头）。原 axum `Json<T>`
/// 强校验头使无头/异头 POST 全部 415，官方教程/脚本生态全量不兼容。
/// 解析失败回 400（对齐官方对非法 JSON 的状态码）。
pub struct LenientJson<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequest<S> for LenientJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("读取请求体失败：{e}")})),
                )
                    .into_response()
            })?;
        serde_json::from_slice(&bytes)
            .map(LenientJson)
            .map_err(|e| {
                // 迭代18 BUG-9 整形（2026-09-09 05-58）：解析细节（行列号）
                // 落服务端日志供调试，客户端仅收通用文案——原「解析 JSON
                // 失败：expected value at line 1 column 78」向客户端暴露
                // 内部解析位置（BUG 清单 v1.2.0 L42）
                tracing::warn!("请求体 JSON 解析失败：{e}");
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "请求体不是合法 JSON"})),
                )
                    .into_response()
            })
    }
}

/// 构造完整路由（Ollama API + 通用端点；M10/M11 扩展 /v1/*）
pub fn build_router(state: Arc<AppState>) -> Router {
    let ollama_scope = Router::new()
        .route(
            "/version",
            get(|| async { axum::Json(serde_json::json!({"version": ROXID_VERSION})) }),
        )
        .route("/tags", get(ollama::tags))
        .route("/ps", get(ollama::ps))
        .route("/show", post(ollama::show))
        .route("/generate", post(ollama::generate))
        .route("/chat", post(ollama::chat))
        .route("/embed", post(ollama::embed))
        .route("/embeddings", post(ollama::embeddings_legacy))
        .route("/create", post(ollama::create))
        .route("/pull", post(ollama::pull))
        .route("/push", post(ollama::push))
        .route("/copy", post(ollama::copy))
        .route("/delete", delete(ollama::delete_model))
        .route("/stop", post(ollama::stop))
        // M20（Q1 裁决 2026-08-24 22:08）：按摘要取本地 blob，流式直通
        .route(
            "/blobs/:digest",
            get(ollama::blobs_get).head(ollama::blobs_head),
        );
    // OpenAI 兼容 + Responses API（透传 llama-server）
    let openai_scope = Router::new()
        .route("/models", get(openai::models))
        .route("/chat/completions", post(openai::chat_completions))
        .route("/completions", post(openai::completions))
        .route("/embeddings", post(openai::embeddings))
        .route("/rerank", post(openai::rerank))
        .route("/responses", post(openai::responses));
    // llama.cpp 原生端点直通（M18，Q4 裁决 2026-08-24 22:11：8 端点全部纳入）：
    // POST 类 body 携带 model 路由键；GET 类 ?model= 指定实例
    let llamacpp_routes = Router::new()
        .route("/tokenize", post(llamacpp::tokenize))
        .route("/detokenize", post(llamacpp::detokenize))
        .route("/infill", post(llamacpp::infill))
        .route("/completion", post(llamacpp::completion))
        .route("/embedding", post(llamacpp::embedding))
        .route("/props", get(llamacpp::props))
        .route("/slots", get(llamacpp::slots))
        .route("/metrics", get(llamacpp::metrics));
    Router::new()
        .nest("/api", ollama_scope)
        .nest("/v1", openai_scope)
        .merge(llamacpp_routes)
        .layer(cors_layer())
        .with_state(state)
}

/// 启动 API 服务器（阻塞直至退出信号）。
///
/// - 参数 bind：监听地址（默认 127.0.0.1:11434）
/// - 返回：进程退出
pub async fn serve_main(bind: SocketAddr) -> RoxidResult<()> {
    // 迭代11 F1：存量旧布局迁移（路由挂载前单次执行，fail-fast——
    // 半迁移状态中止启动，重试幂等恢复；用户确认 2026-09-06 22:33/22:38）
    crate::repo::migrate::migrate_legacy_layout(&models_root())?;
    let state = Arc::new(AppState {
        scheduler: Arc::new(RunnerRegistry::new(models_root())),
        models_root: models_root(),
        registry: Arc::new(OllamaRegistry::new()),
        pulls: tokio::sync::Mutex::new(HashMap::new()),
    });
    // keep_alive 到期自动卸载循环（5s 扫描）
    state
        .scheduler
        .spawn_reaper(std::time::Duration::from_secs(5));

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        // M34 BUG-5：端口绑定失败属服务进程自身错误，归 ServeStartup——
        // 原归 RunnerFailure 使 11434 被占时误报「llama-server process error」
        .map_err(|e| crate::error::RoxidError::ServeStartup(format!("监听 {bind} 失败：{e}")))?;
    tracing::info!("roxid 监听于 http://{bind}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| crate::error::RoxidError::ServeStartup(format!("服务器退出：{e}")))?;
    Ok(())
}

/// Ctrl-C / SIGTERM 优雅退出
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("安装 SIGTERM 处理失败")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M34 BUG-2：宽松提取器不校验 Content-Type（无头/异头 POST 均解析 body；
    /// 非法 JSON 400 拒绝）
    #[tokio::test]
    async fn lenient_json_ignores_content_type() {
        use axum::extract::FromRequest as _;
        // 无 Content-Type 头（官方文档裸 -d 场景）
        let req = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .body(axum::body::Body::from(r#"{"model":"x"}"#))
            .unwrap();
        let extracted = LenientJson::<serde_json::Value>::from_request(req, &()).await;
        assert!(extracted.is_ok(), "无头 POST 应解析成功");
        // 异头（x-www-form-urlencoded，curl -d 默认）同样解析
        let req2 = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from(r#"{"model":"y"}"#))
            .unwrap();
        let extracted2 = LenientJson::<serde_json::Value>::from_request(req2, &()).await;
        assert_eq!(extracted2.unwrap().0["model"], "y");
        // 非法 JSON：400 拒绝
        let req3 = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .body(axum::body::Body::from("not-json"))
            .unwrap();
        assert!(LenientJson::<serde_json::Value>::from_request(req3, &())
            .await
            .is_err());
    }

    /// M21 D4d：origins 解析（变量逗号分隔覆盖；缺省默认 localhost 系）
    #[test]
    fn cors_origins_resolution() {
        let d = resolve_origins(None, None);
        assert!(
            d.contains(&"http://localhost:*".to_string()),
            "默认集必须含 localhost 通配"
        );
        let v = resolve_origins(Some("https://a.com, https://b.com ".into()), None);
        assert_eq!(v, vec!["https://a.com", "https://b.com"]);
        assert_eq!(
            resolve_origins(None, Some("https://c.com".into())),
            vec!["https://c.com"],
            "回退 OLLAMA_ORIGINS"
        );
    }

    /// M32 碴2：订阅序竞态回归——多等待者与即刻 finish 高频交错，全部必须
    /// 在时限内携带正确终态返回（原实现「先查终态后订阅」存在丢唤醒窗口；
    /// 本测试为回归锚定，防止未来改动重新引入挂起）
    #[tokio::test]
    async fn pull_gate_no_lost_wakeup_under_concurrent_finish() {
        for _ in 0..50u32 {
            let gate = Arc::new(PullGate::new());
            let waiters: Vec<_> = (0..8)
                .map(|_| {
                    let g = gate.clone();
                    tokio::spawn(async move { g.wait().await })
                })
                .collect();
            // 即刻 finish：与各 waiter 的「检查-订阅」阶段高频交错
            gate.finish(Ok(()));
            for w in waiters {
                let got = tokio::time::timeout(std::time::Duration::from_secs(2), w)
                    .await
                    .expect("等待者必须限时返回（唤醒丢失即挂起超时）");
                assert!(got.unwrap().is_ok(), "终态必须为登记的成功值");
            }
        }
    }

    /// M28 碴2：PullGate 完成门——等待者阻塞至 finish、finish 后新等待者立即返回
    #[tokio::test]
    async fn pull_gate_wait_and_finish() {
        let gate = Arc::new(PullGate::new());
        let g = gate.clone();
        let waiter = tokio::spawn(async move { g.wait().await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !waiter.is_finished(),
            "前置：未 finish 前等待者必须仍在等待"
        );
        gate.finish(Ok(()));
        assert!(waiter.await.unwrap().is_ok(), "finish 后等待者必须返回成功");
        // 终态已登记：新等待者立即返回同终态
        assert!(gate.wait().await.is_ok());
        // finish 幂等：终态不被二次覆盖
        gate.finish(Err("late".into()));
        assert!(gate.wait().await.is_ok(), "终态必须保持首次登记值");
    }

    /// M93 碴B：进度快照读写——未写入时 (0, 0)（等待者据此跳过进度帧），
    /// 写入后读得最新值；终态语义不受快照影响
    #[test]
    fn pull_gate_progress_snapshot_write_then_read() {
        let gate = PullGate::new();
        assert_eq!(gate.snapshot_progress(), (0, 0), "未写入时快照必须为零值");
        gate.update_progress(1024, 91727296);
        assert_eq!(gate.snapshot_progress(), (1024, 91727296));
        gate.update_progress(8192, 91727296);
        assert_eq!(
            gate.snapshot_progress(),
            (8192, 91727296),
            "快照必须随写覆盖为最新值"
        );
        gate.finish(Ok(()));
        assert_eq!(
            gate.snapshot_progress(),
            (8192, 91727296),
            "终态登记不改动快照"
        );
    }

    /// M21 D4d：通配匹配语义
    #[test]
    fn wildcard_origin_matching() {
        assert!(wildcard_match(
            "http://localhost:*",
            "http://localhost:3000"
        ));
        assert!(wildcard_match("http://localhost", "http://localhost"));
        assert!(!wildcard_match("http://localhost:*", "http://evil.com"));
        assert!(wildcard_match("app://*", "app://obsidian.md"));
        assert!(wildcard_match("*", "https://any.example"));
    }
}
