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

/// runtime 二级子命令位候选
const RUNTIME_SUBS: &[&str] = &["list", "install", "use", "rm"];

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
/// - 返回：候选列表（调用方逐行打印）
pub fn complete_for(words: &[String], models: &[String], runtime_tags: &[String]) -> Vec<String> {
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
        // runtime 动态值位：已装 tag（+manual）
        ("roxid runtime use", 0) | ("roxid runtime rm", 0) => {
            filter_prefix(runtime_tags.iter().map(String::as_str), cur)
        }
        // model 动态值位：本地已装模型名（cp 两位置参数均补；
        // run 第二位起为 prompt 不补；create 不在裁决清单不补）
        ("roxid cp", 0)
        | ("roxid cp", 1)
        | ("roxid run", 0)
        | ("roxid show", 0)
        | ("roxid stop", 0)
        | ("roxid pull", 0)
        | ("roxid push", 0)
        | ("roxid rm", 0) => filter_prefix(models.iter().map(String::as_str), cur),
        // 其余（prompt 段 / install 的 tag / 无位置参数命令）零候选静默
        _ => Vec::new(),
    }
}

/// 本地已装模型名（直读 models 根目录，与 cmd_list 的服务端 tags 同源能力，
/// 零网络——serve 未运行时 TAB 补全不受影响）
fn local_model_names() -> Vec<String> {
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

/// `__complete` 子命令入口：计算候选并逐行打印到 stdout
///
/// - 参数 words：光标上下文 token（到当前词为止，不含程序名与 __complete）
pub fn complete(words: &[String]) {
    for candidate in complete_for(words, &local_model_names(), &local_runtime_tags()) {
        println!("{candidate}");
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

    /// 顶层子命令位：空当前词出全量（含 ls 别名）；前缀过滤生效
    #[test]
    fn top_level_subcommand_position() {
        let all = complete_for(&v(&[]), &sv(MODELS), &sv(TAGS));
        assert!(all.contains(&"serve".to_string()));
        assert!(all.contains(&"ls".to_string()));
        assert_eq!(all.len(), TOP_COMMAND_COUNT);

        let s = complete_for(&v(&["s"]), &sv(MODELS), &sv(TAGS));
        assert_eq!(
            s,
            vec!["serve", "show", "stop", "signin", "signout", "setup"]
        );
    }

    /// model 值位：run/show/stop/pull/push/rm 首位补本地模型名
    #[test]
    fn model_value_position() {
        let out = complete_for(&v(&["run", ""]), &sv(MODELS), &sv(TAGS));
        assert_eq!(out, sv(MODELS));
        // 前缀过滤
        let out = complete_for(&v(&["run", "ll"]), &sv(MODELS), &sv(TAGS));
        assert_eq!(out, vec!["llama3.2:3b".to_string()]);
    }

    /// cp 两位置参数均补模型；run 第二位起（prompt 段）不补
    #[test]
    fn cp_both_positions_and_prompt_silent() {
        let src = complete_for(&v(&["cp", ""]), &sv(MODELS), &sv(TAGS));
        assert_eq!(src, sv(MODELS));
        let dst = complete_for(&v(&["cp", "llama3.2:3b", ""]), &sv(MODELS), &sv(TAGS));
        assert_eq!(dst, sv(MODELS));
        let prompt = complete_for(&v(&["run", "llama3.2:3b", ""]), &sv(MODELS), &sv(TAGS));
        assert!(prompt.is_empty(), "prompt 段不补全");
    }

    /// runtime 族：二级子命令位与 use/rm 的动态 tag 位
    #[test]
    fn runtime_nested_and_tag_values() {
        let subs = complete_for(&v(&["runtime", ""]), &sv(MODELS), &sv(TAGS));
        assert_eq!(subs, vec!["list", "install", "use", "rm"]);
        let use_tags = complete_for(&v(&["runtime", "use", ""]), &sv(MODELS), &sv(TAGS));
        assert_eq!(use_tags, vec!["b10605", "b10700", "manual"]);
        let rm_b = complete_for(&v(&["runtime", "rm", "b1"]), &sv(MODELS), &sv(TAGS));
        assert_eq!(rm_b, vec!["b10605", "b10700"]);
    }

    /// 选项位：- 前缀词补该命令选项；全局选项仅在顶层上下文
    #[test]
    fn option_position() {
        let run_opts = complete_for(&v(&["run", "--"]), &sv(MODELS), &sv(TAGS));
        assert_eq!(run_opts, vec!["--hf", "--runtime", "--help"]);
        let top_opts = complete_for(&v(&["--"]), &sv(MODELS), &sv(TAGS));
        assert!(top_opts.contains(&"--nowordwrap".to_string()));
        let prefix = complete_for(&v(&["run", "--h"]), &sv(MODELS), &sv(TAGS));
        assert_eq!(prefix, vec!["--hf", "--help"]);
    }

    /// ls 别名归一：进入 list 后上下文正确（无动态值位，零候选）
    #[test]
    fn ls_alias_canonicalized() {
        let out = complete_for(&v(&["ls", "x"]), &sv(MODELS), &sv(TAGS));
        assert!(out.is_empty(), "list 无位置参数，第二 token 后零候选");
    }

    /// completion 二级子命令位候选
    #[test]
    fn completion_subcommands() {
        let out = complete_for(&v(&["completion", ""]), &sv(MODELS), &sv(TAGS));
        assert_eq!(out, vec!["bash", "zsh", "fish", "install"]);
    }

    /// 顶层子命令总数常量（与 TOP_COMMANDS 漂移防护）
    const TOP_COMMAND_COUNT: usize = 18;
}
