//! help 双语国际化（迭代20 M56 + 迭代47 M176/M177/M178）：`--help` 文本与
//! 参数解析错误均跟随用户 locale 动态显示（zh* → 中文，其余 → 英文），
//! 输出 GNU 风格完整帮助（用法行 + 平实描述 + 逐选项说明 + 尾部详解段；
//! 全命令统一「说明独立成行」布局）。
//!
//! 裁决来源（用户确认 2026-09-09 21:28 Q1）：help 语言跟随用户语言环境，
//! 不提供静态单语。locale 判定走 POSIX 惯例 LC_ALL → LC_MESSAGES → LANG，
//! 纯 Rust 零 C 依赖（不影响 musl 静态单二进制）。
//!
//! 实现要点：derive doc comment 保留为英文兜底；本模块集中承载双语文本表
//! （键 = 命令路径），`apply_localized_help` 在 `Cli::command()` 构建后、
//! `parse_from` 之前递归覆盖 about / long_about / after_long_help 与参数
//! help。命令树漂移防护：单测断言每个非隐藏子命令路径双语条目非空。
//!
//! 迭代47（M176/M177/M178，用户裁决链 2026-09-12 03:01–03:06）：
//! - M176：`render_clap_error` 按 locale 渲染 clap 参数错误（错误行 +
//!   用法行 + 提示行），退出码 2（GNU 惯例，Q3-A 裁决）；
//! - M177：`localize_help_headings` 在渲染层将 help 输出的段落标题
//!   （Usage:/Options:/Arguments:/Commands:）与 clap 自动生成的 -h
//!   说明替换为中文（help flag 惰性注入，mut_arg 声明期不可及故走
//!   渲染后处理——与标题替换同机制，零 parse 干扰）；
//! - M178：命令树全量 `next_line_help(true)`——参数说明统一独立成行
//!   （Q2-A 裁决，消除跨命令「同行/独立行」布局混杂）。
//!
//! 修改历史：M56 新增 2026-09-09 21-40（迭代19 已占用 M53–M55，
//! 本迭代自 M56 起编号）；M176–M178 增补 2026-09-12 03-13（迭代47）

use clap::Command;

/// 显示语言
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// 简体中文（zh* locale）
    Zh,
    /// 英文（缺省/C/POSIX/其他语言）
    En,
}

/// 从 locale 字符串判定显示语言（纯函数，便于单测）。
/// zh 前缀（zh_CN.UTF-8 / zh_TW / zh）→ 中文；C、POSIX、空、其他 → 英文。
///
/// - 参数 locale：LC_ALL/LC_MESSAGES/LANG 之一的环境值
/// - 返回：判定后的显示语言
pub fn lang_from_locale(locale: &str) -> Lang {
    // 取编码修饰前的主段（zh_CN.UTF-8 → zh_CN），大小写不敏感匹配 zh 前缀
    let primary = locale.split('.').next().unwrap_or("");
    if primary.to_lowercase().starts_with("zh") {
        Lang::Zh
    } else {
        Lang::En
    }
}

/// 按 POSIX 优先级读环境变量判定当前显示语言：LC_ALL → LC_MESSAGES → LANG。
pub fn detect_lang() -> Lang {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    lang_from_locale(&locale)
}

/// 单命令的本地化帮助文本（静态表条目）
struct CmdHelp {
    /// 命令一句话描述（about / long_about 同源，短长 help 一致展示）
    about: &'static str,
    /// 尾部详解段（after_long_help：示例 / 环境变量 / 退出状态等）
    after: &'static str,
    /// 参数级 help 覆盖表：(clap arg id, 本地化说明)
    args: &'static [(&'static str, &'static str)],
}

