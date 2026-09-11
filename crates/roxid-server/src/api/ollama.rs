//! Ollama 原生 API 端点（/api/*，M7）。
//!
//! 端点：tags/ps/show/generate/chat/pull/copy/delete/stop/version(见 mod.rs)/
//! blobs(M20)/create/push/embed/embeddings。
//! 流式：NDJSON（Ollama 语义），由 llama-server SSE 实时转换。
//!
//! 修改历史：M7 新增 2026-08-24 19:31；M20 blobs 2026-08-24 22:42；
//! M21 D4c 请求级 num_ctx 2026-08-24 23:15；M22 对齐项（tags.model 完整名/
//! show license+model_info/create NDJSON/pull insecure/移除硬超时）2026-08-24 23:16；
//! M25 移除两处多余 mut（unused_mut 警告清偿）2026-08-26 05:48
//! M26 system/messages 注入（迭代6 P0碴2）：chat/generate 组装请求时注入
//! meta.system（请求级优先）与 meta.messages 预置对话（R1 裁决：api 层）2026-08-26 06-48
//! M27 租约接入（迭代7 P0碴1，R1 全调用点）：chat/generate/embed/embeddings
//! 持 RunnerLease 至响应结束——流式经 sse_to_ndjson 移入 body 状态
//! 2026-08-26 21-28
//! M28 碴3 Client 治理（迭代8）：http() 改 OnceLock 进程内共享（原每请求
//! 新建 Client 零复用）+ connect_timeout 10s（原构造无连接超时）
//! 2026-08-30 02-12
//! M28 五碴（迭代8）：碴2 pull 去重（PullGate 完成门，R1-A）、碴7 回显名
//! 规范化、碴9 embedding 家族判定、碴14 create 后停同名实例、碴15 blobs
//! 读错误显式中止 2026-08-30 02-18
//! M29 三碴（迭代9）：碴2 pull 去重键规范化（别名并发去重失效回归）、
//! 碴4 embed 补 total/load_duration 与 prompt_eval_count、碴11 进度
//! 事件通道满丢弃时留 debug 痕迹 2026-09-05 12-45
//! M30 三碴（迭代10）：碴1 show 复原数组参数值展开多行（M29 碴8 数组
//! 落地后紧凑 JSON 单行无法被 parse 往返——show→create 丢多值语义）、
//! 碴9 embed/embeddings_legacy 转发体剥离 model 路由字段、碴12 chat/
//! generate 非流式上游非 JSON 体显式 502（原容错 Null 生成空响应）
//! 2026-09-06 22-10
//! M32 两碴（迭代12）：碴7 tags 的 model_size 改消费聚合扫描携带的真实
//! 来源目录（原 ModelRef 重新 locate 丢弃来源，同名跨源共存时 size 错
//! 位）、碴9b create 状态事件 try_send 丢弃留 debug 痕迹 2026-09-07 01-05
//! M33 两碴（迭代13）：碴2 embed total_duration 口径对齐「请求到达→响应
//! 完成」（原计至响应头到达截断 body 读取耗时，与 M28 碴6 R2-A 裁决
//! 口径不一致）、碴4 tokenize_context 补 5s 请求超时（原仅 connect 超时，
//! 上游建连后挂起则 generate 流末事件永不下发、租约滞留）
//! 2026-09-07 01-50
//! M34 两项（迭代14 BUG-1/BUG-3）：BUG-1 流式单终止包——sse_to_ndjson 增
//! FinalEventBuffer 暂存合并（上游 finish_reason 包与 usage 包分两包下发，
//! 原逐包直转产生双 done 终止包，违反官方「单终止包含 usage」契约；R1-A
//! 裁决：api 层暂存，converter 纯函数维持）；BUG-3 参数合并链——chat/
//! generate 元数据读取前置，meta.parameters 默认层并入 options（请求逐键
//! 覆盖，官方 Modelfile 语义），合并先于 want_ctx 使模型级 num_ctx 正确
//! 触发 D4c 重建 2026-09-07 19-05
//! M34 后四项（迭代14 BUG-2/6/7）：BUG-2 全部 POST 提取器换 LenientJson
//!（不校验 Content-Type，对齐官方直读 body；官方文档示例即裸 curl -d）；
//! BUG-6 embed/embeddings_legacy 非 2xx 错误经 format_upstream_error 单层化
//!（解包内层 message，Pooling none 转官方口径文案）；BUG-7 上游请求失败
//! 经 upstream_failure_message 整形（连接类转模型级文案，without_url 剥离
//! 内部 URL/动态端口）2026-09-07 19-35
//! M35 P0 三碴（迭代15）：D2 show capabilities 改官方字符串数组（原布尔
//! 对象含自定 rerank 键，SDK 按数组迭代直接解析失败）；D3 parameters 改
//! 多行文本（复用 parameter_lines，原 BTreeMap 对象与官方字符串类型不
//! 符）；D13 model_info 官方键名与数字类型（{arch}.context_length /
//! general.parameter_count / general.file_type，GGUF header 实时解析数据
//! 源零存储变更）+ modified_at + verbose 读取 2026-09-07 21-44
//! M35 推理链路三碴（迭代15）：D1 generate context 续传——tokenize 拼接
//! 走原生 /completion token 数组（R1-A；tokenize 失败降级常规通道），
//! context 回填前置请求 context；D6 suffix → 原生 /infill（R2-A）；D10
//! generate 非流式 thinking 顶层提升（与流式对齐）；D11 embed truncate=
//! false 超长显式 400（原恒静默截断）2026-09-07 22-00
//! M35 D4 create 结构化形态（迭代15）：官方 0.5+ 结构化请求支持——from=
//! 基础模型名 + system/parameters/messages/template 结构化字段构造等价
//! Modelfile（resolve_modelfile 三形态分派：modelfile 文本 > from 文本
//! 兼容 > from 基础模型名；license/quantize/renderer/parser 无对应实现
//! 注记忽略）2026-09-07 22-12
//! M93 碴B（迭代29）：pull 等待者分支补进度转发——200ms ticker 读
//! PullGate 共享快照，下发与首请求同形态进度事件（原「等待终态并转发，
//! 无中途进度」使 CLI 中断后二次 run 复用后台任务时全程静默数十分钟）；
//! 首任务事件下发侧顺带写快照（update_gate_snapshot）2026-09-10 06-40

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{json, Value};

use crate::adapter::{
    self, build_openai_chat_request, build_openai_completion_request,
    openai_chat_response_to_ollama, openai_chunk_to_ollama_chat_events,
    openai_chunk_to_ollama_generate_event, openai_completion_response_to_ollama, parse_keep_alive,
    OllamaChatRequest, OllamaGenerateRequest,
};
use crate::api::{AppState, LenientJson, PullGate};
use crate::repo::{self, ModelRef};
use crate::scheduler::RunnerLease;

/// 共享 HTTP 客户端（透传 llama-server）。
/// M22：移除请求级总超时，长生成不再被 600s 截断。
/// M28 碴3：OnceLock 进程内共享（原每请求新建 Client，连接池/TLS 会话
/// 零复用）+ connect_timeout 10s 兜底建连失败（原构造无连接超时）。
/// Client 内部为 Arc，clone 廉价。
fn http() -> reqwest::Client {
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

/// Ollama 风格错误响应：{"error": "..."}
fn error_response(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({"error": msg.to_string()}))).into_response()
}

/// acquire 错误分类整形（迭代30 碴A，M96）：仅未安装报 404——客户端
/// （roxid run M33 碴8/M94 预检）依赖 404 触发自动拉取；加载失败/超时等
/// 其他错误报 500。原实现统一 404，加载失败被客户端误判「未安装」触发
/// 无效重拉，真实报错被拉取噪音掩盖（2026-09-10 gemma4 案实证：
/// wrong number of tensors 被报 404 → 重拉 9.6GB → 仍失败）。对齐原版
/// 语义：404=模型未找到，5xx=服务端加载/运行失败。llamacpp/openai 层
/// acquire 同语义复用（pub(crate)）。
///
/// - 参数 e：acquire 返回的错误
/// - 返回：分类后的错误响应（ModelNotFound→404，其余→500）
pub(crate) fn acquire_error_response(e: crate::error::RoxidError) -> Response {
    if matches!(e, crate::error::RoxidError::ModelNotFound(_)) {
        error_response(StatusCode::NOT_FOUND, e)
    } else {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, e)
    }
}

/// 上游（llama-server 实例）错误响应体整形为单层人类可读文案（M34 BUG-6）。
/// 原实现把上游错误 JSON 原文（含 {"error":{"code":400,"message":"Pooling
/// type 'none' is not OAI compatible..."}}）直接塞入 error 字段成转义
/// 字符串——双层嵌套且内部术语暴露；官方同场景返回单层人类可读文案。
///
/// - 参数 text：上游错误响应体文本
/// - 返回：单层文案（嵌套解包内层 message；Pooling none 场景转写官方口径；
///   解析失败保留原文兜底）
fn format_upstream_error(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if let Some(inner) = v["error"]["message"].as_str() {
            if inner.contains("Pooling type") {
                // 生成模型请求 embedding：原对齐官方 A/B 实证英文口径（M34
                // BUG-6）；M104（迭代32 碴5）按用户新裁决转写中文——全仓
                // API 错误文案中文基线统一（上游英文 message 仍原样透传，
                // 裁决 2026-09-10 19:00「仅①中文化」）
                return "此模型不支持向量化，请使用 embedding 模型".to_string();
            }
            return inner.to_string();
        }
        if let Some(flat) = v["error"].as_str() {
            return flat.to_string();
        }
    }
    text.to_string()
}

/// 上游请求失败错误整形（M34 BUG-7，R5-A 裁决：模型级语义文案——rm/stop
/// 杀实例后在途请求收到连接类错误原文，含 llama-server 动态端口 URL，
/// 既不可理解又暴露内部拓扑）。
///
/// - 参数 e：reqwest 发送错误（按值接收——without_url 消费所有权）
/// - 返回：连接/请求类失败转「模型实例已停止」模型级文案；其余保留
///   Display 并经 without_url 剥离内部 URL
fn upstream_failure_message(mut e: reqwest::Error) -> String {
    // is_decode：流中途 body 解码失败（连接被掐）——rm/stop 杀实例后
    // 在途流的典型形态（e2e 实测 2026-09-07 19:35）
    if e.is_connect() || e.is_request() || e.is_decode() {
        return "模型实例已停止或卸载，请求中断".to_string();
    }
    if e.url().is_some() {
        e = e.without_url();
    }
    e.to_string()
}

/// 模型持久化参数与请求 options 合并（M34 BUG-3，R3 裁决同 M26 R1 分层：
/// api 层合并；官方语义——Modelfile PARAMETER 为默认值层，请求 options
/// 逐键覆盖，无需改 Modelfile）。映射链复用 adapter::build_inference_params
/// 既有实现（14 同名采样参数 + num_predict→max_tokens + stop）。
///
/// - 参数 meta_parameters：model.json 持久化参数（ModelMeta.parameters）
/// - 参数 request_options：请求携带的 options（None/非对象视为未携带）
/// - 返回：合并后的 options（meta 空层且请求未携带时返回原值——无参数
///   模型行为零变化）
fn merge_model_parameters(
    meta_parameters: &std::collections::BTreeMap<String, Value>,
    request_options: Option<Value>,
) -> Option<Value> {
    if meta_parameters.is_empty() {
        return request_options; // 空参数层：透传（无 meta 模型行为零变化）
    }
    // BTreeMap（ModelMeta 形态）→ serde_json::Map（options Value 形态）
    let mut merged: serde_json::Map<String, Value> = meta_parameters
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Some(req_map) = request_options.as_ref().and_then(|o| o.as_object()) {
        for (k, v) in req_map {
            merged.insert(k.clone(), v.clone()); // 请求键覆盖同键默认层
        }
    }
    Some(Value::Object(merged))
}

/// 终包合并缓冲（M34 BUG-1，R1-A 裁决：api 层暂存，converter 保持纯函数）。
/// 上游 llama-server（include_usage 已启用）流末通常分两包下发：先
/// finish_reason 包（无统计键）、后 usage 包（统计完整）——逐包直转产生
/// 两个 done 终止包，违反官方「usage 字段含于唯一终止包」契约。本缓冲
/// 将首个 done 事件暂存，usage 包到达时替换合并（保留统计）；流结束时
/// flush 兜底（usage 未见达则按 M33 碴3 语义缺省统计键）。
struct FinalEventBuffer {
    /// 暂存的待合并终包（None=流中尚无终包）
    pending: Option<Value>,
}

impl FinalEventBuffer {
    fn new() -> Self {
        Self { pending: None }
    }

