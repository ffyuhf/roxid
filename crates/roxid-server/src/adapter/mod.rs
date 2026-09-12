//! 协议适配层：Ollama 请求/响应格式与 llama-server（OpenAI 风格）互转。
//!
//! 透传原则（来源：用户确认 2026-08-24 18:26）：推理侧能力全部交给
//! llama-server，本层只做字段映射：
//! - options.* → 采样与上下文参数（temperature/top_p/top_k/num_predict/stop/seed 等）
//! - tools/tool_calls → OpenAI function calling（结构同构，直接映射）
//! - images（base64 数组）→ image_url（data URI）
//! - format（"json"/JSON Schema）→ response_format
//! - think → reasoning 映射（false→none + enable_thinking=false 模板侧
//!   抑制（迭代18 BUG-8）；true/档位→auto + chat_template_kwargs（M35 D9））
//! - tool 角色消息 → tool_call_id 配对（M21：assistant 调用生成稳定 id，
//!   后续 tool 消息按 tool_name 匹配或按序消费）
//! - keep_alive → 调度层生命周期（非推理参数，由 api 层消费）
//!
//! 修改历史：占位 2026-08-24 18:35；M7 实装 2026-08-24 19:30；
//! M21 工具配对与 think 分级 2026-08-24 22:52；
//! M28 三碴（迭代8）：碴8 tool_call id 进程级全局递增（原每请求 call-0
//! 重计，混合历史消息表 id 歧义）、碴18 raw 通道 format 改拼 llama.cpp
//! 原生 json_schema 语法字段（原 response_format 被 /v1/completions 静默
//! 忽略）、碴6 duration 四字段真实计量（原硬编码 0）2026-08-30 06-10
//! M33 碴3（迭代13）：chat/generate 流末包统计键改「usage 存在才写 +
//! as_u64 兜底 0」——原空对象 clone 序列化 null（非 0 也非缺省），CLI
//! unwrap_or(0) 掩盖 2026-09-07 01-50
//! M35（迭代15 十三碴）：D9 think 四档补全（原 medium/max 落默认分支被静默
//! 忽略）；D12 mirostat 三参数白名单同名直传（/v1/completions 官方支持，
//! Context7 实证 2026-09-07）；D8 OllamaMessage 增 thinking 字段映射
//! reasoning_content（历史思考回传）；D1/D6/D7 generate 请求增 context/
//! suffix/logprobs/top_logprobs 字段（context/suffix 消费在 api 层）、chat
//! 请求增 logprobs/top_logprobs 双构建器拼装 2026-09-07 21-30
//! M45a（迭代18 BUG-8，用户裁决 Q3-A 双保险 2026-09-09 05:49）：
//! think:false 补发 chat_template_kwargs.enable_thinking=false（模板侧
//! 思考抑制，llama.cpp 官方通道）+ 响应侧 <think> 块剥离兜底（老版本
//! 后端防护）2026-09-09 05-53

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Ollama 聊天消息（请求侧）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    /// base64 图像列表（vision）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// 工具调用（assistant 消息）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
    /// 工具调用结果（tool 角色）对应的工具名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// 思考内容（assistant 消息回传；思考模型多轮上下文保持。M35 D8）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

/// Ollama 工具调用
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaToolCall {
    pub function: OllamaToolCallFunction,
}

/// 工具调用的函数描述
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaToolCallFunction {
    pub name: String,
    /// JSON 字符串形式的参数
    pub arguments: Value,
}

/// Ollama /api/chat 请求
#[derive(Debug, Clone, Deserialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    /// "json" / JSON Schema 对象 / true
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Value>,
    /// 采样与上下文参数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    /// 工具定义（结构与 OpenAI 一致）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    /// 流式开关（原版默认 true）
    #[serde(default)]
    pub stream: Option<bool>,
    /// "5m"/"0"/数字秒/None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<Value>,
    /// 思考开关：bool 或 "low"/"medium"/"high"/"max"（M35 D9 四档）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think: Option<Value>,
    /// token 对数概率开关（M35 D7；透传 /v1 标准 logprobs）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    /// 每 token 返回的概率条目数上限（M35 D7；需 logprobs=true）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
}

/// Ollama /api/generate 请求
#[derive(Debug, Clone, Deserialize)]
pub struct OllamaGenerateRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<bool>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think: Option<Value>,
    /// 上轮 context（token id 数组）——官方续传协议（M35 D1；消费在 api 层 M35c）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<u32>>,
    /// FIM 后缀提示（M35 D6；存在时 api 层路由原生 /infill）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// token 对数概率开关（M35 D7；透传 /v1 标准 logprobs）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    /// 每 token 返回的概率条目数上限（M35 D7；需 logprobs=true）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
}

/// 解析 keep_alive 字段：数字（秒）/"5m"/"1h"/"0"/null → Duration。
///
/// - 参数 v：keep_alive 原始 JSON 值
/// - 返回：空闲存活时长（默认 5m）
pub fn parse_keep_alive(v: Option<&Value>) -> std::time::Duration {
    use std::time::Duration;
    match v {
        Some(Value::Number(n)) => n.as_f64().map(|s| Duration::from_secs_f64(s.max(0.0))),
        Some(Value::String(s)) => parse_duration_str(s),
        _ => None,
    }
    .unwrap_or(crate::scheduler::DEFAULT_KEEP_ALIVE)
}

/// "5m"/"30s"/"1h"/"0" → Duration
fn parse_duration_str(s: &str) -> Option<std::time::Duration> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: f64 = num.parse().ok()?;
    let secs = match unit.trim() {
        "s" | "" => n,
        "m" => n * 60.0,
        "h" => n * 3600.0,
        _ => return None,
    };
    Some(std::time::Duration::from_secs_f64(secs.max(0.0)))
}

/// 从 options 提取请求级 num_ctx（D4c：请求级上下文触发实例重建的依据）。
///
/// - 参数 options：Ollama options 对象
/// - 返回：请求的每 slot 上下文窗口；未提供 None
pub fn requested_num_ctx(options: Option<&Value>) -> Option<u32> {
    options?
        .get("num_ctx")?
        .as_u64()
        .map(|n| n.clamp(512, u32::MAX as u64) as u32)
}