// 中文文本表：键 = 命令路径（与 Commands 枚举树逐一对齐）
const ZH: &[(&str, CmdHelp)] = &[
    (
        "roxid",
        CmdHelp {
            about: "大语言模型运行器",
            after: r##"环境变量：
  ROXID_HOST      roxid 服务地址（默认 http://127.0.0.1:11434）
  OLLAMA_HOST     兼容原版 Ollama 的同义变量（ROXID_HOST 优先）

模型名三形态：
  llama3.2            ollama 主源模型（缺省 tag 即 latest）
  llama3.2:3b         指定 tag 的主源模型
  hf.co/user/repo:Q4_K_M
                      HuggingFace 直引（可用 --hf 标志省写前缀）

Shell 补全：
  roxid completion install
                      安装 bash/zsh/fish 自动补全（重开终端生效）

退出状态：
  0    成功
  1    失败（服务不可达 / 模型不存在 / 网络错误等）"##,
            args: &[
                ("nowordwrap", "不自动折行（长行原样输出）"),
                ("verbose", "显示响应耗时统计（输入/输出 token 数）"),
                ("_version", "显示版本信息"),
            ],
        },
    ),
    (
        "roxid serve",
        CmdHelp {
            about: "启动 roxid 服务（前台阻塞）",
            after: r##"示例：
  roxid serve                    # 默认监听 127.0.0.1:11434
  roxid serve --addr 0.0.0.0:11434
                                 # 监听全部网卡

首次启动自动下载 llama.cpp 运行时（锁定链；探测 GPU → vulkan / 无 GPU → cpu）。
如需自定义版本：
  roxid setup --llama-url <url>  # 手动包直装
  roxid runtime install <tag>    # 指定官方版本"##,
            args: &[("addr", "监听地址（默认 127.0.0.1:11434）")],
        },
    ),
    (
        "roxid create",
        CmdHelp {
            about: "从 Modelfile 创建模型",
            after: r##"示例：
  roxid create mymodel -f Modelfile
  roxid create mymodel -i        # 全屏 TUI 向导（列表选基础模型/SYSTEM/RUNTIME/预览）

Modelfile 关键指令：FROM（基础模型）、SYSTEM（系统提示，""" 多行）、
PARAMETER（参数，可多条）、RUNTIME（llama.cpp 启动参数）。"##,
            args: &[
                ("model", "新模型名"),
                ("file", "Modelfile 路径（缺省时配合 -i 进入交互创建）"),
                (
                    "interactive",
                    "全屏 TUI 交互创建向导（-f 缺省时生效；终端不支持时降级逐行问答）",
                ),
            ],
        },
    ),
    (
        "roxid show",
        CmdHelp {
            about: "显示模型信息",
            after: r##"示例：
  roxid show llama3.2:3b

输出：模型家族、参数量与量化、System 提示、PARAMETER 键值区、
Capabilities、RUNTIME 启动参数。"##,
            args: &[("model", "模型名")],
        },
    ),
    (
        "roxid run",
        CmdHelp {
            about: "运行模型（单次提示或交互式 REPL 多轮对话）",
            after: r##"示例：
  roxid run llama3.2:3b 为什么天空是蓝色的？   # 单次提示
  roxid run llama3.2:3b                        # 进入 REPL
  roxid run -hf qwen/qwen3:4b 你好              # HF 直引（省写前缀）
  roxid run m --runtime "-ngl 30 --no-mmap"     # 本次临时启动参数

REPL 命令：/bye 退出、/clear 清空对话、Ctrl-D 退出。
模型未安装时自动拉取一次后重试。"##,
            args: &[
                ("model", "模型名"),
                ("prompt", "首条提示（缺省进入交互 REPL）"),
                ("hf", "模型名按 HuggingFace 直引解析：{user}/{repo}:{quant}"),
                (
                    "runtime",
                    "本次运行的 llama.cpp 启动参数（临时覆盖模型 RUNTIME，不落盘）",
                ),
            ],
        },
    ),
    (
        "roxid stop",
        CmdHelp {
            about: "停止运行中的模型",
            after: r##"示例：
  roxid stop llama3.2:3b"##,
            args: &[("model", "模型名")],
        },
    ),
    (
        "roxid pull",
        CmdHelp {
            about: "从模型仓库拉取模型",
            after: r##"示例：
  roxid pull llama3.2:3b          # ollama 主源
  roxid pull -hf qwen/qwen3:4b    # HuggingFace 直引

国内网络可在 roxid setup 中配置下载代理加速。"##,
            args: &[
                ("model", "模型名"),
                ("hf", "模型名按 HuggingFace 直引解析：{user}/{repo}:{quant}"),
            ],
        },
    ),
    (
        "roxid push",
        CmdHelp {
            about: "推送模型到 ollama.com 仓库",
            after: r##"示例：
  roxid push mymodel:latest

前置：roxid signin 登录（凭据存于 ~/.roxid/auth.json）。"##,
            args: &[("model", "模型名")],
        },
    ),
    (
        "roxid signin",
        CmdHelp {
            about: "登录 ollama.com",
            after: r##"凭据保存于 ~/.roxid/auth.json（权限 0600 仅属主可读）。
令牌输入不回显。"##,
            args: &[],
        },
    ),
    (
        "roxid signout",
        CmdHelp {
            about: "退出 ollama.com 登录",
            after: r##"删除 ~/.roxid/auth.json 凭据文件。"##,
            args: &[],
        },
    ),
    (
        "roxid list",
        CmdHelp {
            about: "列出本地模型",
            after: r##"示例：
  roxid list
  roxid ls          # 别名"##,
            args: &[],
        },
    ),
    (
        "roxid ps",
        CmdHelp {
            about: "列出运行中的模型",
            after: r##"示例：
  roxid ps"##,
            args: &[],
        },
    ),
    (
        "roxid cp",
        CmdHelp {
            about: "复制模型",
            after: r##"示例：
  roxid cp llama3.2:3b mymodel:latest

大文件经硬链接落位，秒级完成零磁盘占用。"##,
            args: &[("source", "源模型名"), ("destination", "目标模型名")],
        },
    ),
    (
        "roxid rm",
        CmdHelp {
            about: "删除模型",
            after: r##"示例：
  roxid rm llama3.2:3b"##,
            args: &[("model", "模型名")],
        },
    ),
    (
        "roxid launch",
        CmdHelp {
            about: "启动 roxid 菜单或集成",
            after: r##"菜单集成在 roxid 后续版本提供（当前可用命令见 roxid --help）。"##,
            args: &[],
        },
    ),
    (
        "roxid setup",
        CmdHelp {
            about: "首次运行配置（下载代理 / 自定义后端）",
            after: r##"示例：
  roxid setup                        # 交互式引导
  roxid setup --llama-url <url>      # 直装手动后端（无需 TTY）

配置持久化于 ~/.roxid/config.toml；联网命令首次运行时自动触发引导。"##,
            args: &[("llama_url", "手动指定 llama.cpp 包下载链接并立即安装")],
        },
    ),
    (
        "roxid runtime",
        CmdHelp {
            about: "管理 llama.cpp 运行时版本",
            after: r##"运行时解析优先级：环境变量 ROXID_LLAMA_SERVER → manual → 默认版本 → 锁定链。
示例：
  roxid runtime list
  roxid runtime install b10700
  roxid runtime use b10700
  roxid runtime rm b10700"##,
            args: &[],
        },
    ),
    (
        "roxid runtime list",
        CmdHelp {
            about: "列出已安装的运行时版本",
            after: r##"输出每版本的全部已装变体（vulkan/cpu）与当前默认标记。"##,
            args: &[],
        },
    ),
    (
        "roxid runtime install",
        CmdHelp {
            about: "安装运行时版本（官方 tag 或自定义链接）",
            after: r##"示例：
  roxid runtime install b10700          # 官方 tag
  roxid runtime install --url <url>     # 自定义包（落位 manual）

自动探测后端变体（GPU → vulkan / 无 GPU → cpu）；
国内网络可用 ROXID_GH_PROXY 加速。"##,
            args: &[
                ("tag", "版本 tag（b\\d+ 形态，如 b10700）"),
                ("url", "自定义包链接（tar.gz 或裸 llama-server）"),
            ],
        },
    ),
    (
        "roxid runtime use",
        CmdHelp {
            about: "设置默认运行时版本",
            after: r##"示例：
  roxid runtime use b10700
  roxid runtime use manual

切换后正在运行的实例按需重建。"##,
            args: &[("tag", "版本 tag 或 \"manual\"")],
        },
    ),
    (
        "roxid runtime rm",
        CmdHelp {
            about: "删除已安装的运行时版本",
            after: r##"示例：
  roxid runtime rm b10700

当前默认版本需先切换后才能删除。"##,
            args: &[("tag", "版本 tag")],
        },
    ),
    (
        "roxid completion",
        CmdHelp {
            about: "管理 shell 自动补全（bash / zsh / fish）",
            after: r##"示例：
  roxid completion install      # 自动探测 $SHELL 并安装（推荐）
  roxid completion bash         # 输出 bash 补全脚本
  eval "$(roxid completion bash)"   # 当前会话立即生效

补全候选直读 ~/.roxid 本地数据（模型名 / 运行时 tag），零网络零延迟。"##,
            args: &[],
        },
    ),
    (
        "roxid completion bash",
        CmdHelp {
            about: "输出 bash 补全脚本",
            after: r##"用法：eval "$(roxid completion bash)" 当前会话生效；
或 roxid completion install 落位后重开终端。"##,
            args: &[],
        },
    ),
    (
        "roxid completion zsh",
        CmdHelp {
            about: "输出 zsh 补全脚本",
            after: r##"用法：eval "$(roxid completion zsh)" 当前会话生效；
或 roxid completion install 落位后重开终端。"##,
            args: &[],
        },
    ),
    (
        "roxid completion fish",
        CmdHelp {
            about: "输出 fish 补全脚本",
            after: r##"用法：roxid completion fish | source 当前会话生效；
或 roxid completion install 落位后重开终端。"##,
            args: &[],
        },
    ),
    (
        "roxid completion install",
        CmdHelp {
            about: "安装补全到标准目录（重开终端生效）",
            after: r##"自动探测 $SHELL 落位：
  bash → ~/.local/share/bash-completion/completions/roxid
  zsh  → ~/.zfunc/_roxid（必要时补 .zshrc 的 fpath/compinit）
  fish → ~/.config/fish/completions/roxid.fish

幂等：重复执行覆盖旧脚本。"##,
            args: &[],
        },
    ),
];

// 英文文本表：about 层与 derive doc comment 一致（官方对齐输出），
// 补齐 after_long_help 详解段（GNU 风格尾部说明）
const EN: &[(&str, CmdHelp)] = &[
    (
        "roxid",
        CmdHelp {
            about: "Large language model runner",
            after: r##"Environment variables:
  ROXID_HOST      roxid server address (default http://127.0.0.1:11434)
  OLLAMA_HOST     Ollama-compatible alias (ROXID_HOST takes precedence)

Model name forms:
  llama3.2            ollama registry model (default tag: latest)
  llama3.2:3b         registry model with explicit tag
  hf.co/user/repo:Q4_K_M
                      HuggingFace direct reference (--hf shorthand)

Shell completion:
  roxid completion install
                      set up bash/zsh/fish TAB completion (new terminal)

Exit status:
  0    success
  1    failure (server unreachable / model missing / network error)"##,
            args: &[
                (
                    "nowordwrap",
                    "Don't wrap words to the next line automatically",
                ),
                ("verbose", "Show timings for response"),
                ("_version", "Show version information"),
            ],
        },
    ),
    (
        "roxid serve",
        CmdHelp {
            about: "Start roxid",
            after: r##"Examples:
  roxid serve                    # default 127.0.0.1:11434
  roxid serve --addr 0.0.0.0:11434

Downloads the llama.cpp runtime automatically on first start
(GPU detected -> vulkan / otherwise cpu). Custom builds:
  roxid setup --llama-url <url>  # manual package
  roxid runtime install <tag>    # pinned official version"##,
            args: &[("addr", "Listen address (default 127.0.0.1:11434)")],
        },
    ),
    (
        "roxid create",
        CmdHelp {
            about: "Create a model",
            after: r##"Examples:
  roxid create mymodel -f Modelfile
  roxid create mymodel -i        # full-screen TUI wizard

Modelfile directives: FROM (base model), SYSTEM (""" blocks),
PARAMETER (repeatable), RUNTIME (llama.cpp flags)."##,
            args: &[
                ("model", "Name of the new model"),
                (
                    "file",
                    "Path to the Modelfile (omit with -i for interactive mode)",
                ),
                ("interactive", "Run the full-screen TUI creation wizard"),
            ],
        },
    ),
    (
        "roxid show",
        CmdHelp {
            about: "Show information for a model",
            after: r##"Example:
  roxid show llama3.2:3b

Prints family, parameter size and quantization, system prompt,
PARAMETER entries, capabilities and RUNTIME flags."##,
            args: &[("model", "Model name")],
        },
    ),
    (
        "roxid run",
        CmdHelp {
            about: "Run a model (single prompt or interactive REPL)",
            after: r##"Examples:
  roxid run llama3.2:3b why is the sky blue?   # single prompt
  roxid run llama3.2:3b                        # interactive REPL
  roxid run -hf qwen/qwen3:4b hello            # HF direct reference
  roxid run m --runtime "-ngl 30 --no-mmap"    # one-off runtime flags

REPL commands: /bye to quit, /clear to reset the conversation, Ctrl-D to exit.
Missing models are pulled automatically once, then retried."##,
            args: &[
                ("model", "Model name"),
                (
                    "prompt",
                    "First prompt (omit to enter the interactive REPL)",
                ),
                (
                    "hf",
                    "Resolve the model as HuggingFace reference: {user}/{repo}:{quant}",
                ),
                (
                    "runtime",
                    "llama.cpp flags for this run (one-off override, not persisted)",
                ),
            ],
        },
    ),
    (
        "roxid stop",
        CmdHelp {
            about: "Stop a running model",
            after: r##"Example:
  roxid stop llama3.2:3b"##,
            args: &[("model", "Model name")],
        },
    ),
    (
        "roxid pull",
        CmdHelp {
            about: "Pull a model from a registry",
            after: r##"Examples:
  roxid pull llama3.2:3b          # ollama registry
  roxid pull -hf qwen/qwen3:4b    # HuggingFace direct

Configure a download proxy via `roxid setup` to accelerate pulls
in restricted networks."##,
            args: &[
                ("model", "Model name"),
                (
                    "hf",
                    "Resolve the model as HuggingFace reference: {user}/{repo}:{quant}",
                ),
            ],
        },
    ),
    (
        "roxid push",
        CmdHelp {
            about: "Push a model to a registry",
            after: r##"Example:
  roxid push mymodel:latest

Requires `roxid signin` (credentials in ~/.roxid/auth.json)."##,
            args: &[("model", "Model name")],
        },
    ),
    (
        "roxid signin",
        CmdHelp {
            about: "Sign in to ollama.com",
            after: r##"Credentials are stored in ~/.roxid/auth.json (mode 0600).
The token is typed with echo disabled."##,
            args: &[],
        },
    ),
    (
        "roxid signout",
        CmdHelp {
            about: "Sign out from ollama.com",
            after: r##"Removes ~/.roxid/auth.json."##,
            args: &[],
        },
    ),
    (
        "roxid list",
        CmdHelp {
            about: "List models",
            after: r##"Example:
  roxid list
  roxid ls          # alias"##,
            args: &[],
        },
    ),
    (
        "roxid ps",
        CmdHelp {
            about: "List running models",
            after: r##"Example:
  roxid ps"##,
            args: &[],
        },
    ),
    (
        "roxid cp",
        CmdHelp {
            about: "Copy a model",
            after: r##"Example:
  roxid cp llama3.2:3b mymodel:latest

Large files are hard-linked, so copies are instant and take
no extra disk space."##,
            args: &[
                ("source", "Source model name"),
                ("destination", "Destination model name"),
            ],
        },
    ),
    (
        "roxid rm",
        CmdHelp {
            about: "Remove a model",
            after: r##"Example:
  roxid rm llama3.2:3b"##,
            args: &[("model", "Model name")],
        },
    ),
    (
        "roxid launch",
        CmdHelp {
            about: "Launch the roxid menu or an integration",
            after: r##"The menu integration ships in a later roxid release
(see `roxid --help` for available commands)."##,
            args: &[],
        },
    ),
    (
        "roxid setup",
        CmdHelp {
            about: "Configure first-run settings (download proxy)",
            after: r##"Examples:
  roxid setup                        # interactive wizard
  roxid setup --llama-url <url>      # install a manual backend (no TTY)

Settings persist in ~/.roxid/config.toml; network commands trigger
the wizard automatically on first use."##,
            args: &[(
                "llama_url",
                "Download URL of a custom llama.cpp package, installed immediately",
            )],
        },
    ),
    (
        "roxid runtime",
        CmdHelp {
            about: "Manage llama.cpp runtime versions",
            after: r##"Resolution order: ROXID_LLAMA_SERVER env -> manual ->
default version -> locked tag.
Examples:
  roxid runtime list
  roxid runtime install b10700
  roxid runtime use b10700
  roxid runtime rm b10700"##,
            args: &[],
        },
    ),
    (
        "roxid runtime list",
        CmdHelp {
            about: "List installed runtime versions",
            after: r##"Prints every installed variant (vulkan/cpu) per tag
along with the default marker."##,
            args: &[],
        },
    ),
    (
        "roxid runtime install",
        CmdHelp {
            about: "Install a runtime version by tag or custom URL",
            after: r##"Examples:
  roxid runtime install b10700          # official tag
  roxid runtime install --url <url>     # custom package (manual)

Detects the backend variant automatically (GPU -> vulkan /
otherwise cpu); ROXID_GH_PROXY accelerates downloads."##,
            args: &[
                ("tag", "Version tag (b\\d+ form, e.g. b10700)"),
                ("url", "Custom package URL (tar.gz or bare llama-server)"),
            ],
        },
    ),
    (
        "roxid runtime use",
        CmdHelp {
            about: "Set the default runtime version",
            after: r##"Examples:
  roxid runtime use b10700
  roxid runtime use manual

Running instances are rebuilt on demand after the switch."##,
            args: &[("tag", "Version tag or \"manual\"")],
        },
    ),
    (
        "roxid runtime rm",
        CmdHelp {
            about: "Remove an installed runtime version",
            after: r##"Example:
  roxid runtime rm b10700

The current default must be switched away before removal."##,
            args: &[("tag", "Version tag")],
        },
    ),
    (
        "roxid completion",
        CmdHelp {
            about: "Manage shell completion (bash / zsh / fish)",
            after: r##"Examples:
  roxid completion install      # auto-detect $SHELL and install
  roxid completion bash         # print the bash script
  eval "$(roxid completion bash)"   # enable for this session

Candidates are read straight from ~/.roxid local data
(model names / runtime tags) with zero network latency."##,
            args: &[],
        },
    ),
    (
        "roxid completion bash",
        CmdHelp {
            about: "Print the bash completion script",
            after: r##"Use eval "$(roxid completion bash)" for the current session,
or `roxid completion install` for new terminals."##,
            args: &[],
        },
    ),
    (
        "roxid completion zsh",
        CmdHelp {
            about: "Print the zsh completion script",
            after: r##"Use eval "$(roxid completion zsh)" for the current session,
or `roxid completion install` for new terminals."##,
            args: &[],
        },
    ),
    (
        "roxid completion fish",
        CmdHelp {
            about: "Print the fish completion script",
            after: r##"Use `roxid completion fish | source` for the current session,
or `roxid completion install` for new terminals."##,
            args: &[],
        },
    ),
    (
        "roxid completion install",
        CmdHelp {
            about: "Install completion into the standard directory",
            after: r##"Detects $SHELL and writes:
  bash -> ~/.local/share/bash-completion/completions/roxid
  zsh  -> ~/.zfunc/_roxid (adds fpath/compinit to .zshrc if missing)
  fish -> ~/.config/fish/completions/roxid.fish

Idempotent: re-running overwrites the previous script."##,
            args: &[],
        },
    ),
];