    /// 接收一个转换后事件，返回应立即写出的事件序列。
    ///
    /// - 终包（done=true）：首个暂存；后到终包并入——仅补充先到包缺失的
    ///   键（usage 包补统计键；done_reason 等先到真实值不被后到兜底值
    ///   覆盖——usage 包无 choice，adapter 恒兜底 stop，真实 length/
    ///   tools 场景后到值是错的）。终包返回空——延后至 flush 输出
    /// - 增量（done=false）：透传；若此前有暂存终包（上游异常时序的防御
    ///   分支）先保序 flush
    fn push(&mut self, ev: Value) -> Vec<Value> {
        if ev["done"].as_bool().unwrap_or(false) {
            match &mut self.pending {
                Some(prev) => {
                    if let (Some(new_obj), Some(prev_obj)) = (ev.as_object(), prev.as_object_mut())
                    {
                        for (k, v) in new_obj {
                            prev_obj.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
                None => self.pending = Some(ev),
            }
            return Vec::new();
        }
        match self.pending.take() {
            Some(fin) => vec![fin, ev],
            None => vec![ev],
        }
    }

    /// 流结束时取出暂存终包（无暂存返回 None）。
    fn flush(&mut self) -> Option<Value> {
        self.pending.take()
    }
}

/// SSE 事件 → NDJSON 行（chunk 形态转换器：chat 或 generate）
type ChunkConverter = fn(&str, &Value) -> Vec<Value>;

/// 流式计时与上下文回填参数（M28 碴6/碴8：duration 真实计量 + generate
/// context 回填，由 handler 测得后传入流状态机）。
struct StreamExtras {
    /// 请求到达时刻（total_duration 起点）
    request_start: std::time::Instant,
    /// 上游响应首字节（headers）到达时刻（prompt/eval 切分点）
    first_byte: std::time::Instant,
    /// 本次 acquire 冷加载耗时（load_duration；复用实例≈0）
    load: std::time::Duration,
    /// generate 通道的 context 回填参数（None=chat 通道，原版 chat 无 context）
    context: Option<StreamContext>,
}

/// generate 流式的 context 回填参数（M28 碴8）。
struct StreamContext {
    /// 实例端口（/tokenize 调用目标）
    port: u16,
    /// 用户 prompt（与累积生成文本拼接后整体 tokenize，对齐原版续传语义）
    prompt: String,
}

/// 流转换状态机（M27 租约 + M28 计时/上下文回填的载体）。
struct StreamState {
    /// 上游字节流
    upstream: std::pin::Pin<Box<dyn futures::Stream<Item = reqwest::Result<Bytes>> + Send>>,
    /// SSE 缓冲
    buf: String,
    /// 回显模型名
    model: String,
    /// 事件形态转换器
    convert: ChunkConverter,
    /// 实例租约（M27：随流 Drop 释放；仅持有不读取，豁免 dead_code 告警）
    #[allow(dead_code)]
    lease: RunnerLease,
    /// 计时参数（M28 碴6）
    request_start: std::time::Instant,
    /// 上游首字节时刻（M28 碴6）
    first_byte: std::time::Instant,
    /// 冷加载耗时（M28 碴6）
    load: std::time::Duration,
    /// generate 上下文回填参数（M28 碴8）
    ctx: Option<StreamContext>,
    /// generate 生成文本累积（M28 碴8：done 事件时与 prompt 拼接 tokenize）
    accumulated: String,
    /// 终包合并缓冲（M34 BUG-1：done 事件暂存，usage 末包到达替换合并）
    finals: FinalEventBuffer,
}

/// llama-server SSE 流 → Ollama NDJSON 流（Body::from_stream 状态机）。
/// 缓缓冲按 "\n\n" 切分 SSE 事件；"data: [DONE]" 结束；解析失败行跳过。
/// M27（碴1）：租约移入流状态——流耗尽或 body 被客户端断开丢弃时 Drop，
/// 在途计数减一并自该时刻重计 keep_alive 窗口（请求全程实例受保护）。
/// M28（碴6/碴8）：done 事件注入 duration 四字段；generate 通道 context
/// 经 /tokenize 回填（失败降级空数组不阻断）。
fn sse_to_ndjson(
    model: String,
    resp: reqwest::Response,
    convert: ChunkConverter,
    lease: RunnerLease,
    extras: StreamExtras,
) -> Body {
    let state = StreamState {
        upstream: Box::pin(resp.bytes_stream()),
        buf: String::new(),
        model,
        convert,
        lease,
        request_start: extras.request_start,
        first_byte: extras.first_byte,
        load: extras.load,
        ctx: extras.context,
        accumulated: String::new(),
        finals: FinalEventBuffer::new(),
    };
    Body::from_stream(futures::stream::unfold(state, |mut st| {
        Box::pin(async move {
            loop {
                if let Some(pos) = st.buf.find("\n\n") {
                    let event = st.buf[..pos].to_string();
                    st.buf.drain(..pos + 2);
                    let lines: Vec<&str> = event
                        .lines()
                        .filter_map(|l| l.strip_prefix("data:").map(|d| d.trim()))
                        .collect();
                    let mut out = String::new();
                    for line in lines {
                        if line == "[DONE]" {
                            continue;
                        }
                        if let Ok(chunk) = serde_json::from_str::<Value>(line) {
                            for ev in (st.convert)(&st.model, &chunk) {
                                // M28 碴8：累积 generate 文本增量（chat 事件无 response 字段天然跳过）
                                if let Some(t) = ev["response"].as_str() {
                                    st.accumulated.push_str(t);
                                }
                                // M34 BUG-1：终包经合并缓冲延后输出（usage 末包
                                // 到达替换合并为单包），本轮仅增量与保序 flush
                                // 事件写出
                                for mut ev in st.finals.push(ev) {
                                    write_final_event(
                                        &mut ev,
                                        st.request_start,
                                        st.first_byte,
                                        st.load,
                                        st.ctx.as_ref(),
                                        &st.accumulated,
                                        &mut out,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    if !out.is_empty() {
                        return Some((Ok::<Bytes, std::io::Error>(Bytes::from(out)), st));
                    }
                    continue; // 空事件（注释/心跳）继续拉取
                }
                match st.upstream.next().await {
                    Some(Ok(chunk)) => st.buf.push_str(&String::from_utf8_lossy(&chunk)),
                    Some(Err(e)) => {
                        // M34 BUG-7：流中断错误整形（模型级语义，剥离内部 URL/端口）
                        let err = format!(
                            "{{\"error\":\"上游中断：{}\"}}\n",
                            upstream_failure_message(e)
                        );
                        return Some((Ok(Bytes::from(err)), st));
                    }
                    None => {
                        // M34 BUG-1：流结束 flush 暂存终包——全流恰好一个
                        // done 终止包（usage 未见达时按 M33 碴3 缺省统计键）
                        if let Some(mut fin) = st.finals.flush() {
                            let mut out = String::new();
                            write_final_event(
                                &mut fin,
                                st.request_start,
                                st.first_byte,
                                st.load,
                                st.ctx.as_ref(),
                                &st.accumulated,
                                &mut out,
                            )
                            .await;
                            return Some((Ok::<Bytes, std::io::Error>(Bytes::from(out)), st));
                        }
                        return None; // 上游正常结束（lease 随状态 drop 释放）
                    }
                }
            }
        })
    }))
}

/// 终包写出：按需注入计量与上下文后写行（M34 BUG-1：注入点随终包合并
/// 缓冲移至实际写出时刻——仅对最终写出的终包执行一次；M28 碴6 duration
/// 四字段与 M28 碴8 generate context 回填语义不变）。
/// 字段级参数而非整体借 &StreamState：upstream 流对象非 Sync，整体借用
/// 跨 await 会使 unfold future 失去 Send。
///
/// - 参数 ev：待写出事件（done=true 时注入计量字段）
/// - 参数 request_start/first_byte/load：计时三元组（M28 碴6 语义）
/// - 参数 ctx：generate 上下文回填参数（None=chat 通道，原版 chat 无 context）
/// - 参数 accumulated：generate 生成文本累积（与 prompt 拼接 tokenize）
/// - 参数 out：NDJSON 行输出缓冲
async fn write_final_event(
    ev: &mut Value,
    request_start: std::time::Instant,
    first_byte: std::time::Instant,
    load: std::time::Duration,
    ctx: Option<&StreamContext>,
    accumulated: &str,
    out: &mut String,
) {
    if ev["done"].as_bool().unwrap_or(false) {
        // M28 碴6：末事件注入 duration 四字段（原硬编码 0/缺失）
        crate::adapter::apply_duration_fields(ev, request_start, first_byte, load);
        // M28 碴8：generate 末事件 context 回填
        if let Some(ctx) = ctx {
            let full = format!("{}{}", ctx.prompt, accumulated);
            ev["context"] = json!(tokenize_context(ctx.port, &full).await);
        }
    }
    out.push_str(&ev.to_string());
    out.push('\n');
}

/// 对完整上下文文本取 token ids（M28 碴8：generate 响应 context 回填——
/// 原版语义返回 prompt+生成的完整上下文 ids 供下轮续传）。
///
/// - 参数 port：实例端口（/tokenize 端点，b10605 支持，M18 已透通实证）
/// - 参数 text：完整上下文文本（prompt + 生成内容）
/// - 返回：token id 数组（失败降级空数组，不阻断响应）
async fn tokenize_context(port: u16, text: &str) -> Vec<u64> {
    match http()
        .post(format!("http://127.0.0.1:{port}/tokenize"))
        .json(&json!({"content": text}))
        // M33 碴4：5s 请求级超时——共享 http() 仅 connect_timeout 10s，
        // 上游建连后挂起（实例僵死等）会使 generate 流末事件永不下发、
        // 租约滞留；失败降级空数组语义不变（本调用仅 context 回填）
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| {
                v["tokens"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|t| t.as_u64()).collect::<Vec<u64>>())
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// tokenize 单段文本（M35 D1：context 通道的 prompt token 化）。与
/// tokenize_context 的降级空数组语义区分：本函数失败返回 None 供通道
/// 降级判定——tokenize 不可用时不走 token 拼接通道而降级常规通道。
///
/// - 参数 port：实例端口（/tokenize 端点）
/// - 参数 text：待 tokenize 文本
/// - 返回：token id 数组；上游失败/解析失败 None
async fn tokenize_prompt(port: u16, text: &str) -> Option<Vec<u64>> {
    let resp = http()
        .post(format!("http://127.0.0.1:{port}/tokenize"))
        .json(&json!({"content": text}))
        // M33 碴4 同款 5s 请求级超时（共享 http() 仅 connect 超时）
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    v["tokens"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|t| t.as_u64()).collect())
}

/// M35 D1/D6（迭代15）：generate 原生通道描述——context 续传走 /completion
/// token 数组（官方支持 sequence of tokens，Context7 实证 2026-09-07）、
/// suffix FIM 走 /infill（M18 直通先例端点）。
enum NativeChannel {
    /// /infill：input_prefix=prompt、input_suffix=suffix
    Infill,
    /// /completion：完整输入 token 序列（请求 context ++ tokenize(prompt)）
    Completion { prompt_tokens: Vec<u64> },
}

/// 原生通道公共参数段：build_inference_params 产出 merge + max_tokens →
/// n_predict 重映射（原生端点参数名为 n_predict，max_tokens 被忽略）。
fn merge_native_params(obj: &mut serde_json::Map<String, Value>, req: &OllamaGenerateRequest) {
    if let Value::Object(extra) =
        crate::adapter::build_inference_params(req.options.as_ref(), None, req.think.as_ref())
    {
        for (k, v) in extra {
            if k == "max_tokens" {
                obj.insert("n_predict".into(), v);
            } else {
                obj.insert(k, v);
            }
        }
    }
}

/// M35 D1：原生 /completion 请求体——token 数组 prompt + 采样参数 +
/// format 原生 json_schema 语法（raw 通道同款）。
///
/// - 参数 req：Ollama generate 请求
/// - 参数 prompt_tokens：完整输入 token 序列（context ++ tokenize(prompt)）
/// - 参数 stream：流式开关
/// - 返回：/completion 请求 JSON
fn build_native_completion_request(
    req: &OllamaGenerateRequest,
    prompt_tokens: Vec<u64>,
    stream: bool,
) -> Value {
    let mut body = json!({ "prompt": prompt_tokens, "stream": stream });
    let obj = body.as_object_mut().unwrap();
    merge_native_params(obj, req);
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

/// M35 D6：原生 /infill 请求体——input_prefix/input_suffix + 参数段。
///
/// - 参数 req：Ollama generate 请求（suffix 非空）
/// - 参数 stream：流式开关
/// - 返回：/infill 请求 JSON
fn build_native_infill_request(req: &OllamaGenerateRequest, stream: bool) -> Value {
    let mut body = json!({
        "input_prefix": req.prompt,
        "input_suffix": req.suffix,
        "stream": stream,
    });
    let obj = body.as_object_mut().unwrap();
    merge_native_params(obj, req);
    body
}

/// 原生 stop_type → Ollama done_reason（limit → length；word/eos/缺省 → stop）。
///
/// - 参数 v：原生响应 stop_type 字段
/// - 返回：done_reason 值
fn native_stop_to_done_reason(v: &Value) -> Value {
    match v.as_str() {
        Some("limit") => json!("length"),
        _ => json!("stop"),
    }
}

/// M35 D1/D6：原生非流式响应（/completion 与 /infill 同构）→ Ollama
/// generate 响应形态。timings 统计存在才写（M33 碴3 语义）。
///
/// - 参数 model：回显模型名
/// - 参数 v：原生响应 JSON（content/stop_type/timings）
/// - 返回：Ollama generate 响应 JSON
fn native_completion_to_ollama(model: &str, v: &Value) -> Value {
    let mut out = json!({
        "model": model,
        "created_at": crate::registry::now_rfc3339(),
        "response": v["content"].as_str().unwrap_or_default(),
        "done": true,
        "done_reason": native_stop_to_done_reason(&v["stop_type"]),
    });
    if let Some(pn) = v["timings"]["prompt_n"].as_u64() {
        out["prompt_eval_count"] = json!(pn);
    }
    if let Some(dn) = v["timings"]["predicted_n"].as_u64() {
        out["eval_count"] = json!(dn);
    }
    out
}

/// M35 D1/D6：原生 NDJSON 流状态机（每行 {content, stop, stop_type?,
/// timings?}；终行携带统计）。lease 随流 Drop（M27 在途保护语义同
/// sse_to_ndjson）。
struct NativeStreamState {
    /// 上游字节流
    upstream: std::pin::Pin<Box<dyn futures::Stream<Item = reqwest::Result<Bytes>> + Send>>,
    /// 行缓冲（跨 chunk 半行拼接）
    buf: String,
    /// 回显模型名
    model: String,
    /// 实例租约（M27：随流 Drop 释放）
    #[allow(dead_code)]
    lease: RunnerLease,
    /// 计时三元组（M28 碴6 语义）
    request_start: std::time::Instant,
    first_byte: std::time::Instant,
    load: std::time::Duration,
    /// /tokenize 端口（context 回填）
    port: u16,
    /// 用户 prompt（context 回填拼接）
    prompt: String,
    /// 请求 context 前置（Completion 通道回填前置；Infill 为空）
    context_prefix: Vec<u64>,
    /// 生成文本累积（context 回填）
    accumulated: String,
    /// 终包已产出（防御上游多发终行）
    done_emitted: bool,
}

/// M35 D1/D6：原生 NDJSON 流 → Ollama generate NDJSON 事件流。
/// 非终行 → {response, done:false}；终行（stop:true）→ 单终包含
/// done_reason/timings 统计/duration 四字段/context 回填（请求 context
/// 前置 ++ tokenize(prompt+生成全文)，失败降级前置段不阻断）。
fn native_ndjson_to_ollama(
    model: String,
    resp: reqwest::Response,
    lease: RunnerLease,
    request_start: std::time::Instant,
    first_byte: std::time::Instant,
    load: std::time::Duration,
    port: u16,
    prompt: String,
    context_prefix: Vec<u64>,
) -> Body {
    let state = NativeStreamState {
        upstream: Box::pin(resp.bytes_stream()),
        buf: String::new(),
        model,
        lease,
        request_start,
        first_byte,
        load,
        port,
        prompt,
        context_prefix,
        accumulated: String::new(),
        done_emitted: false,
    };
    Body::from_stream(futures::stream::unfold(state, |mut st| {
        Box::pin(async move {
            loop {
                while let Some(pos) = st.buf.find('\n') {
                    let line = st.buf[..pos].trim().to_string();
                    st.buf.drain(..pos + 1);
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(chunk) = serde_json::from_str::<Value>(&line) else {
                        continue; // 非 JSON 行跳过（sse_to_ndjson 同款容错）
                    };
                    let content = chunk["content"].as_str().unwrap_or_default();
                    if !chunk["stop"].as_bool().unwrap_or(false) {
                        if !content.is_empty() {
                            st.accumulated.push_str(content);
                        }
                        let ev = json!({
                            "model": st.model,
                            "created_at": crate::registry::now_rfc3339(),
                            "response": content,
                            "done": false,
                        });
                        return Some((
                            Ok::<Bytes, std::io::Error>(Bytes::from(format!("{ev}\n"))),
                            st,
                        ));
                    }
                    if st.done_emitted {
                        continue; // 防御：上游多发终行只取首个
                    }
                    st.done_emitted = true;
                    let mut ev = json!({
                        "model": st.model,
                        "created_at": crate::registry::now_rfc3339(),
                        "response": content,
                        "done": true,
                        "done_reason": native_stop_to_done_reason(&chunk["stop_type"]),
                    });
                    if let Some(pn) = chunk["timings"]["prompt_n"].as_u64() {
                        ev["prompt_eval_count"] = json!(pn);
                    }
                    if let Some(dn) = chunk["timings"]["predicted_n"].as_u64() {
                        ev["eval_count"] = json!(dn);
                    }
                    write_native_final(
                        &mut ev,
                        st.request_start,
                        st.first_byte,
                        st.load,
                        st.port,
                        &st.prompt,
                        &st.accumulated,
                        &st.context_prefix,
                    )
                    .await;
                    return Some((
                        Ok::<Bytes, std::io::Error>(Bytes::from(format!("{ev}\n"))),
                        st,
                    ));
                }
                match st.upstream.next().await {
                    Some(Ok(chunk)) => st.buf.push_str(&String::from_utf8_lossy(&chunk)),
                    Some(Err(e)) => {
                        // M34 BUG-7 同款：流中断整形（模型级语义，剥离内部 URL）
                        let err = format!(
                            "{{\"error\":\"上游中断：{}\"}}\n",
                            upstream_failure_message(e)
                        );
                        return Some((Ok(Bytes::from(err)), st));
                    }
                    None => {
                        // 上游结束无终行（异常截断）：补空终包兜底（M33 碴3
                        // 缺省统计键语义；已产出终包则正常收束）
                        if st.done_emitted {
                            return None;
                        }
                        st.done_emitted = true;
                        let mut ev = json!({
                            "model": st.model,
                            "created_at": crate::registry::now_rfc3339(),
                            "response": "",
                            "done": true,
                            "done_reason": "stop",
                        });
                        write_native_final(
                            &mut ev,
                            st.request_start,
                            st.first_byte,
                            st.load,
                            st.port,
                            &st.prompt,
                            &st.accumulated,
                            &st.context_prefix,
                        )
                        .await;
                        return Some((
                            Ok::<Bytes, std::io::Error>(Bytes::from(format!("{ev}\n"))),
                            st,
                        ));
                    }
                }
            }
        })
    }))
}

/// 原生通道终包收尾：duration 四字段 + context 回填（请求 context 前置 ++
/// tokenize(prompt+生成全文)，失败降级前置段）。字段级参数而非整体借
/// &NativeStreamState：upstream 流对象非 Sync，整体借用跨 await 会使
/// unfold future 失去 Send（write_final_event 同款拆分先例）。
///
/// - 参数 ev：终包事件（原地注入）
/// - 参数 request_start/first_byte/load：计时三元组（M28 碴6 语义）
/// - 参数 port：/tokenize 端口
/// - 参数 prompt/accumulated：用户 prompt 与生成文本累积
/// - 参数 context_prefix：请求 context 前置 token
async fn write_native_final(
    ev: &mut Value,
    request_start: std::time::Instant,
    first_byte: std::time::Instant,
    load: std::time::Duration,
    port: u16,
    prompt: &str,
    accumulated: &str,
    context_prefix: &[u64],
) {
    crate::adapter::apply_duration_fields(ev, request_start, first_byte, load);
    let full = format!("{prompt}{accumulated}");
    let mut tokens = context_prefix.to_vec();
    tokens.extend(tokenize_context(port, &full).await);
    ev["context"] = json!(tokens);
}

/// GGUF 训练长度（迭代43 M160/M161：扩窗目标封顶数据源，R1-A 裁决
/// 2026-09-11 23:03）。show 的 M35 D13 同源读取；模型定位或 GGUF 解析
/// 失败宽容 None（R1-A「GGUF 不可读时宽容不封顶」——仅超窗罕见路径调用）。
///
/// - 参数 models_root：模型仓库根目录
/// - 参数 name：请求模型名
/// - 返回：GGUF {arch}.context_length；不可得 None
fn gguf_context_length(models_root: &std::path::Path, name: &str) -> Option<u64> {
    let r = ModelRef::parse(name).ok()?;
    let meta = repo::find_model(models_root, name).ok()?;
    crate::registry::gguf::parse_metadata(&r.dir(models_root).join(&meta.files.model))
        .ok()
        .and_then(|g| g.context_length)
}

/// /api/chat：Ollama 聊天端点（全字段透传转换）
pub async fn chat(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<OllamaChatRequest>,
) -> Response {
    // M28 碴7：响应/事件回显规范化完整名（对齐原版与 acquire 查找键）
    let display_name = display_model_name(&req.model);
    let request_start = std::time::Instant::now(); // M28 碴6：全程计时起点
    let keep_alive = parse_keep_alive(req.keep_alive.as_ref());
    // M26 碴2：注入模型元数据预置内容（meta.system 缺省补位 + meta.messages
    // 预置对话；用户消息表首条 system 优先）；M34 BUG-3：meta.parameters
    // 作为默认层并入 options（请求 options 逐键覆盖，官方 Modelfile 语义），
    // 合并先于 want_ctx 计算——模型级 num_ctx 正确触发 D4c 重建；
    // find_model 失败按无预置处理不阻断
    let mut req = req;
    // 迭代43 M160：meta 保留 Option（generate 同构形态）——超窗扩窗封顶
    // 需模型定位，原 if let 消费形态不保留句柄
    let meta = repo::find_model(&st.models_root, &req.model).ok();
    if let Some(m) = &meta {
        req.messages = apply_model_preset(req.messages, &m.system, &m.messages);
        req.options = merge_model_parameters(&m.parameters, req.options.take());
    }
    // D4c：请求级（含模型级合并后）options.num_ctx 不一致时按原版语义重建实例
    let want_ctx = crate::adapter::requested_num_ctx(req.options.as_ref());
    let want_rt = crate::adapter::requested_runtime(req.options.as_ref());
    // 迭代43 M160：lease 可变——超窗重建重放需整体替换；want_rt 首次传
    // clone 保留原值（重放 acquire 复用，迭代43）
    let mut lease = match st
        .scheduler
        .acquire(&req.model, keep_alive, want_ctx, want_rt.clone())
        .await
    {
        Ok(r) => r,
        // 迭代30 碴A（M96）：错误分类整形（原统一 404）
        Err(e) => return acquire_error_response(e),
    };
    // M28 碴6：acquire 耗时即 load_duration（复用实例≈0，冷加载为真实加载时长）
    let mut load = request_start.elapsed();
    let mut port = lease.port().await;
    let stream = req.stream.unwrap_or(true); // 原版默认流式
    let upstream_body = build_openai_chat_request(&req, stream);
    let mut resp = match http()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&upstream_body)
        .send()
        .await
    {
        Ok(r) => r,
        // M34 BUG-7：连接类失败转模型级文案（不暴露内部 URL/端口）
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, upstream_failure_message(e)),
    };
    // 迭代43 M160（2026-09-11 23:03 批准）：超窗自动扩窗重建重放——
    // Q1=B 裁决（22:56）：400 且判型 exceed_context_size_error 时按
    // n_prompt_tokens 扩窗（R1-A：+1024 余量、512 对齐、GGUF 训练长度封顶）、
    // 经 acquire 的 D4c 重建链换新实例后重放同款请求体（零协议漂移）；
    // R2：重试上限 1 次（二次失败落入下方统一透传，防循环）
    if resp.status() == StatusCode::BAD_REQUEST {
        let text = resp.text().await.unwrap_or_default();
        match crate::adapter::parse_exceed_context_error(&text) {
            Some(n_prompt) => {
                let train_ctx = gguf_context_length(&st.models_root, &req.model);
                let target = crate::adapter::expanded_ctx_target(n_prompt, train_ctx);
                tracing::info!(
                    model = %display_name, n_prompt_tokens = n_prompt, target_ctx = target,
                    "超窗触发自动扩窗重建（迭代43 M160）"
                );
                lease = match st
                    .scheduler
                    .acquire(&req.model, keep_alive, Some(target), want_rt)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => return acquire_error_response(e),
                };
                load = request_start.elapsed(); // M28 碴6 口径：重建耗时并入 load_duration
                port = lease.port().await;
                resp = match http()
                    .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
                    .json(&upstream_body)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        return error_response(StatusCode::BAD_GATEWAY, upstream_failure_message(e))
                    }
                };
            }
            None => return error_response(StatusCode::BAD_REQUEST, text),
        }
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return error_response(status, text);
    }
    // M28 碴6：上游 headers 已到即首字节时刻（prompt/eval 切分点）
    let first_byte = std::time::Instant::now();
    if stream {
        let body = sse_to_ndjson(
            display_name, // M28 碴7：规范化名回显
            resp,
            adapter::openai_chunk_to_ollama_chat_events,
            lease,
            StreamExtras {
                request_start,
                first_byte,
                load,
                context: None, // 原版 chat 响应无 context 字段
            },
        );
        Response::builder()
            .header("content-type", "application/x-ndjson")
            .body(body)
            .unwrap()
    } else {
        // M30 碴12：上游非 JSON 体显式 502——原容错 Null 生成空响应 200，
        // 故障被吞（embed 的容错为 M29 单测锚定行为，不在此列）
        let openai: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => {
                return error_response(StatusCode::BAD_GATEWAY, "上游响应非 JSON");
            }
        };
        let mut out = openai_chat_response_to_ollama(&display_name, &openai);
        // M28 碴6：duration 四字段真实计量（原硬编码 0）
        crate::adapter::apply_duration_fields(&mut out, request_start, first_byte, load);
        // M102（迭代32 碴3）：非流式墙钟切分失真（生成完毕才收 headers），
        // 上游 timings 精确计时覆盖 prompt/eval 两 duration（缺失回退墙钟）
        crate::adapter::apply_timings_durations(&mut out, &openai["timings"]);
        Json(out).into_response()
    }
}

/// generate 无元数据时的空预置对话兜底（M34 BUG-3：meta 前置读取后
/// find_model 失败分支引用，避免每请求构造临时 Vec）
static EMPTY_PRESET_MESSAGES: Vec<crate::repo::ChatMessage> = Vec::new();

/// /api/generate：文本生成端点。
/// 默认经 chat 通道（prompt 作为 user 消息，llama-server 套内建模板，贴近原版行为）；
/// raw=true 时经 completions 通道（prompt 原样直发，不套模板）。
pub async fn generate(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<OllamaGenerateRequest>,
) -> Response {
    // M28 碴7：响应/事件回显规范化完整名
    let display_name = display_model_name(&req.model);
    let request_start = std::time::Instant::now(); // M28 碴6：全程计时起点
    let keep_alive = parse_keep_alive(req.keep_alive.as_ref());
    // M26 碴2 + M34 BUG-3：元数据一次读取前置——system/messages 预置与
    // parameters 默认层合并（请求 options 逐键覆盖，官方 Modelfile 语义；
    // raw 与 chat 通道共用——官方 raw 仅影响 prompt 格式化，参数照常生效）；
    // 合并先于 want_ctx 计算——模型级 num_ctx 正确触发 D4c 重建；
    // find_model 失败按无预置处理不阻断
    let mut req = req;
    let meta = repo::find_model(&st.models_root, &req.model).ok();
    if let Some(m) = &meta {
        req.options = merge_model_parameters(&m.parameters, req.options.take());
    }
    // D4c：请求级（含模型级合并后）options.num_ctx 不一致时按原版语义重建实例
    let want_ctx = crate::adapter::requested_num_ctx(req.options.as_ref());
    let want_rt = crate::adapter::requested_runtime(req.options.as_ref());
    // 迭代43 M161：lease/load/port 可变——超窗重建重放需整体替换；want_rt
    // 首次传 clone 保留原值（重放 acquire 复用，迭代43）
    let mut lease = match st
        .scheduler
        .acquire(&req.model, keep_alive, want_ctx, want_rt.clone())
        .await
    {
        Ok(r) => r,
        // 迭代30 碴A（M96）：错误分类整形（原统一 404）
        Err(e) => return acquire_error_response(e),
    };
    // M28 碴6：acquire 耗时即 load_duration
    let mut load = request_start.elapsed();
    let mut port = lease.port().await;
    let stream = req.stream.unwrap_or(true);
    let raw = req.raw.unwrap_or(false);

    // M35 D1/D6（迭代15）：原生通道分派——优先级 suffix（/infill FIM）>
    // context（/completion token 数组续传；tokenize 失败降级常规通道）>
    // 常规 raw/chat 通道。context 通道近似语义注记：官方对 context 的拼接
    // 发生在模板格式化后，roxid 透传架构下模板在 llama-server 内部，
    // token 级前置只能在原生通道实现（模型裸续传，对续写场景行为一致）
    let native: Option<NativeChannel> = if req.suffix.as_deref().is_some_and(|s| !s.is_empty()) {
        Some(NativeChannel::Infill)
    } else if req.context.as_ref().is_some_and(|c| !c.is_empty()) {
        match tokenize_prompt(port, &req.prompt).await {
            Some(mut tokens) => {
                let mut prompt_tokens: Vec<u64> = req
                    .context
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&t| u64::from(t))
                    .collect();
                prompt_tokens.append(&mut tokens);
                Some(NativeChannel::Completion { prompt_tokens })
            }
            None => {
                // tokenize 不可用：降级常规通道（context 丢弃），不阻断请求
                tracing::warn!("context 通道 tokenize 失败，降级常规通道（context 丢弃）");
                None
            }
        }
    } else {
        None
    };

    // 迭代43 M161：path 与 port 分离——超窗重建后端口变化，url 按新 port
    // 重排（重放复用同款 body，零协议漂移）
    let (path, body, chat_mode) = match &native {
        Some(NativeChannel::Infill) => {
            ("/infill", build_native_infill_request(&req, stream), false)
        }
        Some(NativeChannel::Completion { prompt_tokens }) => (
            "/completion",
            build_native_completion_request(&req, prompt_tokens.clone(), stream),
            false,
        ),
        None if raw => (
            "/v1/completions",
            build_openai_completion_request(&req, stream),
            false,
        ),
        None => {
            // prompt → user 消息；images 并入该消息。
            // M26 碴2：system 解析——请求级优先，缺省回退 meta.system（空串视为未设置）；
            // meta.messages 预置对话注入在 system 之后、user 之前
            let system = req
                .system
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| meta.as_ref().map(|m| m.system.clone()).unwrap_or_default());
            let meta_messages = match &meta {
                Some(m) => &m.messages,
                None => &EMPTY_PRESET_MESSAGES,
            };
            let messages = apply_model_preset(
                vec![crate::adapter::OllamaMessage {
                    role: "user".into(),
                    content: req.prompt.clone(),
                    images: req.images.clone().unwrap_or_default(),
                    tool_calls: None,
                    tool_name: None,
                    thinking: None,
                }],
                &system,
                meta_messages,
            );
            let chat_req = OllamaChatRequest {
                model: req.model.clone(),
                messages,
                format: req.format.clone(),
                options: req.options.clone(),
                tools: None,
                stream: Some(stream),
                keep_alive: req.keep_alive.clone(),
                think: req.think.clone(),
                // M35 D7：generate 的 logprobs 需求经 chat 通道同款透传
                logprobs: req.logprobs,
                top_logprobs: req.top_logprobs,
            };
            let body = build_openai_chat_request(&chat_req, stream);
            ("/v1/chat/completions", body, true)
        }
    };
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut resp = match http().post(&url).json(&body).send().await {
        Ok(r) => r,
        // M34 BUG-7：连接类失败转模型级文案（不暴露内部 URL/端口）
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, upstream_failure_message(e)),
    };
    // 迭代43 M161（2026-09-11 23:03 批准）：超窗自动扩窗重建重放——与
    // chat（M160）同款：Q1=B 裁决（22:56）；三通道（infill/completion/
    // chat 模板）统一按 n_prompt_tokens 扩窗重建后重放；R2 重试上限 1 次
    if resp.status() == StatusCode::BAD_REQUEST {
        let text = resp.text().await.unwrap_or_default();
        match crate::adapter::parse_exceed_context_error(&text) {
            Some(n_prompt) => {
                let train_ctx = gguf_context_length(&st.models_root, &req.model);
                let target = crate::adapter::expanded_ctx_target(n_prompt, train_ctx);
                tracing::info!(
                    model = %display_name, n_prompt_tokens = n_prompt, target_ctx = target,
                    "超窗触发自动扩窗重建（迭代43 M161）"
                );
                lease = match st
                    .scheduler
                    .acquire(&req.model, keep_alive, Some(target), want_rt)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => return acquire_error_response(e),
                };
                load = request_start.elapsed(); // M28 碴6 口径：重建耗时并入 load_duration
                port = lease.port().await;
                let url = format!("http://127.0.0.1:{port}{path}");
                resp = match http().post(&url).json(&body).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        return error_response(StatusCode::BAD_GATEWAY, upstream_failure_message(e))
                    }
                };
            }
            None => return error_response(StatusCode::BAD_REQUEST, text),
        }
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return error_response(status, text);
    }

    // M28 碴6：上游 headers 已到即首字节时刻
    let first_byte = std::time::Instant::now();
    if stream {
        // M35 D1/D6：原生通道走独立 NDJSON 状态机（上游为 NDJSON 非 SSE）
        if let Some(native_ch) = native {
            let context_prefix: Vec<u64> = match &native_ch {
                NativeChannel::Completion { .. } => req
                    .context
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(u64::from)
                    .collect(),
                NativeChannel::Infill => Vec::new(),
            };
            let body = native_ndjson_to_ollama(
                display_name,
                resp,
                lease,
                request_start,
                first_byte,
                load,
                port,
                req.prompt.clone(),
                context_prefix,
            );
            return Response::builder()
                .header("content-type", "application/x-ndjson")
                .body(body)
                .unwrap();
        }
        // generate 形态：chat delta.content → response 字段；completions text → response
        let model = display_name; // M28 碴7：规范化名回显
        let converter: ChunkConverter = if chat_mode {
            |m, chunk| {
                // 复用 chat 转换后重塑为 generate 形态
                openai_chunk_to_ollama_chat_events(m, chunk)
                    .into_iter()
                    .map(|mut ev| {
                        let content = ev["message"]["content"].take();
                        let thinking = ev["message"]["thinking"].take();
                        ev.as_object_mut().unwrap().remove("message");
                        ev["response"] = content;
                        if !thinking.is_null() {
                            ev["thinking"] = thinking;
                        }
                        ev
                    })
                    .collect()
            }
        } else {
            openai_chunk_to_ollama_generate_event_single
        };
        let body = sse_to_ndjson(
            model,
            resp,
            converter,
            lease,
            StreamExtras {
                request_start,
                first_byte,
                load,
                // M28 碴8：generate 流式末事件 context 回填（raw/chat 通道同语义）
                context: Some(StreamContext {
                    port,
                    prompt: req.prompt.clone(),
                }),
            },
        );
        Response::builder()
            .header("content-type", "application/x-ndjson")
            .body(body)
            .unwrap()
    } else {
        // M30 碴12：上游非 JSON 体显式 502（语义同 chat 非流式分支）
        let openai: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => {
                return error_response(StatusCode::BAD_GATEWAY, "上游响应非 JSON");
            }
        };
        let mut out = if native.is_some() {
            // M35 D1/D6：原生响应（/completion 与 /infill 同构）→ Ollama 形态
            native_completion_to_ollama(&display_name, &openai)
        } else if chat_mode {
            // 聚合：复用 chat 非流式转换后重塑
            let mut ev = openai_chat_response_to_ollama(&display_name, &openai);
            let content = ev["message"]["content"].take();
            // M35 D10：非流式 thinking 顶层提升（与流式通道对齐——原复用
            // chat 转换后仅取 content，thinking 随 message 移除被丢弃）
            let thinking = ev["message"]["thinking"].take();
            ev.as_object_mut().unwrap().remove("message");
            ev["response"] = content;
            if !thinking.is_null() {
                ev["thinking"] = thinking;
            }
            ev
        } else {
            openai_completion_response_to_ollama(&display_name, &openai)
        };
        // M28 碴6：duration 四字段真实计量（原硬编码 0）
        crate::adapter::apply_duration_fields(&mut out, request_start, first_byte, load);
        // M102（迭代32 碴3）：非流式墙钟切分失真，上游 timings 覆盖 prompt/eval
        // 两 duration——chat_mode（/v1/chat/completions）、raw（/v1/completions）、
        // native（/completion、/infill）三通道响应顶层同构携带 timings
        crate::adapter::apply_timings_durations(&mut out, &openai["timings"]);
        // M28 碴8：context 回填（prompt + 生成全文 tokenize，失败降级空数组）；
        // M35 D1：context 通道前置请求 context（完整续传序列）
        let response_text = out["response"].as_str().unwrap_or_default().to_string();
        let base = tokenize_context(port, &format!("{}{}", req.prompt, response_text)).await;
        out["context"] = match &native {
            Some(NativeChannel::Completion { .. }) => {
                let mut full: Vec<u64> = req
                    .context
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(u64::from)
                    .collect();
                full.extend(base);
                json!(full)
            }
            _ => json!(base),
        };
        Json(out).into_response()
    }
}

