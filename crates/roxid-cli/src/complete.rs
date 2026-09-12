//! 补全候选统一源 `__complete`（迭代20 M57）：隐藏子命令，接收 shell shim
//! 转交的光标上下文（到当前词为止的全部 token），stdout 每行一个候选。
//!
//! 三系 shim（bash/zsh/fish，见 completion.rs）均为薄脚本，仅负责把
//! `COMP_WORDS`/`words`/`commandline -cop` 转交本命令；候选逻辑集中于此
//! 单一事实源。前缀分析手写而非走 clap 解析：补全输入天然为半成品
//! （未知选项/缺失必填值），clap 报错路径不可靠。
//!
//! 动态值候选直读 ~/.roxid 本地数据（零网络零延迟，裁决 2026-09-09 21:30）：
//! - model 位 → `repo::list_models`（与 cmd_list 同源的 tags 枚举能力）
//! - runtime use/rm 位 → `runtime::list_installed` + manual 可用性
//! - runtime use 变体位（迭代50 M191）→ `runtime::VARIANT_KEYWORDS` +
//!   `variant_dirs_of(tag)`（短词三词 + 该 tag 已装变体目录名，磁盘事实）
//!
//! 联网例外（各自裁决留痕）：
//! - stop 值位 → GET /api/ps（1s 超时，迭代36 M125，Q1/Q7 裁决
//!   2026-09-11 05:43/06:02）
//! - runtime install 值位 → GET GitHub Releases（2s 超时单次不重试，
//!   迭代48 M183，Q3-B 裁决 2026-09-12 04:24 裁决区间「1-2 秒」内取 2s
//!   ——实测 GitHub API 响应常超 1s，1s 掐断致候选恒空（2026-09-12
//!   04:42 实测 1008ms 被掐断 0 候选，update 无超时链路同刻查询成功）；
//!   限流 60 次/时风险用户裁决自担；候选数量经 config
//!   [runtime].tag_complete_limit，默认 10）
//!
//! 候选为空时静默输出零行（shell 自然不补，不误报）。
//!
//! 修改历史：M57 新增 2026-09-09 21-45

/// 顶层子命令位候选（含 ls 别名）
const TOP_COMMANDS: &[&str] = &[
    "serve",
    "create",
    "show",
    "run",
    "stop",
    "pull",
    "push",
    "signin",
    "signout",
    "list",
    "ls",
    "ps",
    "cp",
    "rm",
    "launch",
    "setup",
    "runtime",
    "completion",
];

/// runtime 二级子命令位候选（迭代48 M182 增 update）
const RUNTIME_SUBS: &[&str] = &["list", "install", "update", "use", "rm"];

/// completion 二级子命令位候选
const COMPLETION_SUBS: &[&str] = &["bash", "zsh", "fish", "install"];

/// ls 别名 → list 归一（选项表与值位映射按正式名匹配）
fn canonical_command(token: &str) -> &str {
    match token {
        "ls" => "list",
        t => t,
    }
}

/// 命令 → 选项位候选（长名优先，短名单列；与 Commands 枚举声明对齐）
fn options_for(cmd_path: &str) -> Vec<&'static str> {
    match cmd_path {
        "roxid" => vec!["--nowordwrap", "--verbose", "--version", "--help"],
        "roxid serve" => vec!["--addr", "--help"],
        "roxid create" => vec!["-f", "--file", "-i", "--interactive", "--help"],
        "roxid run" => vec!["--hf", "--runtime", "--help"],
        "roxid pull" => vec!["--hf", "--help"],
        "roxid setup" => vec!["--llama-url", "--help"],
        "roxid runtime install" => vec!["--url", "--help"],
        _ => vec!["--help"],
    }
}