/// 查表（按命令路径取对应语言的静态条目）
fn table(lang: Lang, path: &str) -> Option<&'static CmdHelp> {
    let entries = match lang {
        Lang::Zh => ZH,
        Lang::En => EN,
    };
    entries.iter().find(|(p, _)| *p == path).map(|(_, h)| h)
}

/// 对整棵命令树应用本地化 help（入口）：按 locale 覆盖 about / long_about /
/// after_long_help 与参数 help；未匹配路径保持 derive doc comment 原文。
///
/// - 参数 cmd：`Cli::command()` 构建出的顶层命令（parse 之前注入）
pub fn apply_localized_help(cmd: &mut Command) {
    apply_recursive(cmd, "roxid", detect_lang());
}

/// 递归注入单命令及其全部子命令（含 runtime / completion 二级嵌套）。
/// clap 4 builder 为 consuming 风格（self -> Self）：mem::take 取出、
/// 链式修改、放回；mut_arg 对未知 id 会 panic，先做存在性防御
///（表条目漂移时静默跳过，由单测 arg_ids_match_command_tree 兜底断言）。
fn apply_recursive(cmd: &mut Command, path: &str, lang: Lang) {
    if let Some(h) = table(lang, path) {
        let known_args: Vec<(&str, &str)> = h
            .args
            .iter()
            .copied()
            .filter(|(id, _)| cmd.get_arguments().any(|a| a.get_id() == id))
            .collect();
        let mut builder = std::mem::take(cmd);
        // 短长 help 统一展示同一 about；详解段挂长 help 尾部（GNU 风格）
        builder = builder
            .about(h.about)
            .long_about(h.about)
            .after_long_help(h.after)
            // M178（Q2-A 裁决 2026-09-12 03:05）：参数说明统一独立成行，
            // 消除 clap 自动布局在长短说明命令间的「同行/独立行」混杂
            .next_line_help(true);
        for (id, help) in known_args {
            builder = builder.mut_arg(id, |a| a.help(help));
        }
        *cmd = builder;
    }
    for sub in cmd.get_subcommands_mut() {
        apply_recursive(sub, &format!("{path} {}", sub.get_name()), lang);
    }
}