/// completions 单事件包装（generate raw 模式流式）
fn openai_chunk_to_ollama_generate_event_single(model: &str, chunk: &Value) -> Vec<Value> {
    vec![openai_chunk_to_ollama_generate_event(model, chunk)]
}

/// /api/tags：本地模型列表
/// M32 碴7：经 list_models_with_dirs 以聚合扫描的真实持有目录计算 size
///（原 ModelRef 重新 locate 丢弃来源，同名跨源共存时 size 错位）
pub async fn tags(State(st): State<Arc<AppState>>) -> Response {
    let models: Vec<Value> = repo::list_models_with_dirs(&st.models_root)
        .into_iter()
        .map(|(dir, m)| {
            json!({
                "name": m.name,
                // M22 对齐原版：model 字段为完整名（model:tag）
                "model": m.name,
                "modified_at": m.created_at,
                "size": model_size(&dir, &m),
                "digest": m.digest,
                "details": {
                    "parent_model": "",
                    "format": "gguf",
                    "family": m.family,
                    "families": if m.families.is_empty() { json!(null) } else { json!(m.families) },
                    "parameter_size": m.parameter_size,
                    "quantization_level": m.quantization_level,
                }
            })
        })
        .collect();
    Json(json!({"models": models})).into_response()
}

/// 模型目录内全部大文件字节和（gguf+mmproj+lora）。
/// M32 碴7：dir 为聚合扫描携带的真实持有目录（原 ModelRef::parse(name)
/// 重新 locate 在同名跨源共存时错位到另一源目录，size 随之错位）。
///
/// - 参数 dir：模型真实持有目录（list_models_with_dirs 所得）
/// - 参数 m：模型元数据
/// - 返回：目录内全部大文件字节和
fn model_size(dir: &std::path::Path, m: &crate::repo::ModelMeta) -> u64 {
    let mut names = vec![m.files.model.clone()];
    if let Some(mm) = &m.files.mmproj {
        names.push(mm.clone());
    }
    names.extend(m.adapters.iter().cloned());
    names
        .iter()
        .filter_map(|n| std::fs::metadata(dir.join(n)).ok())
        .map(|md| md.len())
        .sum()
}