/// 从 options 提取请求级 RUNTIME 启动参数串（M39：roxid 扩展字段，
/// `roxid run --runtime "<flags>"` 透传位；非官方字段，官方 SDK 不传即无影响）。
/// 空白串视为未设置（None——不触发重建）。
///
/// - 参数 options：Ollama options 对象
/// - 返回：RUNTIME 参数原始串；未提供或空白 None
pub fn requested_runtime(options: Option<&Value>) -> Option<String> {
    options?
        .get("runtime")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 判型并解析 llama-server 超窗错误体（迭代43 M160/M161：自动扩窗重建依据）。
/// 错误体形态（b10883 实证，用户报告 2026-09-11 22:50）：
/// `{"error":{"code":400,"message":"request (N tokens) exceeds ...","type":"exceed_context_size_error","n_prompt_tokens":N,"n_ctx":M}}`
/// 调用方须先确认 HTTP 状态为 400（本函数只判 body）。
///
/// - 参数 body：上游错误响应体文本
/// - 返回：超窗错误时返回 n_prompt_tokens；非超窗错误或字段缺失 None
pub fn parse_exceed_context_error(body: &str) -> Option<u64> {
    let err = serde_json::from_str::<Value>(body)
        .ok()?
        .get("error")?
        .clone();
    if err.get("type").and_then(Value::as_str) != Some("exceed_context_size_error") {
        return None;
    }
    err.get("n_prompt_tokens").and_then(Value::as_u64)
}

/// 扩窗目标值计算（迭代43 R1-A 裁决 2026-09-11 23:03）：
/// 目标 = n_prompt_tokens + 1024 生成余量，向上对齐 512 倍数；
/// GGUF 训练长度封顶（官方 effectiveContext 语义，/ollama/ollama
/// server/sched.go 实证 2026-09-11 22:53）；GGUF 不可读（None）宽容不封顶；
/// 下限 512 与 [requested_num_ctx] 同构（防发出不可用窗口）。
///
/// - 参数 n_prompt_tokens：超窗错误体携带的实际 prompt token 数
/// - 参数 train_ctx：GGUF {arch}.context_length（不可读传 None）
/// - 返回：重建实例的每 slot 目标窗口
pub fn expanded_ctx_target(n_prompt_tokens: u64, train_ctx: Option<u64>) -> u32 {
    const GEN_MARGIN: u64 = 1024;
    const ALIGN: u64 = 512;
    const FLOOR: u64 = 512;
    let raw = n_prompt_tokens.saturating_add(GEN_MARGIN);
    let aligned = raw.div_ceil(ALIGN).saturating_mul(ALIGN);
    let capped = match train_ctx {
        Some(train) => aligned.min(train.max(FLOOR)),
        None => aligned,
    };
    capped.clamp(FLOOR, u32::MAX as u64) as u32
}

/// options + format + think → llama-server chat 请求的参数补丁。
/// 返回可直接 merge 进 OpenAI 请求 JSON 的字段集合。
///
/// - 参数 options：Ollama options 对象
/// - 参数 format：Ollama format 字段
/// - 参数 think：Ollama think 字段
/// - 返回：llama-server 侧参数 JSON（对象）
/// tool_call id 进程级全局递增游标（M28 碴8：兑现「call-{n} 全局递增」注释
/// 语义——原实现为每请求局部计数从 call-0 重开，混合多来源历史的消息表
/// 出现重复 tool_call_id，llama-server 配对歧义）。
static TOOL_CALL_ID_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 取下一个全局唯一 tool_call id。
fn next_tool_call_id() -> String {
    format!(
        "call-{}",
        TOOL_CALL_ID_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// 上游 timings 覆盖 duration 两字段（M102，迭代32 碴3）。
/// 非流式响应的墙钟口径以 headers 到达切分 prompt/eval——但 llama-server
/// 生成完毕才一次性返回，eval 被计成 body 读取耗时（µs 级）、
/// prompt_eval 混入全部生成时间（用户实测 eval_duration=69844ns≈7µs/token）。
/// llama-server 非流式响应顶层携带精确计时 timings.prompt_ms/predicted_ms
///（/v1/chat/completions、/v1/completions、/completion、/infill 同构，
/// 官方 README 实证），本函数以之为准覆盖；total/load 维持 roxid 侧实测
///（调用序：先 apply_duration_fields 墙钟兜底，再本函数覆盖两 duration）。
///
/// - 参数 ev：待注入的响应事件（原地修改）
/// - 参数 timings：上游响应 timings 对象（缺失字段跳过覆盖——墙钟兜底保留）
pub fn apply_timings_durations(ev: &mut Value, timings: &Value) {
    let ms_to_ns = |v: &Value| v.as_f64().map(|ms| (ms * 1e6) as u64);
    if let Some(ns) = ms_to_ns(&timings["prompt_ms"]) {
        ev["prompt_eval_duration"] = json!(ns);
    }
    if let Some(ns) = ms_to_ns(&timings["predicted_ms"]) {
        ev["eval_duration"] = json!(ns);
    }
}

/// 注入 Ollama duration 四字段（纳秒；M28 碴6：原四字段硬编码 0）。
/// 口径（R2-A 裁决）：total=请求到达→响应完成；load=本次 acquire 冷加载
/// 耗时（复用实例为 0）；prompt_eval=响应首字节前（扣除 load）；
/// eval=首字节→完成。
///
/// - 参数 ev：待注入的响应事件（原地修改）
/// - 参数 request_start：请求到达时刻
/// - 参数 first_byte：上游响应首字节（headers）到达时刻
/// - 参数 load：本次请求触发的实例冷加载耗时
pub fn apply_duration_fields(
    ev: &mut Value,
    request_start: std::time::Instant,
    first_byte: std::time::Instant,
    load: std::time::Duration,
) {
    let end = std::time::Instant::now();
    let total = end.saturating_duration_since(request_start);
    let prompt_eval = first_byte
        .saturating_duration_since(request_start)
        .saturating_sub(load);
    let eval = end.saturating_duration_since(first_byte);
    let ns = |d: std::time::Duration| json!(d.as_nanos() as u64);
    ev["total_duration"] = ns(total);
    ev["load_duration"] = ns(load);
    ev["prompt_eval_duration"] = ns(prompt_eval);
    ev["eval_duration"] = ns(eval);
}

pub fn build_inference_params(
    options: Option<&Value>,
    format: Option<&Value>,
    think: Option<&Value>,
) -> Value {
    let mut out = serde_json::Map::new();
    let opts = options
        .and_then(|o| o.as_object())
        .cloned()
        .unwrap_or_default();

    // 直接同名映射的采样参数（M22 扩展：b10605 请求级支持全集见 /props params）
    for key in [
        "temperature",
        "top_p",
        "top_k",
        "min_p",
        "repeat_penalty",
        "seed",
        "presence_penalty",
        "frequency_penalty",
        "typical_p",
        "repeat_last_n",
        "dynatemp_range",
        "dynatemp_exponent",
        "xtc_probability",
        "xtc_threshold",
        // M35 D12：mirostat 家族同名直传（/v1/completions 官方支持，
        // Context7 实证 2026-09-07 llama.cpp server README）
        "mirostat",
        "mirostat_eta",
        "mirostat_tau",
    ] {
        if let Some(v) = opts.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    // Ollama num_predict → OpenAI max_tokens
    if let Some(np) = opts.get("num_predict") {
        out.insert("max_tokens".into(), np.clone());
    }
    // Ollama stop（数组）→ OpenAI stop
    if let Some(stop) = opts.get("stop") {
        out.insert("stop".into(), stop.clone());
    }
    // num_ctx 不在请求级透传（llama-server 为启动参数；请求级由调度层重建实例兑现，D4c）

    // format → response_format
    match format {
        Some(Value::String(s)) if s == "json" => {
            out.insert("response_format".into(), json!({"type": "json_object"}));
        }
        Some(Value::Bool(true)) => {
            out.insert("response_format".into(), json!({"type": "json_object"}));
        }
        Some(schema @ Value::Object(_)) => {
            out.insert(
                "response_format".into(),
                json!({"type": "json_schema", "json_schema": {"schema": schema}}),
            );
        }
        _ => {}
    }

    // think 分级（M21 D4b；M35 D9 四档补全 + b10605 值域修正；迭代18
    // BUG-8 补发模板侧开关）：false → none + enable_thinking=false；
    // true/low/medium/high/max → auto（b10605 reasoning_format 值域
    // 仅 none|auto——deep_think 已被 llama.cpp 移除，e2e 实证 2026-09-07
    // 22:41「Unknown reasoning format: deep_think」500）；档位值原样经
    // chat_template_kwargs.reasoning_effort 传给模板，由模板解释生效。
    // BUG-8 根因：reasoning_format=none 仅是输出侧不拆分——thinking 模型
    // 模板仍生成 <think> 块原文进 content（Qwen3 泄漏实证）；模板侧生成
    // 必须经 enable_thinking 关闭（server-common.cpp 官方通道，Context7
    // 实证 2026-09-09）
    match think {
        Some(Value::Bool(false)) => {
            out.insert("reasoning_format".into(), json!("none"));
            // 迭代18 BUG-8：必须 bool 形态——server 端 dump 后按 "false"
            // 字符串比较命中；字符串形态会被 throw invalid_argument（500）
            out.insert(
                "chat_template_kwargs".into(),
                json!({"enable_thinking": false}),
            );
        }
        Some(Value::Bool(true)) => {
            out.insert("reasoning_format".into(), json!("auto"));
        }
        Some(Value::String(s)) if matches!(s.as_str(), "low" | "medium" | "high" | "max") => {
            out.insert("reasoning_format".into(), json!("auto"));
            out.insert(
                "chat_template_kwargs".into(),
                json!({"reasoning_effort": s}),
            );
        }
        _ => {}
    }
    Value::Object(out)
}

/// Ollama 消息列表 → OpenAI messages 数组。
/// - images → image_url data URI（content 分段数组）
/// - assistant tool_calls → OpenAI 结构 + 稳定 id（call-{n} 全局递增）
/// - tool 角色 → role=tool + tool_call_id（按 tool_name 匹配，否则按序消费；
///   M21 D4a：多轮工具调用配对修复）
///
/// - 参数 messages：Ollama 消息列表
/// - 参数 extra_system：generate 请求的 system 覆盖（置于首位）
/// - 返回：OpenAI messages JSON 数组
pub fn to_openai_messages(messages: &[OllamaMessage], extra_system: Option<&str>) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(sys) = extra_system {
        out.push(json!({"role": "system", "content": sys}));
    }
    // 待消费的调用队列：(id, 工具名)；assistant 产生，tool 消费
    let mut pending: Vec<(String, String)> = Vec::new();
    for m in messages {
        if m.role == "tool" {
            // 优先按 tool_name 精确匹配，否则按序消费队首
            let idx = m
                .tool_name
                .as_ref()
                .and_then(|name| pending.iter().position(|(_, n)| n == name))
                .unwrap_or(0);
            let (id, _) = if pending.is_empty() {
                // 无前置调用（异常输入）：生成占位 id，不阻断请求
                (next_tool_call_id(), String::new())
            } else {
                pending.remove(idx.min(pending.len() - 1))
            };
            out.push(json!({
                "role": "tool",
                "content": m.content,
                "tool_call_id": id,
            }));
            continue;
        }
        let mut om = serde_json::Map::new();
        om.insert("role".into(), json!(m.role));
        if m.images.is_empty() {
            om.insert("content".into(), json!(m.content));
        } else {
            // 多模态：content 为分段数组（text + image_url data URI）
            let mut parts = vec![json!({"type": "text", "text": m.content})];
            for b64 in &m.images {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:image/png;base64,{b64}")}
                }));
            }
            om.insert("content".into(), Value::Array(parts));
        }
        // M35 D8：assistant 历史思考回传 → reasoning_content（llama-server
        // 侧由 chat template 消费，思考模型多轮上下文保持）
        if let Some(thinking) = &m.thinking {
            om.insert("reasoning_content".into(), json!(thinking));
        }
        if let Some(calls) = &m.tool_calls {
            let mapped: Vec<Value> = calls
                .iter()
                .map(|c| {
                    let id = next_tool_call_id();
                    pending.push((id.clone(), c.function.name.clone()));
                    json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": c.function.name,
                            "arguments": if c.function.arguments.is_string() {
                                c.function.arguments.clone()
                            } else {
                                Value::String(c.function.arguments.to_string())
                            }
                        }
                    })
                })
                .collect();
            om.insert("tool_calls".into(), Value::Array(mapped));
        }
        out.push(Value::Object(om));
    }
    out
}