// ==================== 错误输出双语（迭代47 M176/M177） ====================
// 背景：clap 4 无内建错误 i18n，参数解析失败时 e.exit() 直接透出英文
// （"error: the following required arguments..."），绕过 locale 判定链。
// 本区按 ErrorKind + ContextKind 重建双语文案；覆盖范围与形态对齐
// clap 原生输出（错误行 + 缩进上下文 + 用法行 + 提示行）。

use clap::error::{ContextKind, ContextValue, ErrorKind};

/// 渲染 clap 参数错误为本地化文本（M176，纯函数便于单测）。
///
/// 结构对齐 clap 原生三段式：错误行（含上下文）→ 空行 + 用法行（zh 下
/// 前缀「用法：」）→ 空行 + 提示行。表内 kind 双语渲染；表外 kind 与
/// 上下文缺失的极端形态回退 clap 英文渲染（与 derive doc 英文兜底同哲学，
/// 不吞上游信息）。
///
/// - 参数 e：clap 解析错误
/// - 参数 lang：显示语言
/// - 返回：完整错误文本（不含「错误：」前缀——前缀与颜色由调用方组装）
pub fn render_clap_error(e: &clap::Error, lang: Lang) -> String {
    let headline = match error_headline(e, lang) {
        Some(h) => h,
        // Display* 三类为 help/version 正常输出、未知 kind 极端路径：
        // 英文兜底（调用方对 Display* 不会走到本函数）
        None => return e.render().to_string(),
    };
    let mut out = headline;
    if let Some(ContextValue::StyledStr(usage)) = e.get(ContextKind::Usage) {
        let usage = usage.to_string();
        // 用法行前缀替换：zh「用法：」，en 保持 "Usage:"
        let usage = match (lang, usage.strip_prefix("Usage:")) {
            (Lang::Zh, Some(rest)) => format!("用法：{rest}"),
            (_, _) => usage,
        };
        out.push_str("\n\n");
        out.push_str(&usage);
    }
    out.push_str(match lang {
        Lang::Zh => "\n\n更多信息请尝试 '--help'。",
        Lang::En => "\n\nFor more information, try '--help'.",
    });
    out
}

