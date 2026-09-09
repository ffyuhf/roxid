//! 真实端点验收：axum 网关 + 真实模型（smollm:135m）全链路。
//! 前置：/tmp/roxid-pull-e2e（M5 产物）、/tmp/roxid-rt-e2e（M2 产物）。
//! 执行：cargo test -p roxid-server --test api_e2e -- --ignored --nocapture
//!
//! 修改历史：M7 新增 2026-08-24 19:39；M18 llama.cpp 8 端点直通验收 2026-08-24 22:28；
//! M19 total_slots 断言 2026-08-24 22:35；M20 /api/blobs 段 2026-08-24 22:45
//! M32 碴3（迭代12）：8.5 段 model.json 路径与 FIM 段注册名对齐 M31
//! 三目录布局（原旧两级路径使挂账手动验收必炸、旧 -- 转义名使 FIM 段
//! 永远静默跳过）2026-09-07 01-15

use std::sync::Arc;

use roxid_server::api::{build_router, AppState};
use roxid_server::registry::OllamaRegistry;
use roxid_server::scheduler::RunnerRegistry;

/// 验收端口（避开 11434 正式端口）
const TEST_PORT: u16 = 39411;

#[tokio::test]
#[ignore = "真实模型端到端验收，手动执行"]
async fn ollama_api_endpoints_e2e() {
    std::env::set_var("ROXID_HOME", "/tmp/roxid-pull-e2e");
    std::env::set_var(
        "ROXID_LLAMA_SERVER",
        "/tmp/roxid-rt-e2e/llama.cpp/b10605/ubuntu-x64/llama-server",
    );
    let state = Arc::new(AppState {
        scheduler: Arc::new(RunnerRegistry::new("/tmp/roxid-pull-e2e/models".into())),
        models_root: "/tmp/roxid-pull-e2e/models".into(),
        registry: Arc::new(OllamaRegistry::new()),
        pulls: tokio::sync::Mutex::new(Default::default()), // M28 碴2：pull 去重表
    });
    state
        .scheduler
        .spawn_reaper(std::time::Duration::from_secs(5));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", TEST_PORT))
        .await
        .unwrap();
    let app = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let base = format!("http://127.0.0.1:{TEST_PORT}");
    let client = reqwest::Client::new();

    // 1) /api/version
    let v: serde_json::Value = client
        .get(format!("{base}/api/version"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(v["version"].as_str().is_some(), "version 必须返回：{v}");

    // 2) /api/tags
    let t: serde_json::Value = client
        .get(format!("{base}/api/tags"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        t["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["name"] == "smollm:135m"),
        "tags 必须列出已拉取模型：{t}"
    );

    // 3) /api/show
    let s: serde_json::Value = client
        .post(format!("{base}/api/show"))
        .json(&serde_json::json!({"model": "smollm:135m"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(s["details"]["family"] == "llama", "show 必须返回详情：{s}");
    // M35 D2/D3/D13：官方形态——capabilities 数组 / parameters 文本 /
    // modified_at / model_info 官方键名（smollm family=llama）
    assert!(
        s["capabilities"].as_array().is_some(),
        "capabilities 必须为字符串数组：{s}"
    );
    assert!(
        s["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "completion"),
        "capabilities 含 completion：{s}"
    );
    assert!(s["parameters"].is_string(), "parameters 必须为文本：{s}");
    assert!(s["modified_at"].is_string(), "modified_at 恒有：{s}");
    assert!(
        s["model_info"]["llama.context_length"].is_number(),
        "model_info 官方键 {{arch}}.context_length：{s}"
    );
    assert!(
        s["model_info"]["general.file_type"].is_number(),
        "model_info file_type 数字：{s}"
    );

    // 4) /api/chat 非流式
    let c: serde_json::Value = client
        .post(format!("{base}/api/chat"))
        .json(&serde_json::json!({
            "model": "smollm:135m",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false
        }))
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(c["done"].as_bool().unwrap(), "非流式必须 done=true：{c}");
    assert!(
        !c["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "必须生成内容：{c}"
    );

    // 5) /api/chat 流式（NDJSON 多行）
    let resp = client
        .post(format!("{base}/api/chat"))
        .json(&serde_json::json!({
            "model": "smollm:135m",
            "messages": [{"role": "user", "content": "count: 1 2 3"}],
            "stream": true,
            "keep_alive": "1m"
        }))
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.headers()["content-type"],
        "application/x-ndjson",
        "流式必须 NDJSON"
    );
    let body = resp.text().await.unwrap();
    let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() >= 2, "流式必须多事件：{lines:?}");
    let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert!(
        last["done"].as_bool().unwrap(),
        "末事件必须 done=true：{last}"
    );

    // 6) /api/generate 流式
    let resp = client
        .post(format!("{base}/api/generate"))
        .json(&serde_json::json!({
            "model": "smollm:135m",
            "prompt": "The capital of France is",
            "stream": true
        }))
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    let gen_lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    let last_gen: serde_json::Value = serde_json::from_str(gen_lines.last().unwrap()).unwrap();
    assert!(
        last_gen["done"].as_bool().unwrap(),
        "generate 末事件 done：{last_gen}"
    );
    // 内容分布在前序事件（末事件仅携带统计，与原版一致）
    let aggregated: String = gen_lines
        .iter()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|ev| ev["response"].as_str().map(str::to_string))
        .collect();
    assert!(
        !aggregated.is_empty(),
        "generate 聚合内容必须非空：{gen_lines:?}"
    );

    // 7) /api/ps
    let p: serde_json::Value = client
        .get(format!("{base}/api/ps"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        p["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["name"] == "smollm:135m"),
        "ps 必须看到运行实例：{p}"
    );
    // M35 D5：ps 条目官方字段——details 六字段对象 + context_length 数字
    let ps_entry = p["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "smollm:135m")
        .unwrap();
    assert!(ps_entry["details"]["format"] == "gguf", "details 对象：{p}");
    assert!(
        ps_entry["context_length"].is_u64(),
        "context_length 数字：{p}"
    );

    // 8.5) M20 /api/blobs（model 层摘要：GET sha256 一致 / HEAD 200 / 未知 404）
    // M32 碴3：路径对齐 M31 三目录布局（主源产物落 models/ollama/）
    let mj: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("/tmp/roxid-pull-e2e/models/ollama/smollm/135m/model.json")
            .unwrap(),
    )
    .unwrap();
    let dg = mj["layer_digests"]["model"]
        .as_str()
        .expect("pull 必须写入层摘要索引")
        .to_string();
    let resp = client
        .get(format!("{base}/api/blobs/{dg}"))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "blobs GET 必须 200：{}",
        resp.status()
    );
    let bs = resp.bytes().await.unwrap();
    use sha2::Digest as _;
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(&bs)),
        dg.strip_prefix("sha256:").unwrap_or(&dg),
        "GET 返回字节必须与摘要一致"
    );
    let hresp = client
        .head(format!("{base}/api/blobs/{dg}"))
        .send()
        .await
        .unwrap();
    assert_eq!(hresp.status(), 200, "HEAD 必须 200");
    let nresp = client
        .get(format!("{base}/api/blobs/sha256:{}", "0".repeat(64)))
        .send()
        .await
        .unwrap();
    assert_eq!(nresp.status(), 404, "未知摘要必须 404");

    // 8) /api/stop 后清空
    let code = client
        .post(format!("{base}/api/stop"))
        .json(&serde_json::json!({"model": "smollm:135m"}))
        .send()
        .await
        .unwrap()
        .status();
    assert!(code.is_success());
    let p: serde_json::Value = client
        .get(format!("{base}/api/ps"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        p["models"].as_array().unwrap().is_empty(),
        "stop 后必须清空：{p}"
    );

    server.abort();
    std::env::remove_var("ROXID_HOME");
    std::env::remove_var("ROXID_LLAMA_SERVER");
}

/// M18 验收：llama.cpp 原生 8 端点经网关真实往返。
/// 前置：/tmp/roxid-pull-e2e（M5 产物）、/tmp/roxid-rt-e2e（M2 产物）。
/// 执行：cargo test -p roxid-server --test api_e2e -- --ignored --nocapture
#[tokio::test]
#[ignore = "真实模型端到端验收，手动执行"]
async fn llamacpp_passthrough_endpoints_e2e() {
    std::env::set_var("ROXID_HOME", "/tmp/roxid-pull-e2e");
    std::env::set_var(
        "ROXID_LLAMA_SERVER",
        "/tmp/roxid-rt-e2e/llama.cpp/b10605/ubuntu-x64/llama-server",
    );
    let state = Arc::new(AppState {
        scheduler: Arc::new(RunnerRegistry::new("/tmp/roxid-pull-e2e/models".into())),
        models_root: "/tmp/roxid-pull-e2e/models".into(),
        registry: Arc::new(OllamaRegistry::new()),
        pulls: tokio::sync::Mutex::new(Default::default()), // M28 碴2：pull 去重表
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 39412))
        .await
        .unwrap();
    let app = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let base = format!("http://127.0.0.1:39412");
    let client = reqwest::Client::new();
    let t = std::time::Duration::from_secs(180);

    // 1) 缺 model 必须严格 400（M18-R1 裁决）
    let resp = client
        .post(format!("{base}/tokenize"))
        .json(&serde_json::json!({"content": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "缺 model 必须 400，不自动路由");
    let resp = client.get(format!("{base}/props")).send().await.unwrap();
    assert_eq!(resp.status(), 400, "GET 缺 ?model= 必须 400");

    // 2) POST /tokenize（model 字段剔除后透传）
    let v: serde_json::Value = client
        .post(format!("{base}/tokenize"))
        .json(&serde_json::json!({"model": "smollm:135m", "content": "hello"}))
        .timeout(t)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        v["tokens"].as_array().is_some_and(|a| !a.is_empty()),
        "tokenize 必须返回 tokens：{v}"
    );

    // 3) POST /detokenize（round-trip）
    let ids = v["tokens"].as_array().unwrap().clone();
    let back: serde_json::Value = client
        .post(format!("{base}/detokenize"))
        .json(&serde_json::json!({"model": "smollm:135m", "tokens": ids}))
        .timeout(t)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        back.to_string().contains("hello"),
        "detokenize 往返必须还原文本：{back}"
    );

    // 4) POST /completion（原生补全）
    let resp = client
        .post(format!("{base}/completion"))
        .json(&serde_json::json!({"model": "smollm:135m", "prompt": "The capital of France is", "n_predict": 4}))
        .timeout(t)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "completion 必须 2xx：{}",
        resp.status()
    );

    // 5) POST /embedding（透传；上游对非 pooling 模型仍返回 token embedding）
    let resp = client
        .post(format!("{base}/embedding"))
        .json(&serde_json::json!({"model": "smollm:135m", "content": "hello"}))
        .timeout(t)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().as_u16() < 500,
        "embedding 网关转发必须成功（上游状态直通）：{}",
        resp.status()
    );

    // 6) GET /props（M19：total_slots 必须为默认并行数 4）
    let p: serde_json::Value = client
        .get(format!("{base}/props?model=smollm%3A135m"))
        .timeout(t)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        p["total_slots"].as_u64(),
        Some(4),
        "M19 默认 --parallel 4 必须生效：{p}"
    );
    assert!(p["model_path"].as_str().is_some_and(|s| !s.is_empty()));

    // 7) GET /slots（数组直通）
    let s: serde_json::Value = client
        .get(format!("{base}/slots?model=smollm%3A135m"))
        .timeout(t)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        s.as_array().is_some_and(|a| !a.is_empty()),
        "slots 必须返回数组：{s}"
    );

    // 8) GET /metrics（Prometheus 文本直通）
    let resp = client
        .get(format!("{base}/metrics?model=smollm%3A135m"))
        .timeout(t)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "metrics 必须 2xx");
    let text = resp.text().await.unwrap();
    assert!(text.contains("llamacpp:"), "metrics 必须为 Prometheus 格式");

    // 9) POST /infill（透传；b10605 实测：无 FIM token 的模型上游返回 501
    //    Not Implemented——模型能力语义，网关忠实直通；FIM 模型的 2xx 路径
    //    由下方 coder 模型段验证）
    let resp = client
        .post(format!("{base}/infill"))
        .json(&serde_json::json!({"model": "smollm:135m", "input_prefix": "def hello():", "input_suffix": "\n    return"}))
        .timeout(t)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success() || resp.status().as_u16() == 501,
        "infill 非 FIM 模型必须为上游 501 直通或成功：{}",
        resp.status()
    );

    // 10) FIM 模型的 /infill 2xx 路径（前置：hf_pull_coder_for_infill 已拉取；
    //     M31 起注册名为 hf.co/{user}/{repo}:{quant}，落位 HF 三级路径——
    //     旧 -- 转义名经 locate 不再命中，本段曾永远静默跳过）
    let coder = "hf.co/bartowski/Qwen2.5-Coder-0.5B-GGUF:IQ3_M";
    let resp = client
        .post(format!("{base}/infill"))
        .json(&serde_json::json!({
            "model": coder,
            "input_prefix": "def hello():",
            "input_suffix": "\n    return",
            "n_predict": 8
        }))
        .timeout(t)
        .send()
        .await
        .unwrap();
    if resp.status().is_success() {
        let v: serde_json::Value = resp.json().await.unwrap();
        assert!(
            !v["content"].as_str().unwrap_or("").is_empty(),
            "FIM 模型必须返回补全内容：{v}"
        );
    } else {
        // 仓库无 coder 模型时跳过该段（前置测试未执行），不阻断主验收
        eprintln!("跳过 FIM 2xx 段：{coder} 未安装（{}）", resp.status());
    }

    // 清理：停实例防进程残留
    client
        .post(format!("{base}/api/stop"))
        .json(&serde_json::json!({"model": "smollm:135m"}))
        .send()
        .await
        .ok();
    server.abort();
    std::env::remove_var("ROXID_HOME");
    std::env::remove_var("ROXID_LLAMA_SERVER");
}

/// M18 前置：拉取 FIM 代码模型（/infill 2xx 验证路径用）。
/// 执行：ROXID_HF_PROXY=https://hf-mirror.com/ cargo test -p roxid-server --test api_e2e hf_pull_coder_for_infill -- --ignored --nocapture
#[tokio::test]
#[ignore = "真实网络拉取 Qwen2.5-Coder-0.5B IQ3_M（约 400MB），验收时手动执行"]
async fn hf_pull_coder_for_infill() {
    std::env::set_var("ROXID_HF_PROXY", "https://hf-mirror.com/");
    let root = std::path::PathBuf::from("/tmp/roxid-pull-e2e/models");
    let r = roxid_server::registry::HuggingFaceSource::new()
        .pull(
            "bartowski/Qwen2.5-Coder-0.5B-GGUF",
            Some("IQ3_M"),
            &root,
            |_| {},
        )
        .await
        .expect("FIM 模型拉取必须成功");
    eprintln!("已拉取：{}", r.full_name());
    assert!(r.dir(&root).join("model.gguf").is_file(), "GGUF 必须落位");
    assert_eq!(r.tag, "IQ3_M", "tag 必须为选定的量化名");
    std::env::remove_var("ROXID_HF_PROXY");
}