/// /api/show：模型详情（modelfile 复原 / 参数 / 模板 / 能力）。
/// M35（迭代15）：D2 capabilities 官方字符串数组；D3 parameters 多行文本；
/// D13 model_info 官方键名与数字类型 + modified_at + verbose 读取
pub async fn show(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let name = req["model"].as_str().unwrap_or_default();
    // M35 D13：官方 verbose 字段读取（true 时请求大字段形态；roxid 的
    // model_info 键集合不因 verbose 变化，读取消费为协议接入口）
    let _verbose = req["verbose"].as_bool().unwrap_or(false);
    let meta = match repo::find_model(&st.models_root, name) {
        Ok(m) => m,
        Err(e) => return error_response(StatusCode::NOT_FOUND, e),
    };
    // M35 D13：GGUF header 实时解析（官方键名/数字口径数据源，零存储
    // 变更；文件缺失/解析失败静默降级 meta 兜底链）
    let gguf = ModelRef::parse(name)
        .ok()
        .and_then(|r| repo::locate_model_dir(&st.models_root, &r))
        .and_then(|(_, dir)| crate::registry::gguf::parse_metadata(&dir.join("model.gguf")).ok());
    // Modelfile 复原（与原版 show 输出一致的 FROM/SYSTEM/TEMPLATE/PARAMETER 文本）
    let mut modelfile = format!("# Modelfile generated by roxid\nFROM {}\n", meta.name);
    if !meta.system.is_empty() {
        modelfile.push_str(&format!("\nSYSTEM \"\"\"{}\"\"\"\n", meta.system));
    }
    if let Some(t) = &meta.template {
        modelfile.push_str(&format!("\nTEMPLATE \"\"\"{t}\"\"\"\n"));
    }
    modelfile.push_str(&parameter_lines(&meta.parameters));
    // M38：RUNTIME 复原（roxid 扩展指令；有值时输出，往返 parse 可恢复）
    if let Some(rt) = &meta.runtime {
        modelfile.push_str(&format!("\nRUNTIME {rt}\n"));
    }
    // M35 D13：model_info 官方键名与类型——arch 取 GGUF general.architecture
    // （兜底 meta.family）；context_length 取 GGUF {arch}.context_length
    // （兜底 meta 参数层 num_ctx → 4096）；parameter_count 取 tensor 累加
    // （兜底规模标签估算，无法估算省略键）；file_type 取 GGUF 原始枚举
    // （兜底量化名反查）；动态键名 {arch}.context_length 需手工构造 Map
    let arch = gguf
        .as_ref()
        .and_then(|g| g.architecture.clone())
        .unwrap_or_else(|| meta.family.clone());
    let num_ctx = gguf
        .as_ref()
        .and_then(|g| g.context_length)
        .unwrap_or_else(|| {
            meta.parameters
                .get("num_ctx")
                .and_then(|v| v.as_u64())
                .unwrap_or(4096)
        });
    let parameter_count = gguf
        .as_ref()
        .and_then(|g| g.parameter_count)
        .or_else(|| parse_size_label_count(&meta.parameter_size));
    let file_type = gguf
        .as_ref()
        .and_then(|g| g.file_type)
        .or_else(|| crate::registry::gguf::quant_name_to_file_type(&meta.quantization_level));
    let mut model_info = serde_json::Map::new();
    model_info.insert("general.architecture".into(), json!(arch));
    model_info.insert(format!("{arch}.context_length"), json!(num_ctx));
    if let Some(pc) = parameter_count {
        model_info.insert("general.parameter_count".into(), json!(pc));
    }
    if let Some(ft) = file_type {
        model_info.insert("general.file_type".into(), json!(ft));
    }
    // M35 D2：capabilities 官方字符串数组（completion 恒有；tools 透传架构
    // 下 chat template 工具调用恒可达；vision 按 mmproj；embedding 按家族）
    let mut capabilities = vec![json!("completion"), json!("tools")];
    if meta.files.mmproj.is_some() {
        capabilities.push(json!("vision"));
    }
    if is_embedding_family(&meta.family) {
        // M28 碴9：家族集合判定（原 contains("embed") 漏判 bert 系）
        capabilities.push(json!("embedding"));
    }
    Json(json!({
        // M22：license 来自 pull 时保存的 license 层（旧元数据为空串）
        "license": meta.license,
        "modelfile": modelfile,
        // M35 D3：官方形态为序列化多行文本（PARAMETER 行，与 modelfile 复原
        // 同源；原 BTreeMap 对象与官方字符串类型不符）
        "parameters": parameter_lines(&meta.parameters),
        "template": meta.template,
        "system": meta.system,
        // M35 D13：官方响应恒有字段（ISO 8601；meta.created_at 同源 tags）
        "modified_at": meta.created_at,
        "details": {
            "parent_model": "",
            "format": "gguf",
            "family": meta.family,
            "families": if meta.families.is_empty() { json!(null) } else { json!(meta.families) },
            "parameter_size": meta.parameter_size,
            "quantization_level": meta.quantization_level,
        },
        "model_info": Value::Object(model_info),
        "capabilities": capabilities,
    }))
    .into_response()
}