/// 从光标前 token 分析命令上下文。
/// 跳过 `-` 开头的选项 token（带值选项的值会被误计为位置参数——仅影响
/// install/use 的位置计数，二者均无动态值候选，误计零影响）。
///
/// - 参数 prior：当前词之前的全部 token（不含正在输入的词）
/// - 返回：(命令路径, 已输入位置参数个数)
fn analyze(prior: &[String]) -> (String, usize) {
    let mut path = String::from("roxid");
    let mut positional = 0usize;
    for tok in prior {
        if tok.starts_with('-') {
            continue;
        }
        match path.as_str() {
            "roxid" => {
                let name = canonical_command(tok);
                if TOP_COMMANDS.contains(&name) {
                    path = format!("roxid {name}");
                    positional = 0;
                }
                // 非已知子命令（全局选项的值等）忽略
            }
            "roxid runtime" => {
                if RUNTIME_SUBS.contains(&tok.as_str()) {
                    path = format!("roxid runtime {tok}");
                    positional = 0;
                }
            }
            "roxid completion" => {
                if COMPLETION_SUBS.contains(&tok.as_str()) {
                    path = format!("roxid completion {tok}");
                    positional = 0;
                }
            }
            // 叶子命令：token 计为位置参数（model/prompt/tag 等）
            _ => positional += 1,
        }
    }
    (path, positional)
}

/// 按前缀过滤候选（zsh compadd 不做前缀过滤，统一在此完成；
/// AsRef 泛化同时接纳 &[&str] 静态表与 Vec<String> 动态候选）
fn filter_prefix<I, S>(candidates: I, prefix: &str) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    candidates
        .into_iter()
        .filter(|c| c.as_ref().starts_with(prefix))
        .map(|c| c.as_ref().to_string())
        .collect()
}

/// 补全核心（纯函数，单测直接注入候选数据）。
/// 判定顺序：选项位（当前词以 - 开头）→ 子命令位 / 动态值位。
///
/// - 参数 words：到光标为止的全部 token（末位为正在输入的词，可为空串）
/// - 参数 models：本地已装模型名（完整 model:tag 形态）
/// - 参数 runtime_tags：已装 runtime tag（含可用时的 manual）
/// - 参数 running_models：运行中模型名（stop 值位专用源，来自 /api/ps；
///   serve 未运行时为空。迭代36 M125，Q1 裁决 2026-09-11 05:43）
/// - 参数 install_tags：GitHub 最新预发布 tag（install 值位专用源，
///   联网 1s 超时查得；失败时为空。迭代48 M183，Q3-B 裁决
///   2026-09-12 04:24）
/// - 参数 use_variants：use 值位第二位的变体候选（迭代50 M191：
///   短词 cuda/vulkan/cpu + 该 tag 已装变体目录名，complete() 侧本地
///   扫描注入；非该值位为空）
/// - 返回：候选列表（调用方逐行打印）
pub fn complete_for(
    words: &[String],
    models: &[String],
    runtime_tags: &[String],
    running_models: &[String],
    install_tags: &[String],
    use_variants: &[String],
) -> Vec<String> {
    let cur = words.last().map(String::as_str).unwrap_or("");
    let prior = &words[..words.len().saturating_sub(1)];
    let (path, positional) = analyze(prior);

    // 选项位：正在输入 - 开头的词（纯 "-" 交由 shell 文件名补全）
    if cur.starts_with('-') && cur != "-" {
        return filter_prefix(options_for(&path), cur);
    }

    match (path.as_str(), positional) {
        // 子命令位
        ("roxid", _) => filter_prefix(TOP_COMMANDS, cur),
        ("roxid runtime", _) => filter_prefix(RUNTIME_SUBS, cur),
        ("roxid completion", _) => filter_prefix(COMPLETION_SUBS, cur),
        // install 值位：GitHub 最新预发布 tag（联网 2s 超时，失败零候选
        // ——迭代48 M183，Q3-B 裁决 2026-09-12 04:24 + Q6-A 实测修正
        // 2026-09-12 04:44 维持区间上限 2s；--url 值位误触发联网查询按
        // 既有「误计零影响」先例维持，失败静默无干扰）
        ("roxid runtime install", 0) => filter_prefix(install_tags.iter().map(String::as_str), cur),
        // runtime 动态值位：已装 tag（+manual）
        ("roxid runtime use", 0) | ("roxid runtime rm", 0) => {
            filter_prefix(runtime_tags.iter().map(String::as_str), cur)
        }
        // use 值位第二位：变体候选（迭代50 M191，用户裁决链 15:47/15:55/
        // 15:58——短词 + 已装目录名由 complete() 侧本地扫描注入；
        // rm 无第二位不涉及）
        ("roxid runtime use", 1) => filter_prefix(use_variants.iter().map(String::as_str), cur),
        // stop 值位：仅运行中模型（stop 只作用于活跃实例；官方差异——
        // 用户实测报告全列出无法辨别。迭代36 M125，Q1 裁决 2026-09-11 05:43）
        ("roxid stop", 0) => filter_prefix(running_models.iter().map(String::as_str), cur),
        // model 动态值位：本地已装模型名（cp 两位置参数均补；
        // run 第二位起为 prompt 不补；create 不在裁决清单不补）
        ("roxid cp", 0)
        | ("roxid cp", 1)
        | ("roxid run", 0)
        | ("roxid show", 0)
        | ("roxid pull", 0)
        | ("roxid push", 0)
        | ("roxid rm", 0) => filter_prefix(models.iter().map(String::as_str), cur),
        // 其余（prompt 段 / install 的 tag / 无位置参数命令）零候选静默
        _ => Vec::new(),
    }
}

