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
//! 迭代42 D7（O-2 清偿）：响应 model 字段改写为规范化模型名（原透传
//! llama-server 的 --model 实参即本地 GGUF 绝对路径，OpenAI 契约应为
//! 模型标识符且兼有路径暴露面）——SSE 逐事件行缓冲改写、非流式整体
//! 改写，relay_response 保持纯透传供 llamacpp 层共用 2026-09-11 21-10

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
    // 迭代42 D7（O-2）：改写目标 = 规范化全名（与 acquire 实例键同源
    // 规则；parse 在 acquire 内已成功，此处必过）
    let display_name = crate::repo::ModelRef::parse(&model)
        .ok()
        .map(|r| r.full_name());
    relay_response_remodel(resp, lease, display_name).await
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

/// 迭代42 D7（O-2 清偿 2026-09-11）：直通响应 model 字段改写。
///
/// llama-server 响应的 model 为 --model 实参（本地 GGUF 绝对路径，
/// 兼有本机路径信息暴露），OpenAI 契约为模型标识符（官方 Ollama 返回
/// 规范化模型名）。SSE 逐事件行缓冲改写（跨 chunk 边界安全，流式
/// 语义保留）；非流式 JSON 缓冲整体改写；非 JSON 或无 model 键响应
/// 原样透传。
///
/// - 参数 resp：上游响应
/// - 参数 lease：本次请求的实例租约（SSE 路径移入流闭包随流释放，M27 语义）
/// - 参数 display_name：改写目标模型名（规范化全名；None 时纯透传）
/// - 返回：可直接返回给客户端的 axum Response
pub(crate) async fn relay_response_remodel(
    resp: reqwest::Response,
    lease: RunnerLease,
    display_name: Option<String>,
) -> Response {
    let Some(name) = display_name else {
        return relay_response(resp, lease).await;
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    if content_type.contains("text/event-stream") {
        // SSE：行缓冲逐事件改写——残余不完整行留缓冲等下一 chunk，
        // 事件分隔（空行）与非 data 行天然按行原样透出
        let mut pending: Vec<u8> = Vec::new();
        let stream = resp.bytes_stream().map(move |chunk| {
            let _ = &lease; // 闭包持有租约：流结束/丢弃时 Drop 释放
            match chunk {
                Ok(bytes) => {
                    pending.extend_from_slice(&bytes);
                    let mut out = Vec::new();
                    while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=pos).collect();
                        out.extend_from_slice(&rewrite_sse_line(&line, &name));
                    }
                    Ok(out)
                }
                Err(e) => Err(e),
            }
        });
        return Response::builder()
            .status(status)
            .header("content-type", content_type)
            .body(Body::from_stream(stream))
            .unwrap();
    }
    // 非流式：整体缓冲后改写（无 model 键或非 JSON 时原样透传）
    match resp.bytes().await {
        Ok(bytes) => {
            let body = rewrite_json_model(&bytes, &name).unwrap_or(bytes.to_vec());
            Response::builder()
                .status(status)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap()
        }
        Err(e) => err_json(StatusCode::BAD_GATEWAY, e),
    }
}

/// SSE 单行改写：`data: ` 前缀 JSON 的 model 字符串键（顶层与 response
/// 对象嵌套，覆盖 chat chunk 与 /v1/responses 事件两形态）替换为目标名；
/// [DONE] 哨兵、非 data 行、非 JSON data 行原样。
fn rewrite_sse_line(line: &[u8], name: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(line);
    if let Some(payload) = s.strip_prefix("data: ") {
        if let Ok(mut v) = serde_json::from_str::<Value>(payload.trim_end()) {
            if rewrite_model_value(&mut v, name) {
                return format!("data: {v}\n").into_bytes();
            }
        }
    }
    line.to_vec()
}

/// 就地改写 model 字符串键（顶层 + response 对象嵌套一层）；
/// 发生改写返回 true，非对象形态返回 false。
fn rewrite_model_value(v: &mut Value, name: &str) -> bool {
    let Some(obj) = v.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    if let Some(m) = obj.get_mut("model") {
        if m.is_string() {
            *m = json!(name);
            changed = true;
        }
    }
    if let Some(inner) = obj.get_mut("response").and_then(|r| r.as_object_mut()) {
        if let Some(m) = inner.get_mut("model") {
            if m.is_string() {
                *m = json!(name);
                changed = true;
            }
        }
    }
    changed
}

/// 非流式 JSON 整体改写；无 model 键或解析/序列化失败返回 None
///（调用方原样透传）。
fn rewrite_json_model(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut v: Value = serde_json::from_slice(bytes).ok()?;
    if rewrite_model_value(&mut v, name) {
        serde_json::to_vec(&v).ok()
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 迭代42 D7（O-2）：SSE 行改写——model 键替换、[DONE] 与非 data 行
    /// 原样、response 嵌套形态覆盖、非 JSON data 行透传
    #[test]
    fn sse_model_rewrite_forms() {
        let line = b"data: {\"model\":\"/home/u/.roxid/m.gguf\",\"choices\":[]}\n";
        let out = rewrite_sse_line(line, "smollm:135m");
        // 语义断言（serde_json 默认 BTreeMap 键序，不锚定字面键序）
        let payload = String::from_utf8_lossy(&out);
        let json_part = payload.strip_prefix("data: ").expect("data 前缀必须保持");
        let v: Value = serde_json::from_str(json_part.trim_end()).unwrap();
        assert_eq!(v["model"], "smollm:135m", "model 键必须已改写");
        assert_eq!(v["choices"], json!([]), "其余键原样");
        // [DONE] 哨兵原样
        assert_eq!(rewrite_sse_line(b"data: [DONE]\n", "m"), b"data: [DONE]\n");
        // 非 data 行原样
        assert_eq!(rewrite_sse_line(b"event: ping\n", "m"), b"event: ping\n");
        // /v1/responses 嵌套形态（model 在 response 对象内）
        let nested =
            b"data: {\"type\":\"response.created\",\"response\":{\"model\":\"/abs/path\"}}\n";
        let out = rewrite_sse_line(nested, "qwen3:0.6b");
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("\"model\":\"qwen3:0.6b\""));
        // 非 JSON data 行原样
        assert_eq!(
            rewrite_sse_line(b"data: not-json\n", "m"),
            b"data: not-json\n"
        );
    }

    /// 迭代42 D7（O-2）：非流式 JSON 改写与透传边界
    #[test]
    fn json_model_rewrite_forms() {
        let body = b"{\"model\":\"/abs/m.gguf\",\"object\":\"chat.completion\"}";
        let out = rewrite_json_model(body, "smollm:135m").unwrap();
        assert!(String::from_utf8_lossy(&out).contains("\"model\":\"smollm:135m\""));
        // 无 model 键：None（调用方原样透传信号）
        assert!(rewrite_json_model(b"{\"object\":\"list\"}", "m").is_none());
        // 非 JSON：None
        assert!(rewrite_json_model(b"<html>err</html>", "m").is_none());
        // model 非字符串（数值）：不改写，None
        assert!(rewrite_json_model(b"{\"model\":123}", "m").is_none());
    }
}