/// 规模标签（"0.6B"/"8.0B"/"135M"）→ 参数数量级估算（M35 D13 parameter_count
/// 兜底；GGUF tensor 累加缺位时使用——标签粒度估算非精确计数，宁缺毋滥
/// 仅在可解析时提供）。
///
/// - 参数 label：meta.parameter_size 规模标签
/// - 返回：估算参数数；无法解析 None（省略键）
fn parse_size_label_count(label: &str) -> Option<u64> {
    let s = label.trim();
    let split_at = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num_part, unit) = s.split_at(split_at);
    let n: f64 = num_part.parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    let mult = match unit.trim().to_ascii_uppercase().as_str() {
        "B" => 1e9,
        "M" => 1e6,
        "K" => 1e3,
        "" => 1.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// /api/ps：运行中模型
pub async fn ps(State(st): State<Arc<AppState>>) -> Response {
    let mut models = st.scheduler.list_running().await;
    // 补充 digest（仓库元数据；迭代38 M135：HF 直引回退 model 层 sha256）
    for m in &mut models {
        if let Ok(meta) = repo::find_model(&st.models_root, &m.name) {
            m.digest = resolve_ps_digest(&meta);
        }
    }
    Json(json!({"models": models})).into_response()
}

/// /api/ps digest 补齐值：顶层 digest 空时回退 model 层 sha256
/// （迭代38 M135）——HF 直引落 meta 时顶层 digest 置空、文件 sha256 仅
/// 入 layer_digests["model"]（M20），原只读顶层致 CLI ID 列恒「?」断链。
///
/// - 参数 meta：仓库模型元数据
/// - 返回：顶层摘要；空则 model 层摘要；两处皆空返回空串（CLI 兜底 "?"）
/// 来源：用户确认「有sha256，取哈希值后12位」2026-09-11 07:31
fn resolve_ps_digest(meta: &repo::ModelMeta) -> String {
    if meta.digest.is_empty() {
        meta.layer_digests.get("model").cloned().unwrap_or_default()
    } else {
        meta.digest.clone()
    }
}

/// /api/pull：拉取模型（NDJSON 进度流）。
/// M22：接受 insecure 字段（对齐原版语义——显式请求时跳过 TLS 证书校验）
pub async fn pull(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let Some(model) = req["model"].as_str().map(str::to_string) else {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model 字段");
    };
    // M28 碴16（R4-A）：insecure 贯穿下载链（原实现读取即弃，
    // 注释声称跳过证书校验从未生效）——true 时按请求构造危险证书客户端
    let insecure = req["insecure"].as_bool().unwrap_or(false);
    // 迭代44 M164（Q3-X 裁决 2026-09-11 23:36）：CLI 交互选定的投影器
    // 文件名经可选 mmproj 字段传入（官方客户端不发送该字段——缺省 None
    // 时单一变体自动选中、多变体经 hf.pull 报错引导 CLI 交互，Q2-B）
    let mmproj_choice = req["mmproj"].as_str().map(str::to_string);
    // hf.co/{user}/{repo}[:tag] 语法走 HF 辅源（Q3 裁决）
    let use_hf = model.starts_with("hf.co/");
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);

    // M29 碴2：去重键规范化——registry 内部经 ModelRef::parse 规范化到同一
    // 下载目录，键若用原始名则别名并发（"llama3.2" vs "llama3.2:latest"）
    // 去重失效、.parts 同名 tmp 竞争回归（M28 碴2 修复不完整面）
    let pull_key = normalize_pull_key(&model);
    // M28 碴2（R1-A 裁决）：同模型并发 pull 去重——表内已有同模型任务时本请求
    // 等待其终态并转发（无中途进度），不重复下载、不互踩 .parts 目录
    let (existing_gate, own_gate) = {
        let mut pulls = st.pulls.lock().await;
        match pulls.get(&pull_key) {
            Some(gate) => (Some(gate.clone()), None),
            None => {
                let gate = Arc::new(PullGate::new());
                pulls.insert(pull_key.clone(), gate.clone());
                (None, Some(gate))
            }
        }
    };
    if let Some(gate) = existing_gate {
        tokio::spawn(async move {
            // 等待者：终态转发为 NDJSON 事件（与首个请求的终态事件语义一致）
            // M93 碴B（迭代29）：终态前 200ms ticker 读 PullGate 共享快照，
            // 转发与首请求同形态的真实进度事件（completed/total 随首任务
            // 下载递增；200ms 对齐 downloader 既有上报粒度）。total==0
            //（首任务 manifest 阶段尚无字节级快照）跳过进度帧防 0/0 误显
            // 100%；tx 发送失败（接收端断开，如 CLI 已退出）即停止转发，
            // 终态无需再发
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(200));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let terminal = loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let (completed, total) = gate.snapshot_progress();
                        if total > 0 {
                            let line = json!({
                                "status": "pulling",
                                "completed": completed,
                                "total": total,
                            })
                            .to_string();
                            if tx.send(line).await.is_err() {
                                return; // 接收端断开：转发无意义，任务收尾
                            }
                        }
                    }
                    r = gate.wait() => break r,
                }
            };
            let line = match terminal {
                Ok(()) => json!({"status": "success"}).to_string(),
                Err(e) => json!({"error": e}).to_string(),
            };
            let _ = tx.send(line).await;
        });
    } else if let Some(gate) = own_gate {
        {
            // 首个请求：执行下载；结束经 PullGuard 完成门并清表（panic 兜底）
            // M28 碴16：insecure=true 时按请求构造危险证书客户端（默认路径
            // 复用 AppState 单例，行为与现状逐字节一致）
            let registry = if insecure {
                Arc::new(crate::registry::OllamaRegistry::with_insecure(true))
            } else {
                st.registry.clone()
            };
            let hf = Arc::new(if insecure {
                crate::registry::HuggingFaceSource::with_insecure(true)
            } else {
                crate::registry::HuggingFaceSource::new()
            });
            let root = st.models_root.clone();
            // M93 碴B：快照写入句柄——回调经此同步进度到 PullGate 供等待者
            // 转发；克隆须先于下方 guard 对 gate 的 move 消费
            let progress_gate = gate.clone();
            // M29 碴2：清表键同步使用规范化键（guard 持键与登记键一致）
            let guard = PullGuard::new(st.clone(), pull_key.clone(), gate);
            tokio::spawn(async move {
                // M29 碴11：进度丢弃计数——通道满（慢客户端）时非终态进度事件
                // 可丢，但静默丢弃使进度条停滞无从排查，留 debug 痕迹
                let mut dropped = 0u32;
                let result = if use_hf {
                    let rest = model.trim_start_matches("hf.co/");
                    let (repo, tag) = match rest.split_once(':') {
                        Some((r, t)) => (r.to_string(), Some(t.to_string())),
                        None => (rest.to_string(), None),
                    };
                    hf.pull(
                        &repo,
                        tag.as_deref(),
                        &root,
                        mmproj_choice.as_deref(),
                        |ev| {
                            send_progress_event(&tx, &ev, &mut dropped);
                            update_gate_snapshot(&progress_gate, &ev);
                        },
                    )
                    .await
                } else {
                    registry
                        .pull_model(&model, &root, |ev| {
                            send_progress_event(&tx, &ev, &mut dropped);
                            update_gate_snapshot(&progress_gate, &ev);
                        })
                        .await
                };
                if let Err(e) = &result {
                    let _ = tx
                        .send(
                            serde_json::to_string(&crate::registry::PullEvent::error(
                                e.to_string(),
                            ))
                            .unwrap_or_default(),
                        )
                        .await;
                }
                guard
                    .finish(result.map(|_| ()).map_err(|e| e.to_string()))
                    .await;
            });
        }
    }
    let body = Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|line| {
            let mut l = line;
            l.push('\n');
            (Ok::<Bytes, std::io::Error>(Bytes::from(l)), rx)
        })
    }));
    Response::builder()
        .header("content-type", "application/x-ndjson")
        .body(body)
        .unwrap()
}

/// /api/copy：复制模型（硬链接，Q9 裁决）
pub async fn copy(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let (Some(src), Some(dst)) = (req["source"].as_str(), req["destination"].as_str()) else {
        return error_response(StatusCode::BAD_REQUEST, "缺少 source/destination");
    };
    let src_ref = match ModelRef::parse(src) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e),
    };
    let dst_ref = match ModelRef::parse(dst) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e),
    };
    match repo::copy_model(&st.models_root, &src_ref, &dst_ref) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e),
    }
}

/// /api/embed：向量化（M8）。input 支持字符串或字符串数组；
/// llama-server 对 embedding 模型自动启用 pooling（GGUF 元数据驱动）。
pub async fn embed(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let model = req["model"].as_str().unwrap_or_default().to_string();
    if model.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model 字段");
    }
    let input = match &req["input"] {
        v if v.is_string() || v.is_array() => v.clone(),
        _ => return error_response(StatusCode::BAD_REQUEST, "缺少 input 字段"),
    };
    // M35 D11：truncate 语义——官方默认 true（超长截断）；显式 false 时
    // 超长输入报 400 而非静默截断
    let truncate = req["truncate"].as_bool().unwrap_or(true);
    let keep_alive = parse_keep_alive(req.get("keep_alive"));
    let want_ctx = crate::adapter::requested_num_ctx(req.get("options"));
    let want_rt = crate::adapter::requested_runtime(req.get("options"));
    let request_start = std::time::Instant::now(); // M29 碴4：计时起点
    let lease = match st
        .scheduler
        .acquire(&model, keep_alive, want_ctx, want_rt)
        .await
    {
        Ok(r) => r,
        // 迭代30 碴A（M96）：错误分类整形（原统一 404）
        Err(e) => return acquire_error_response(e),
    };
    let load = request_start.elapsed(); // M29 碴4：acquire 耗时即 load_duration
    let port = lease.port().await;
    // M35 D11：truncate=false 时 tokenize 检查超限（任一输入超 num_ctx 即
    // 400；限值与实例窗口一致——请求级 want_ctx 或默认 4096）
    if !truncate {
        let limit = u64::from(want_ctx.unwrap_or(4096));
        let texts: Vec<&str> = match &input {
            Value::String(s) => vec![s.as_str()],
            Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
            _ => vec![],
        };
        for t in texts {
            if let Some(tokens) = tokenize_prompt(port, t).await {
                if tokens.len() as u64 > limit {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "输入长度超过上下文窗口：truncate=false 不截断",
                    );
                }
            }
        }
    }
    let resp = match http()
        .post(format!("http://127.0.0.1:{port}/v1/embeddings"))
        // M30 碴9：转发体剥离 model 路由字段（model 仅 roxid 调度键，
        // llama-server 单模型实例忽略属侥幸容错——对齐 M29 碴3 /v1 层先例）
        .json(&json!({"input": input}))
        .send()
        .await
    {
        Ok(r) => r,
        // M34 BUG-7：连接类失败转模型级文案（不暴露内部 URL/端口）
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, upstream_failure_message(e)),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // M34 BUG-6：上游错误单层化（解包内层 message；Pooling 场景转官方口径）
        return error_response(status, format_upstream_error(&text));
    }
    let openai: Value = resp.json().await.unwrap_or(Value::Null);
    // OpenAI {data:[{embedding}]} → Ollama {embeddings:[[...]]}
    let embeddings: Vec<Value> = openai["data"]
        .as_array()
        .map(|arr| arr.iter().map(|d| d["embedding"].clone()).collect())
        .unwrap_or_default();
    // M29 碴4：补齐原版 embed 响应计量字段（原仅 model+embeddings；chat/
    // generate 已于 M28 修复，embed 遗漏）——total/load 实测（口径同 M28
    // 碴6），prompt_eval_count 取上游 usage.prompt_tokens
    // M33 碴2：total 取 body 读取完成时刻——原计至响应头到达（first_byte），
    // 截断 body 传输与解析耗时，与「请求到达→响应完成」R2-A 口径不一致
    let total = std::time::Instant::now().saturating_duration_since(request_start);
    let prompt_eval_count = openai["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    // M28 碴7：回显规范化完整名
    let display_name = display_model_name(&model);
    Json(json!({
        "model": display_name,
        "embeddings": embeddings,
        "total_duration": total.as_nanos() as u64,
        "load_duration": load.as_nanos() as u64,
        "prompt_eval_count": prompt_eval_count,
    }))
    .into_response()
}

/// /api/embeddings：旧版向量化端点（prompt 单值 → {embedding}）
pub async fn embeddings_legacy(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let model = req["model"].as_str().unwrap_or_default().to_string();
    let prompt = req["prompt"].as_str().unwrap_or_default().to_string();
    if model.is_empty() || prompt.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model/prompt");
    }
    let keep_alive = parse_keep_alive(req.get("keep_alive"));
    let want_ctx = crate::adapter::requested_num_ctx(req.get("options"));
    let want_rt = crate::adapter::requested_runtime(req.get("options"));
    let lease = match st
        .scheduler
        .acquire(&model, keep_alive, want_ctx, want_rt)
        .await
    {
        Ok(r) => r,
        // 迭代30 碴A（M96）：错误分类整形（原统一 404）
        Err(e) => return acquire_error_response(e),
    };
    let port = lease.port().await;
    let resp = match http()
        .post(format!("http://127.0.0.1:{port}/v1/embeddings"))
        // M30 碴9：转发体剥离 model 路由字段（语义同 embed 主端点）
        .json(&json!({"input": prompt}))
        .send()
        .await
    {
        Ok(r) => r,
        // M34 BUG-7：连接类失败转模型级文案（不暴露内部 URL/端口）
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, upstream_failure_message(e)),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // M34 BUG-6：上游错误单层化（解包内层 message；Pooling 场景转官方口径）
        return error_response(status, format_upstream_error(&text));
    }
    let openai: Value = resp.json().await.unwrap_or(Value::Null);
    Json(json!({"embedding": openai["data"][0]["embedding"]})).into_response()
}

/// M35 D4（迭代15）：create 请求来源解析。
/// 形态优先级：modelfile 文本字段（官方弃用但接受）> from 值含 FROM 指令
///（roxid 既有 from 承载 Modelfile 文本形态，兼容保留）> from 为基础模型
/// 名（官方 0.5+ 结构化形态）→ 结构化字段构造等价 Modelfile。
/// 结构化字段映射：system→SYSTEM、parameters(对象)→PARAMETER 逐键、
/// messages→MESSAGE 逐条、template→TEMPLATE；license/quantize/renderer/
/// parser 为官方推送/量化转换管线概念（roxid 无对应实现，注记忽略——
/// 本地衍生无消费点）；stream 恒 NDJSON 维持（P2 超集注记）。
///
/// - 参数 req：create 请求 JSON
/// - 返回：等价 Modelfile；无有效来源报错
fn resolve_modelfile(req: &Value) -> crate::error::RoxidResult<crate::modelfile::Modelfile> {
    use crate::error::RoxidError;
    if let Some(text) = req["modelfile"].as_str() {
        return crate::modelfile::parse(text);
    }
    let from = req["from"].as_str().unwrap_or_default().trim();
    if from.to_uppercase().starts_with("FROM") {
        // 既有形态兼容：from 字段承载完整 Modelfile 文本
        return crate::modelfile::parse(from);
    }
    if from.is_empty() {
        return Err(RoxidError::InvalidRequest(
            "缺少 from/modelfile 字段".into(),
        ));
    }
    let mut mf = crate::modelfile::Modelfile {
        from: from.to_string(),
        ..Default::default()
    };
    if let Some(s) = req["system"].as_str() {
        mf.system = Some(s.to_string());
    }
    if let Some(t) = req["template"].as_str() {
        mf.template = Some(t.to_string());
    }
    if let Some(params) = req["parameters"].as_object() {
        for (k, v) in params {
            mf.parameters.insert(k.clone(), v.clone());
        }
    }
    if let Some(msgs) = req["messages"].as_array() {
        for m in msgs {
            mf.messages.push(crate::repo::ChatMessage {
                role: m["role"].as_str().unwrap_or("user").to_string(),
                content: m["content"].as_str().unwrap_or_default().to_string(),
            });
        }
    }
    Ok(mf)
}