/// 本地已装模型名（直读 models 根目录，与 cmd_list 的服务端 tags 同源能力，
/// 零网络——serve 未运行时 TAB 补全不受影响）。
/// M148（迭代41）改 pub：create TUI 向导的 FROM 候选复用同一数据源
pub fn local_model_names() -> Vec<String> {
    roxid_server::repo::list_models(&roxid_server::config::models_root())
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// 已装 runtime tag 列表（manual 已安装时附于末尾）
fn local_runtime_tags() -> Vec<String> {
    let mut tags: Vec<String> = roxid_server::runtime::list_installed()
        .into_iter()
        .map(|(tag, _)| tag)
        .collect();
    if roxid_server::runtime::manual_server_path().is_file() {
        tags.push("manual".to_string());
    }
    tags
}

/// use 值位第二位的变体候选（迭代50 M191，用户裁决链 15:47/15:55/15:58）：
/// 短词三词（VARIANT_KEYWORDS 单一事实源）在前 + 该 tag 已装变体目录名
///（variant_dirs_of 磁盘事实零硬编码，去重）在后；tag 形态非法/未取到时
/// 仅短词。
///
/// - 参数 prior：光标前 token（尾部首个非 - 词即 use 的 tag 实参）
/// - 返回：变体候选列表
fn local_use_variant_candidates(prior: &[String]) -> Vec<String> {
    let mut out: Vec<String> = roxid_server::runtime::VARIANT_KEYWORDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let tag = prior.iter().rev().find(|t| !t.starts_with('-'));
    if let Some(tag) = tag.filter(|t| roxid_server::runtime::is_valid_tag(t)) {
        for dir in roxid_server::runtime::variant_dirs_of(tag) {
            if !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out
}

/// `__complete` 子命令入口：计算候选并逐行打印到 stdout。
/// 迭代36 M125（Q1/Q7 裁决 2026-09-11 05:43/06:02）：stop 值位候选改为
/// 运行中模型——GET /api/ps，客户端 1s 超时；serve 未运行/超时/响应异常
/// → 零候选静默（Tab 不卡顿不报错）。
/// 迭代48 M183（Q3-B 裁决 2026-09-12 04:24）：install 值位候选改为
/// GitHub 最新预发布 tag——GET Releases，2s 超时单次不重试，失败零候选
/// 静默。其余命令保持本地直读零网络。
///
/// - 参数 words：光标上下文 token（到当前词为止，不含程序名与 __complete）
pub async fn complete(words: &[String]) {
    // 仅 stop / install 值位发起网络查询；选项位（- 开头词）走
    // complete_for 选项分支
    let cur = words.last().map(String::as_str).unwrap_or("");
    let (path, positional) = analyze(&words[..words.len().saturating_sub(1)]);
    let is_value_pos = !(cur.starts_with('-') && cur != "-");
    let is_stop_value = path == "roxid stop" && positional == 0 && is_value_pos;
    let is_install_value = path == "roxid runtime install" && positional == 0 && is_value_pos;
    // 迭代50 M191：use 值位第二位——变体候选（本地扫描零联网）
    let is_use_variant_value = path == "roxid runtime use" && positional == 1 && is_value_pos;
    let (running, install_tags, use_variants) = if is_stop_value {
        (running_model_names().await, Vec::new(), Vec::new())
    } else if is_install_value {
        (Vec::new(), remote_install_tags().await, Vec::new())
    } else if is_use_variant_value {
        (
            Vec::new(),
            Vec::new(),
            local_use_variant_candidates(&words[..words.len().saturating_sub(1)]),
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    let candidates = complete_for(
        words,
        &local_model_names(),
        &local_runtime_tags(),
        &running,
        &install_tags,
        &use_variants,
    );
    for candidate in candidates {
        println!("{candidate}");
    }
}

/// install 值位的 GitHub 最新 tag 候选（迭代48 M183，Q3-B 裁决
/// 2026-09-12 04:24：每次实时联网查，2 秒总超时（裁决区间 1-2s 内取
/// 上限——实测 API 响应常超 1s，1s 恒空候选），失败零候选静默——
/// 对齐 stop 值位 /api/ps 先例（M125）；GitHub API 未认证限流 60 次/时
/// 风险用户裁决自担）。单次尝试不重试（Tab 场景重试徒增卡顿）；
/// 过滤复用 server 侧 pick_prerelease_tags 纯函数（与 ensure 兜底链
/// 同源语义），数量经 config [runtime].tag_complete_limit（默认 10）。
///
/// - 返回：最新预发布 tag 列表（降序）；超时/网络失败/响应异常时为空
async fn remote_install_tags() -> Vec<String> {
    let fetch = async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;
        let releases: Vec<serde_json::Value> = client
            .get("https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=100")
            .header("User-Agent", "roxid")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .json()
            .await?;
        Ok::<_, reqwest::Error>(releases)
    };
    match fetch.await {
        Ok(releases) => roxid_server::runtime::pick_prerelease_tags(
            &releases,
            roxid_server::config::tag_complete_limit(),
        ),
        Err(_) => Vec::new(),
    }
}

/// 运行中模型名（stop 值位候选源；迭代36 M125）。
///
/// - 返回：模型名列表；1s 超时 / serve 未运行 / 响应异常时为空
///   （Q7 裁决超时 1 秒；Q1 裁决失败零候选）
async fn running_model_names() -> Vec<String> {
    let fetch = async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(1))
            .build()?;
        let v: serde_json::Value = client
            .get(format!("{}/api/ps", crate::base_url()))
            .send()
            .await?
            .json()
            .await?;
        Ok::<_, reqwest::Error>(v)
    };
    match fetch.await {
        Ok(v) => v["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::complete_for;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
    fn sv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const MODELS: &[&str] = &["llama3.2:3b", "qwen3:4b", "hf.co/deepseek/r1:Q4"];
    const TAGS: &[&str] = &["b10605", "b10700", "manual"];
    /// 运行中模型（迭代36 M125：stop 值位专用候选源；刻意为 MODELS
    /// 的真子集以验证 stop 不再吃本地全量）
    const RUNNING: &[&str] = &["qwen3:4b"];
    /// GitHub 最新 tag（迭代48 M183：install 值位专用候选源；刻意与
    /// 本地 TAGS 部分交叠以验证两源不串）
    const INSTALL_TAGS: &[&str] = &["b10909", "b10883"];
    /// use 值位第二位的变体候选（迭代50 M191：短词 + 已装目录名注入源；
    /// 刻意含完整目录名形态以验证两形态并列）
    const USE_VARIANTS: &[&str] = &["cuda", "vulkan", "cpu", "ubuntu-vulkan-x64"];

    /// 顶层子命令位：空当前词出全量（含 ls 别名）；前缀过滤生效
    #[test]
    fn top_level_subcommand_position() {
        let all = complete_for(
            &v(&[]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert!(all.contains(&"serve".to_string()));
        assert!(all.contains(&"ls".to_string()));
        assert_eq!(all.len(), TOP_COMMAND_COUNT);

        let s = complete_for(
            &v(&["s"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(
            s,
            vec!["serve", "show", "stop", "signin", "signout", "setup"]
        );
    }

    /// model 值位：run/show/pull/push/rm 首位补本地模型名
    #[test]
    fn model_value_position() {
        let out = complete_for(
            &v(&["run", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(out, sv(MODELS));
        // 前缀过滤
        let out = complete_for(
            &v(&["run", "ll"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(out, vec!["llama3.2:3b".to_string()]);
    }

    /// stop 值位只补运行中模型（迭代36 M125，Q1 裁决 2026-09-11 05:43）：
    /// 候选来自 running 源而非本地全量；serve 未运行（running 空）零候选
    #[test]
    fn stop_value_position_uses_running_models() {
        let out = complete_for(
            &v(&["stop", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(out, sv(RUNNING), "stop 候选必须是运行中模型而非本地全量");
        // 前缀过滤：本地有但未运行的不出现
        let out = complete_for(
            &v(&["stop", "ll"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert!(out.is_empty(), "未运行的 llama3.2:3b 不得出现在 stop 候选");
        // serve 未运行：零候选静默
        let out = complete_for(&v(&["stop", ""]), &sv(MODELS), &sv(TAGS), &[], &[], &[]);
        assert!(out.is_empty(), "running 为空时 stop 零候选");
    }

    /// cp 两位置参数均补模型；run 第二位起（prompt 段）不补
    #[test]
    fn cp_both_positions_and_prompt_silent() {
        let src = complete_for(
            &v(&["cp", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(src, sv(MODELS));
        let dst = complete_for(
            &v(&["cp", "llama3.2:3b", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(dst, sv(MODELS));
        let prompt = complete_for(
            &v(&["run", "llama3.2:3b", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert!(prompt.is_empty(), "prompt 段不补全");
    }

    /// runtime 族：二级子命令位与 use/rm/install 的动态 tag 位
    ///（install 值位为 GitHub 最新 tag——迭代48 M183；use/rm 维持本地已装源；
    /// use 第二位为变体候选——迭代50 M191）
    #[test]
    fn runtime_nested_and_tag_values() {
        let subs = complete_for(
            &v(&["runtime", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(subs, vec!["list", "install", "update", "use", "rm"]);
        let use_tags = complete_for(
            &v(&["runtime", "use", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(use_tags, vec!["b10605", "b10700", "manual"]);
        let rm_b = complete_for(
            &v(&["runtime", "rm", "b1"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(rm_b, vec!["b10605", "b10700"]);
        // install 值位：GitHub 最新 tag 源（与本地已装源独立，不串）
        let inst = complete_for(
            &v(&["runtime", "install", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(inst, vec!["b10909", "b10883"]);
        // 前缀过滤 + 联网失败（install_tags 空）零候选静默
        let inst_b = complete_for(
            &v(&["runtime", "install", "b108"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(inst_b, vec!["b10883"]);
        let inst_offline = complete_for(
            &v(&["runtime", "install", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &[],
            &[],
        );
        assert!(inst_offline.is_empty(), "联网失败时 install 零候选静默");
        // 迭代50 M191：use 值位第二位——变体候选（短词+完整目录名并列、
        // 前缀过滤、与 tag 位候选源不串；候选源为空时零候选静默）
        let use_variants = complete_for(
            &v(&["runtime", "use", "b10700", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &[],
            &sv(USE_VARIANTS),
        );
        assert_eq!(use_variants, sv(USE_VARIANTS));
        let use_c = complete_for(
            &v(&["runtime", "use", "b10700", "c"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &[],
            &sv(USE_VARIANTS),
        );
        assert_eq!(use_c, sv(&["cuda", "cpu"]));
        let use_empty = complete_for(
            &v(&["runtime", "use", "b10700", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &[],
            &[],
        );
        assert!(use_empty.is_empty(), "变体候选源为空时零候选静默");
    }

    /// 选项位：- 前缀词补该命令选项；全局选项仅在顶层上下文
    #[test]
    fn option_position() {
        let run_opts = complete_for(
            &v(&["run", "--"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(run_opts, vec!["--hf", "--runtime", "--help"]);
        let top_opts = complete_for(
            &v(&["--"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert!(top_opts.contains(&"--nowordwrap".to_string()));
        let prefix = complete_for(
            &v(&["run", "--h"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(prefix, vec!["--hf", "--help"]);
    }

    /// ls 别名归一：进入 list 后上下文正确（无动态值位，零候选）
    #[test]
    fn ls_alias_canonicalized() {
        let out = complete_for(
            &v(&["ls", "x"]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert!(out.is_empty(), "list 无位置参数，第二 token 后零候选");
    }

    /// completion 二级子命令位候选
    #[test]
    fn completion_subcommands() {
        let out = complete_for(
            &v(&["completion", ""]),
            &sv(MODELS),
            &sv(TAGS),
            &sv(RUNNING),
            &sv(INSTALL_TAGS),
            &[],
        );
        assert_eq!(out, vec!["bash", "zsh", "fish", "install"]);
    }

    /// 顶层子命令总数常量（与 TOP_COMMANDS 漂移防护）
    const TOP_COMMAND_COUNT: usize = 18;
}