/// 构造发往 llama-server /v1/chat/completions 的完整请求体。
///
/// - 参数 req：Ollama chat 请求
/// - 参数 stream：实际流式开关
/// - 返回：OpenAI 请求 JSON
pub fn build_openai_chat_request(req: &OllamaChatRequest, stream: bool) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": to_openai_messages(&req.messages, None),
        "stream": stream,
    });
    if stream {
        // 流式需要逐 token 统计（usage 末包携带）
        body["stream_options"] = json!({"include_usage": true});
    }
    if let Some(tools) = &req.tools {
        body["tools"] = json!(tools);
        // 工具调用自主决策（与原版一致）
        body["tool_choice"] = json!("auto");
    }
    // M35 D7：logprobs 透传（/v1 标准参数对：logprobs bool + top_logprobs int；
    // top_logprobs 仅在 logprobs=true 时有意义，附随下发）
    if let Some(lp) = req.logprobs {
        body["logprobs"] = json!(lp);
        if let Some(tlp) = req.top_logprobs {
            body["top_logprobs"] = json!(tlp);
        }
    }
    let obj = body.as_object_mut().unwrap();
    if let Value::Object(extra) = build_inference_params(
        req.options.as_ref(),
        req.format.as_ref(),
        req.think.as_ref(),
    ) {
        for (k, v) in extra {
            obj.insert(k, v);
        }
    }
    body
}

/// 构造发往 llama-server /v1/completions 的 raw 请求（generate raw 模式）。
///
/// - 参数 req：Ollama generate 请求
/// - 参数 stream：实际流式开关
/// - 返回：completions 请求 JSON
pub fn build_openai_completion_request(req: &OllamaGenerateRequest, stream: bool) -> Value {
    let mut body = json!({
        "model": req.model,
        "prompt": req.prompt,
        "stream": stream,
    });
    let obj = body.as_object_mut().unwrap();
    // format 不经 build_inference_params（其 response_format 映射对
    // /v1/completions 无效，见下方原生字段拼接）
    if let Value::Object(extra) =
        build_inference_params(req.options.as_ref(), None, req.think.as_ref())
    {
        for (k, v) in extra {
            obj.insert(k, v);
        }
    }
    // M35 D7：logprobs 透传（同 chat 通道参数对语义）
    if let Some(lp) = req.logprobs {
        obj.insert("logprobs".into(), json!(lp));
        if let Some(tlp) = req.top_logprobs {
            obj.insert("top_logprobs".into(), json!(tlp));
        }
    }
    // M28 碴18（R3-A）：raw 通道 format 改拼 llama.cpp 原生 json_schema 语法
    // 约束字段——/v1/completions 不支持 response_format（OpenAI 规范外，
    // 被 llama-server 静默忽略，原实现 format 全然失效）；"json" 模式以空
    // schema 表达「任意合法 JSON」，schema 对象以字符串形式承载（llama.cpp 语义）
    match req.format.as_ref() {
        Some(Value::String(s)) if s == "json" => {
            obj.insert("json_schema".into(), json!("{}"));
        }
        Some(Value::Bool(true)) => {
            obj.insert("json_schema".into(), json!("{}"));
        }
        Some(schema @ Value::Object(_)) => {
            obj.insert("json_schema".into(), json!(schema.to_string()));
        }
        _ => {}
    }
    body
}

/// finish_reason → Ollama done_reason（M22 对齐：tool_calls → tools）
fn finish_reason_to_done_reason(fr: &Value) -> Value {
    match fr.as_str() {
        Some("tool_calls") => json!("tools"),
        Some(other) => json!(other),
        None => json!("stop"),
    }
}

/// 思考块标签（迭代18 BUG-8 兜底剥离）
const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// 剥离 content 中残留的思考块（迭代18 BUG-8：think:false 兜底）。
///
/// 背景：reasoning_format=none 只关输出侧拆分，模板侧思考生成需
/// chat_template_kwargs.enable_thinking=false 抑制（根治路径）；老版本
/// 后端不支持该开关时 <think>…</think> 原文泄漏 content——本函数兜底
/// 剥离。仅剥离成对完整的块；未闭合保守保留（不破坏输出语义）。
///
/// - 参数 content：响应 content 原文
/// - 返回：剥离后的文本（发生剥离时首尾空白一并清理，未剥离原样返回）
pub fn strip_think_blocks(content: &str) -> String {
    if !content.contains(THINK_OPEN) {
        return content.to_string();
    }
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find(THINK_OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + THINK_OPEN.len()..];
        match after.find(THINK_CLOSE) {
            Some(end) => rest = &after[end + THINK_CLOSE.len()..],
            None => {
                // 未闭合：保守原样保留，避免截断合法输出
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// M187（迭代49）：llama-server tool_calls 的 function.arguments 字符串 →
/// Ollama 官方对象形态。官方 /api/chat 的 arguments 为 map 对象（如
/// {"city":"Tokyo"}），llama-server 按 OpenAI 协议返回 JSON 字符串；
/// 客户端（Roo Code 等）按官方形态消费时字符串直传引发解析失败。
/// parse 失败（空串/畸形/流式部分分片）宽容保留字符串原值，避免二次
/// 伤害（流式完整重组不在本轮范围，形态差异已记入架构文档注记）。
///
/// - 参数 raw：上游 function.arguments 值（预期字符串形态）
/// - 返回：解析成功为 JSON 值（对象/数组等），失败为原值克隆
fn tool_call_arguments_to_ollama(raw: &Value) -> Value {
    match raw
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        Some(parsed) => parsed,
        None => raw.clone(),
    }
}

/// 流式 JSON 顶层对象前缀解析三态（迭代51 M195 建立，迭代55 M206 三态
/// 改造 2026-09-12 23-50，Q2-A 勘误裁决 23:46）：M195 曾按「官方
/// json.Accumulator 增量发射」实现逐键增量对象下发，经官方 docs/api.md
/// 流式示例（tool_calls 单 chunk 完整对象）+ routes.go builtinParser
/// 实证——官方 Accumulator 为服务端内部累积器，客户端下发形态是完整
/// 解析后单事件一次性完整对象；增量形态致 open-webui 等按官方语义
/// （tool_calls.extend + **arguments 解包）消费的客户端取参失败
/// （{}×N 残缺条目）。本函数不再产出部分落定集，仅判定完整闭合；
/// 字符串内的 `,`/`}`/`{` 与转义引号正确跳过。
///
/// 实现：顶层边界扫描（深度计数）定位顶层闭合 `}`，截断后交 serde_json
/// 严格解析——借 serde 正确性免去手写转义/嵌套全量状态机，扫描器只
/// 承担边界判定单一职责。顶层闭合但 serde 解析失败（畸形 JSON）按
/// Incomplete 处理：保守不下发残缺，流末丢弃。
///
/// - 参数 s：跨片累积的 arguments 原始串
/// - 返回：三态判定（NotObject 畸形 / Incomplete 未闭合 / Complete 完整）
fn parse_complete_prefix(s: &str) -> PrefixParseOutcome {
    let t = s.trim_start();
    if !t.starts_with('{') {
        return PrefixParseOutcome::NotObject;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in t.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && c == '}' {
                    // 顶层闭合：首个完整对象段（后续多余字符宽容忽略）
                    return match serde_json::from_str::<Value>(&t[..=i]) {
                        Ok(Value::Object(m)) => PrefixParseOutcome::Complete(m),
                        _ => PrefixParseOutcome::Incomplete,
                    };
                }
            }
            _ => {}
        }
    }
    PrefixParseOutcome::Incomplete
}