/// /api/create：Modelfile 创建模型（M9）。
/// M22：NDJSON 流式进度（对齐原版 create 的状态事件序列）；
/// M35 D4：官方 0.5+ 结构化形态支持（from=基础模型名 + system/parameters/
/// messages/template 结构化字段，R5-A 字段形态分派）
pub async fn create(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let name = req["model"].as_str().unwrap_or_default().to_string();
    let has_source = req["modelfile"].is_string() || req["from"].is_string();
    if name.is_empty() || !has_source {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model/from");
    }
    // 创建过程经 NDJSON 流式输出（pulling manifest/writing manifest/success 序列）
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
    let root = st.models_root.clone();
    let scheduler = st.scheduler.clone();
    tokio::spawn(async move {
        // M32 碴9b：try_send 失败（通道满/接收端已断）留 debug 痕迹——
        // 对齐 send_progress_event 先例，创建状态事件不再静默丢失
        let send_status = |tx: &tokio::sync::mpsc::Sender<String>, s: &str| {
            if tx.try_send(json!({"status": s}).to_string()).is_err() {
                tracing::debug!("create 状态事件丢弃（通道满或客户端已断）：{s}");
            }
        };
        send_status(&tx, "pulling manifest");
        // M35 D4：三形态分派解析（modelfile 文本 / from 文本兼容 / from
        // 基础模型名结构化构造）
        let result = resolve_modelfile(&req).and_then(|mf| {
            send_status(&tx, "writing manifest");
            crate::modelfile::create_model(&mf, &name, &root)
        });
        match result {
            Ok(_) => {
                // M28 碴14：覆盖更新成功后停同名运行实例（对齐 delete 先例——
                // 否则旧实例持旧 GGUF inode 继续跑旧模型，新模型不生效）
                if let Err(e) = scheduler.stop(&name).await {
                    tracing::debug!("create 后停止运行实例（未运行为常态）：{e}");
                }
                send_status(&tx, "success");
            }
            Err(e) => {
                let _ = tx.try_send(json!({"error": e.to_string()}).to_string());
            }
        }
    });
    let body = Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|line| {
            let mut l = line;
            l.push('\n');
            (Ok::<Bytes, std::io::Error>(Bytes::from(l)), rx)
        })
    }));
    Response::builder()
        .header("content-type", "application/x-ndjson")
        .body(body)
        .unwrap()
}

/// /api/push：推送模型到 registry（M9）。
/// 需要 ~/.roxid/auth.json 凭据（signin 写入）。
/// 推送管线维持挂账（用户裁决 Q3 2026-08-24 22:11：迭代3不含 push/launch）
pub async fn push(
    State(_st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let Some(name) = req["model"].as_str() else {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model 字段");
    };
    let auth_file = crate::config::roxid_home().join("auth.json");
    if !auth_file.is_file() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "未登录：请先 roxid signin（凭据保存于 ~/.roxid/auth.json）",
        );
    }
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        format!("push {name}：推送管线维持挂账（迭代2/迭代3 裁决排除）"),
    )
}

/// /api/delete：删除模型
pub async fn delete_model(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let Some(name) = req["model"].as_str() else {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model 字段");
    };
    let r = match ModelRef::parse(name) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e),
    };
    // 运行中的实例先停止
    let _ = st.scheduler.stop(name).await;
    match repo::delete_model(&st.models_root, &r) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => error_response(StatusCode::NOT_FOUND, e),
    }
}

/// /api/stop：卸载运行中模型（roxid 扩展端点；原版 CLI 经内部通道）
pub async fn stop(
    State(st): State<Arc<AppState>>,
    LenientJson(req): LenientJson<Value>,
) -> Response {
    let Some(name) = req["model"].as_str() else {
        return error_response(StatusCode::BAD_REQUEST, "缺少 model 字段");
    };
    match st.scheduler.stop(name).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => error_response(StatusCode::NOT_FOUND, e),
    }
}

/// /api/blobs/{digest}：按摘要返回本地 blob 字节流（M20，Q1 裁决）。
/// 命中层摘要索引（model/projection/adapter-N → GGUF；config → model.json），
/// 大文件分块流式直通，禁止全量读入内存。
pub async fn blobs_get(State(st): State<Arc<AppState>>, Path(digest): Path<String>) -> Response {
    let Some(loc) = repo::find_blob(&st.models_root, &digest) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = match tokio::fs::File::open(&loc.path).await {
        Ok(f) => f,
        Err(e) => return error_response(StatusCode::NOT_FOUND, e),
    };
    let body = blobs_body(file); // M28 碴15：读错误显式中止（见函数注释）
    Response::builder()
        .header("content-type", "application/octet-stream")
        .header("content-length", loc.size.to_string())
        .body(body)
        .unwrap()
}

/// HEAD /api/blobs/{digest}：存在性探测（200 + Content-Length / 404，无 body）
pub async fn blobs_head(State(st): State<Arc<AppState>>, Path(digest): Path<String>) -> Response {
    match repo::find_blob(&st.models_root, &digest) {
        Some(loc) => Response::builder()
            .status(StatusCode::OK)
            .header("content-length", loc.size.to_string())
            .body(Body::empty())
            .unwrap(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// pull 任务收尾守卫（M28 碴2）：正常路径 finish 完成门并清表；
/// panic/提前退出由 Drop 兜底（等待者以失败终态释放、表项不泄漏）。
struct PullGuard {
    /// 应用状态（清表用）
    state: Arc<AppState>,
    /// 去重键（原始模型名）
    model: String,
    /// 本任务的完成门
    gate: Arc<PullGate>,
    /// 正常 finish 已执行标记（Drop 兜底据此跳过）
    finished: bool,
}

impl PullGuard {
    /// 构造守卫（此时门已在 AppState.pulls 表内注册）。
    ///
    /// - 参数 state / model / gate：状态、去重键与完成门
    fn new(state: Arc<AppState>, model: String, gate: Arc<PullGate>) -> Self {
        Self {
            state,
            model,
            gate,
            finished: false,
        }
    }

    /// 正常收尾：登记终态并移除表项（同门校验，防误删同 key 后续新任务）。
    ///
    /// - 参数 r：下载任务终态
    async fn finish(mut self, r: Result<(), String>) {
        self.finished = true;
        self.gate.finish(r);
        let mut pulls = self.state.pulls.lock().await;
        // 同门指针校验：仅当表内仍是本任务的门才移除（防误删同 key 后续新任务）
        if pulls
            .get(&self.model)
            .is_some_and(|g| Arc::ptr_eq(g, &self.gate))
        {
            pulls.remove(&self.model);
        }
    }
}

impl Drop for PullGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // panic 兜底：完成门（失败终态）+ 异步清表（Drop 内禁止阻塞，复用
        // RunnerLease::drop 的 Handle::try_current 先例）
        self.gate.finish(Err("pull 任务异常终止".into()));
        let state = self.state.clone();
        let model = self.model.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                state.pulls.lock().await.remove(&model);
            });
        }
    }
}

/// PARAMETER 复原文本（M30 碴1）：数组值（M29 碴8 stop 白名单多值）逐元素
/// 展开多行 `PARAMETER {k} {元素}`（对齐原版多条 PARAMETER stop 形态）——
/// 紧凑 JSON 单行 `PARAMETER stop ["a","b"]` 无法被 parse 往返（值以 '['
/// 开头，trim_matches('"') 剥不开、coerce_value 产出字符串字面量），下游
/// 会把整串 JSON 当停止词。标量维持单行。
///
/// - 参数 params：模型元数据参数集合
/// - 返回：可直接拼接的 PARAMETER 行文本（含前导换行）
fn parameter_lines(params: &std::collections::BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    for (k, v) in params {
        match v.as_array() {
            Some(arr) => {
                for elem in arr {
                    out.push_str(&format!("\nPARAMETER {k} {elem}\n"));
                }
            }
            None => out.push_str(&format!("\nPARAMETER {k} {v}\n")),
        }
    }
    out
}

/// 规范化回显名（M28 碴7）：补全缺省 tag（"llama3.2" → "llama3.2:latest"），
/// 对齐原版 Ollama 响应 model 字段语义与 acquire 的查找键（full_name）；
/// 非法输入原样返回（acquire 已先行拒绝，此处兜底不二次报错）。
///
/// - 参数 name：用户请求的模型名
/// - 返回：规范化完整名（model:tag）
fn display_model_name(name: &str) -> String {
    ModelRef::parse(name)
        .map(|r| r.full_name())
        .unwrap_or_else(|_| name.to_string())
}

/// pull 去重键规范化（M29 碴2）：以 ModelRef 规范化完整名（model:tag）为键，
/// 使别名并发（"llama3.2" 与 "llama3.2:latest"）命中同一任务（registry 内部
/// 即按 full_name 规范化落目录）；解析失败（非法名或 hf.co/ 路径形态）
/// 回退原名——报错语义仍由 registry 层承担，键不做二次报错。
/// M30 碴10：与 display_model_name 语义完全同构（M28/M29 分别引入的逐字
/// 重复实现合一）——委托单点实现，两名称保留各自调用点语义锚点。
///
/// - 参数 name：用户请求的原始模型名
/// - 返回：去重表键（规范化完整名或原样回退）
fn normalize_pull_key(name: &str) -> String {
    display_model_name(name)
}

/// 进度事件发送（M29 碴11）：通道满（慢客户端消费不及）时丢弃该条非终态
/// 进度事件并留 debug 日志与累计计数——进度可丢（终态事件经 finish 路径
/// 可靠送达），但静默丢弃使 NDJSON/CLI 进度停滞无从排查。
///
/// - 参数 tx：进度通道发送端
/// - 参数 ev：进度事件
/// - 参数 dropped：累计丢弃计数（调用方持有，跨事件累加）
fn send_progress_event(
    tx: &tokio::sync::mpsc::Sender<String>,
    ev: &crate::registry::PullEvent,
    dropped: &mut u32,
) {
    let line = serde_json::to_string(ev).unwrap_or_default();
    if tx.try_send(line).is_err() {
        *dropped += 1;
        tracing::debug!("pull 进度事件丢弃（通道满，慢客户端）：累计 {dropped} 条");
    }
}

/// M93 碴B（迭代29）：进度事件同步写入 PullGate 共享快照——首任务每发
/// 一条进度事件即更新快照，等待者 ticker 据此转发真实进度。completed/
/// total 缺失的状态事件（pulling manifest 等）跳过，快照保持最近一次
/// 字节级进度（等待者 manifest 阶段转发最近已知值而非回退为零）。
///
/// - 参数 gate / ev：完成门与待下发的进度事件
fn update_gate_snapshot(gate: &Arc<PullGate>, ev: &crate::registry::PullEvent) {
    if let (Some(completed), Some(total)) = (ev.completed, ev.total) {
        gate.update_progress(completed, total);
    }
}

/// 已知 embedding 模型家族判定（M28 碴9：原 contains("embed") 子串判定漏判
/// bert 系——nomic-bert/jina-bert-v2 等 GGUF architecture 均为 bert 词根的
/// embedding 模型；纯生成家族不含这些词根，无误报）。
///
/// - 参数 family：GGUF general.architecture 家族名
/// - 返回：true 表示 embedding 模型
fn is_embedding_family(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    f.contains("embed")
        || f == "bert"
        || f.starts_with("bert-")
        || f.contains("-bert")
        || f.starts_with("bge")
        || f.contains("-bge")
}

/// blobs 响应体：1MB 分块直通（futures::stream::unfold，零额外依赖）。
/// M28 碴15：读错误以 Err 项终结流——原实现 Err 静默当 EOF，流被截断但
/// content-length 已声明全量，客户端挂起/半包；Err 项使 axum 中断连接，
/// 客户端明确感知错误。
///
/// - 参数 file：已打开的 blob 文件
/// - 返回：分块直通 body
fn blobs_body(file: tokio::fs::File) -> Body {
    const CHUNK: usize = 1024 * 1024;
    // 状态为 Option<File>：Err 上报后置 None 使流终止（http-body 合约：
    // 错误帧之后不得继续产出——否则消费者轮询将收到无限错误流）
    Body::from_stream(futures::stream::unfold(
        Some(file),
        move |state| async move {
            let mut f = match state {
                Some(f) => f,
                None => return None, // 错误已上报：流终止
            };
            let mut buf = vec![0u8; CHUNK];
            match tokio::io::AsyncReadExt::read(&mut f, &mut buf).await {
                Ok(0) => None, // 正常 EOF：流自然结束
                Ok(n) => Some((
                    Ok::<Bytes, std::io::Error>(Bytes::from(buf[..n].to_vec())),
                    Some(f),
                )),
                Err(e) => {
                    tracing::warn!("blobs 流读取错误，响应以错误终结：{e}");
                    Some((
                        Err(std::io::Error::new(
                            e.kind(),
                            format!("blobs 读取中断：{e}"),
                        )),
                        None,
                    ))
                }
            }
        },
    ))
}