/// 按错误种类与上下文构建错误首段（含缩进的上下文行）。
/// - 返回 None：该 kind 不在双语表（Display* / 未知），调用方英文兜底
fn error_headline(e: &clap::Error, lang: Lang) -> Option<String> {
    let zh = matches!(lang, Lang::Zh);
    let single = |kind: ContextKind| -> Option<String> {
        match e.get(kind) {
            Some(ContextValue::String(s)) => Some(s.clone()),
            _ => None,
        }
    };
    Some(match e.kind() {
        ErrorKind::MissingRequiredArgument => {
            let mut s = if zh {
                "以下必选参数未提供：".to_string()
            } else {
                "the following required arguments were not provided:".to_string()
            };
            if let Some(ContextValue::Strings(list)) = e.get(ContextKind::InvalidArg) {
                for v in list {
                    s.push_str(&format!("\n  {v}"));
                }
            }
            s
        }
        ErrorKind::UnknownArgument => match (zh, single(ContextKind::InvalidArg)) {
            (true, Some(a)) => format!("未预期的参数 '{a}'"),
            (true, None) => "未预期的参数".to_string(),
            (false, Some(a)) => format!("unexpected argument '{a}' found"),
            (false, None) => "unexpected argument found".to_string(),
        },
        ErrorKind::InvalidSubcommand => match (zh, single(ContextKind::InvalidSubcommand)) {
            (true, Some(s)) => format!("未识别的子命令 '{s}'"),
            (true, None) => "未识别的子命令".to_string(),
            (false, Some(s)) => format!("unrecognized subcommand '{s}'"),
            (false, None) => "unrecognized subcommand".to_string(),
        },
        ErrorKind::MissingSubcommand => if zh {
            "缺少子命令：请从可用子命令中选择一个"
        } else {
            "'roxid' requires a subcommand but one was not provided"
        }
        .to_string(),
        ErrorKind::ArgumentConflict => match (zh, single(ContextKind::InvalidArg)) {
            (true, Some(a)) => format!("参数 '{a}' 不能与其他互斥参数同时使用"),
            (true, None) => "参数不能与其他互斥参数同时使用".to_string(),
            (false, Some(a)) => format!("the argument '{a}' cannot be used with other conflicting arguments"),
            (false, None) => "arguments cannot be used together".to_string(),
        },
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => {
            let (a, v) = (
                single(ContextKind::InvalidArg),
                single(ContextKind::InvalidValue),
            );
            match (zh, a, v) {
                (true, Some(a), Some(v)) => format!("参数 '{a}' 的值 '{v}' 无效"),
                (true, _, _) => "参数值无效".to_string(),
                (false, Some(a), Some(v)) => format!("invalid value '{v}' for argument '{a}'"),
                (false, _, _) => "invalid value".to_string(),
            }
        }
        ErrorKind::TooManyValues => if zh {
            "提供的参数值数量超出上限"
        } else {
            "too many values were provided"
        }
        .to_string(),
        ErrorKind::TooFewValues => if zh {
            "提供的参数值数量不足"
        } else {
            "too few values were provided"
        }
        .to_string(),
        ErrorKind::WrongNumberOfValues => if zh {
            "参数值数量与要求不符"
        } else {
            "wrong number of values were provided"
        }
        .to_string(),
        ErrorKind::NoEquals => if zh {
            "该参数需要 --名称=值 的等号形态"
        } else {
            "equal sign is needed when assigning values to one of the arguments"
        }
        .to_string(),
        ErrorKind::InvalidUtf8 => if zh {
            "参数中检测到无效的 UTF-8 序列"
        } else {
            "invalid UTF-8 was detected in one or more arguments"
        }
        .to_string(),
        ErrorKind::Io => if zh { "标准流读写失败" } else { "I/O error" }.to_string(),
        ErrorKind::Format => if zh { "输出格式化失败" } else { "formatting error" }.to_string(),
        // Display* 三类不经本函数（help/version 正常输出路径）；ErrorKind
        // 标记 non_exhaustive——未来新增变体一并走英文兜底（clap 升级防炸）
        _ => return None,
    })
}

