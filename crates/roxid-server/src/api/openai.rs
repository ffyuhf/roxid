//! OpenAI 兼容层（/v1/*，M10）与 Responses API（/v1/responses，M11）。
//!
//! 透传原则（用户确认 2026-08-24 18:26）：llama-server 的响应/SSE 本就是
//! OpenAI 格式（含 Responses API），因此请求体按模型名 acquire 后
//! 原样字节转发，响应字节直通——零转换损耗。
//!
//! 修改历史：M10/M11 新增 2026-08-24 19:34;
//! M27 租约接入（迭代7 P0碴1，R1 全调用点）：proxy_json 持 RunnerLease，
//! relay_response 将租约移入流闭包（流耗尽/断开时释放）2026-08-26 21-32;
//! M28 碴1/碴3 Client 治理（迭代8）：透传 Client 移除 600s 总超时残留
//! （M22 只清了 ollama.rs，/v1 与 llamacpp 8 端点的 SSE 长生成超 10 分钟
//! 被掐断）+ OnceLock 进程内共享（原每请求新建，连接池零复用）
//! 2026-08-30 02-12
//! M29 碴3（迭代9）：proxy_json 转发前剥离 model/keep_alive 路由字段
//! （复用 llamacpp 层 strip_routing_fields，对齐原版兼容层消费语义）
//! 2026-09-05 12-50
//! M34 BUG-2（迭代14）：POST 提取器换 LenientJson（不校验 Content-Type，
//! 对齐官方直读 body 行为；官方文档示例即裸 curl -d）2026-09-07 19-25

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use serde_json::{json, Value};

use crate::adapter::parse_keep_alive;
use crate::api::{AppState, LenientJson};
use crate::scheduler::RunnerLease;

/// 透传 HTTP 客户端（流式响应需禁用自动解压缓冲语义干扰，直通字节）。
/// M18 起供 llamacpp 直通层共用。
/// M28 碴1/碴3：移除 600s 总超时残留（对齐 M22 在 ollama.rs 已落地的
/// 「无请求级总超时」语义——SSE 长流不再被掐断）；OnceLock 进程内共享
/// （原每请求新建 Client，连接池/TLS 会话零复用）；connect_timeout 10s
/// 兜底建连失败。Client 内部为 Arc，clone 廉价。
pub(crate) fn http() -> reqwest::Client {
    static SHARED: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("构建透传客户端失败")
        })
        .clone()
}

/// 通用透传：acquire 模型 → 转发请求体 → 响应字节（含 SSE）直通。
///
/// - 参数 st：应用状态
/// - 参数 path：llama-server 端点路径（如 /v1/chat/completions）
/// - 参数 body：客户端原始请求 JSON
/// - 返回：上游响应（头与体原样）
async fn proxy_json(st: &Arc<AppState>, path: &str, body: Value) -> Response {
    let model = body["model"].as_str().unwrap_or_default().to_string();
    if model.is_empty() {
        return err_json(StatusCode::BAD_REQUEST, "缺少 model 字段");
    }
    let keep_alive = parse_keep_alive(body.get("keep_alive"));
    // OpenAI 协议无 num_ctx 概念：请求级窗口不约束（透传原则）
    let lease = match st.scheduler.acquire(&model, keep_alive, None, None).await {
        Ok(r) => r,
        // 迭代30 碴A（M96）：错误分类整形——404 仅未安装（原统一 404）
        Err(e) => return crate::api::ollama::acquire_error_response(e),
    };
    let port = lease.port().await;
    // M29 碴3：转发前剥离 roxid 调度字段（model/keep_alive）——原样透传使
    // llama-server 收到非 OpenAI 协议字段（其容忍未知字段属侥幸容错，
    // 未来版本严格校验即碎）；llamacpp 层 M18 起即剥离，本层对齐
    let (forward_body, _) = crate::api::llamacpp::strip_routing_fields(body);
    let resp = match http()
        .post(format!("http://127.0.0.1:{port}{path}"))
        .json(&forward_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return err_json(StatusCode::BAD_GATEWAY, e.to_string()),
    };
    relay_response(resp, lease).await
}

/// 上游响应转客户端响应：状态/类型头透传，body 流式直通。
/// M18 起供 llamacpp 直通层共用。
/// M27（碴1）：租约移入流闭包——流耗尽或 body 被客户端断开丢弃时释放，
/// 在途保护覆盖整个响应生命周期（含 SSE 长流）。
///
/// - 参数 resp：上游响应
/// - 参数 lease：本次请求的实例租约（移入流后随流生命周期释放）
/// - 返回：可直接返回给客户端的 axum Response
pub(crate) async fn relay_response(resp: reqwest::Response, lease: RunnerLease) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let stream = resp.bytes_stream().map(move |chunk| {
        let _ = &lease; // 闭包持有租约：流结束/丢弃时 Drop 释放
        chunk
    });
    let body = Body::from_stream(stream);
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(body)
        .unwrap()
}

/// JSON 错误响应（M18 起供 llamacpp 直通层共用）
pub(crate) fn err_json(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({"error": {"message": msg.to_string()}}))).into_response()
}

/// POST /v1/chat/completions：OpenAI Chat（SSE 直通）
pub async fn chat_completions(
    State(st): State<Arc<AppState>>,
    LenientJson(body): LenientJson<Value>,
) -> Response {
    proxy_json(&st, "/v1/chat/completions", body).await
}

/// POST /v1/completions：OpenAI Completions（SSE 直通）
pub async fn completions(
    State(st): State<Arc<AppState>>,
    LenientJson(body): LenientJson<Value>,
) -> Response {
    proxy_json(&st, "/v1/completions", body).await
}

/// POST /v1/embeddings：OpenAI Embeddings（直通）
pub async fn embeddings(
    State(st): State<Arc<AppState>>,
    LenientJson(body): LenientJson<Value>,
) -> Response {
    proxy_json(&st, "/v1/embeddings", body).await
}

/// POST /v1/responses：Responses API（SSE 事件流直通，M11）
pub async fn responses(
    State(st): State<Arc<AppState>>,
    LenientJson(body): LenientJson<Value>,
) -> Response {
    proxy_json(&st, "/v1/responses", body).await
}

/// POST /v1/rerank：重排序端点（透传，Q5：llama.cpp 功能全覆盖）
pub async fn rerank(
    State(st): State<Arc<AppState>>,
    LenientJson(body): LenientJson<Value>,
) -> Response {
    proxy_json(&st, "/v1/rerank", body).await
}

/// GET /v1/models：OpenAI 模型列表（本地仓库视图）
/// M101（迭代32 碴2）：created 填充真实创建时间（原硬编码 0）——数据源
/// meta.created_at（RFC3339，与 /api/tags modified_at 同源）经双形态
/// 解析转 epoch 秒；老数据/结构非法兜底 0（不劣于原值）
pub async fn models(State(st): State<Arc<AppState>>) -> Response {
    let list: Vec<Value> = crate::repo::list_models(&st.models_root)
        .into_iter()
        .map(|m| {
            json!({
                "id": m.name,
                "object": "model",
                "created": crate::repo::parse_rfc3339_secs(&m.created_at).unwrap_or(0),
                "owned_by": "roxid",
            })
        })
        .collect();
    Json(json!({"object": "list", "data": list})).into_response()
}