/// 合并模型预置内容到消息表头部（M26 碴2，R1 裁决：api 层组装，adapter 保持纯函数）。
/// 组装顺序：请求级 system（消息表首条，保持首位）或 meta.system 补位（非空时）
/// → meta.messages 预置对话 → 其余原消息表原样置尾。
///
/// - 参数 base：请求原始消息表
/// - 参数 system：解析后的 system 文本（chat 路径恒为 meta.system；
///   generate 路径为请求级与 meta.system 择一后的生效值）
/// - 参数 preset_messages：模型元数据 MESSAGE 预置对话
/// - 返回：组装后的完整消息表
fn apply_model_preset(
    base: Vec<crate::adapter::OllamaMessage>,
    system: &str,
    preset_messages: &[repo::ChatMessage],
) -> Vec<crate::adapter::OllamaMessage> {
    fn plain(role: &str, content: &str) -> crate::adapter::OllamaMessage {
        crate::adapter::OllamaMessage {
            role: role.into(),
            content: content.into(),
            images: vec![],
            tool_calls: None,
            tool_name: None,
            thinking: None,
        }
    }
    // 请求级 system（首条）优先保持首位；meta.system 仅在缺位时补位
    let (request_system, rest) = match base.split_first() {
        Some((first, rest)) if first.role == "system" => (Some(first.clone()), rest.to_vec()),
        _ => (None, base),
    };
    let mut out = Vec::new();
    if let Some(s) = request_system {
        out.push(s);
    } else if !system.is_empty() {
        out.push(plain("system", system));
    }
    for c in preset_messages {
        out.push(plain(&c.role, &c.content));
    }
    out.extend(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M34 BUG-6：上游错误单层化——解包内层 message + Pooling 场景转官方口径
    #[test]
    fn upstream_error_unwrapped_single_layer() {
        // OpenAI 风格嵌套：取内层 message；Pooling 场景转写中文口径文案
        //（M104 迭代32 碴5：原英文官方口径按用户裁决 2026-09-10 19:00 中文化）
        let nested = r#"{"error":{"code":400,"message":"Pooling type 'none' is not OAI compatible. Use pooling type 'mean'."}}"#;
        assert_eq!(
            format_upstream_error(nested),
            "此模型不支持向量化，请使用 embedding 模型"
        );
        // 普通内层 message：单层透出
        let nested2 = r#"{"error":{"code":400,"message":"bad request body"}}"#;
        assert_eq!(format_upstream_error(nested2), "bad request body");
        // 单层 {"error":"..."}：透出
        assert_eq!(format_upstream_error(r#"{"error":"plain"}"#), "plain");
        // 非 JSON 原文：兜底透传
        assert_eq!(format_upstream_error("oops"), "oops");
    }

    /// M34 BUG-1：终包合并缓冲——分包形态（finish_reason 包 + usage 包）合并为单包
    #[test]
    fn final_event_buffer_merges_split_final_packets() {
        let mut buf = FinalEventBuffer::new();
        // finish_reason 包先到：携带真实 done_reason（length 场景），无统计键
        assert!(buf
            .push(json!({"done": true, "done_reason": "length"}))
            .is_empty());
        // usage 包后到：统计键完整，done_reason 为 adapter 无 choice 兜底值
        // （stop）——不得覆盖先到的真实值
        assert!(buf
            .push(
                json!({"done": true, "done_reason": "stop", "eval_count": 2009,
                            "prompt_eval_count": 26})
            )
            .is_empty());
        let out = buf.flush().expect("flush 应产出合并后的单终包");
        assert!(out["done"].as_bool().unwrap());
        assert_eq!(
            out["done_reason"], "length",
            "先到真实 done_reason 不被兜底值覆盖"
        );
        assert_eq!(out["eval_count"], 2009, "后到统计键并入");
        assert_eq!(out["prompt_eval_count"], 26);
        assert!(buf.flush().is_none(), "flush 幂等（不重复产出）");
    }

    /// 迭代38 M135：/api/ps digest 补齐回退——顶层空 → model 层 sha256
    /// （HF 直引断链修复）；主源非空不触发回退；两处皆空保持空串
    #[test]
    fn ps_digest_fallback_to_model_layer() {
        let hf_meta = || repo::ModelMeta {
            name: "hf.co/u/r:Q4".into(),
            family: "qwen".into(),
            families: vec!["qwen".into()],
            parameter_size: "4B".into(),
            quantization_level: "Q4_K_M".into(),
            system: String::new(),
            template: None,
            parameters: Default::default(),
            messages: vec![],
            layer_digests: Default::default(),
            adapters: vec![],
            files: repo::ModelFiles {
                model: "model.gguf".into(),
                mmproj: None,
            },
            digest: String::new(),
            license: String::new(),
            source: "huggingface:u/r".into(),
            created_at: "2026-09-11T07:00:00Z".into(),
            runtime: None,
        };
        // 顶层空 + model 层有 sha256 → 回退取 model 层（HF 直引形态）
        let mut m = hf_meta();
        m.layer_digests
            .insert("model".into(), "sha256:0123456789abcdef".into());
        assert_eq!(
            resolve_ps_digest(&m),
            "sha256:0123456789abcdef",
            "顶层空必须回退 model 层 sha256"
        );
        // 主源形态：顶层非空 → 原值直出（回退不触发）
        let mut main_meta = hf_meta();
        main_meta.digest = "sha256:maindigest".into();
        assert_eq!(resolve_ps_digest(&main_meta), "sha256:maindigest");
        // 两处皆空 → 空串（CLI 兜底 "?"）
        assert_eq!(resolve_ps_digest(&hf_meta()), "");
    }

    /// M34 BUG-1：单包形态（finish_reason+usage 同包）与无 usage 兜底（M33 碴3 键缺省语义）
    #[test]
    fn final_event_buffer_single_packet_and_missing_usage() {
        // 单包含全统计：暂存后 flush 原样产出
        let mut buf = FinalEventBuffer::new();
        assert!(buf
            .push(json!({"done": true, "done_reason": "stop", "eval_count": 5}))
            .is_empty());
        let out = buf.flush().unwrap();
        assert_eq!(out["eval_count"], 5);

        // 无 usage（上游异常）：flush 产出缺统计键终包（M33 碴3 语义）
        let mut buf2 = FinalEventBuffer::new();
        assert!(buf2
            .push(json!({"done": true, "done_reason": "length"}))
            .is_empty());
        let out2 = buf2.flush().unwrap();
        assert!(out2.get("eval_count").is_none(), "usage 缺失不写统计键");
        assert!(out2.get("prompt_eval_count").is_none());
    }

    /// M34 BUG-1：终包后异常增量序列的保序防御（暂存终包先于增量输出）
    #[test]
    fn final_event_buffer_flushes_before_late_increment() {
        let mut buf = FinalEventBuffer::new();
        assert!(buf.push(json!({"done": true, "eval_count": 1})).is_empty());
        let seq = buf.push(json!({"done": false, "response": "tail"}));
        assert_eq!(seq.len(), 2, "保序：暂存终包 + 后到增量");
        assert!(seq[0]["done"].as_bool().unwrap());
        assert_eq!(seq[1]["response"], "tail");
        assert!(!seq[1]["done"].as_bool().unwrap());
    }

    /// M34 BUG-3：参数合并层——meta 默认层 + 请求逐键覆盖 + 空层透传
    #[test]
    fn merge_model_parameters_layering() {
        let meta_params: std::collections::BTreeMap<String, Value> =
            serde_json::from_value(json!({"temperature": 0.1, "num_predict": 16, "top_p": 0.9}))
                .unwrap();
        // 请求覆盖同键、未覆盖键沿用模型持久化值
        let out = merge_model_parameters(&meta_params, Some(json!({"temperature": 0.8}))).unwrap();
        assert_eq!(out["temperature"], 0.8, "请求键覆盖默认层");
        assert_eq!(out["num_predict"], 16, "未覆盖键沿用模型持久化值");
        assert_eq!(out["top_p"], 0.9);
        // 请求新增键并入
        let out2 = merge_model_parameters(&meta_params, Some(json!({"seed": 42}))).unwrap();
        assert_eq!(out2["seed"], 42);
        assert_eq!(out2["num_predict"], 16);
        // 空默认层：请求原样透传（无参数模型行为零变化）
        let req_only = json!({"temperature": 0.5});
        let empty: std::collections::BTreeMap<String, Value> = Default::default();
        assert_eq!(
            merge_model_parameters(&empty, Some(req_only.clone())).unwrap(),
            req_only
        );
        // 空层 + 无请求：None 透传
        assert!(merge_model_parameters(&empty, None).is_none());
    }
    use crate::adapter::OllamaMessage;
    use crate::repo::ChatMessage;

    fn msg(role: &str, content: &str) -> OllamaMessage {
        OllamaMessage {
            role: role.into(),
            content: content.into(),
            images: vec![],
            tool_calls: None,
            tool_name: None,
            thinking: None,
        }
    }

    fn preset_pair() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "assistant".into(),
            content: "ahoy".into(),
        }]
    }

    /// M26 碴2：注入顺序 system → 预置对话 → 原消息表
    #[test]
    fn model_preset_injection_order() {
        let out = apply_model_preset(vec![msg("user", "hi")], "be pirate", &preset_pair());
        assert_eq!(out.len(), 3);
        assert_eq!(
            (out[0].role.as_str(), out[0].content.as_str()),
            ("system", "be pirate")
        );
        assert_eq!(
            (out[1].role.as_str(), out[1].content.as_str()),
            ("assistant", "ahoy")
        );
        assert_eq!(
            (out[2].role.as_str(), out[2].content.as_str()),
            ("user", "hi")
        );
    }

    /// M26 碴2：请求级 system（消息表首条）优先，meta.system 不重复注入；
    /// 预置对话仍注入（MESSAGE 与 system 为独立维度）
    #[test]
    fn request_level_system_takes_priority() {
        let out = apply_model_preset(
            vec![msg("system", "req-sys"), msg("user", "hi")],
            "meta-sys",
            &preset_pair(),
        );
        assert_eq!(out.len(), 3, "不得出现第二个 system");
        assert_eq!(out[0].content, "req-sys");
        assert_eq!(out[1].content, "ahoy");
    }

    /// M26 碴2：空 system 与空预置必须原样透传（无 meta 模型行为零变化）
    #[test]
    fn empty_preset_passthrough() {
        let out = apply_model_preset(vec![msg("user", "hi")], "", &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "hi");
    }

    /// M28 碴7：回显名规范化（短名补 latest；带 tag 原样；非法名兜底原样）
    #[test]
    fn display_name_normalization() {
        assert_eq!(display_model_name("llama3.2"), "llama3.2:latest");
        assert_eq!(display_model_name("llama3.2:3b"), "llama3.2:3b");
        assert_eq!(
            display_model_name("a/b"),
            "a/b",
            "非法名（含斜杠）原样返回，不二次报错"
        );
    }

    /// M28 碴9：embedding 家族判定（embed 子串 + bert/bge 词根边界）
    #[test]
    fn embedding_family_detection() {
        assert!(is_embedding_family("nomic-embed-text"));
        assert!(is_embedding_family("nomic-bert"), "bert 词根漏判修复");
        assert!(is_embedding_family("jina-bert-v2"));
        assert!(is_embedding_family("bert"));
        assert!(is_embedding_family("bge-m3"));
        assert!(!is_embedding_family("llama"));
        assert!(!is_embedding_family("qwen2"));
        assert!(!is_embedding_family("gemma"));
    }

    /// M29 碴2：去重键规范化——短名/完整名同键；非法名与 hf.co/ 形态原样回退
    #[test]
    fn pull_key_normalization() {
        assert_eq!(normalize_pull_key("llama3.2"), "llama3.2:latest");
        assert_eq!(normalize_pull_key("llama3.2:3b"), "llama3.2:3b");
        assert_eq!(
            normalize_pull_key("hf.co/user/repo:Q4"),
            "hf.co/user/repo:Q4",
            "HF 形态 ModelRef 不可解析，回退原名（tag 语义在 hf.pull 内部）"
        );
        assert_eq!(normalize_pull_key("a/b"), "a/b", "非法名回退原名不二次报错");
    }

    /// M30 碴1：show 复原数组参数展开多行——parse(复原文本) 必须恢复同数组
    /// （原紧凑 JSON 单行 `PARAMETER stop ["a","b"]` 经 parse 变字符串字面量，
    /// show→create 往返丢多值语义）；标量维持单行往返
    #[test]
    fn show_parameter_roundtrip() {
        let mut params = std::collections::BTreeMap::new();
        params.insert("stop".to_string(), serde_json::json!(["<|a|>", "<|b|>"]));
        params.insert("temperature".to_string(), serde_json::json!(0.7));
        let lines = parameter_lines(&params);
        // 数组展开为逐元素多行（元素字符串带引号形态，parse 剥引号恢复）
        assert_eq!(
            lines,
            "\nPARAMETER stop \"<|a|>\"\n\nPARAMETER stop \"<|b|>\"\n\nPARAMETER temperature 0.7\n"
        );
        // 往返锚定：复原文本可被 parse 正确重建参数集合
        let mf = crate::modelfile::parse(&format!("FROM m\n{lines}")).unwrap();
        assert_eq!(mf.parameters["stop"], serde_json::json!(["<|a|>", "<|b|>"]));
        assert_eq!(mf.parameters["temperature"], serde_json::json!(0.7));
    }

    /// M29 碴11：通道满时进度事件丢弃并计数（终态事件不走此路径不受影响）
    #[tokio::test]
    async fn progress_event_drop_counted_when_channel_full() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<String>(1);
        tx.try_send("占位填满".to_string()).unwrap();
        let mut dropped = 0u32;
        send_progress_event(
            &tx,
            &crate::registry::PullEvent::status("pulling"),
            &mut dropped,
        );
        assert_eq!(dropped, 1, "通道满必须丢弃并计数");
        send_progress_event(
            &tx,
            &crate::registry::PullEvent::status("pulling"),
            &mut dropped,
        );
        assert_eq!(dropped, 2, "计数跨事件累加");
    }

    /// M30 碴9：embed 转发体必须剥离 model 路由字段（替身捕获上游请求体）
    #[tokio::test]
    async fn embed_strips_model_from_upstream_body() {
        use crate::api::AppState;
        use crate::scheduler::RunnerRegistry;
        use std::process::Stdio;

        let root = std::env::temp_dir().join(format!("roxid-embed-strip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = ModelRef::parse("embedm2:1").unwrap();
        let dir = r.dir(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &dir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "embedm2:1".into(),
                family: "bert".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-06T00:00:00Z".into(),
            },
        )
        .unwrap();

        // 替身：POST body 落盘供断言（捕获 roxid 实际转发给上游的请求体）
        let body_file = root.join("upstream-body.json");
        let port = 38309u16;
        let stub = r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def do_POST(self):
        n = int(self.headers.get('Content-Length', 0))
        with open(sys.argv[2], 'wb') as f:
            f.write(self.rfile.read(n))
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
"#;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c")
            .arg(stub)
            .arg(port.to_string())
            .arg(body_file.to_str().unwrap().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let runner = crate::scheduler::Runner::spawn_with(
            "embedm2:1",
            port,
            std::time::Duration::from_secs(300),
            &mut cmd,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let scheduler = Arc::new(RunnerRegistry::new(root.clone()));
        scheduler.inject(runner).await;

        let state = Arc::new(AppState {
            scheduler,
            models_root: root.clone(),
            registry: Arc::new(crate::registry::OllamaRegistry::new()),
            pulls: tokio::sync::Mutex::new(Default::default()),
        });
        let resp = embed(
            axum::extract::State(state),
            LenientJson(json!({"model": "embedm2:1", "input": "文本"})),
        )
        .await;
        let _ = axum::body::to_bytes(resp.into_body(), usize::MAX).await;
        let body: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&body_file).unwrap()).unwrap();
        assert!(
            body.get("model").is_none(),
            "M30 碴9：转发体必须剥离 model 路由字段：{body}"
        );
        assert_eq!(body["input"], "文本", "业务字段原样保留");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M30 碴12：chat 非流式上游 200 非 JSON 体必须显式 502（原容错 Null
    /// 生成空响应 200 吞掉故障）
    #[tokio::test]
    async fn chat_non_stream_non_json_errors() {
        use crate::api::AppState;
        use crate::scheduler::RunnerRegistry;
        use std::process::Stdio;

        let root = std::env::temp_dir().join(format!("roxid-chat-nonjson-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = ModelRef::parse("chatm:1").unwrap();
        let dir = r.dir(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &dir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "chatm:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-06T00:00:00Z".into(),
            },
        )
        .unwrap();

        // 替身：200 但回非 JSON 纯文本（触发 json 解析失败分支）
        let port = 38311u16;
        let stub = r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def do_POST(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'not-json')
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
"#;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c")
            .arg(stub)
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let runner = crate::scheduler::Runner::spawn_with(
            "chatm:1",
            port,
            std::time::Duration::from_secs(300),
            &mut cmd,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let scheduler = Arc::new(RunnerRegistry::new(root.clone()));
        scheduler.inject(runner).await;

        let state = Arc::new(AppState {
            scheduler,
            models_root: root.clone(),
            registry: Arc::new(crate::registry::OllamaRegistry::new()),
            pulls: tokio::sync::Mutex::new(Default::default()),
        });
        let resp = chat(
            axum::extract::State(state),
            LenientJson(crate::adapter::OllamaChatRequest {
                model: "chatm:1".into(),
                messages: vec![OllamaMessage {
                    role: "user".into(),
                    content: "hi".into(),
                    images: vec![],
                    tool_calls: None,
                    tool_name: None,
                    thinking: None,
                }],
                format: None,
                options: None,
                tools: None,
                stream: Some(false),
                keep_alive: None,
                think: None,
                logprobs: None,
                top_logprobs: None,
            }),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_GATEWAY,
            "M30 碴12：非 JSON 上游必须 502"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M29 碴4：embed 响应含计量字段（替身实例 + HTTP 200 非_json 体走
    /// 容错路径——total/load 必为实测正值，prompt_eval_count 键存在）
    #[tokio::test]
    async fn embed_includes_duration_fields() {
        use crate::api::AppState;
        use crate::scheduler::RunnerRegistry;
        use std::process::Stdio;

        let root = std::env::temp_dir().join(format!("roxid-embed-dur-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = ModelRef::parse("embedm:1").unwrap();
        let dir = r.dir(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &dir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "embedm:1".into(),
                family: "bert".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-05T00:00:00Z".into(),
            },
        )
        .unwrap();

        // HTTP 200 替身（POST 返回非 json 体 → 上游 json 解析容错 Null 路径）
        let port = 38307u16;
        let stub = r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def do_POST(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
"#;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c")
            .arg(stub)
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let runner = crate::scheduler::Runner::spawn_with(
            "embedm:1",
            port,
            std::time::Duration::from_secs(300),
            &mut cmd,
        )
        .await
        .unwrap();
        // 等替身就绪（沿 downloader 测试先例：spawn 后固定等待）
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let scheduler = Arc::new(RunnerRegistry::new(root.clone()));
        scheduler.inject(runner).await;

        let state = Arc::new(AppState {
            scheduler,
            models_root: root.clone(),
            registry: Arc::new(crate::registry::OllamaRegistry::new()),
            pulls: tokio::sync::Mutex::new(Default::default()),
        });
        let resp = embed(
            axum::extract::State(state),
            LenientJson(json!({"model": "embedm:1", "input": "文本"})),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["model"], "embedm:1", "回显规范化完整名");
        assert!(
            v["total_duration"].as_u64().unwrap_or(0) > 0,
            "M29 碴4：total_duration 必须为实测正值：{v}"
        );
        assert!(
            v["load_duration"].as_u64().unwrap_or(0) > 0,
            "load_duration 必须为实测正值（acquire 计时）"
        );
        assert!(
            v.get("prompt_eval_count").is_some(),
            "prompt_eval_count 键必须存在（上游 usage 缺失时为 0）"
        );
        assert_eq!(v["prompt_eval_count"], 0, "非 json 上游体容错为 0");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M28 碴15：blobs 流读错误必须以 Err 项终结流（原缺陷：静默当 EOF 截断）
    #[tokio::test]
    async fn blobs_body_errors_on_read_failure() {
        // Linux：目录作为文件打开成功但 read 返回 EISDIR 错误——天然错误源
        let dir = tokio::fs::File::open(std::env::temp_dir()).await.unwrap();
        let body = blobs_body(dir);
        let mut stream = body.into_data_stream();
        let mut saw_error = false;
        while let Some(item) = stream.next().await {
            if item.is_err() {
                saw_error = true;
            }
        }
        assert!(saw_error, "读错误必须以 Err 终结流而非静默 EOF");
    }

    /// M28 碴14：create 覆盖成功后同名运行实例必须被停止（对齐 delete 先例）
    #[tokio::test]
    async fn create_stops_running_instance() {
        use crate::api::AppState;
        use crate::scheduler::RunnerRegistry;
        use std::process::Stdio;

        let root = std::env::temp_dir().join(format!("roxid-create-stop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // seed 基础模型 basem:1
        let base = ModelRef::parse("basem:1").unwrap();
        let bdir = base.dir(&root);
        std::fs::create_dir_all(&bdir).unwrap();
        std::fs::write(bdir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &bdir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "basem:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-08-29T00:00:00Z".into(),
            },
        )
        .unwrap();

        // 注入同名运行实例替身（newm:v1）
        let scheduler = Arc::new(RunnerRegistry::new(root.clone()));
        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
        let runner = crate::scheduler::Runner::spawn_with(
            "newm:v1",
            0,
            std::time::Duration::from_secs(300),
            &mut cmd,
        )
        .await
        .unwrap();
        scheduler.inject(runner).await;

        let state = Arc::new(AppState {
            scheduler: scheduler.clone(),
            models_root: root.clone(),
            registry: Arc::new(crate::registry::OllamaRegistry::new()),
            pulls: tokio::sync::Mutex::new(Default::default()),
        });
        let resp = create(
            axum::extract::State(state),
            LenientJson(json!({
                "model": "newm:v1",
                "from": "FROM basem:1"
            })),
        )
        .await;
        // 读尽 NDJSON 流（stop 先于 success 事件执行，收到 success 即可断言）
        let mut stream = resp.into_body().into_data_stream();
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            if let Ok(b) = item {
                text.push_str(&String::from_utf8_lossy(&b));
            }
        }
        assert!(text.contains("success"), "create 必须成功：{text}");
        assert!(
            scheduler.list_running().await.is_empty(),
            "M28 碴14：create 覆盖后同名运行实例必须被停止"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M35 D2/D3/D13：show 官方形态——capabilities 字符串数组（无 rerank
    /// 键）、parameters 多行文本、modified_at 恒有、model_info 官方键名与
    /// 数字类型（GGUF 缺失/非法时 meta 兜底链：context_length←参数层、
    /// parameter_count←规模标签、file_type←量化名反查）
    #[tokio::test]
    async fn show_official_shape_capabilities_parameters_model_info() {
        use crate::api::AppState;
        use crate::scheduler::RunnerRegistry;
        use std::process::Stdio;

        let root = std::env::temp_dir().join(format!("roxid-show-m35-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = ModelRef::parse("showm:1").unwrap();
        let dir = r.dir(&root);
        std::fs::create_dir_all(&dir).unwrap();
        // 非法 GGUF 内容——强制走 meta 兜底链（真实 GGUF 解析路径由 gguf.rs
        // 单测与 e2e 覆盖）
        std::fs::write(dir.join("model.gguf"), b"not-gguf").unwrap();
        let mut parameters = std::collections::BTreeMap::new();
        parameters.insert("num_ctx".to_string(), json!(2048));
        parameters.insert("temperature".to_string(), json!(0.5));
        crate::repo::model_json::save_meta(
            &dir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "showm:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "0.6B".into(),
                quantization_level: "Q4_K_M".into(),
                system: String::new(),
                template: None,
                parameters,
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-07T00:00:00Z".into(),
            },
        )
        .unwrap();

        let scheduler = Arc::new(RunnerRegistry::new(root.clone()));
        let state = Arc::new(AppState {
            scheduler,
            models_root: root.clone(),
            registry: Arc::new(crate::registry::OllamaRegistry::new()),
            pulls: tokio::sync::Mutex::new(Default::default()),
        });
        let resp = show(
            axum::extract::State(state),
            LenientJson(json!({"model": "showm:1", "verbose": true})),
        )
        .await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let s: Value = serde_json::from_slice(&bytes).unwrap();
        // D2：capabilities 字符串数组（无 rerank 键；含 completion）
        let caps = s["capabilities"]
            .as_array()
            .expect("capabilities 必须为数组");
        assert!(caps.iter().any(|c| c == "completion"));
        assert!(!caps.iter().any(|c| c == "rerank"), "官方无 rerank 能力键");
        assert!(
            s.get("capabilities").unwrap().as_object().is_none(),
            "不得再为布尔对象"
        );
        // D3：parameters 为多行文本（PARAMETER 行；含两个参数两行）
        let params = s["parameters"].as_str().expect("parameters 必须为文本");
        assert!(params.contains("PARAMETER num_ctx 2048"), "{params}");
        assert!(params.contains("PARAMETER temperature 0.5"), "{params}");
        // D13：modified_at 恒有；model_info 官方键名与数字类型（兜底链）
        assert_eq!(s["modified_at"], "2026-09-07T00:00:00Z");
        assert_eq!(s["model_info"]["general.architecture"], "llama");
        assert_eq!(
            s["model_info"]["llama.context_length"], 2048,
            "GGUF 缺失兜底参数层 num_ctx"
        );
        assert_eq!(
            s["model_info"]["general.parameter_count"], 600_000_000,
            "GGUF 缺失兜底规模标签估算"
        );
        assert_eq!(
            s["model_info"]["general.file_type"], 15,
            "GGUF 缺失兜底量化名反查（Q4_K_M=15）"
        );
        // 旧自定键清理：general.parameter_size（字符串形态）不得再出现
        assert!(s["model_info"].get("general.parameter_size").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M35 D1/D6：原生通道请求构造与响应转换（token 数组 prompt、
    /// max_tokens→n_predict 重映射、infill 前后缀、stop_type/done_reason、
    /// timings 统计）
    #[test]
    fn native_channel_request_and_response_conversion() {
        let req = OllamaGenerateRequest {
            model: "m".into(),
            prompt: "hello".into(),
            images: None,
            format: Some(json!("json")),
            options: Some(json!({"num_predict": 32, "temperature": 0.7, "mirostat": 2})),
            system: None,
            template: None,
            raw: None,
            stream: None,
            keep_alive: None,
            think: None,
            context: Some(vec![1, 2, 3]),
            suffix: None,
            logprobs: None,
            top_logprobs: None,
        };
        // /completion：token 数组 prompt + n_predict 重映射 + 采样透传 + json_schema
        let body = build_native_completion_request(&req, vec![1, 2, 3, 100, 101], true);
        assert_eq!(
            body["prompt"],
            json!([1, 2, 3, 100, 101]),
            "token 数组 prompt"
        );
        assert_eq!(body["n_predict"], 32, "num_predict → n_predict 重映射");
        assert!(body.get("max_tokens").is_none(), "max_tokens 不得残留");
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["mirostat"], 2);
        assert_eq!(body["json_schema"], "{}");

        // /infill：input_prefix/input_suffix
        let mut infill_req = req.clone();
        infill_req.suffix = Some("</code>".into());
        let body = build_native_infill_request(&infill_req, false);
        assert_eq!(body["input_prefix"], "hello");
        assert_eq!(body["input_suffix"], "</code>");
        assert_eq!(body["stream"], false);
        assert_eq!(body["n_predict"], 32);

        // 非流式响应转换：content/stop_type/timings
        let native = json!({
            "content": "世界",
            "stop_type": "limit",
            "timings": {"prompt_n": 5, "predicted_n": 9},
        });
        let out = native_completion_to_ollama("m:1", &native);
        assert_eq!(out["response"], "世界");
        assert_eq!(out["done"], true);
        assert_eq!(out["done_reason"], "length", "limit → length");
        assert_eq!(out["eval_count"], 9);
        assert_eq!(out["prompt_eval_count"], 5);
        // word/eos → stop
        assert_eq!(native_stop_to_done_reason(&json!("eos")), json!("stop"));
        assert_eq!(native_stop_to_done_reason(&json!(null)), json!("stop"));
        // timings 缺失时统计键缺省（M33 碴3 语义）
        let bare = native_completion_to_ollama("m:1", &json!({"content": "x"}));
        assert!(bare.get("eval_count").is_none());
    }

    /// M35 D13：规模标签 → 参数数量级估算（parameter_count 兜底）
    #[test]
    fn size_label_count_parsing() {
        assert_eq!(parse_size_label_count("0.6B"), Some(600_000_000));
        assert_eq!(parse_size_label_count("8.0B"), Some(8_000_000_000));
        assert_eq!(parse_size_label_count("135M"), Some(135_000_000));
        assert_eq!(parse_size_label_count("700K"), Some(700_000));
        assert_eq!(parse_size_label_count(""), None);
        assert_eq!(parse_size_label_count("unknown"), None);
    }

    /// M35 D4：create 来源三形态解析（modelfile 文本 / from 文本兼容 /
    /// from 基础模型名结构化构造）
    #[test]
    fn create_source_resolution_three_forms() {
        // 1) modelfile 文本字段（官方弃用形态）
        let req = json!({"model": "m", "modelfile": "FROM base:1\nSYSTEM \"\"\"x\"\"\""});
        let mf = resolve_modelfile(&req).unwrap();
        assert_eq!(mf.from, "base:1");
        assert_eq!(mf.system.as_deref(), Some("x"));
        // 2) from 承载 Modelfile 文本（roxid 既有形态兼容）
        let req = json!({"model": "m", "from": "FROM base:2"});
        let mf = resolve_modelfile(&req).unwrap();
        assert_eq!(mf.from, "base:2");
        // 3) from 基础模型名 + 结构化字段（官方 0.5+ 形态）
        let req = json!({
            "model": "mario",
            "from": "gemma3:4b",
            "system": "You are Mario",
            "parameters": {"temperature": 0.5, "stop": ["<|end|>"]},
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "yo"}
            ],
            "template": "{{ .Prompt }}"
        });
        let mf = resolve_modelfile(&req).unwrap();
        assert_eq!(mf.from, "gemma3:4b");
        assert_eq!(mf.system.as_deref(), Some("You are Mario"));
        assert_eq!(mf.template.as_deref(), Some("{{ .Prompt }}"));
        assert_eq!(mf.parameters.get("temperature"), Some(&json!(0.5)));
        assert_eq!(mf.parameters.get("stop"), Some(&json!(["<|end|>"])));
        assert_eq!(mf.messages.len(), 2);
        assert_eq!(mf.messages[0].role, "user");
        assert_eq!(mf.messages[1].content, "yo");
        // 无有效来源报错
        assert!(resolve_modelfile(&json!({"model": "m"})).is_err());
    }
}
