//! llama.cpp 原生端点直通层（M18，迭代3计划 D3）。
//!
//! 将 llama-server 特有、OpenAI/Ollama 协议均未覆盖的 8 个端点纳入网关代理，
//! 使其全部能力对外可达（透传原则延续：零转换字节直通）：
//! - POST /tokenize /detokenize /infill /completion /embedding（请求体透传）
//! - GET  /props /slots /metrics（实例状态查询转发）
//!
//! 路由约定（来源：用户确认 Q4 2026-08-24 22:11「GET 类按模型 acquire 后转发，
//! tokenize/detokenize/completion/infill 走 POST 透传」；M18-R1 2026-08-24 22:16
//! 「严格报错：一律 400 要求 body 带 model 字段」）：
//! - POST body 必须携带 model 字段（roxid 路由键），缺失一律 400；
//! - GET 必须携带 ?model= 查询参数，缺失一律 400；
//! - model 与 keep_alive 为 roxid 调度字段，转发前剔除，其余字段原样透传。
//!
//! 修改历史：M18 新增 2026-08-24 22-16；M21 D4c acquire 补 want_ctx=None 2026-08-24 23-04;
//! M27 租约接入（迭代7 P0碴1，R1 全调用点）：proxy_post/proxy_get 持 RunnerLease
//! 并经 relay_response 移入流闭包（请求全程保护）2026-08-26 21-32
//! M34 BUG-2（迭代14）：POST 提取器换 LenientJson（不校验 Content-Type，
//! 对齐官方直读 body 行为；官方文档示例即裸 curl -d）2026-09-07 19-25

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;
use serde_json::Value;

use crate::adapter::parse_keep_alive;
use crate::api::openai::{err_json, http, relay_response};
use crate::api::AppState;

/// 从请求体提取 model 字段（缺失一律 400，M18-R1 裁决：严格报错不自动路由）。
///
/// - 参数 body：客户端原始请求 JSON
/// - 返回：Ok(模型名) 或 Err(400 响应)
fn extract_model(body: &Value) -> Result<String, Response> {
    match body["model"].as_str() {
        Some(m) if !m.is_empty() => Ok(m.to_string()),
        _ => Err(err_json(
            axum::http::StatusCode::BAD_REQUEST,
            "缺少 model 字段：llama.cpp 原生端点经 roxid 多模型网关路由，body 必须携带 model",
        )),
    }
}

/// 剔除 roxid 调度字段（model/keep_alive）后的透传体。
/// M29 碴3：升 pub(crate) 供 openai 透传层共用（/v1/* 原样转发 model/
/// keep_alive 属协议卫生碴——llama-server 容忍未知字段属侥幸容错）。
///
/// - 参数 body：客户端原始请求 JSON
/// - 返回：(可原样转发给 llama-server 的请求体, 取出的 keep_alive 原始值)
pub(crate) fn strip_routing_fields(mut body: Value) -> (Value, Option<Value>) {
    let keep_alive = body.get("keep_alive").cloned();
    if let Some(obj) = body.as_object_mut() {
        obj.remove("model");
        obj.remove("keep_alive");
    }
    (body, keep_alive)
}

/// POST 直通：body{model} → acquire → 剔除路由字段 → 转发 → 字节直通。
///
/// - 参数 st：应用状态
/// - 参数 path：llama-server 端点路径（如 /tokenize）
/// - 参数 body：客户端原始请求 JSON
/// - 返回：上游响应原样
async fn proxy_post(st: &Arc<AppState>, path: &str, body: Value) -> Response {
    let model = match extract_model(&body) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let keep_alive = parse_keep_alive(body.get("keep_alive"));
    // llama.cpp 原生协议无 num_ctx 概念：请求级窗口不约束
    let lease = match st.scheduler.acquire(&model, keep_alive, None, None).await {
        Ok(r) => r,
        Err(e) => return err_json(axum::http::StatusCode::NOT_FOUND, e.to_string()),
    };
    let port = lease.port().await;
    let (forward_body, _) = strip_routing_fields(body);
    let resp = match http()
        .post(format!("http://127.0.0.1:{port}{path}"))
        .json(&forward_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return err_json(axum::http::StatusCode::BAD_GATEWAY, e.to_string()),
    };
    relay_response(resp, lease).await
}

/// GET 直通：?model= → acquire → 转发 → 字节直通。
///
/// - 参数 st：应用状态
/// - 参数 model：模型名（查询参数）
/// - 参数 path：llama-server 端点路径（如 /props）
/// - 返回：上游响应原样
async fn proxy_get(st: &Arc<AppState>, model: &str, path: &str) -> Response {
    // GET 无请求体，keep_alive 沿用默认窗口（查询同样视为使用，acquire 自动续期）
    let keep_alive = parse_keep_alive(None);
    let lease = match st.scheduler.acquire(model, keep_alive, None, None).await {
        Ok(r) => r,
        Err(e) => return err_json(axum::http::StatusCode::NOT_FOUND, e.to_string()),
    };
    let port = lease.port().await;
    let resp = match http()
        .get(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return err_json(axum::http::StatusCode::BAD_GATEWAY, e.to_string()),
    };
    relay_response(resp, lease).await
}