/// 流式 JSON 顶层对象前缀解析三态结果（迭代55 M206）。
enum PrefixParseOutcome {
    /// 非 `{` 起头畸形形态（M187 兼容透传依据）
    NotObject,
    /// 已起头未顶层闭合（分片继续累积，本片不下发）
    Incomplete,
    /// 顶层闭合且 serde 严格解析通过的完整对象（一次性下发）
    Complete(serde_json::Map<String, Value>),
}

/// 单个 tool_call 的跨片累积桶（M195 建立，迭代55 M206 改造）
#[derive(Default)]
struct ArgsBucket {
    /// 已累积的 arguments 原始串（各分片顺序拼接）
    accumulated: String,
    /// 首包/含名包捕获的 function.name（完整落定时组装进下发条目）
    name: Option<String>,
    /// 已完整落定标记（落定后同桶后续分片防御性忽略）
    settled: bool,
}

/// 单片 tool_call 的下发动作判定（迭代55 M206）。
#[derive(Debug)]
pub enum ToolCallEmit {
    /// 未闭合：本片剥除该 tool_call（不下发残缺条目）
    Suppress,
    /// 完整闭合：以完整 name + arguments 组装一次性下发（官方形态）
    Emit {
        /// function.name（首包桶记录优先，本包名兜底）
        name: String,
        /// 完整落定的 arguments 对象
        arguments: Value,
    },
    /// 畸形（非对象起头）：M187 兼容——本片原样透传
    Passthrough,
}

/// M195（迭代51）→ 迭代55 M206 改造（Q2-A 勘误裁决 2026-09-12 23:46）：
/// 流式 tool_calls arguments 跨片重组状态机——按 tool_call index/name
/// 分桶累积字符串分片，**完整闭合后单次下发完整对象**（官方 Ollama
/// 实证下发形态：docs/api.md 流式示例单 chunk 完整 tool_calls；服务端
/// json.Accumulator/builtinParser 为内部累积器不外发增量）。未闭合分片
/// 剥除不下发（Q3-A 裁决：剥除后空事件保留）；非对象畸形形态宽容回退
/// 本片原值（M187 兼容路径不产生二次伤害）。
pub struct ToolCallArgsAccumulator {
    /// 定位键（`#index` 优先，无 index 用 function.name）→ 累积桶
    buckets: std::collections::HashMap<String, ArgsBucket>,
}

impl ToolCallArgsAccumulator {
    /// 构造空状态机（每条 SSE 流一个实例，随流生命周期创建销毁）。
    pub fn new() -> Self {
        Self {
            buckets: std::collections::HashMap::new(),
        }
    }

    /// 摄入一个 tool_call 分片，判定该 tool_call 本片的下发动作。
    ///
    /// - 参数 idx：OpenAI tool_calls[].index（llama-server 恒携带；
    ///   缺位时回退 name 定位）
    /// - 参数 name：function.name（首包携带；含名分片持续刷新桶记录）
    /// - 参数 raw：本片 arguments 原值（字符串分片或罕见单片完整对象——
    ///   对象统一序列化并入累积，保持桶状态一致性）
    /// - 返回：三态下发动作（Suppress 剥除 / Emit 完整下发 / Passthrough 透传）
    pub fn ingest(&mut self, idx: Option<u64>, name: &str, raw: &Value) -> ToolCallEmit {
        let key = match idx {
            Some(i) => format!("#{i}"),
            None => name.to_string(),
        };
        let bucket = self.buckets.entry(key).or_default();
        if !name.is_empty() {
            bucket.name = Some(name.to_string());
        }
        // 已落定桶的后续分片：防御性忽略（正常流闭合后不再发同桶分片）
        if bucket.settled {
            return ToolCallEmit::Suppress;
        }
        match raw {
            Value::String(s) => bucket.accumulated.push_str(s),
            other => bucket.accumulated.push_str(&other.to_string()),
        }
        // 空累积（首包空串常见）：剥除（官方不发 name-only 残缺条目）
        if bucket.accumulated.trim().is_empty() {
            return ToolCallEmit::Suppress;
        }
        match parse_complete_prefix(&bucket.accumulated) {
            PrefixParseOutcome::Complete(m) => {
                bucket.settled = true;
                ToolCallEmit::Emit {
                    name: bucket.name.clone().unwrap_or_else(|| name.to_string()),
                    arguments: Value::Object(m),
                }
            }
            PrefixParseOutcome::Incomplete => ToolCallEmit::Suppress,
            PrefixParseOutcome::NotObject => ToolCallEmit::Passthrough,
        }
    }
}

impl Default for ToolCallArgsAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// OpenAI 非流式 chat 响应 → Ollama /api/chat 响应。
///
/// - 参数 model：模型名
/// - 参数 openai：llama-server 响应 JSON
/// - 返回：Ollama 响应 JSON
pub fn openai_chat_response_to_ollama(model: &str, openai: &Value) -> Value {
    let choice = &openai["choices"][0];
    let msg = &choice["message"];
    let mut ollama_msg = json!({
        "role": "assistant",
        "content": msg["content"].as_str().unwrap_or_default(),
    });
    if let Some(rc) = msg["reasoning_content"].as_str() {
        ollama_msg["thinking"] = json!(rc);
    } else {
        // 迭代18 BUG-8 兜底：thinking 未拆分且 content 残留思考块 →
        // 剥离（老版本后端无 enable_thinking 支持时的防护；非 thinking
        // 模型走 contains 快速路径近零开销）
        ollama_msg["content"] = json!(strip_think_blocks(
            msg["content"].as_str().unwrap_or_default()
        ));
    }
    if let Some(calls) = msg["tool_calls"].as_array() {
        let mapped: Vec<Value> = calls
            .iter()
            .map(|c| {
                json!({
                    "function": {
                        "name": c["function"]["name"],
                        // M187：arguments 字符串 → Ollama 官方对象形态
                        "arguments": tool_call_arguments_to_ollama(&c["function"]["arguments"]),
                    }
                })
            })
            .collect();
        ollama_msg["tool_calls"] = Value::Array(mapped);
    }
    let usage = &openai["usage"];
    let eval_count = usage["completion_tokens"].as_u64().unwrap_or(0);
    let mut out = json!({
        "model": model,
        "created_at": crate::registry::now_rfc3339(),
        "message": ollama_msg,
        "done": true,
        "done_reason": finish_reason_to_done_reason(&choice["finish_reason"]),
        "total_duration": 0,
        "load_duration": 0,
        "prompt_eval_count": usage["prompt_tokens"].as_u64().unwrap_or(0),
        "prompt_eval_duration": 0,
        "eval_count": eval_count,
        "eval_duration": 0,
    });
    // M35 D7：logprobs 数组回填（OpenAI choices[].logprobs.content 与官方
    // Ollama Logprob 数组形态同构：token/logprob/bytes/top_logprobs）
    if let Some(lp) = choice["logprobs"]["content"].as_array() {
        out["logprobs"] = json!(lp);
    }
    out
}

/// OpenAI 流式 chunk → Ollama NDJSON 事件（chat 形态：message 增量）。
/// 末包（finish_reason 或 usage 出现）返回 done 事件。
///
/// - 参数 model：模型名
/// - 参数 chunk：SSE data JSON
/// - 返回：0~2 个 NDJSON 事件（正常 1 个，含统计的末包 1 个合并）
pub fn openai_chunk_to_ollama_chat_events(model: &str, chunk: &Value) -> Vec<Value> {
    let choice = &chunk["choices"].get(0);
    let usage = chunk.get("usage").filter(|u| !u.is_null());
    let is_final = usage.is_some()
        || choice
            .map(|c| !c["finish_reason"].is_null())
            .unwrap_or(false);

    let mut msg = json!({"role": "assistant"});
    if let Some(choice) = choice {
        let delta = &choice["delta"];
        if let Some(content) = delta["content"].as_str() {
            msg["content"] = json!(content);
        }
        // 迭代42 D8（O-3 清偿 2026-09-11）：空增量补空串——官方流式事件
        // message.content 恒为字符串（含 role-only 首 chunk）；原缺键形态
        // 与官方 "" 不符（generate 侧 unwrap_or_default 已是 ""，chat 侧
        // 补齐同口径；usage-only 空 choices 包不在此路径，终包合并不动）
        if msg.get("content").is_none() {
            msg["content"] = json!("");
        }
        if let Some(rc) = delta["reasoning_content"].as_str() {
            msg["thinking"] = json!(rc);
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            let mapped: Vec<Value> = calls
                .iter()
                .map(|c| {
                    json!({
                        "function": {
                            "name": c["function"]["name"],
                            // M187：arguments 字符串 → Ollama 官方对象形态
                            // （流式部分分片 parse 失败回退字符串原值）
                            "arguments": tool_call_arguments_to_ollama(
                                &c["function"]["arguments"]
                            ),
                        }
                    })
                })
                .collect();
            msg["tool_calls"] = Value::Array(mapped);
        }
    }
    let mut event = json!({
        "model": model,
        "created_at": crate::registry::now_rfc3339(),
        "message": msg,
        "done": is_final,
    });
    // M35 D7：流式 logprobs 附随（chunk 级增量，形态同非流式）
    if let Some(choice) = choice {
        if let Some(lp) = choice["logprobs"]["content"].as_array() {
            event["logprobs"] = json!(lp);
        }
    }
    if is_final {
        event["done_reason"] = choice
            .map(|c| finish_reason_to_done_reason(&c["finish_reason"]))
            .filter(|v| {
                v.as_str() != Some("stop")
                    || choice
                        .map(|c| !c["finish_reason"].is_null())
                        .unwrap_or(false)
            })
            .unwrap_or(json!("stop"));
        // M33 碴3：usage 存在才写统计键且 as_u64 兜底 0——原空对象
        // `u["completion_tokens"].clone()` 序列化出 null（非 0 也非缺省），
        // 对齐 generate 形态「仅 usage 存在时写键」既有语义
        if let Some(u) = usage {
            event["eval_count"] = json!(u["completion_tokens"].as_u64().unwrap_or(0));
            event["prompt_eval_count"] = json!(u["prompt_tokens"].as_u64().unwrap_or(0));
        }
    }
    vec![event]
}