/// help 输出段落标题与 -h 说明中文化（M177，Q1 裁决 2026-09-12 03:02）：
/// Usage:/Options:/Arguments:/Commands: → 用法：/选项：/参数：/命令：；
/// clap 自动生成的 help flag 说明（惰性注入，mut_arg 声明期不可及）
/// 一并替换为中文。
///
/// 采用子串替换而非行首锚定：clap 标题以样式 span 原子写入（模板
/// `"{}Usage:{}"`），标题文本在 ANSI 转义序列内保持连续可命中；本项目
/// 双语表正文经核对不含四个英文标题词与 "Print help" 字样，无误替换面
///（单测锚定）。带尾注形态先替换、裸形态后兜底，避免二次误伤。
///
/// - 参数 rendered：clap 渲染出的 help 文本（可含 ANSI 转义）
/// - 返回：标题替换后的文本
pub fn localize_help_headings(rendered: &str) -> String {
    rendered
        .replace("Usage:", "用法：")
        .replace("Options:", "选项：")
        .replace("Arguments:", "参数：")
        .replace("Commands:", "命令：")
        // help flag 说明：--help 长形态（带短概览尾注）→ -h 短形态（带长
        // 详情尾注）→ 裸 "Print help"（-h 短输出 / help()==long_help 场景）
        .replace(
            "Print help (see a summary with '-h')",
            "打印帮助（概览见 '-h'）",
        )
        .replace(
            "Print help (see more with '--help')",
            "打印帮助（完整内容见 '--help'）",
        )
        .replace("Print help", "打印帮助")
        // clap 自动生成的 help 子命令条目（二级命令列表尾部）
        .replace(
            "Print this message or the help of the given subcommand(s)",
            "打印本消息或指定子命令的帮助",
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// locale 判定分支：zh 前缀各变体中文；C/POSIX/空/其他语言英文
    #[test]
    fn locale_branches() {
        assert_eq!(lang_from_locale("zh_CN.UTF-8"), Lang::Zh);
        assert_eq!(lang_from_locale("zh_TW"), Lang::Zh);
        assert_eq!(lang_from_locale("zh"), Lang::Zh);
        assert_eq!(lang_from_locale("ZH_HK.UTF-8"), Lang::Zh);
        assert_eq!(lang_from_locale("C"), Lang::En);
        assert_eq!(lang_from_locale("POSIX"), Lang::En);
        assert_eq!(lang_from_locale(""), Lang::En);
        assert_eq!(lang_from_locale("en_US.UTF-8"), Lang::En);
        assert_eq!(lang_from_locale("ja_JP.UTF-8"), Lang::En);
    }

    /// M176：错误渲染双语锚定——缺必选参数（用户报告场景）zh/en 双语
    /// 关键词、缩进上下文、用法行前缀与提示行全覆盖
    #[test]
    fn error_render_missing_required_both_languages() {
        use clap::CommandFactory;
        let e = crate::Cli::command()
            .try_get_matches_from(["roxid", "create"])
            .unwrap_err();
        let zh = render_clap_error(&e, Lang::Zh);
        assert!(zh.contains("以下必选参数未提供："), "zh 缺主文案：{zh}");
        assert!(zh.contains("\n  <MODEL>"), "zh 缺参数上下文：{zh}");
        assert!(zh.contains("用法："), "zh 缺用法行前缀：{zh}");
        assert!(zh.contains("--help"), "zh 缺提示行：{zh}");
        let en = render_clap_error(&e, Lang::En);
        assert!(
            en.contains("the following required arguments were not provided:"),
            "en 缺主文案：{en}"
        );
        assert!(en.contains("Usage:"), "en 缺用法行前缀：{en}");
    }

    /// M176：未知参数与未识别子命令的双语渲染锚定
    #[test]
    fn error_render_unknown_and_invalid_subcommand() {
        use clap::CommandFactory;
        let e = crate::Cli::command()
            .try_get_matches_from(["roxid", "create", "m", "--foo"])
            .unwrap_err();
        assert_eq!(e.kind(), clap::error::ErrorKind::UnknownArgument);
        let zh = render_clap_error(&e, Lang::Zh);
        assert!(zh.contains("未预期的参数 '--foo'"), "{zh}");
        let en = render_clap_error(&e, Lang::En);
        assert!(en.contains("unexpected argument '--foo' found"), "{en}");

        let e = crate::Cli::command()
            .try_get_matches_from(["roxid", "frobnicate"])
            .unwrap_err();
        assert_eq!(e.kind(), clap::error::ErrorKind::InvalidSubcommand);
        let zh = render_clap_error(&e, Lang::Zh);
        assert!(zh.contains("未识别的子命令 'frobnicate'"), "{zh}");
    }

    /// M177：help 段落标题与 -h 说明替换锚定——四标题、help flag 三形态
    #[test]
    fn help_headings_localization() {
        let src = "Usage: roxid create [OPTIONS] <MODEL>\nOptions:\n  -h, --help\nArguments:\nCommands:";
        let zh = localize_help_headings(src);
        assert!(zh.starts_with("用法： roxid create"), "{zh}");
        assert!(zh.contains("选项：\n"), "{zh}");
        assert!(zh.contains("参数：\n"), "{zh}");
        assert!(zh.ends_with("命令："), "{zh}");
        // help flag 说明三形态：长形态尾注 / 短形态尾注 / 裸形态
        assert_eq!(
            localize_help_headings("Print help (see a summary with '-h')"),
            "打印帮助（概览见 '-h'）"
        );
        assert_eq!(
            localize_help_headings("Print help (see more with '--help')"),
            "打印帮助（完整内容见 '--help'）"
        );
        assert_eq!(localize_help_headings("Print help"), "打印帮助");
        // clap 自动生成的 help 子命令条目
        assert_eq!(
            localize_help_headings(
                "Print this message or the help of the given subcommand(s)"
            ),
            "打印本消息或指定子命令的帮助"
        );
        // 正文行内的同形词（非标题语境）同样被替换——经核对双语表正文
        // 不含四个英文标题词与 "Print help" 字样，此为已接受设计边界
        assert!(!localize_help_headings("打印帮助").contains("用法"));
    }

    /// M178：命令树全量 next_line_help 布局断言（Q2-A 裁决）——
    /// 每个非隐藏命令的参数说明都必须是「独立成行」形态
    #[test]
    fn next_line_help_all_set() {
        use clap::CommandFactory;
        let mut cmd = crate::Cli::command();
        apply_localized_help(&mut cmd);
        assert_next_line(&mut cmd);
    }

    /// 递归断言单命令及全部非隐藏子命令的 next_line_help 设置
    fn assert_next_line(cmd: &mut clap::Command) {
        assert!(
            cmd.is_next_line_help_set(),
            "命令 {} 未设独立成行布局",
            cmd.get_name()
        );
        for sub in cmd.get_subcommands_mut() {
            if sub.is_hide_set() {
                continue; // __complete 等内部命令不进 help 体系
            }
            assert_next_line(sub);
        }
    }

    /// 文本表完整性：树上每个非隐藏子命令（含二级）双语条目都必须存在且
    /// about/after 非空——命令树漂移时此断言即失败，防止漏翻
    #[test]
    fn every_visible_command_has_both_languages() {
        use clap::CommandFactory;
        let mut cmd = crate::Cli::command();
        // 收集路径后对 zh/en 两表逐路径断言
        let mut paths = Vec::new();
        collect_paths(&mut cmd, "roxid", &mut paths);
        assert!(!paths.is_empty());
        for p in &paths {
            let zh = table(Lang::Zh, p).unwrap_or_else(|| panic!("zh 帮助条目缺失：{p}"));
            let en = table(Lang::En, p).unwrap_or_else(|| panic!("en 帮助条目缺失：{p}"));
            assert!(
                !zh.about.is_empty() && !zh.after.is_empty(),
                "zh 空文本：{p}"
            );
            assert!(
                !en.about.is_empty() && !en.after.is_empty(),
                "en 空文本：{p}"
            );
        }
    }

    /// 参数 id 表与命令树对齐：每个表条目的 arg id 必须真实存在——
    /// 防拼写漂移被注入防御逻辑静默吞掉（derive 字段改名即触发本断言）
    #[test]
    fn arg_ids_match_command_tree() {
        use clap::CommandFactory;
        let mut root = crate::Cli::command();
        assert_arg_ids(&mut root, "roxid");
    }

    /// 递归校验单命令及全部子命令的 args 表条目
    fn assert_arg_ids(cmd: &mut clap::Command, path: &str) {
        for lang in [Lang::Zh, Lang::En] {
            if let Some(h) = table(lang, path) {
                for (id, _) in h.args {
                    assert!(
                        cmd.get_arguments().any(|a| a.get_id() == id),
                        "arg id `{id}` 不在命令 {path} 的参数树中（检查 derive 字段名）"
                    );
                }
            }
        }
        for sub in cmd.get_subcommands_mut() {
            if sub.is_hide_set() {
                continue;
            }
            assert_arg_ids(sub, &format!("{path} {}", sub.get_name()));
        }
    }

    /// 收集全部非隐藏命令路径（顶层 + 一级 + 二级）
    fn collect_paths(cmd: &mut clap::Command, path: &str, out: &mut Vec<String>) {
        out.push(path.to_string());
        for sub in cmd.get_subcommands_mut() {
            if sub.is_hide_set() {
                continue; // __complete 等内部命令不进 help 体系
            }
            collect_paths(sub, &format!("{path} {}", sub.get_name()), out);
        }
    }
}