/// GET 查询参数提取（?model=xxx，缺失一律 400，M18-R1 对称语义）
type ModelQuery = std::collections::HashMap<String, String>;

/// GET 参数校验：必须携带非空 model。
///
/// - 参数 q：查询参数映射
/// - 返回：Ok(模型名) 或 Err(400 响应)
fn query_model(q: &ModelQuery) -> Result<String, Response> {
    match q.get("model").map(String::as_str) {
        Some(m) if !m.is_empty() => Ok(m.to_string()),
        _ => Err(err_json(
            axum::http::StatusCode::BAD_REQUEST,
            "缺少 ?model= 查询参数：GET 类端点经 roxid 多模型网关路由，必须指定模型",
        )),
    }
}

/// POST /tokenize：文本 → token id 序列（响应 {tokens:[...]} 直通）
pub async fn tokenize(
    State(st): State<Arc<AppState>>,
    body: crate::api::LenientJson<Value>,
) -> Response {
    proxy_post(&st, "/tokenize", body.0).await
}

/// POST /detokenize：token id 序列 → 文本
pub async fn detokenize(
    State(st): State<Arc<AppState>>,
    body: crate::api::LenientJson<Value>,
) -> Response {
    proxy_post(&st, "/detokenize", body.0).await
}

/// POST /infill：FIM 代码补全（前缀/后缀/中间填充）
pub async fn infill(
    State(st): State<Arc<AppState>>,
    body: crate::api::LenientJson<Value>,
) -> Response {
    proxy_post(&st, "/infill", body.0).await
}

/// POST /completion：llama.cpp 原生补全（非 OpenAI 格式）
pub async fn completion(
    State(st): State<Arc<AppState>>,
    body: crate::api::LenientJson<Value>,
) -> Response {
    proxy_post(&st, "/completion", body.0).await
}

/// POST /embedding：llama.cpp 原生向量化（非 OpenAI 格式）
pub async fn embedding(
    State(st): State<Arc<AppState>>,
    body: crate::api::LenientJson<Value>,
) -> Response {
    proxy_post(&st, "/embedding", body.0).await
}

/// GET /props：实例配置与生成参数快照（含 total_slots/chat_template/modalities）
pub async fn props(State(st): State<Arc<AppState>>, Query(q): Query<ModelQuery>) -> Response {
    match query_model(&q) {
        Ok(m) => proxy_get(&st, &m, "/props").await,
        Err(resp) => resp,
    }
}

/// GET /slots：slot 状态数组（含每 slot 的 params 与处理进度）
pub async fn slots(State(st): State<Arc<AppState>>, Query(q): Query<ModelQuery>) -> Response {
    match query_model(&q) {
        Ok(m) => proxy_get(&st, &m, "/slots").await,
        Err(resp) => resp,
    }
}

/// GET /metrics：Prometheus 指标（文本格式直通）
pub async fn metrics(State(st): State<Arc<AppState>>, Query(q): Query<ModelQuery>) -> Response {
    match query_model(&q) {
        Ok(m) => proxy_get(&st, &m, "/metrics").await,
        Err(resp) => resp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// M18-R1 裁决：body 缺 model 字段必须 400，不自动路由
    #[test]
    fn missing_model_rejected_strictly() {
        assert!(extract_model(&json!({"content": "hi"})).is_err());
        assert!(extract_model(&json!({})).is_err());
        assert!(
            extract_model(&json!({"model": ""})).is_err(),
            "空串同样拒绝"
        );
        assert_eq!(
            extract_model(&json!({"model": "llama3.2"})).unwrap(),
            "llama3.2"
        );
    }

    /// GET ?model= 同语义：缺失/空串 400
    #[test]
    fn query_model_required() {
        let mut q = std::collections::HashMap::new();
        assert!(query_model(&q).is_err());
        q.insert("model".into(), "".into());
        assert!(query_model(&q).is_err());
        q.insert("model".into(), "qwen3".into());
        assert_eq!(query_model(&q).unwrap(), "qwen3");
    }

    /// 转发前剔除 model/keep_alive 路由字段，其余原样保留
    #[test]
    fn routing_fields_stripped_before_forward() {
        let (forwarded, keep) = strip_routing_fields(json!({
            "model": "llama3.2", "keep_alive": "5m", "content": "你好", "add_special": true
        }));
        assert!(forwarded.get("model").is_none(), "model 必须剔除");
        assert!(forwarded.get("keep_alive").is_none(), "keep_alive 必须剔除");
        assert_eq!(forwarded["content"], "你好", "业务字段原样保留");
        assert_eq!(forwarded["add_special"], true);
        assert_eq!(keep, Some(json!("5m")), "keep_alive 取出供 acquire 用");
    }
}