/// OpenAI 流式 chunk → Ollama NDJSON 事件（generate 形态：response 增量）。
///
/// - 参数 model：模型名
/// - 参数 chunk：SSE data JSON（completions 形态）
/// - 返回：NDJSON 事件
pub fn openai_chunk_to_ollama_generate_event(model: &str, chunk: &Value) -> Value {
    let text = chunk["choices"][0]["text"].as_str().unwrap_or_default();
    let is_final = chunk["choices"][0]
        .get("finish_reason")
        .map(|f| !f.is_null())
        .unwrap_or(false)
        || chunk.get("usage").map(|u| !u.is_null()).unwrap_or(false);
    let mut event = json!({
        "model": model,
        "created_at": crate::registry::now_rfc3339(),
        "response": text,
        "done": is_final,
    });
    // M35 D7：流式 logprobs 附随（completions 形态同构 choices[].logprobs.content）
    if let Some(lp) = chunk["choices"][0]["logprobs"]["content"].as_array() {
        event["logprobs"] = json!(lp);
    }
    if is_final {
        event["done_reason"] = finish_reason_to_done_reason(&chunk["choices"][0]["finish_reason"]);
        event["context"] = json!([]);
        if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
            // M33 碴3：as_u64 兜底 0（原 clone 在字段缺省/非数字时序列化 null）
            event["eval_count"] = json!(u["completion_tokens"].as_u64().unwrap_or(0));
            event["prompt_eval_count"] = json!(u["prompt_tokens"].as_u64().unwrap_or(0));
        }
    }
    event
}

/// OpenAI 非流式 completions 响应 → Ollama generate 响应
pub fn openai_completion_response_to_ollama(model: &str, openai: &Value) -> Value {
    let text = openai["choices"][0]["text"].as_str().unwrap_or_default();
    json!({
        "model": model,
        "created_at": crate::registry::now_rfc3339(),
        "response": text,
        "done": true,
        "done_reason": finish_reason_to_done_reason(&openai["choices"][0]["finish_reason"]),
        "context": [],
        "eval_count": openai["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        "prompt_eval_count": openai["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 迭代51 M195 建立 / 迭代55 M206 三态改写：流式 JSON 前缀解析——
    /// 未闭合一律 Incomplete（不再有部分落定集）、顶层闭合 Complete 全键、
    /// 非对象起头 NotObject；字符串内分隔符跳过、嵌套整体闭合。
    /// 来源：Q2-A 勘误裁决 2026-09-12 23:46（官方完整下发形态实证闭环）。
    #[test]
    fn tool_args_prefix_parse_matrix() {
        use PrefixParseOutcome::{self, *};
        let keys = |o: PrefixParseOutcome| match o {
            Complete(m) => Some(m.keys().cloned().collect::<Vec<_>>()),
            Incomplete => None,
            NotObject => panic!("预期非 NotObject 形态"),
        };
        // 未闭合前缀（半个 key / 值未闭合 / 无确认符 / 逗号后尾键未闭合）：
        // 全部 Incomplete——增量部分落定语义已废止
        assert!(matches!(parse_complete_prefix(r#"{"a"#), Incomplete));
        assert!(matches!(parse_complete_prefix(r#"{"a":"v"#), Incomplete));
        assert!(matches!(parse_complete_prefix(r#"{"a":1"#), Incomplete));
        assert!(matches!(
            parse_complete_prefix(r#"{"a":1,"b":"x"#),
            Incomplete
        ));
        // 顶层闭合：Complete 全键
        assert_eq!(
            keys(parse_complete_prefix(r#"{"a":1,"b":"x"}"#)),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        // 字符串内的 , } { 与转义引号跳过（未闭合）
        assert!(matches!(
            parse_complete_prefix(r#"{"a":"x,y}z{\"k":1""#),
            Incomplete
        ));
        assert!(matches!(
            parse_complete_prefix(r#"{"a":"x,y}b","c":2"#),
            Incomplete
        ));
        // 字符串内假闭合跳过后真闭合：Complete
        assert_eq!(
            keys(parse_complete_prefix(r#"{"a":"x,y}b","c":2}"#)),
            Some(vec!["a".to_string(), "c".to_string()])
        );
        // 嵌套对象/数组：整体闭合后才 Complete
        assert!(matches!(
            parse_complete_prefix(r#"{"a":{"x":1},"b":[1,2]"#),
            Incomplete
        ));
        assert_eq!(
            keys(parse_complete_prefix(r#"{"a":{"x":1},"b":[1,2]}"#)),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        // 非对象形态：NotObject（畸形透传依据）
        assert!(matches!(parse_complete_prefix("not json"), NotObject));
        assert!(matches!(parse_complete_prefix(""), NotObject));
        assert!(matches!(parse_complete_prefix(r#"[1,2]"#), NotObject));
        // 空对象：{ 未闭合 → {} 闭合 Complete（空键集）
        assert!(matches!(parse_complete_prefix("{"), Incomplete));
        assert_eq!(keys(parse_complete_prefix("{}")), Some(vec![]));
    }

    /// 迭代55 M206：跨片重组完整落定序列——未闭合分片全 Suppress（剥除
    /// 不下发），顶层闭合单次 Emit 完整对象（name 取首包桶记录）；
    /// settled 后续分片防御性忽略；畸形回退透传；多 index 分桶互不串扰。
    /// 锚定用户报错场景（2026-09-12 23:38）：{}×N 残缺条目根治。
    #[test]
    fn tool_args_accumulator_streaming_reassembly() {
        use ToolCallEmit::*;
        let mut acc = ToolCallArgsAccumulator::new();
        // 首包 name 包（arguments=""）：Suppress 剥除，name 入桶
        assert!(matches!(
            acc.ingest(Some(0), "get_weather", &json!("")),
            Suppress
        ));
        // 参数分片未闭合（分片边界不落在引号上——raw string 内容以 "
        // 结尾会被定界符吞掉，实测词法陷阱）：全部 Suppress
        assert!(matches!(
            acc.ingest(Some(0), "", &json!(r#"{"city":"T"#)),
            Suppress
        ));
        assert!(matches!(acc.ingest(Some(0), "", &json!("ok")), Suppress));
        // 顶层闭合：单次 Emit 完整对象（name 取首包桶记录）
        match acc.ingest(Some(0), "", &json!(r#"yo","unit":1}"#)) {
            Emit { name, arguments } => {
                assert_eq!(name, "get_weather", "name 取首包桶记录");
                assert_eq!(arguments, json!({"city": "Tokyo", "unit": 1}));
            }
            other => panic!("预期 Emit，实际 {other:?}"),
        }
        // 落定后同桶后续分片：防御性忽略
        assert!(matches!(
            acc.ingest(Some(0), "", &json!("garbage")),
            Suppress
        ));

        // 另一 tool_call（index 1）并行分片：互不串扰，各自闭合各自 Emit
        assert!(matches!(
            acc.ingest(Some(1), "get_time", &json!(r#"{"tz":"+8"#)),
            Suppress
        ));
        match acc.ingest(Some(1), "", &json!(r#""}"#)) {
            Emit { name, arguments } => {
                assert_eq!(name, "get_time");
                assert_eq!(arguments, json!({"tz": "+8"}));
            }
            other => panic!("预期 Emit，实际 {other:?}"),
        }

        // 畸形流（模型输出非 JSON 文本）：本片透传（M187 兼容）
        assert!(matches!(
            acc.ingest(Some(2), "broken", &json!("not-json")),
            Passthrough
        ));

        // 无 index：name 定位（老形态兼容）
        match acc.ingest(None, "by_name", &json!(r#"{"k":"v"}"#)) {
            Emit { name, arguments } => {
                assert_eq!(name, "by_name");
                assert_eq!(arguments, json!({"k": "v"}));
            }
            other => panic!("预期 Emit，实际 {other:?}"),
        }
    }

    /// keep_alive 全形态解析
    #[test]
    fn keep_alive_parsing() {
        assert_eq!(parse_keep_alive(Some(&json!(0))), std::time::Duration::ZERO);
        assert_eq!(
            parse_keep_alive(Some(&json!(120))),
            std::time::Duration::from_secs(120)
        );
        assert_eq!(
            parse_keep_alive(Some(&json!("5m"))),
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            parse_keep_alive(Some(&json!("1h"))),
            std::time::Duration::from_secs(3600)
        );
        assert_eq!(parse_keep_alive(None), crate::scheduler::DEFAULT_KEEP_ALIVE);
    }

    /// images base64 → image_url data URI
    #[test]
    fn multimodal_message_mapping() {
        let msgs = vec![OllamaMessage {
            role: "user".into(),
            content: "看图".into(),
            images: vec!["QUJD".into()],
            tool_calls: None,
            tool_name: None,
            thinking: None,
        }];
        let out = to_openai_messages(&msgs, None);
        let content = &out[0]["content"];
        assert!(content.is_array(), "含图消息 content 必须为分段数组");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

    /// format 三形态 → response_format
    #[test]
    fn format_mapping() {
        let p = build_inference_params(None, Some(&json!("json")), None);
        assert_eq!(p["response_format"]["type"], "json_object");
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let p = build_inference_params(None, Some(&schema), None);
        assert_eq!(
            p["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
    }

    /// options 数值映射（num_predict → max_tokens；M22 扩展参数透传）
    #[test]
    fn options_mapping() {
        let p = build_inference_params(
            Some(
                &json!({"temperature": 0.7, "num_predict": 64, "stop": ["<|end|>"], "presence_penalty": 0.1}),
            ),
            None,
            None,
        );
        assert_eq!(p["temperature"], 0.7);
        assert_eq!(p["max_tokens"], 64);
        assert_eq!(p["stop"][0], "<|end|>");
        assert_eq!(p["presence_penalty"], 0.1, "M22 扩展采样参数必须透传");
    }

    /// M21 D4b；M35 D9 四档 + b10605 值域修正（deep_think→auto）：think 分级映射
    #[test]
    fn think_level_mapping() {
        let p = build_inference_params(None, None, Some(&json!(false)));
        assert_eq!(p["reasoning_format"], "none");
        // 迭代18 BUG-8：模板侧思考抑制开关（bool 形态，字符串形态 server 端 500）
        assert_eq!(p["chat_template_kwargs"]["enable_thinking"], false);
        let p = build_inference_params(None, None, Some(&json!(true)));
        assert_eq!(p["reasoning_format"], "auto");
        let p = build_inference_params(None, None, Some(&json!("low")));
        assert_eq!(p["reasoning_format"], "auto");
        assert_eq!(p["chat_template_kwargs"]["reasoning_effort"], "low");
        let p = build_inference_params(None, None, Some(&json!("high")));
        assert_eq!(p["chat_template_kwargs"]["reasoning_effort"], "high");
        // M35 D9：medium/max 不再落默认分支被静默忽略——四档全覆盖
        let p = build_inference_params(None, None, Some(&json!("medium")));
        assert_eq!(p["reasoning_format"], "auto");
        assert_eq!(p["chat_template_kwargs"]["reasoning_effort"], "medium");
        let p = build_inference_params(None, None, Some(&json!("max")));
        assert_eq!(p["reasoning_format"], "auto");
        assert_eq!(p["chat_template_kwargs"]["reasoning_effort"], "max");
        let p = build_inference_params(None, None, None);
        assert!(p.get("reasoning_format").is_none(), "未设置时不注入");
    }

    /// 迭代18 BUG-8：think:false 兜底剥离——整块/多块/未闭合/无块形态
    #[test]
    fn strip_think_blocks_variants() {
        // 整块剥离 + 首尾空白清理
        assert_eq!(
            strip_think_blocks("<think>\n思考中\n</think>\n\n答案"),
            "答案"
        );
        // 中置块：保留前后正文
        assert_eq!(strip_think_blocks("前文<think>x</think>后文"), "前文后文");
        // 多块全部剥离
        assert_eq!(
            strip_think_blocks("<think>a</think>B<think>c</think>D"),
            "BD"
        );
        // 未闭合：保守原样保留
        assert_eq!(strip_think_blocks("答案<think>未完"), "答案<think>未完");
        // 无块快速路径 + 空串
        assert_eq!(strip_think_blocks("普通答案"), "普通答案");
        assert_eq!(strip_think_blocks(""), "");
    }

    /// 迭代18 BUG-8：响应兜底——thinking 缺失且 content 残留思考块时剥离；
    /// thinking 正常拆分时 content 原样（不误伤）
    #[test]
    fn chat_response_strips_orphan_think_block() {
        let openai = json!({
            "choices": [{"message": {"role": "assistant",
                "content": "<think>\n我该想想\n</think>\n\n最终答案",
                "finish_reason": "stop"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2}
        });
        let out = openai_chat_response_to_ollama("m", &openai);
        assert_eq!(out["message"]["content"], "最终答案");
        assert!(out["message"].get("thinking").is_none());

        let openai2 = json!({
            "choices": [{"message": {"role": "assistant",
                "content": "答案 <think> 字面演示",
                "reasoning_content": "已拆分的思考", "finish_reason": "stop"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2}
        });
        let out2 = openai_chat_response_to_ollama("m", &openai2);
        assert_eq!(out2["message"]["thinking"], "已拆分的思考");
        assert_eq!(out2["message"]["content"], "答案 <think> 字面演示");
    }

    /// M21 D4a：assistant tool_calls 与 tool 消息的 tool_call_id 配对
    #[test]
    fn tool_call_id_pairing() {
        let msgs = vec![
            OllamaMessage {
                role: "user".into(),
                content: "天气如何".into(),
                images: vec![],
                tool_calls: None,
                tool_name: None,
                thinking: None,
            },
            OllamaMessage {
                role: "assistant".into(),
                content: String::new(),
                images: vec![],
                tool_calls: Some(vec![
                    OllamaToolCall {
                        function: OllamaToolCallFunction {
                            name: "get_weather".into(),
                            arguments: json!({"city": "北京"}),
                        },
                    },
                    OllamaToolCall {
                        function: OllamaToolCallFunction {
                            name: "get_time".into(),
                            arguments: json!({}),
                        },
                    },
                ]),
                tool_name: None,
                thinking: None,
            },
            // tool 结果乱序提供（time 在前），靠 tool_name 精确匹配
            OllamaMessage {
                role: "tool".into(),
                content: "12:00".into(),
                images: vec![],
                tool_calls: None,
                tool_name: Some("get_time".into()),
                thinking: None,
            },
            OllamaMessage {
                role: "tool".into(),
                content: "晴".into(),
                images: vec![],
                tool_calls: None,
                tool_name: Some("get_weather".into()),
                thinking: None,
            },
        ];
        let out = to_openai_messages(&msgs, None);
        // assistant 消息的两个 call 有不同 id
        let calls = out[1]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2);
        let id_weather = calls[0]["id"].as_str().unwrap().to_string();
        let id_time = calls[1]["id"].as_str().unwrap().to_string();
        assert_ne!(id_weather, id_time, "同消息多个调用 id 必须互异");
        // tool 消息按 tool_name 匹配到正确 id
        assert_eq!(out[2]["role"], "tool");
        assert_eq!(out[2]["tool_call_id"], id_time.as_str());
        assert_eq!(out[3]["tool_call_id"], id_weather.as_str());
    }

    /// M21 D4a：无 tool_name 时按序消费（队首配对）
    #[test]
    fn tool_call_id_sequential_fallback() {
        let msgs = vec![
            OllamaMessage {
                role: "assistant".into(),
                content: String::new(),
                images: vec![],
                tool_calls: Some(vec![OllamaToolCall {
                    function: OllamaToolCallFunction {
                        name: "a".into(),
                        arguments: json!({}),
                    },
                }]),
                tool_name: None,
                thinking: None,
            },
            OllamaMessage {
                role: "tool".into(),
                content: "结果".into(),
                images: vec![],
                tool_calls: None,
                tool_name: None,
                thinking: None,
            },
        ];
        let out = to_openai_messages(&msgs, None);
        assert_eq!(out[1]["tool_call_id"], out[0]["tool_calls"][0]["id"]);
    }

    /// M22：finish_reason → done_reason（tool_calls → tools）
    #[test]
    fn done_reason_mapping() {
        assert_eq!(
            finish_reason_to_done_reason(&json!("tool_calls")),
            json!("tools")
        );
        assert_eq!(
            finish_reason_to_done_reason(&json!("length")),
            json!("length")
        );
        assert_eq!(finish_reason_to_done_reason(&json!(null)), json!("stop"));
    }

    /// requested_num_ctx 提取与下限保护
    #[test]
    fn request_ctx_extraction() {
        assert_eq!(
            requested_num_ctx(Some(&json!({"num_ctx": 8192}))),
            Some(8192)
        );
        assert_eq!(
            requested_num_ctx(Some(&json!({"num_ctx": 100}))),
            Some(512),
            "低于 512 钳制"
        );
        assert_eq!(requested_num_ctx(Some(&json!({}))), None);
        assert_eq!(requested_num_ctx(None), None);
    }

    /// 迭代43 M162：超窗错误体判型解析与扩窗目标值（R1-A 裁决 2026-09-11 23:03：
    /// +1024 余量、512 对齐、GGUF 训练长度封顶、512 下限）
    #[test]
    fn exceed_context_error_parsing_and_expansion() {
        // 用户报告原始形态（2026-09-11 22:50，b10883 实证）
        let body = r#"{"error":{"code":400,"message":"request (6676 tokens) exceeds the available context size (4096 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":6676,"n_ctx":4096}}"#;
        assert_eq!(parse_exceed_context_error(body), Some(6676));
        // 非超窗错误体 / type 不匹配 / 非 JSON：None（不触发扩窗）
        assert_eq!(
            parse_exceed_context_error(r#"{"error":"model not found"}"#),
            None
        );
        assert_eq!(
            parse_exceed_context_error(r#"{"error":{"type":"api_error","n_prompt_tokens":10}}"#),
            None
        );
        assert_eq!(parse_exceed_context_error("not json"), None);
        // R1-A：6676 + 1024 = 7700 → 向上对齐 512 → 8192；无封顶
        assert_eq!(expanded_ctx_target(6676, None), 8192);
        // 恰好对齐边界：5120 + 1024 = 6144（512 的整数倍，不再上取）
        assert_eq!(expanded_ctx_target(5120, None), 6144);
        // 封顶命中：训练长度 4096 < 计算值 8192 → 4096
        assert_eq!(expanded_ctx_target(6676, Some(4096)), 4096);
        // 封顶高于计算值：不放大（封顶是上界不是目标）
        assert_eq!(expanded_ctx_target(100, Some(65536)), 1536);
        // 封顶低于下限：训练长度 100 → 下限 512（不发出不可用窗口）
        assert_eq!(expanded_ctx_target(6676, Some(100)), 512);
        // 极小输入下限保护：0 + 1024 = 1024 已对齐
        assert_eq!(expanded_ctx_target(0, None), 1024);
    }

    /// 流式 chunk → chat NDJSON（含末包统计）
    #[test]
    fn chunk_to_chat_events() {
        let mid = json!({"choices": [{"delta": {"content": "你"}}]});
        let events = openai_chunk_to_ollama_chat_events("m", &mid);
        assert_eq!(events[0]["message"]["content"], "你");
        assert!(!events[0]["done"].as_bool().unwrap());

        let last = json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 5, "completion_tokens": 9}});
        let events = openai_chunk_to_ollama_chat_events("m", &last);
        assert!(events[0]["done"].as_bool().unwrap());
        assert_eq!(events[0]["eval_count"], 9);
        assert_eq!(events[0]["done_reason"], "stop");

        let tools = json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]});
        let events = openai_chunk_to_ollama_chat_events("m", &tools);
        assert_eq!(
            events[0]["done_reason"], "tools",
            "tool_calls 必须映射为 tools"
        );
    }

    /// M187（迭代49）：arguments 字符串 → Ollama 官方对象形态三态——
    /// 合法 JSON 字符串对象化、畸形/部分分片宽容回退字符串原值
    #[test]
    fn tool_call_arguments_objectified() {
        // 非流式：合法 JSON 字符串 → 对象（对齐官方 map 形态）
        let resp = json!({"choices": [{"message": {"role": "assistant", "content": "",
            "tool_calls": [{"function": {"name": "get_weather",
                "arguments": "{\"city\": \"Tokyo\", \"unit\": \"c\"}"}}]},
            "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 3, "completion_tokens": 7}});
        let out = openai_chat_response_to_ollama("m", &resp);
        assert_eq!(
            out["message"]["tool_calls"][0]["function"]["arguments"],
            json!({"city": "Tokyo", "unit": "c"}),
            "字符串必须对象化为官方 map 形态"
        );
        // 空对象字符串 → 空对象（非空字符串）
        let resp = json!({"choices": [{"message": {"role": "assistant", "content": "",
            "tool_calls": [{"function": {"name": "f", "arguments": "{}"}}]},
            "finish_reason": "tool_calls"}]});
        let out = openai_chat_response_to_ollama("m", &resp);
        assert_eq!(
            out["message"]["tool_calls"][0]["function"]["arguments"],
            json!({}),
            "空对象字符串对象化为空对象"
        );
        // 畸形字符串（用户报错原始形态：截断仅剩 "{"）→ 回退字符串原值
        let resp = json!({"choices": [{"message": {"role": "assistant", "content": "",
            "tool_calls": [{"function": {"name": "f", "arguments": "{"}}]},
            "finish_reason": "tool_calls"}]});
        let out = openai_chat_response_to_ollama("m", &resp);
        assert_eq!(
            out["message"]["tool_calls"][0]["function"]["arguments"],
            json!("{"),
            "畸形字符串宽容保留原值，不产生二次伤害"
        );
        // 流式：单片完整 JSON → 对象；部分分片（token 级片段）→ 字符串原值
        let chunk = json!({"choices": [{"delta": {"tool_calls": [
            {"function": {"name": "get_weather", "arguments": "{\"city\": \"Tokyo\"}"}}]}}]});
        let events = openai_chunk_to_ollama_chat_events("m", &chunk);
        assert_eq!(
            events[0]["message"]["tool_calls"][0]["function"]["arguments"],
            json!({"city": "Tokyo"}),
            "流式单片中完整 JSON 同样对象化"
        );
        let frag = json!({"choices": [{"delta": {"tool_calls": [
            {"function": {"name": "", "arguments": "{\"ci"}}]}}]});
        let events = openai_chunk_to_ollama_chat_events("m", &frag);
        assert_eq!(
            events[0]["message"]["tool_calls"][0]["function"]["arguments"],
            json!("{\"ci"),
            "流式部分分片 parse 失败回退字符串原值（客户端累积）"
        );
    }

    /// M33 碴3：末包仅 finish_reason 无 usage——统计键不得出现（原空对象
    /// clone 出 null）；usage 存在时为数字；字段非数字兜底 0 不出 null
    #[test]
    fn final_chunk_without_usage_omits_stats() {
        let last = json!({"choices": [{"delta": {}, "finish_reason": "stop"}]});
        let ev = &openai_chunk_to_ollama_chat_events("m", &last)[0];
        assert!(ev["done"].as_bool().unwrap(), "finish_reason 存在即终包");
        assert!(
            ev.get("eval_count").is_none(),
            "usage 缺失不得写统计键：{ev}"
        );
        assert!(ev.get("prompt_eval_count").is_none());
        let gen = openai_chunk_to_ollama_generate_event("m", &last);
        assert!(
            gen.get("eval_count").is_none(),
            "generate 形态同语义：{gen}"
        );

        // usage 存在：数字直写（既有行为回归锚定）
        let with_usage = json!({
            "choices": [{"delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 9}
        });
        let ev = &openai_chunk_to_ollama_chat_events("m", &with_usage)[0];
        assert_eq!(ev["eval_count"], 9);
        assert_eq!(ev["prompt_eval_count"], 5);

        // usage 存在但字段非数字：兜底 0，不得出 null
        let weird = json!({
            "choices": [{"delta": {}, "finish_reason": "stop"}],
            "usage": {"completion_tokens": null}
        });
        let ev = &openai_chunk_to_ollama_chat_events("m", &weird)[0];
        assert_eq!(ev["eval_count"], 0, "非数字字段兜底 0");
        assert_eq!(ev["prompt_eval_count"], 0, "缺省字段兜底 0");
    }

    /// M28 碴8：tool_call id 进程级全局递增——两次独立调用（模拟两个请求）
    /// 的 id 不得重复（原缺陷：各自从 call-0 重开）
    #[test]
    fn tool_call_ids_unique_across_requests() {
        let mk = || OllamaMessage {
            role: "assistant".into(),
            content: String::new(),
            images: vec![],
            tool_calls: Some(vec![OllamaToolCall {
                function: OllamaToolCallFunction {
                    name: "f".into(),
                    arguments: json!({}),
                },
            }]),
            tool_name: None,
            thinking: None,
        };
        let first = to_openai_messages(&[mk()], None);
        let second = to_openai_messages(&[mk()], None);
        let id1 = first[0]["tool_calls"][0]["id"].as_str().unwrap();
        let id2 = second[0]["tool_calls"][0]["id"].as_str().unwrap();
        assert_ne!(id1, id2, "跨请求 id 必须全局递增不重复（{id1} vs {id2}）");
    }

    /// M28 碴18（R3-A）：raw 通道 format 必须拼 llama.cpp 原生 json_schema
    /// 字段而非 response_format（后者被 /v1/completions 静默忽略）
    #[test]
    fn raw_format_uses_native_json_schema() {
        let req = OllamaGenerateRequest {
            model: "m".into(),
            prompt: "p".into(),
            images: None,
            format: Some(json!("json")),
            options: None,
            system: None,
            template: None,
            raw: Some(true),
            stream: None,
            keep_alive: None,
            think: None,
            context: None,
            suffix: None,
            logprobs: None,
            top_logprobs: None,
        };
        let body = build_openai_completion_request(&req, true);
        assert!(
            body.get("response_format").is_none(),
            "raw 通道不得再拼 response_format"
        );
        assert_eq!(
            body["json_schema"], "{}",
            "json 模式 = 空 schema（任意合法 JSON）"
        );

        let mut schema_req = req;
        schema_req.format = Some(json!({"type": "object"}));
        let body = build_openai_completion_request(&schema_req, true);
        assert_eq!(
            body["json_schema"],
            json!({"type": "object"}).to_string(),
            "schema 对象以字符串形式承载（llama.cpp 语义）"
        );
    }

    /// M28 碴6：duration 四字段注入（全部 >0 纳秒、load 透传）
    #[test]
    fn duration_fields_injected() {
        let mut ev = json!({"done": true});
        let start = std::time::Instant::now() - std::time::Duration::from_millis(30);
        let first_byte = start + std::time::Duration::from_millis(10);
        let load = std::time::Duration::from_millis(5);
        apply_duration_fields(&mut ev, start, first_byte, load);
        for key in [
            "total_duration",
            "load_duration",
            "prompt_eval_duration",
            "eval_duration",
        ] {
            assert!(
                ev[key].as_u64().unwrap_or(0) > 0,
                "{key} 必须为正实测值（纳秒）：{ev}"
            );
        }
        assert_eq!(
            ev["load_duration"].as_u64().unwrap(),
            load.as_nanos() as u64
        );
    }

    /// M35 D12：mirostat 家族三参数白名单同名直传
    #[test]
    fn mirostat_family_passthrough() {
        let p = build_inference_params(
            Some(&json!({
                "mirostat": 2,
                "mirostat_eta": 0.1,
                "mirostat_tau": 5.0
            })),
            None,
            None,
        );
        assert_eq!(p["mirostat"], 2);
        assert_eq!(p["mirostat_eta"], 0.1);
        assert_eq!(p["mirostat_tau"], 5.0);
        // 缺省时不注入（既有语义回归锚定）
        let p = build_inference_params(Some(&json!({"temperature": 0.5})), None, None);
        assert!(p.get("mirostat").is_none());
    }

    /// M35 D8：assistant 历史 thinking → reasoning_content 映射
    #[test]
    fn thinking_maps_to_reasoning_content() {
        let msgs = vec![OllamaMessage {
            role: "assistant".into(),
            content: "答案是 42".into(),
            images: vec![],
            tool_calls: None,
            tool_name: None,
            thinking: Some("我先想一想……".into()),
        }];
        let out = to_openai_messages(&msgs, None);
        assert_eq!(out[0]["reasoning_content"], "我先想一想……");
        assert_eq!(out[0]["content"], "答案是 42");
        // 无 thinking 的消息不得出现该键
        let plain = vec![OllamaMessage {
            role: "user".into(),
            content: "问".into(),
            images: vec![],
            tool_calls: None,
            tool_name: None,
            thinking: None,
        }];
        let out = to_openai_messages(&plain, None);
        assert!(out[0].get("reasoning_content").is_none());
    }

    /// M35 D7：logprobs/top_logprobs 双构建器拼装（仅 logprobs 存在时下发；
    /// top_logprobs 附随）
    #[test]
    fn logprobs_request_assembly() {
        let chat_req = OllamaChatRequest {
            model: "m".into(),
            messages: vec![],
            format: None,
            options: None,
            tools: None,
            stream: Some(true),
            keep_alive: None,
            think: None,
            logprobs: Some(true),
            top_logprobs: Some(5),
        };
        let body = build_openai_chat_request(&chat_req, true);
        assert_eq!(body["logprobs"], true);
        assert_eq!(body["top_logprobs"], 5);

        let gen_req = OllamaGenerateRequest {
            model: "m".into(),
            prompt: "p".into(),
            images: None,
            format: None,
            options: None,
            system: None,
            template: None,
            raw: Some(true),
            stream: None,
            keep_alive: None,
            think: None,
            context: None,
            suffix: None,
            logprobs: Some(true),
            top_logprobs: Some(3),
        };
        let body = build_openai_completion_request(&gen_req, false);
        assert_eq!(body["logprobs"], true);
        assert_eq!(body["top_logprobs"], 3);

        // 缺省不注入
        let body = build_openai_chat_request(
            &OllamaChatRequest {
                model: "m".into(),
                messages: vec![],
                format: None,
                options: None,
                tools: None,
                stream: None,
                keep_alive: None,
                think: None,
                logprobs: None,
                top_logprobs: None,
            },
            false,
        );
        assert!(body.get("logprobs").is_none());
        assert!(body.get("top_logprobs").is_none());
    }

    /// M35 D7：logprobs 数组回填三形态（非流式 chat / 流式 chat / generate）
    #[test]
    fn logprobs_mapped_in_all_forms() {
        let lp = json!([{"token": "世", "logprob": -0.5, "top_logprobs": []}]);
        // 非流式 chat：choices[0].logprobs.content → 顶层 logprobs
        let openai = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop",
                "logprobs": {"content": lp},
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1},
        });
        let out = openai_chat_response_to_ollama("m", &openai);
        assert_eq!(out["logprobs"], lp);
        // 流式 chat chunk
        let chunk = json!({
            "choices": [{
                "delta": {"content": "世"},
                "logprobs": {"content": lp},
            }],
        });
        let events = openai_chunk_to_ollama_chat_events("m", &chunk);
        assert_eq!(events[0]["logprobs"], lp);
        // generate（completions）事件
        let gen_chunk = json!({
            "choices": [{
                "text": "世",
                "logprobs": {"content": lp},
            }],
        });
        let ev = openai_chunk_to_ollama_generate_event("m", &gen_chunk);
        assert_eq!(ev["logprobs"], lp);
        // 无 logprobs 的响应不得出现该键（既有请求零变化）
        let plain = json!({"choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}]});
        let out = openai_chat_response_to_ollama("m", &plain);
        assert!(out.get("logprobs").is_none());
    }

    /// M102（迭代32 碴3）：上游 timings 覆盖 duration 两字段——含 timings
    /// 时精确覆盖（ms×1e6→ns），字段缺失时不覆盖（墙钟兜底保留）
    #[test]
    fn timings_durations_override_and_fallback() {
        let mut ev = json!({
            "total_duration": 100,
            "load_duration": 10,
            "prompt_eval_duration": 20,
            "eval_duration": 5,
        });
        // 官方 README 实证形态：prompt_ms/predicted_ms（浮点毫秒）
        let timings = json!({"prompt_ms": 30.958, "predicted_ms": 661.064});
        apply_timings_durations(&mut ev, &timings);
        assert_eq!(ev["prompt_eval_duration"], json!(30958000));
        assert_eq!(ev["eval_duration"], json!(661064000));
        // total/load 不受影响（roxid 侧实测口径维持）
        assert_eq!(ev["total_duration"], json!(100));
        assert_eq!(ev["load_duration"], json!(10));

        // 缺失字段：不覆盖（墙钟兜底值保留）
        let mut ev2 = json!({"prompt_eval_duration": 20, "eval_duration": 5});
        apply_timings_durations(&mut ev2, &json!({"prompt_n": 12}));
        assert_eq!(ev2["prompt_eval_duration"], json!(20));
        assert_eq!(ev2["eval_duration"], json!(5));
    }

    /// M35 D1/D6：generate 请求 context/suffix 字段反序列化（官方续传与
    /// FIM 协议字段接入）
    #[test]
    fn generate_request_context_suffix_deserialize() {
        let req: OllamaGenerateRequest = serde_json::from_value(json!({
            "model": "m",
            "prompt": "p",
            "context": [1, 2, 3],
            "suffix": "</code>"
        }))
        .unwrap();
        assert_eq!(req.context, Some(vec![1, 2, 3]));
        assert_eq!(req.suffix.as_deref(), Some("</code>"));
        // 缺省为 None（既有请求零影响）
        let req: OllamaGenerateRequest =
            serde_json::from_value(json!({"model": "m", "prompt": "p"})).unwrap();
        assert!(req.context.is_none());
        assert!(req.suffix.is_none());
    }
}
