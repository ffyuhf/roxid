//! roxid CLI：19 命令形态（15 官方对齐子命令 + setup --llama-url 参数模式
//! M31 + runtime 多版本管理族 M37 + completion 补全族 M57 + __complete
//! 内部命令 M56；官方命令集以原版 ollama --help 为准，来源：用户确认
//! 2026-08-24 18:23 Q2；runtime/completion 为 roxid 扩展，迭代16 Q1-B
//! 裁决 2026-09-09 04:20 / 迭代20 裁决 2026-09-09 21:30）。
//!
//! 架构：CLI 经 HTTP 与本机 serve（127.0.0.1:11434）通信（与原版一致）；
//! serve 未运行时给出明确提示。
//!
//! 修改历史：M1 版本骨架 2026-08-24 18:35；M12 全量实装 2026-08-24 19:38；
//! M17 令牌隐蔽输入（rpassword，用户裁决 R2-2A 2026-08-24 19:59）2026-08-24 20-22；
//! M24 初次运行引导与 setup 子命令（迭代4 裁决 Q1–Q5）2026-08-26 05-31；
//! M25 移除未使用变量 full（迭代5 M12 遗留警告清偿）2026-08-26 05:43;
//! M27 REPL assistant 回填（迭代7 P0碴3）：print_assistant_stream 返回拼接文本，
//! cmd_run 每轮回填消息表——多轮对话上下文完整（仅 content，对齐原版）2026-08-26 21-15
//! M28 碴6/碴3 Client 治理（迭代8）：移除 3600s 总超时（pull 大模型进度流
//! 与长生成被客户端侧掐断，服务端 M22/M27 已保证不掐长流）+ OnceLock
//! 进程内共享 + connect_timeout 10s 2026-08-30 02-12
//! M29 碴10（迭代9，R3-A）：REPL thinking 增量以 ANSI dim 暗色渲染
//!（原实现完全静默，与 M27「thinking 仅终端展示不回填」注释的展示半句
//! 不符）；回填语义不变（仅 content）2026-09-05 13-05
//! M30 两碴（迭代10）：碴2 show 非 2xx 显式报错退出（原解析 404 错误体
//! 后各字段 null 打印 "Model: ?" 吞错）、碴13 signin 凭据文件收紧 0600
//!（原默认 0644 可读）2026-09-06 22-12
//! M32 碴5（迭代12）：头注释命令计数对齐——原「15 命令」与架构总览
//! 「16 命令」表述矛盾，统一为「16 命令形态 = 15 子命令 + setup 参数
//! 模式」2026-09-07 01-05
//! M33 两碴（迭代13）：碴1 create 改读 NDJSON 流按 error/success 事件
//! 定退出码（原仅判 HTTP 状态，流式端点恒 200 使失败误报「已创建」）、
//! 碴8 run 首次 chat 404 自动拉取后重试一次（对齐原版按需拉取语义；
//! NDJSON 消费内核抽 consume_pull_stream 复用）2026-09-07 01-55
//! M34 两项（迭代14 BUG-4/O-1）：BUG-4 实例身份探测警告（GET /props 特征：
//! roxid 缺 ?model 恒 400、官方 Ollama 无路由 404；OnceLock 缓存，非 roxid
//! 时 stderr 警告，stdout 保持官方对齐；方案 A 用户裁决 2026-09-07 02:34）；
//! O-1 pull 进度双路径渲染（TTY spinner 带百分比；非 TTY 管道/重定向下
//! indicatif 隐藏——降级周期文本行，对齐官方 Go CLI 非 TTY 打印行为）
//! 2026-09-07 19-45
//! M37（迭代16 Q1-B）：runtime 子命令族（list/install/use/rm）——
//! llama.cpp 后端多版本下载与默认版本切换（config default_version 持久化）
//! 2026-09-09 04-38
//! M56–M58（迭代20）：help locale 双语（LC_ALL→LC_MESSAGES→LANG，zh*
//! 中文/其余英文，裁决 2026-09-09 21:28）、__complete 候选统一源与
//! completion 子命令族（bash/zsh/fish shim + install 自动落位，裁决
//! 2026-09-09 21:30）2026-09-09 21-55
//! M90（迭代28）：list MODIFIED 列人类可读混合格式（绝对时间 + 中文
//! 相对短语；RFC3339 双形态解析：roxid Z 秒级 / 官方纳秒偏移；格式与
//! 短语中文化均为用户裁决 2026-09-10 04:36/04:41）2026-09-10 04-42
//! M92+M94（迭代29）：M92 进度 spinner 补 enable_steady_tick（indicatif
//! {spinner} 字符由 tick 计数驱动，set_message 只重绘不推进 tick，原
//! 永停首字符 ⠁ 形似卡死）；M94 run 启动预检——POST /api/show 判 404，
//! 未安装先拉取（进度可见）完成后再进输入框（时序对齐官方，用户裁决
//! 2026-09-10 06:34）；仅 404 触发预拉取，其余放行走既有 chat 报错路径
//! 2026-09-10 06-42
//! M108–M118（迭代33，用户批准 2026-09-10 22:33）：CLI 交互体验碴清偿——
//! M108 手写 ANSI 颜色层（NO_COLOR/TERM=dumb/非 TTY 三重门控，stdout
//! 与 stderr 独立判定）；M109 终端宽度基建（COLUMNS 优先 + terminal_size，
//! 进度帧/list 列宽 CJK 感知截断，原恒 82 字符窄终端折行残留）；M110
//! 进度阶段区分（校验/重试黄色 spinner、层完成绿✓、status 中文映射，
//! 协议英文原文不变）；M111 非 TTY 每阶段一行（用户裁决 2026-09-10
//! 22:25 对齐官方）；M112 list/ps 列宽自适应 + SIZE 量纲自适应；M113
//! show Model 段键值化（消除双 Parameters 标签）；M114 REPL 横幅/提示符
//! 着色 + 回复后空行 + 半角括号；M115 verbose 补总耗时与输出速率；
//! M117 错误整形（{"error":...} 提取 + 统一红色「错误：」前缀）+ cp/rm/
//! stop 成功回显；M118 verbose/nowordwrap 标 global（子命令后可用）
//! 2026-09-10 22-45
//! M146–M148（迭代41，用户终裁 2026-09-11 09:26「全屏 TUI」/计划批准
//! 09:30）：create -i 交互向导升级为 ratatui 全屏 TUI（四步：列表选基础
//! 模型→SYSTEM 多行→RUNTIME 单行→预览确认；新依赖 ratatui 0.30 +
//! crossterm 0.29，版本经 USTC 镜像索引核实；终端初始化失败降级既有
//! 问答向导）2026-09-11 09-38

mod complete;
mod completion;
mod create_tui;
mod help_i18n;
mod setup;

use std::io::Write;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::{json, Value};

/// roxid 服务地址（兼容原版 OLLAMA_HOST 变量，便于现有脚本无缝切换）
fn base_url() -> String {
    std::env::var("ROXID_HOST")
        .or_else(|_| std::env::var("OLLAMA_HOST"))
        .unwrap_or_else(|_| "http://127.0.0.1:11434".into())
}

/// HTTP 客户端（进程内共享）。
/// M28 碴6：移除 3600s 总超时——长拉取/长生成由服务端流式语义驱动，
/// 客户端总超时只会在中途掐断进度流（原版 ollama CLI 无此硬超时）。
/// M28 碴3：OnceLock 共享 + connect_timeout 10s 兜底建连失败。
fn http() -> reqwest::Client {
    static SHARED: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("构建 CLI 客户端失败")
        })
        .clone()
}

/// 服务不可达的统一提示（M108：红色错误行——正常与失败结果同色不可辨）
fn conn_hint(e: reqwest::Error) -> ! {
    print_error(&format!("无法连接 roxid 服务（{}）：{e}", base_url()));
    eprintln!("请先运行：roxid serve");
    std::process::exit(1);
}

/// M34 BUG-4：连接实例身份探测与警告（用户裁决 2026-09-07 02:34 方案 A：
/// 徽标+探测组合；-v 版本输出自带 roxid 徽标即 clap name）。
/// 特征端点 GET /props：roxid 路由存在且缺 ?model= 恒回 400（query_model
/// 校验先于 acquire，不拉起实例零副作用）；官方 Ollama 无此路由回 404。
/// 仅 404 判非 roxid（不可达/5xx 等跳过探测不误报，后续命令自身报错）；
/// 结果进程内 OnceLock 缓存。警告走 stderr——stdout 保持官方对齐输出，
/// 不污染脚本管道解析；base_url 兼容 OLLAMA_HOST 的无缝切换设计维持。
async fn warn_if_foreign_instance() {
    static VERDICT: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    if VERDICT.get().is_some() {
        // 已探测：Some(true)=非 roxid 实例，每次命令提醒；其余不再动作
        if VERDICT.get() == Some(&Some(true)) {
            eprintln!(
                "⚠ 注意：当前连接 {} 不是 roxid 实例（可能是官方 Ollama），本命令将作用于该实例",
                base_url()
            );
        }
        return;
    }
    // 2s 探测上限：本地回环足够，挂起场景不拖慢命令首跳
    let verdict = match http()
        .get(format!("{}/props", base_url()))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(resp) => Some(resp.status() == reqwest::StatusCode::NOT_FOUND),
        Err(_) => None, // 探测失败（服务未起等）：跳过不误报
    };
    let _ = VERDICT.set(verdict);
    if verdict == Some(true) {
        eprintln!(
            "⚠ 注意：当前连接 {} 不是 roxid 实例（可能是官方 Ollama），后续命令将作用于该实例",
            base_url()
        );
        eprintln!("  如需操作 roxid：先运行 roxid serve（默认 127.0.0.1:11434），或用 ROXID_HOST 指定地址");
    }
}

/// 命令行参数结构
#[derive(Parser)]
#[command(
    name = "roxid",
    version = roxid_server::config::ROXID_VERSION,
    disable_version_flag = true,
    about = "Large language model runner"
)]
struct Cli {
    /// Don't wrap words to the next line automatically
    // M118（迭代33 碴11）：global = true——全局选项可出现在子命令之后
    //（原仅能前置，`roxid run --verbose` 报 unexpected argument）
    #[arg(long, global = true)]
    nowordwrap: bool,
    /// Show timings for response
    #[arg(long, global = true)]
    verbose: bool,
    /// Show version information
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    _version: (),
    #[command(subcommand)]
    command: Commands,
}

/// 子命令树（与原版 ollama --help 逐行对齐）
#[derive(Subcommand)]
enum Commands {
    /// Start roxid
    Serve {
        /// 监听地址（默认 127.0.0.1:11434）
        #[arg(long)]
        addr: Option<String>,
    },
    /// Create a model
    Create {
        /// 新模型名
        model: String,
        /// Modelfile 路径（缺省时配合 -i 进入交互创建）
        #[arg(short = 'f', long = "file")]
        file: Option<String>,
        /// 交互式创建向导（M40，迭代16 Q2-B 裁决：显式开关保持官方一致）
        #[arg(short = 'i', long = "interactive")]
        interactive: bool,
    },
    /// Show information for a model
    Show { model: String },
    /// Run a model
    Run {
        /// 模型名
        model: String,
        /// 首条提示（缺省进入交互 REPL）
        prompt: Vec<String>,
        /// 模型名按 HuggingFace 直引解析：{user}/{repo}:{quant}
        ///（M31 F3：等价于 hf.co/ 前缀写法，quant 直接指定量化格式）
        #[arg(long = "hf")]
        hf: bool,
        /// 本次运行的 llama.cpp 启动参数（M41：临时覆盖模型 RUNTIME，不落盘；
        /// 变化时服务端自动重建实例）
        /// 迭代18 BUG-12（M48）：llama.cpp 参数恒以 -- 开头，clap 默认拒绝
        /// 该形态的选项值（ErrorKind::UnknownArgument，官方文档实证）——
        /// allow_hyphen_values 放行，空格写法 `--runtime "--threads 3"` 可用
        #[arg(long = "runtime", allow_hyphen_values = true)]
        runtime: Option<String>,
    },
    /// Stop a running model
    Stop { model: String },
    /// Pull a model from a registry
    Pull {
        /// 模型名
        model: String,
        /// 模型名按 HuggingFace 直引解析：{user}/{repo}:{quant}
        ///（M31 F3：等价于 hf.co/ 前缀写法）
        #[arg(long = "hf")]
        hf: bool,
    },
    /// Push a model to a registry
    Push { model: String },
    /// Sign in to ollama.com
    Signin,
    /// Sign out from ollama.com
    Signout,
    /// List models
    #[command(alias = "ls")]
    List,
    /// List running models
    Ps,
    /// Copy a model
    Cp { source: String, destination: String },
    /// Remove a model
    Rm { model: String },
    /// Launch the roxid menu or an integration
    Launch,
    /// Configure first-run settings (download proxy)
    Setup {
        /// 手动指定 llama.cpp 包下载链接并立即安装（M31 F2：
        /// tar.gz 或裸 llama-server 二进制；安装后走 manual 优先链）
        #[arg(long = "llama-url")]
        llama_url: Option<String>,
    },
    /// Manage llama.cpp runtime versions（M37：roxid 扩展，多版本管理）
    Runtime {
        #[command(subcommand)]
        cmd: RuntimeCmd,
    },
    /// Manage shell completion（M58：roxid 扩展，迭代20）
    Completion {
        #[command(subcommand)]
        cmd: completion::CompletionCmd,
    },
    /// 补全候选输出（M57：shell shim 内部调用，隐藏命令不进 help 体系）
    #[command(name = "__complete", hide = true)]
    Complete {
        /// 光标上下文 token（到当前词为止；可能以 - 开头，故放行连字值）
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        words: Vec<String>,
    },
}

/// runtime 子命令族（M37，迭代16 Q1-B 裁决：独立命令族而非 setup 参数）
#[derive(Subcommand)]
enum RuntimeCmd {
    /// List installed runtime versions
    List,
    /// Install a runtime version by tag (e.g. b10700) or a custom URL
    Install {
        /// 版本 tag（b\d+ 形态，如 b10700）
        tag: Option<String>,
        /// 自定义包链接（tar.gz 或裸 llama-server；等价 setup --llama-url，
        /// 落位 manual 目录）
        #[arg(long = "url")]
        url: Option<String>,
    },
    /// Set the default runtime version (tag or "manual")
    Use { tag: String },
    /// Remove an installed runtime version
    Rm { tag: String },
}

/// argv 预扫描（M31 F3，R4 裁决）：把 run/pull 子命令后、model 位置前的
/// 字面 `-hf` 重写为 `--hf`——clap 将 `-hf` 拆解为 `-h -f` 无法直用，
/// 预扫描保证用户示例语法 `roxid run -hf user/repo:quant` 可用。
/// model 之后（prompt 首词）的 `-hf` 不受影响（非目标 token 零误改）。
///
/// - 参数 args：原始 argv（不含程序名）
/// - 返回：重写后的 argv
fn rewrite_hf_flag(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut expect_model = false; // 处于 run/pull 后、model 未出现的参数段
    for a in args {
        if a == "run" || a == "pull" {
            expect_model = true;
            out.push(a.clone());
            continue;
        }
        if expect_model {
            if a == "-hf" {
                out.push("--hf".to_string());
                continue;
            }
            // 首个非 flag 位置参数即 model，其后（prompt 段）不再重写
            if !a.starts_with('-') {
                expect_model = false;
            }
        }
        out.push(a.clone());
    }
    out
}

/// hf flag → hf.co/ 前缀形态（M31 F3，Q3 裁决：注册名 hf.co/{repo}:{quant}）：
/// `run -hf user/repo:quant` 与 `run hf.co/user/repo:quant` 等价；
/// 已有前缀不重复添加。
///
/// - 参数 model：用户输入模型名
/// - 参数 hf：是否经 -hf/--hf 指定 HF 源
/// - 返回：规整后的模型名（发往 /api/pull、/api/chat 的最终形态）
fn normalize_hf_arg(model: String, hf: bool) -> String {
    if hf && !model.starts_with("hf.co/") {
        format!("hf.co/{model}")
    } else {
        model
    }
}

#[tokio::main]
async fn main() {
    // parse_from 首元素是程序名位：保留真实 argv[0]，其余经 -hf 预扫描重写
    let mut argv: Vec<String> = std::env::args().collect();
    let rewritten = rewrite_hf_flag(&argv[1..]);
    argv.truncate(1);
    argv.extend(rewritten);
    // 迭代20 M56：构建后、解析前注入 locale 双语 help（zh* → 中文，
    // 其余 → 英文兜底 derive doc comment；命令树结构零改动）
    let mut command = Cli::command();
    help_i18n::apply_localized_help(&mut command);
    let cli = Cli::from_arg_matches(
        &command
            .try_get_matches_from(argv)
            .unwrap_or_else(|e| e.exit()),
    )
    .unwrap_or_else(|e| e.exit());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    // 联网命令首次运行触发引导（Q4 裁决 2026-08-26 05:25）：
    // serve 拉起运行时下载、pull/create 触发模型拉取，均为代理生效场景；
    // M37：runtime install 下载官方包同为联网场景，一并触发
    if matches!(
        cli.command,
        Commands::Serve { .. } | Commands::Pull { .. } | Commands::Create { .. }
    ) || matches!(
        cli.command,
        Commands::Runtime {
            cmd: RuntimeCmd::Install { .. },
        }
    ) {
        setup::maybe_run_first_use_wizard().await;
    }
    // M34 BUG-4：访问服务端的命令统一做实例身份探测（serve 自身即服务、
    // signin/signout/setup/runtime 为本地操作、launch 仅打印提示——均排除；
    // runtime 全族本地文件/配置操作不经 serve，M37 加入排除）
    if !matches!(
        cli.command,
        Commands::Serve { .. }
            | Commands::Signin
            | Commands::Signout
            | Commands::Setup { .. }
            | Commands::Launch
            | Commands::Runtime { .. }
            | Commands::Completion { .. }
            | Commands::Complete { .. }
    ) {
        warn_if_foreign_instance().await;
    }
    let code = match cli.command {
        Commands::Serve { addr } => cmd_serve(addr).await,
        Commands::Create {
            model,
            file,
            interactive,
        } => cmd_create(&model, file, interactive).await,
        Commands::Show { model } => cmd_show(&model).await,
        Commands::Run {
            model,
            prompt,
            hf,
            runtime,
        } => {
            let model = normalize_hf_arg(model, hf);
            cmd_run(&model, prompt.join(" "), cli.verbose, runtime).await
        }
        Commands::Stop { model } => {
            cmd_simple_post("/api/stop", &model, &format!("已停止 {model}")).await
        }
        Commands::Pull { model, hf } => {
            let model = normalize_hf_arg(model, hf);
            cmd_pull(&model).await
        }
        Commands::Push { model } => {
            cmd_simple_post("/api/push", &model, &format!("已推送 {model}")).await
        }
        Commands::Signin => cmd_signin().await,
        Commands::Signout => cmd_signout().await,
        Commands::List => cmd_list().await,
        Commands::Ps => cmd_ps().await,
        Commands::Cp {
            source,
            destination,
        } => cmd_cp(&source, &destination).await,
        Commands::Rm { model } => cmd_rm(&model).await,
        Commands::Launch => {
            eprintln!("launch：菜单集成在 roxid 后续版本提供（当前可用命令见 roxid --help）");
            0
        }
        Commands::Setup { llama_url } => setup::run_setup_command(llama_url).await,
        Commands::Runtime { cmd } => cmd_runtime(cmd).await,
        // 迭代20：补全族本地操作；迭代36 M125：stop 值位候选经 /api/ps（1s 超时）
        Commands::Completion { cmd } => completion::cmd_completion(cmd),
        Commands::Complete { words } => {
            complete::complete(&words).await;
            0
        }
    };
    std::process::exit(code);
}

/// M37：llama.cpp 运行时多版本管理（list/install/use/rm；本地操作，不经 serve）
///
/// - 参数 cmd：runtime 子命令
/// - 返回：进程退出码
async fn cmd_runtime(cmd: RuntimeCmd) -> i32 {
    use roxid_server::runtime as rt;
    match cmd {
        RuntimeCmd::List => {
            let default = roxid_server::config::load_persist_config()
                .runtime
                .default_version;
            let installed = rt::list_installed();
            let manual_ok = rt::manual_server_path().is_file();
            if installed.is_empty() && !manual_ok {
                println!(
                    "尚未安装任何 llama.cpp 运行时（serve 首次启动将自动下载 {}/{{vulkan|cpu}} 锁定链）",
                    rt::LOCKED_LLAMA_CPP_TAG
                );
                return 0;
            }
            println!(
                "已安装 llama.cpp 运行时（解析优先级：env → manual → 默认版本 → {} 锁定链）：",
                rt::LOCKED_LLAMA_CPP_TAG
            );
            if manual_ok {
                let mark = if default.as_deref() == Some("manual") {
                    "  [默认]"
                } else {
                    ""
                };
                println!("  manual  手动版本（setup --llama-url 或 runtime install --url）{mark}");
            }
            for (tag, variants) in installed {
                let mark = if default.as_deref() == Some(tag.as_str()) {
                    "  [默认]"
                } else {
                    ""
                };
                // M99（迭代31，Q3 校验 3，用户裁决 2026-09-10 18:17）：
                // 变体与宿主架构不匹配时标注警示（arm64 机器上历史误装的
                // ubuntu-x64 即此形态；ELF 无法判定时不标注，宽容）
                let rendered: Vec<String> = variants
                    .iter()
                    .map(|v| {
                        if rt::installed_variant_arch_mismatch(&tag, v) {
                            format!("{v}（与宿主架构不匹配）")
                        } else {
                            v.clone()
                        }
                    })
                    .collect();
                println!("  {tag}  {}{mark}", rendered.join(", "));
            }
            if !matches!(default.as_deref(), Some("manual")) {
                println!("（`roxid runtime use <tag|manual>` 切换默认版本）");
            }
            0
        }
        RuntimeCmd::Install { tag, url } => {
            if let Some(url) = url {
                if tag.is_some() {
                    eprintln!("--url 与位置参数 tag 不可同时使用");
                    return 1;
                }
                // M105（迭代32 碴6a）：下载进度可见（量纲/速度/剩余时间）
                let (bar, on_progress) = runtime_download_progress("下载运行时");
                let result = rt::install_manual(&url, on_progress).await;
                bar.finish_and_clear();
                return match result {
                    Ok(path) => {
                        println!("manual 版本已安装：{}", path.display());
                        println!("提示：运行 `roxid runtime use manual` 将其设为默认");
                        0
                    }
                    Err(e) => {
                        eprintln!("manual 安装失败：{e}");
                        1
                    }
                };
            }
            let Some(tag) = tag else {
                eprintln!("用法：roxid runtime install <tag> 或 --url <url>");
                return 1;
            };
            if !rt::is_valid_tag(&tag) {
                eprintln!("非法 tag 形态：{tag}（期望 b\\d+，如 b10700）");
                return 1;
            }
            // M99（迭代31）：变体名含宿主架构片段（Q1 动态探测 2026-09-10
            // 18:15）；未知架构显式报错引导手动链，不再误下 x64 包
            let variant = match rt::detect_backend().asset_variant() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            println!(
                "探测后端变体：{variant}（宿主 {}；GPU → vulkan / 无 GPU → cpu）",
                rt::host_arch_fragment().unwrap_or("未知")
            );
            // M105（迭代32 碴6a）：下载进度可见（量纲/速度/剩余时间）
            let (bar, on_progress) = runtime_download_progress("下载运行时");
            let result = rt::install_version(&tag, &variant, on_progress).await;
            bar.finish_and_clear();
            match result {
                Ok(path) => {
                    println!("已安装 {tag}（{variant}）：{}", path.display());
                    let cur = roxid_server::config::load_persist_config()
                        .runtime
                        .default_version;
                    if cur.as_deref() != Some(tag.as_str()) {
                        println!("提示：运行 `roxid runtime use {tag}` 将其设为默认版本");
                    }
                    0
                }
                Err(e) => {
                    eprintln!(
                        "安装 {tag} 失败：{e}（tag 不存在或网络问题；可用 ROXID_GH_PROXY 加速）"
                    );
                    1
                }
            }
        }
        RuntimeCmd::Use { tag } => {
            if tag == "manual" {
                if !rt::manual_server_path().is_file() {
                    eprintln!("manual 版本未安装（先 roxid setup --llama-url <url> 安装）");
                    return 1;
                }
            } else if !rt::is_valid_tag(&tag) {
                eprintln!("非法 tag 形态：{tag}（期望 b\\d+ 或 manual）");
                return 1;
            } else if !rt::list_installed().iter().any(|(t, _)| t == &tag) {
                eprintln!("版本 {tag} 未安装（先 roxid runtime install {tag}）");
                return 1;
            }
            let mut cfg = roxid_server::config::load_persist_config();
            cfg.runtime.default_version = Some(tag.clone());
            match roxid_server::config::save_persist_config(&cfg) {
                Ok(()) => {
                    println!("默认后端版本已设为 {tag}");
                    0
                }
                Err(e) => {
                    eprintln!("写入配置失败：{e}");
                    1
                }
            }
        }
        RuntimeCmd::Rm { tag } => {
            let cur = roxid_server::config::load_persist_config()
                .runtime
                .default_version;
            if cur.as_deref() == Some(tag.as_str()) {
                eprintln!("版本 {tag} 是当前默认版本，删除前请先 `roxid runtime use <其他版本|manual>` 切换");
                return 1;
            }
            match rt::remove_version(&tag) {
                Ok(()) => {
                    println!("已删除 {tag}");
                    0
                }
                Err(e) => {
                    eprintln!("删除失败：{e}");
                    1
                }
            }
        }
    }
}

/// M31 F3（R4）：-hf 字面短选项预扫描——run/pull 的 model 位前重写、
/// prompt 段零误改、--hf 直用、非目标子命令不动
#[cfg(test)]
mod hf_flag_tests {
    use super::{normalize_hf_arg, rewrite_hf_flag};

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// run/pull 的 -hf 重写为 --hf；model 后（prompt 段）不误改
    #[test]
    fn rewrite_targets_run_and_pull_only() {
        assert_eq!(
            rewrite_hf_flag(&v(&["run", "-hf", "user/repo:IQ2_S"])),
            v(&["run", "--hf", "user/repo:IQ2_S"])
        );
        assert_eq!(
            rewrite_hf_flag(&v(&["pull", "-hf", "user/repo:IQ2_S"])),
            v(&["pull", "--hf", "user/repo:IQ2_S"])
        );
        // 全局 flag 在子命令前：重写不受影响
        assert_eq!(
            rewrite_hf_flag(&v(&["--verbose", "run", "-hf", "m:Q4"])),
            v(&["--verbose", "run", "--hf", "m:Q4"])
        );
        // prompt 首词恰为 -hf（model 之后）：不重写
        assert_eq!(
            rewrite_hf_flag(&v(&["run", "model", "-hf", "word"])),
            v(&["run", "model", "-hf", "word"])
        );
        // 其他子命令的 -hf 不动
        assert_eq!(rewrite_hf_flag(&v(&["show", "-hf"])), v(&["show", "-hf"]));
        // --hf 直用：原样通过
        assert_eq!(
            rewrite_hf_flag(&v(&["run", "--hf", "user/repo"])),
            v(&["run", "--hf", "user/repo"])
        );
    }

    /// hf flag → hf.co/ 前缀规整：补前缀、不重复补、flag 关闭时原样
    #[test]
    fn normalize_adds_prefix_once() {
        assert_eq!(
            normalize_hf_arg("user/repo:IQ2_S".into(), true),
            "hf.co/user/repo:IQ2_S"
        );
        assert_eq!(
            normalize_hf_arg("hf.co/user/repo:IQ2_S".into(), true),
            "hf.co/user/repo:IQ2_S",
            "已有前缀不得重复添加"
        );
        assert_eq!(
            normalize_hf_arg("llama3.2:3b".into(), false),
            "llama3.2:3b",
            "flag 关闭时原样透传"
        );
    }
}

/// serve：启动 API 服务器（前台阻塞）
async fn cmd_serve(addr: Option<String>) -> i32 {
    let bind: std::net::SocketAddr = addr
        .unwrap_or_else(|| roxid_server::config::DEFAULT_BIND_ADDR.into())
        .parse()
        .unwrap_or_else(|_| roxid_server::config::DEFAULT_BIND_ADDR.parse().unwrap());
    match roxid_server::api::serve_main(bind).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("serve 退出：{e}");
            1
        }
    }
}

/// create：Modelfile 创建模型
/// M33 碴1：/api/create 为 NDJSON 流式端点（HTTP 恒 200，失败经流内
/// error 事件传递）——原仅判 HTTP 状态，Modelfile 解析失败/FROM 未安装
/// 时误报「已创建」退出码 0；改读流按 error/success 事件定码（cmd_pull
/// 消费先例）
/// M40（迭代16 Q2-B 裁决 2026-09-09 04:25）：-f 缺省 + -i 进入交互向导；
/// 两者皆无时报错（保持官方 ollama 必须给 Modelfile 的一致行为）
async fn cmd_create(model: &str, file: Option<String>, interactive: bool) -> i32 {
    let text = match &file {
        Some(f) => match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                print_error(&format!("无法读取 Modelfile {f}：{e}"));
                return 1;
            }
        },
        None => {
            if interactive {
                return create_interactive(model).await;
            }
            eprintln!("create 需要 -f <Modelfile>，或 -f 缺省时加 -i 进入交互创建");
            return 1;
        }
    };
    submit_create(model, &text).await
}

/// 提交 Modelfile 文本到 /api/create 并消费 NDJSON 结果（文件与向导共用后段）
///
/// - 参数 model：新模型名
/// - 参数 text：Modelfile 全文
/// - 返回：进程退出码
async fn submit_create(model: &str, text: &str) -> i32 {
    let resp = match http()
        .post(format!("{}/api/create", base_url()))
        .json(&json!({"model": model, "from": text}))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => conn_hint(e),
    };
    if !resp.status().is_success() {
        // M117：错误体整形（{"error":...} 提取纯文本）+ 统一红色前缀
        print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
        return 1;
    }
    match consume_ndjson_progress(resp).await {
        Ok(()) => {
            println!("已创建：{model}");
            0
        }
        Err(e) => {
            print_error(&e);
            1
        }
    }
}

/// 交互创建入口（M148，迭代41）：TUI 全屏向导优先，终端初始化失败
/// （TERM=dumb 等不支持 raw mode 的场景）降级既有逐行问答向导——
/// 双路径共存，-f 路径与提交流复用 submit_create 不变
///
/// - 参数 model：新模型名
/// - 返回：进程退出码
async fn create_interactive(model: &str) -> i32 {
    let models = complete::local_model_names();
    match create_tui::run(model, models) {
        Ok(Some(text)) => submit_create(model, &text).await,
        Ok(None) => {
            eprintln!("已取消");
            1
        }
        Err(e) => {
            eprintln!("全屏 TUI 不可用（{e}），降级为逐行问答向导");
            create_wizard(model).await
        }
    }
}

/// 交互式创建向导（M40）：FROM → SYSTEM（多行）→ RUNTIME → 摘要确认 →
/// 提交 /api/create。RUNTIME 步骤即「自定义设置增强」的简便入口
///（DeepSeek 类参数直接粘贴，回车跳过）。
///
/// - 参数 model：新模型名
/// - 返回：进程退出码
async fn create_wizard(model: &str) -> i32 {
    use std::io::{BufRead, IsTerminal};
    if !std::io::stdout().is_terminal() {
        eprintln!("交互创建需要终端；非交互环境请用 roxid create <model> -f <Modelfile>");
        return 1;
    }
    let mut stdin = std::io::BufReader::new(std::io::stdin());

    println!("=== 交互创建模型：{model} ===");
    println!("（可用基础模型可先运行 roxid list 查看）");
    // M51（迭代18 N-3）：EOF 任一步到达即礼貌终止（原实现把 EOF 空读当
    // 空行，管道/伪终端输入源关闭后向导挂起不退出）
    let Some(from) = ask_line(&mut stdin, "基础模型 (FROM)") else {
        eprintln!("输入已结束（EOF），向导终止");
        return 1;
    };
    if from.is_empty() {
        eprintln!("FROM 不可为空");
        return 1;
    }

    // SYSTEM 多行：单独一行 . 结束；首行直接回车跳过
    println!("系统提示 SYSTEM（多行，单独一行 . 结束；直接回车跳过）：");
    let mut system_lines: Vec<String> = Vec::new();
    loop {
        let mut line = String::new();
        // M51（迭代18 N-3）：EOF 视为输入结束，用已有内容收束——原实现
        // 有内容后 EOF 空读被无限 push，死循环挂起
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed == "." {
            break;
        }
        if trimmed.is_empty() && system_lines.is_empty() {
            break; // 首行空 → 跳过 SYSTEM
        }
        system_lines.push(line.trim_end_matches('\n').to_string());
    }

    // RUNTIME（M40 自定义设置增强）：llama-server 原生启动参数
    println!("llama.cpp 启动参数 RUNTIME（回车跳过）：");
    println!("  示例：--flash-attn --override-tensor exps=CPU -ngl 30 --no-mmap --jinja");
    let runtime = ask_line(&mut stdin, "RUNTIME").unwrap_or_default();

    // 组装 Modelfile（与服务端 parse 往返兼容的形态）
    let mut text = format!("FROM {from}\n");
    if !system_lines.is_empty() {
        let body = system_lines.join("\n");
        text.push_str(&format!("SYSTEM \"\"\"{body}\"\"\"\n"));
    }
    if !runtime.is_empty() {
        text.push_str(&format!("RUNTIME {runtime}\n"));
    }

    println!("\n即将创建的 Modelfile：\n{text}");
    // M51（迭代18 N-3）：确认步 EOF 视为取消（保守不落盘）
    let confirm = ask_line(&mut stdin, "确认创建？(y/N)").unwrap_or_default();
    if !confirm.eq_ignore_ascii_case("y") {
        eprintln!("已取消");
        return 1;
    }
    submit_create(model, &text).await
}

/// 向导单行询问（M40 helper：避免闭包与多行循环的双重可变借用）
/// M51（迭代18 N-3）：EOF（read_line 返回 0 字节）返回 None——管道或
/// 伪终端输入源关闭后向导礼貌终止，不再把 EOF 空读当作空行
///
/// - 参数 stdin：标准输入读取器
/// - 参数 msg：提示语
/// - 返回：Some(去除首尾空白的输入行)；None 表示 stdin 已 EOF
fn ask_line(stdin: &mut std::io::BufReader<std::io::Stdin>, msg: &str) -> Option<String> {
    use std::io::{BufRead, Write};
    print!("{msg}: ");
    std::io::stdout().flush().ok();
    let mut s = String::new();
    match stdin.read_line(&mut s) {
        Ok(0) => None,
        _ => Some(s.trim().to_string()),
    }
}

/// show：模型详情
/// M30 碴2：非 2xx 显式报错并返回退出码 1——原实现直接 json 解析 404
/// 错误体，各字段 null 落 "?" 打印，模型不存在被吞（同文件 pull/cp/rm/
/// stop 均有状态检查；原版 ollama show 报错退出）
async fn cmd_show(model: &str) -> i32 {
    let resp = http()
        .post(format!("{}/api/show", base_url()))
        .json(&json!({"model": model}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if !resp.status().is_success() {
        print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
        return 1;
    }
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    // M113（迭代33 碴6）：对齐官方 Model 段键值化——原独立
    // `Parameters: {size} ({quant})` 行与下挂参数区标题「Parameters:」
    // 同名重复（两个标签分别表示参数量与 stop 等参数条目，语义混淆）；
    // 参数量/量化并入 Model 段键值，下挂区独占 Parameters 标题
    let sc = stdout_color();
    println!("{}", paint(sc, "1", "Model"));
    println!(
        "  {:<18} {}",
        "architecture",
        v["details"]["family"].as_str().unwrap_or("?")
    );
    println!(
        "  {:<18} {}",
        "parameters",
        v["details"]["parameter_size"].as_str().unwrap_or("?")
    );
    println!(
        "  {:<18} {}",
        "quantization",
        v["details"]["quantization_level"].as_str().unwrap_or("?")
    );
    if let Some(sys) = v["system"].as_str() {
        if !sys.is_empty() {
            println!("  {:<18} {sys}", "system");
        }
    }
    // 迭代18 BUG-11（M47）：补齐官方对齐三段——模型参数键值区 /
    // Capabilities / RUNTIME 行（M38 可见性承诺）。数据源 /api/show 的
    // parameters/capabilities/modelfile 字段检测实证齐全，纯渲染层缺失；
    // 空段不渲染（无参数模型零噪音）
    if let Some(ptext) = v["parameters"].as_str() {
        // roxid /api/show 的 parameters 为 Modelfile 指令文本形态（非官方
        // map 形态——协议层对齐属后续迭代范围，本处纯渲染层消费）：
        // 逐行提取 PARAMETER 指令渲染键值区；同键多值（如 stop）逐行展开
        let rows: Vec<&str> = ptext
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                t.to_uppercase()
                    .starts_with("PARAMETER ")
                    .then(|| t["PARAMETER ".len()..].trim())
            })
            .filter(|s| !s.is_empty())
            .collect();
        if !rows.is_empty() {
            println!("  Parameters:");
            for row in rows {
                // 键 = 首个空白前 token；值 = 其余原文（保留引号）
                match row.find(char::is_whitespace) {
                    Some(pos) => {
                        let (k, val) = row.split_at(pos);
                        println!("    {:<14} {}", k, val.trim());
                    }
                    None => println!("    {row}"),
                }
            }
        }
    }
    if let Some(caps) = v["capabilities"].as_array() {
        let caps: Vec<&str> = caps.iter().filter_map(|c| c.as_str()).collect();
        if !caps.is_empty() {
            println!("  Capabilities:");
            for c in caps {
                println!("    {c}");
            }
        }
    }
    if let Some(mf) = v["modelfile"].as_str() {
        // RUNTIME 行提取（大小写不敏感前缀；值段为指令后剩余部分）
        let rt: Vec<&str> = mf
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.len() > 8 && t[..7].eq_ignore_ascii_case("RUNTIME") && t.as_bytes()[7] == b' '
            })
            .collect();
        if !rt.is_empty() {
            println!("  RUNTIME:");
            for line in rt {
                println!("    {}", line.trim_start()[8..].trim());
            }
        }
    }
    0
}

/// run：单次提示或交互式 REPL（多轮对话）
/// M41：runtime_override 经 options.runtime 透传服务端（临时覆盖模型
/// RUNTIME，不落盘；变化触发实例重建）
async fn cmd_run(
    model: &str,
    first_prompt: String,
    verbose: bool,
    runtime_override: Option<String>,
) -> i32 {
    // M94 碴C（迭代29）：run 启动预检——对齐官方「未安装先下载（进度
    // 可见）完成后再显示输入框」时序（用户裁决 2026-09-10 06:34；原
    // 实现先进输入框、待首次 chat 404 才触发拉取）。仅明确 404 触发；
    // 连接失败/5xx 放行，由既有 chat 路径报错（预检不引入新失败面）；
    // REPL 会话内模型被删仍由 M33 碴8 内层兜底闭环
    let not_installed = matches!(
        http()
            .post(format!("{}/api/show", base_url()))
            .json(&json!({"model": model}))
            .send()
            .await,
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND
    );
    if not_installed {
        eprintln!("模型未安装，正在拉取：{model}");
        if pull_and_render(model).await != 0 {
            return 1; // 拉取失败（错误已打印）
        }
    }
    let mut rl = rustyline::DefaultEditor::new().expect("初始化 REPL 失败");
    let mut messages: Vec<Value> = Vec::new();
    let single = !first_prompt.is_empty();
    // 迭代30（M98）：横幅进入会话时打印一次（对齐官方 run；原实现每轮循环
    // 读取输入前重复打印刷屏，2026-09-10 用户反馈）
    if !single {
        // M114（迭代33 碴7）：横幅着色（原单行无颜色）
        println!(
            "{}",
            paint(
                stdout_color(),
                "36;1",
                ">>> 提示词送出，/bye 退出，/clear 清空对话 <<<"
            )
        );
    }
    let mut prompt = first_prompt;
    // M33 碴8：本会话是否已自动拉取过（防 404 循环拉取，最多一次）
    let mut pulled = false;

    loop {
        if !single {
            match rl.readline(&format!("{}> ", paint(stdout_color(), "36", model))) {
                Ok(line) => {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    rl.add_history_entry(&line).ok();
                    match line.as_str() {
                        "/bye" | "/exit" | "/quit" => break,
                        "/clear" => {
                            messages.clear();
                            // M114：半角括号（原全角与其余输出风格不统一）
                            println!("(已清空对话)");
                            continue;
                        }
                        _ => prompt = line,
                    }
                }
                Err(_) => break, // Ctrl-D
            }
        }
        messages.push(json!({"role": "user", "content": prompt}));

        // 流式对话（NDJSON 逐行渲染）；M33 碴8：404（模型未安装）自动
        // 拉取后就地重发一次（内层 loop，不回 REPL 读入）——对齐原版 run
        // 按需拉取语义；其他错误（502 网关等）不触发拉取。
        // hf.co 名缺 quant 场景：拉取落位的注册名带量化，重发仍 404 时
        // 经服务端碴7 报错（附已装量化列表）获得明确指引
        let resp = loop {
            // M41：--runtime 有值时附 options.runtime（服务端请求级覆盖键）
            let body = match &runtime_override {
                Some(rt) => json!({"model": model, "messages": messages, "stream": true,
                                   "options": {"runtime": rt}}),
                None => json!({"model": model, "messages": messages, "stream": true}),
            };
            let r = match http()
                .post(format!("{}/api/chat", base_url()))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    print_error(&format!("请求失败：{e}"));
                    return 1;
                }
            };
            if r.status() != reqwest::StatusCode::NOT_FOUND || pulled {
                break r;
            }
            eprintln!("模型未安装，正在拉取：{model}");
            if pull_and_render(model).await != 0 {
                return 1; // 拉取失败（错误已打印）
            }
            pulled = true;
        };
        if !resp.status().is_success() {
            print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
            if single {
                return 1;
            }
            messages.pop();
            continue;
        }
        // M27（碴3）：assistant 回复回填消息表——多轮对话上下文完整（对齐原版 run）
        let reply = print_assistant_stream(resp, verbose).await;
        messages.push(json!({"role": "assistant", "content": reply}));
        if single {
            break;
        }
        // M114（迭代33 碴7）：多轮之间空行分隔（原回复紧贴下一轮提示符）
        println!();
        prompt = String::new();
    }
    0
}

/// 渲染 /api/chat NDJSON 流并返回拼接的 assistant 文本。
/// M27（碴3）：返回值供 REPL 回填多轮上下文；thinking 增量仅终端展示不回填（对齐原版）。
async fn print_assistant_stream(resp: reqwest::Response, verbose: bool) -> String {
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut timing_info = String::new();
    let mut flushed = false;
    let mut reply = String::new();
    while let Some(chunk) = stream.next().await {
        if let Ok(bytes) = chunk {
            buf.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(ev) = serde_json::from_str::<Value>(line) {
                    if let Some(content) = assistant_content_from_event(&ev) {
                        reply.push_str(content);
                        print!("{content}");
                        flushed = true;
                    }
                    // M29 碴10（R3-A）：thinking 增量暗色渲染（不回填消息表）
                    if let Some(thinking) = ev["message"]["thinking"].as_str() {
                        let view = thinking_view(thinking);
                        if !view.is_empty() {
                            print!("{view}");
                            flushed = true;
                        }
                    }
                    if ev["done"].as_bool().unwrap_or(false) {
                        let count = ev["eval_count"].as_u64().unwrap_or(0);
                        let pcount = ev["prompt_eval_count"].as_u64().unwrap_or(0);
                        // M115（迭代33 碴8）：补总耗时与输出速率——done
                        // 事件已携带 total_duration/eval_duration（M28 碴6 +
                        // M102 timings 语义），原仅消费两个 token 计数；
                        // 字段缺失/为零时省略对应段（不显示无意义值）
                        let mut segs = vec![format!("输入 {pcount} tok / 输出 {count} tok")];
                        let total_ns = ev["total_duration"].as_u64().unwrap_or(0);
                        if total_ns > 0 {
                            segs.push(format!("总耗时 {:.2} 秒", total_ns as f64 / 1e9));
                        }
                        let eval_ns = ev["eval_duration"].as_u64().unwrap_or(0);
                        if eval_ns > 0 && count > 0 {
                            segs.push(format!(
                                "输出 {:.1} tok/s",
                                count as f64 / (eval_ns as f64 / 1e9)
                            ));
                        }
                        timing_info = format!("\n({})", segs.join(" / "));
                    }
                }
            }
        }
    }
    if flushed {
        println!();
        let _ = std::io::stdout().flush();
    }
    if verbose {
        println!("{timing_info}");
    }
    reply
}

/// thinking 增量的终端渲染形态（M29 碴10，R3-A 裁决：ANSI dim 暗色）。
///
/// - 参数 thinking：思考文本增量
/// - 返回：带暗色转义的渲染串（空增量为空串，不输出转义对）；
///   迭代42 D5（N-7 清偿 2026-09-11）：dim 经 M108 颜色层门控——
///   NO_COLOR/TERM=dumb/非 TTY 下纯文本，管道消费零转义污染
fn thinking_view(thinking: &str) -> String {
    if thinking.is_empty() {
        String::new()
    } else {
        paint(stdout_color(), "2", thinking)
    }
}

/// CLI 颜色门控（M108，迭代33 碴1）：对应流为 TTY + NO_COLOR 非空即禁
///（no-color.org 语义）+ TERM=dumb 禁——三重条件全过才放行 ESC 序列；
/// 管道/重定向零转义输出，脚本解析不受污染。
///
/// - 参数 is_tty：目标流是否终端（调用方传入）
/// - 返回：是否允许着色
fn color_ok(is_tty: bool) -> bool {
    if !is_tty {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    std::env::var("TERM").map_or(true, |t| t != "dumb")
}

/// stdout 颜色开关（进程内 OnceLock 缓存——逐行读 env 的开销免除）。
/// stdout 与 stderr 独立判定：`roxid list > file` 时 stdout 无色、
/// stderr 错误行仍可有色。
fn stdout_color() -> bool {
    static C: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *C.get_or_init(|| {
        use std::io::IsTerminal;
        color_ok(std::io::stdout().is_terminal())
    })
}

/// stderr 颜色开关（语义同 [`stdout_color`]，独立探测）。
fn stderr_color() -> bool {
    static C: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *C.get_or_init(|| {
        use std::io::IsTerminal;
        color_ok(std::io::stderr().is_terminal())
    })
}

/// 指定 SGR 码包裹文本（M108；颜色关闭时原样返回——调用方无需分支）。
///
/// - 参数 enabled：颜色开关（[`stdout_color`]/[`stderr_color`]）
/// - 参数 code：SGR 码（"31" 红 / "32" 绿 / "33" 黄 / "36" 青 / "1" 粗体）
/// - 返回：带转义对的渲染串
fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// 统一错误行输出（M117，迭代33 碴10）：红色粗体「错误：」前缀走
/// stderr——原各命令前缀形态不一（删除失败/复制失败/失败：），且
/// rm 失败把 `{"error":"..."}` 原始 JSON 包裹整段透出。
///
/// - 参数 msg：已整形的人类可读错误文本
fn print_error(msg: &str) {
    eprintln!("{} {msg}", paint(stderr_color(), "31;1", "错误："));
}

/// 服务端错误体整形（M117，迭代33 碴10）：解析 `{"error":"..."}`
/// 提取纯文本（兼容 OpenAI 层嵌套 message 形态）；非 JSON / 无
/// error 字段原样返回（不吞上游信息）。
///
/// - 参数 body：HTTP 错误响应原始文本
/// - 返回：人类可读错误消息
fn fmt_api_error(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]
                .as_str()
                .map(str::to_string)
                .or_else(|| v["error"]["message"].as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.to_string())
}

/// 终端宽度探测（M109，迭代33 碴2）：COLUMNS env（可解析且 8..=4096）
/// 优先 → terminal_size 探测 tty 实际列数 → 兜底 80。原 indicatif 未
/// 启用 terminal_size feature，COLUMNS=40 / stty cols 40 下进度帧仍按
/// 默认宽 82 字符渲染，窄终端每帧折 3 行且清理序列只作用末行残留屏幕。
///
/// - 返回：当前终端可用列数
fn term_width() -> usize {
    if let Ok(v) = std::env::var("COLUMNS") {
        if let Ok(n) = v.trim().parse::<usize>() {
            if (8..=4096).contains(&n) {
                return n;
            }
        }
    }
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

/// 字符显示宽度（M109/M112 共用）：CJK 区段计 2 列、其余 1 列。
/// 粗粒度判定（emoji/组合字符按窄计，East Asian Ambiguous 按 1 列）
/// ——服务于终端列收敛的截断场景足够。
fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = (0x2E80..=0x9FFF).contains(&u)        // CJK 部首/汉字/假名
        || (0xF900..=0xFAFF).contains(&u)            // CJK 兼容表意
        || (0xFE30..=0xFE4F).contains(&u)            // CJK 兼容形式
        || (0xFF00..=0xFF60).contains(&u)            // 全角形式
        || (0x20000..=0x2FA1F).contains(&u); // CJK 扩展
                                             // 宽字符 2 列、窄字符 1 列（bool→0/1 的直转会得 0/1，语义反转）
    if wide {
        2
    } else {
        1
    }
}

/// 字符串显示宽度（终端列数）。
fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 按显示宽度截断尾部加 …（M109 进度帧收敛 / M112 NAME 列共用）。
///
/// - 参数 s：原串；max_cols：最大显示列数（… 自身占 1 列）
/// - 返回：显示宽度不超过 max_cols 的串（原串不超宽时原样返回）
fn truncate_cols(s: &str, max_cols: usize) -> String {
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = char_width(c);
        if w + cw > max_cols.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// 按显示宽度右补空格（M130，迭代37：list/ps 列对齐——与 truncate_cols
/// 截断配对构成列基建，对齐 ollama tabwriter 动态列宽形态；Q2 裁决
/// 来源：用户确认 2026-09-11 06:51）。
///
/// - 参数 s：原串；width：目标显示宽度（统一列宽）
/// - 返回：补空格后的串；显示宽已达/超过 width 时原样返回
///   （截断职责归 truncate_cols，调用方先截断后补齐）
fn pad_cols(s: &str, width: usize) -> String {
    let pad = width.saturating_sub(display_width(s));
    if pad == 0 {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + pad);
    out.push_str(s);
    out.push_str(&" ".repeat(pad));
    out
}

/// NAME 统一列宽计算（M130/M131，迭代37）：本轮全部行显示宽最大值
/// （表头 "NAME" 参与下限），受终端宽分配上限约束（M112 窄终端截断
/// 收敛语义保留——超上限行截断后补齐到上限宽）。
///
/// - 参数 names：全部行 NAME 串；cap：终端宽分配上限（name_col）
/// - 返回：统一列宽（≤ cap；空列表退化为表头宽 4）
fn name_col_width(names: &[&str], cap: usize) -> usize {
    names
        .iter()
        .map(|n| display_width(n))
        .chain(std::iter::once(display_width("NAME")))
        .max()
        .unwrap_or(4)
        .min(cap)
}

/// 层摘要短形态（M110/M111）：`sha256:3c1c9d…` → `3c1c9d`（前 8 位，
/// 层身份可辨即可——官方 CLI 同样以摘要前缀标识层）。
///
/// - 参数 digest：完整摘要串
/// - 返回：摘要十六进制前 8 位（无前缀形态原样截取）
fn short_digest(digest: &str) -> &str {
    let hex = digest.rsplit(':').next().unwrap_or(digest);
    &hex[..hex.len().min(8)]
}

/// NDJSON status 中文映射（M110，迭代33 碴3）：协议侧保持官方英文
/// 原文（Ollama 语义对齐，e2e 断言不破），CLI 渲染层转中文；未知
/// status 原样输出（兼容服务端后续新增阶段）。
///
/// - 参数 status：协议 status 字段原文
/// - 返回：中文渲染文案
fn status_zh(status: &str) -> String {
    match status {
        "success" => "成功".into(),
        "verifying sha256 digest" => "校验 sha256 摘要".into(),
        "writing manifest" => "写入清单".into(),
        "pulling manifest" => "拉取清单".into(),
        "reading gguf header" => "读取 GGUF 头".into(),
        _ => {
            if let Some(rest) = status.strip_prefix("pulling ") {
                format!("拉取 {rest}")
            } else if let Some(rest) = status
                .strip_prefix("retrying download (attempt ")
                .and_then(|r| r.strip_suffix(')'))
            {
                format!("下载中断，正在重试（第 {rest}）")
            } else {
                status.to_string()
            }
        }
    }
}

/// 从 /api/chat NDJSON 事件提取 assistant 文本增量。
/// M27（碴3）：聚合与渲染分离（纯函数可单测）；空增量与统计末事件返回 None。
///
/// - 参数 ev：单条 NDJSON 事件（已解析 JSON）
/// - 返回：assistant content 文本切片；无文本增量 None
fn assistant_content_from_event(ev: &Value) -> Option<&str> {
    let content = ev["message"]["content"].as_str()?;
    (!content.is_empty()).then_some(content)
}

/// 字节量纲自适应（M105/M106 共用，迭代32 碴6）。
///
/// - 参数 n：字节数
/// - 返回：人类可读量纲串（KB/MB/GB，两位小数）
fn fmt_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let v = n as f64;
    if v >= GB {
        format!("{:.2} GB", v / GB)
    } else if v >= MB {
        format!("{:.2} MB", v / MB)
    } else {
        format!("{:.2} KB", v / KB)
    }
}

/// 剩余秒数 → 中文时间短语（M105/M106 共用，迭代32 碴6）。
///
/// - 参数 secs：剩余秒数
/// - 返回：如「45秒」「3分20秒」「1时05分」；不足 1 秒返回「<1秒」
fn fmt_eta(secs: f64) -> String {
    if secs < 1.0 {
        return "<1秒".to_string();
    }
    let s = secs.floor() as u64;
    if s < 60 {
        format!("{s}秒")
    } else if s < 3600 {
        format!("{}分{}秒", s / 60, s % 60)
    } else {
        format!("{}时{:02}分", s / 3600, s % 3600 / 60)
    }
}

/// 滑动窗口下载速度（M105/M106 共用，迭代32 碴6）：记录最近 window
/// 内的（时刻, 累计字节）样本，速度 = 窗口首末样本字节差 ÷ 时间差
///（防瞬时抖动；窗口内不足 2 样本或时距过短返回 None）。
struct SpeedTracker {
    /// 窗口内（时刻, 累计字节）样本序列
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
    /// 统计窗口时长
    window: std::time::Duration,
}

impl SpeedTracker {
    /// 构造速度追踪器。
    ///
    /// - 参数 window：统计窗口（建议 5s——进度事件 200ms 粒度下约 25 样本）
    fn new(window: std::time::Duration) -> Self {
        Self {
            samples: std::collections::VecDeque::new(),
            window,
        }
    }

    /// 推入新样本并计算窗口速度。
    ///
    /// - 参数 done：当前累计字节
    /// - 返回：窗口速度（字节/秒）；样本不足/时距过短 None
    fn push(&mut self, done: u64) -> Option<f64> {
        let now = std::time::Instant::now();
        self.samples.push_back((now, done));
        // 淘汰窗口外样本（保留最新 1 条窗口外样本作差分基线——避免
        // 恰好全部淘汰后样本数 <2 的空窗）
        while self.samples.len() > 2 {
            let front = match self.samples.front() {
                Some(f) => f,
                None => break,
            };
            if now.duration_since(front.0) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        let (t0, d0) = self.samples.front()?;
        let dt = now.duration_since(*t0).as_secs_f64();
        if dt < 0.2 {
            return None; // 时距过短：速度无意义（防除零与尖峰）
        }
        Some((done - d0) as f64 / dt)
    }
}

/// runtime 下载进度渲染器（M105，迭代32 碴6a；M109/M111 迭代33 增强）：
/// TTY spinner（量纲/速度/剩余时间/百分比 + msg 按终端宽 CJK 感知截断）；
/// 非 TTY 两行制（开始一行 + 完成汇总一行——对齐 pull 每阶段一行裁决
/// 语义，用户裁决 2026-09-10 22:25；原 1s 周期刷行取消）。
/// 返回 (bar, 回调)——bar 在安装结束后由调用方 finish_and_clear。
///
/// - 参数 label：进度行前缀（如「下载运行时」）
/// - 返回：spinner 句柄与进度回调（已下载字节, 总量 Option）
fn runtime_download_progress(
    label: &str,
) -> (indicatif::ProgressBar, impl FnMut(u64, Option<u64>)) {
    let label = label.to_string(); // 物化：闭包捕获所有权（impl trait 无生命周期参）
    let bar = indicatif::ProgressBar::new_spinner();
    let mut speed = SpeedTracker::new(std::time::Duration::from_secs(5));
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    if tty {
        bar.set_style(
            indicatif::ProgressStyle::default_spinner()
                .template("{spinner} {msg}")
                .unwrap(),
        );
        bar.enable_steady_tick(std::time::Duration::from_millis(100));
    }
    // M111：非 TTY 两行制状态——首回调打开始行；done 到达 total 打完成
    // 汇总行（总量 + 平均速度）
    let label_echo = label.clone(); // 非 TTY 分支独立持有（render 已 move 原件）
    let started = std::time::Instant::now();
    let mut announced = false;
    let mut finished = false;
    let render =
        move |speed: &mut SpeedTracker, done: u64, total: Option<u64>, out: &dyn Fn(String)| {
            let sp = speed.push(done);
            let speed_s = sp
                .map(|s| format!("，{}/s", fmt_bytes(s as u64)))
                .unwrap_or_default();
            let msg = match total {
                Some(t) if t > 0 => {
                    let pct = done * 100 / t;
                    let eta_s = sp
                        .filter(|s| *s > 0.0)
                        .map(|s| format!("，剩余 {}", fmt_eta((t - done) as f64 / s)))
                        .unwrap_or_default();
                    format!(
                        "{label} {}/{}（{pct}%{speed_s}{eta_s}）",
                        fmt_bytes(done),
                        fmt_bytes(t)
                    )
                }
                _ => format!("{label} {}{speed_s}", fmt_bytes(done)),
            };
            out(msg);
        };
    let bar2 = bar.clone();
    let cb = move |done: u64, total: Option<u64>| {
        if tty {
            // M109：msg 按终端宽截断（spinner 前缀 2 列）
            render(&mut speed, done, total, &|msg: String| {
                bar2.set_message(truncate_cols(&msg, term_width().saturating_sub(2)))
            });
            return;
        }
        if !announced {
            announced = true;
            println!("{label_echo}开始…");
            return;
        }
        if !finished && total.is_some_and(|t| t > 0 && done >= t) {
            finished = true;
            let t = total.unwrap_or(0);
            let secs = started.elapsed().as_secs_f64();
            let avg = if secs > 0.05 {
                format!("（平均 {}/s）", fmt_bytes((t as f64 / secs) as u64))
            } else {
                String::new()
            };
            println!("{label_echo}完成：{}{avg}", fmt_bytes(t));
        }
    };
    (bar, cb)
}

/// pull：拉取模型（NDJSON 进度条渲染）
/// M33 碴8：NDJSON 消费与状态判定抽 consume_ndjson_progress（cmd_run
/// 404 自动拉取与 create 判流复用）；本命令行为与原实现逐字节等价
async fn cmd_pull(model: &str) -> i32 {
    let code = pull_and_render(model).await;
    if code == 0 {
        println!("已拉取：{model}");
    }
    code
}

/// 发起 /api/pull 并渲染进度流（M33 碴8：cmd_pull 与 cmd_run 404 自动
/// 拉取共用）。
/// 迭代44 M164：hf.co 形态且 TTY 时前置投影器变体交互选择（R2 裁决
/// 2026-09-11 23:28），选定文件名经 /api/pull 可选 mmproj 字段传入
/// 服务端（Q3-X 裁决 23:36）。
///
/// - 参数 model：模型名（hf.co/ 形态直通服务端辅源分支）
/// - 返回：0 成功；1 失败（错误已打印）
async fn pull_and_render(model: &str) -> i32 {
    let mmproj_choice = prompt_mmproj_choice(model).await;
    let mut body = json!({"model": model});
    if let Some(name) = &mmproj_choice {
        body["mmproj"] = json!(name);
    }
    let resp = match http()
        .post(format!("{}/api/pull", base_url()))
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => conn_hint(e),
    };
    if !resp.status().is_success() {
        // M117：错误体整形 + 统一红色前缀
        print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
        return 1;
    }
    match consume_ndjson_progress(resp).await {
        Ok(()) => 0,
        Err(e) => {
            print_error(&e);
            1
        }
    }
}

/// HF 直引拉取的投影器变体交互选择（迭代44 M164，R2 + Q2-B + Q3-X
/// 裁决 2026-09-11 23:28/23:36）：
/// - hf.co 名 + stdin 为 TTY：本地列 repo GGUF 文件（轻量 tree API），
///   含 ≥2 个 mmproj 变体时打印编号列表（文件名 + 人类可读大小——
///   mmproj_variants 保证 fp16/f16 优先、首项即默认项），回车默认 1，
///   非法输入宽容钳制到范围；选定文件名随请求返回。
/// - 非 hf.co 名 / 非 TTY / list 失败 / 变体 ≤1：返回 None——服务端
///   权威处理（单一变体自动下载、多变体报错引导 CLI 交互，Q2-B）。
///
/// - 参数 model：规整后模型名（hf.co/ 前缀判定）
/// - 返回：选定的投影器文件名（None 表示不指定）
async fn prompt_mmproj_choice(model: &str) -> Option<String> {
    use std::io::IsTerminal;
    if !model.starts_with("hf.co/") || !std::io::stdin().is_terminal() {
        return None;
    }
    // repo 段提取：hf.co/{user}/{repo}[:{quant}] → {user}/{repo}
    let repo = model
        .trim_start_matches("hf.co/")
        .split(':')
        .next()?
        .to_string();
    let files = roxid_server::registry::HuggingFaceSource::new()
        .list_gguf_files(&repo)
        .await
        .ok()?;
    let variants = roxid_server::registry::HuggingFaceSource::mmproj_variants(&files);
    if variants.len() < 2 {
        return None; // 0 个：无投影器；1 个：服务端自动选中（R2 单一自动）
    }
    println!("该仓库含 {} 个多模态投影器（mmproj）变体：", variants.len());
    for (i, v) in variants.iter().enumerate() {
        let name = v.path.rsplit('/').next().unwrap_or(&v.path);
        println!("  {}) {}（{}）", i + 1, name, fmt_bytes(v.size));
    }
    print!("请输入序号 [1-{}]，回车默认 1：", variants.len());
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok()?;
    let choice: usize = input.trim().parse().unwrap_or(1);
    let idx = choice.clamp(1, variants.len()) - 1;
    variants[idx].path.rsplit('/').next().map(str::to_string)
}

/// 消费 NDJSON 进度/状态流并渲染（M33 碴8 自 cmd_pull 抽取；M34 O-1 双
/// 路径；迭代33 M109/M110/M111 重构）：
/// - TTY：spinner 单行覆盖 + msg 按终端宽 CJK 感知截断（原恒 82 字符，
///   窄终端每帧折 3 行且清理序列只作用末行残留屏幕）+ 阶段视觉区分
///   （校验/重试黄色 spinner、层完成绿 ✓、status 中文映射——协议英文
///   原文不变，兼容服务端后续新增阶段）；
/// - 非 TTY：对齐官方每阶段一行（用户裁决 2026-09-10 22:25）——status
///   阶段各一行、层结束（digest 切换/阶段推进/流末）输出层汇总行，
///   取消原 1s 周期刷行（原约 5 行刷屏与 TTY 单行呈现不一致）；
/// - error 事件即时终止；见 success 判成功；流耗尽未见 success 判中断。
///
/// - 参数 resp：上游 2xx 响应（/api/pull 或 /api/create）
/// - 返回：Ok(()) 见 success；Err(错误文本) 见 error 事件或流中断
async fn consume_ndjson_progress(resp: reqwest::Response) -> Result<(), String> {
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let bar = indicatif::ProgressBar::new_spinner();
    // M110：阶段 spinner 样式——下载默认态；校验/重试切黄色 spinner 字符
    let style_plain = indicatif::ProgressStyle::default_spinner()
        .template("{spinner} {msg}")
        .unwrap();
    let style_warn = indicatif::ProgressStyle::default_spinner()
        .template("\x1b[33m{spinner}\x1b[0m {msg}")
        .unwrap();
    bar.set_style(style_plain.clone());
    // M92 碴A（迭代29）：steady tick 驱动 {spinner} 前进（100ms 对齐官方
    // 动画周期）；非 TTY 路径 spinner 隐藏零影响
    bar.enable_steady_tick(std::time::Duration::from_millis(100));
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut ok = false;
    // M111：当前层状态（非 TTY 层结束渲染汇总行；TTY 借其做层切换检测）
    let mut layer: Option<LayerSummaryState> = None;
    // M106：滑动窗口速度（5s——进度事件 200ms 粒度下防抖）；M110：层
    // 边界重置——跨层 done 归零会使窗口首末差分下溢（M106 既有隐患，
    // 多层模型第二层首事件即触发，层边界即新统计窗）
    let mut speed = SpeedTracker::new(std::time::Duration::from_secs(5));
    while let Some(chunk) = stream.next().await {
        let Ok(bytes) = chunk else { continue };
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            if let Ok(ev) = serde_json::from_str::<Value>(line.trim()) {
                if let Some(err) = ev["error"].as_str() {
                    bar.finish_and_clear();
                    // M111：非 TTY 先层收尾再报错（末层汇总行不丢）
                    if let Some(l) = layer.take() {
                        if !tty {
                            println!("{}", layer_summary_line(&l));
                        }
                    }
                    return Err(err.to_string());
                }
                let status = ev["status"].as_str().unwrap_or_default();
                if status == "success" {
                    ok = true;
                    continue;
                }
                if let (Some(done), Some(total)) = (ev["completed"].as_u64(), ev["total"].as_u64())
                {
                    // 进度事件（层下载）：digest 变化即新层——收尾旧层
                    //（非 TTY 汇总行）+ 速度窗口重置（M110 下溢修复）
                    let digest = ev["digest"].as_str().unwrap_or("").to_string();
                    let new_layer = layer.as_ref().map_or(true, |l| l.digest != digest);
                    if new_layer {
                        if let Some(old) = layer.take() {
                            if !tty {
                                println!("{}", layer_summary_line(&old));
                            }
                        }
                        speed = SpeedTracker::new(std::time::Duration::from_secs(5));
                        layer = Some(LayerSummaryState {
                            digest,
                            total,
                            done,
                            started: std::time::Instant::now(),
                        });
                    } else if let Some(l) = &mut layer {
                        l.total = total;
                        l.done = done;
                    }
                    if tty {
                        // M110：进度事件恢复默认 spinner 色（阶段黄色仅
                        // 存续至下载恢复——同层重试后亦自然还原）
                        bar.set_style(style_plain.clone());
                        let pct = if total > 0 {
                            (done as f64 / total as f64 * 100.0) as u64
                        } else {
                            100
                        };
                        let sp = speed.push(done);
                        let speed_s = sp
                            .map(|s| format!("，{}/s", fmt_bytes(s as u64)))
                            .unwrap_or_default();
                        let eta_s = match (total, sp) {
                            (t, Some(s)) if t > done && s > 0.0 => {
                                format!("，剩余 {}", fmt_eta((t - done) as f64 / s))
                            }
                            _ => String::new(),
                        };
                        // M110：层完成绿色 ✓ 前缀（100% 与下载中区分）
                        let check = if total > 0 && done >= total {
                            format!("{} ", paint(stdout_color(), "32", "✓"))
                        } else {
                            String::new()
                        };
                        let msg = format!(
                            "{check}{} {}/{}（{pct}%{speed_s}{eta_s}）",
                            status_zh(status),
                            fmt_bytes(done),
                            fmt_bytes(total)
                        );
                        // M109：msg 按终端宽 CJK 感知截断（spinner 前缀 2 列）
                        bar.set_message(truncate_cols(&msg, term_width().saturating_sub(2)));
                    }
                    // 非 TTY：进度仅入层状态（M111 无周期行）
                } else if !status.is_empty() {
                    // 阶段/状态事件：两路径即时显示（M110 中文映射 + 黄色）
                    if tty {
                        let warnish = status == "verifying sha256 digest"
                            || status == "writing manifest"
                            || status.starts_with("retrying download");
                        bar.set_style(if warnish {
                            style_warn.clone()
                        } else {
                            style_plain.clone()
                        });
                        bar.set_message(truncate_cols(
                            &status_zh(status),
                            term_width().saturating_sub(2),
                        ));
                    } else {
                        if let Some(l) = layer.take() {
                            println!("{}", layer_summary_line(&l));
                        }
                        println!("{}", status_zh(status));
                    }
                }
            }
        }
    }
    bar.finish_and_clear();
    if ok {
        // M111：非 TTY 末层收尾（success 行由调用方「已拉取」承担不重复）
        if let Some(l) = layer.take() {
            if !tty {
                println!("{}", layer_summary_line(&l));
            }
        }
        Ok(())
    } else {
        Err("拉取中断".to_string())
    }
}

/// 单层下载的收尾状态（M111）：digest/总量/完成字节/起始时刻——层结束
/// （digest 切换或阶段推进）时渲染一行层汇总。
struct LayerSummaryState {
    digest: String,
    total: u64,
    done: u64,
    started: std::time::Instant,
}

/// 非 TTY 层汇总行（M111）：`拉取 3c1c9d：100% 2.00 GB（平均 5.3 MB/s）`
/// ——对齐官方非 TTY 层落定一行（含最终百分比/总量/平均速度）。
///
/// - 参数 l：层收尾状态
/// - 返回：单行汇总文本
fn layer_summary_line(l: &LayerSummaryState) -> String {
    let pct = if l.total > 0 {
        l.done * 100 / l.total
    } else {
        100
    };
    let secs = l.started.elapsed().as_secs_f64();
    let avg = if secs > 0.05 {
        format!("（平均 {}/s）", fmt_bytes((l.total as f64 / secs) as u64))
    } else {
        String::new()
    };
    format!(
        "拉取 {}：{pct}% {}{avg}",
        short_digest(&l.digest),
        fmt_bytes(l.total)
    )
}

/// signin：保存凭据到 ~/.roxid/auth.json（推送前置）。
/// 令牌经 rpassword 隐蔽输入（终端不回显；来源：用户裁决 R2-2A 2026-08-24 19:59）
async fn cmd_signin() -> i32 {
    print!("用户名: ");
    std::io::stdout().flush().ok();
    let mut user = String::new();
    std::io::stdin().read_line(&mut user).ok();
    let token = match rpassword::prompt_password("访问令牌（输入不回显）: ") {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => {
            eprintln!("令牌为空，已取消");
            return 1;
        }
        Err(e) => {
            eprintln!("读取令牌失败（需交互终端）：{e}");
            return 1;
        }
    };
    let auth = json!({"username": user.trim(), "token": token});
    let path = roxid_server::config::roxid_home().join("auth.json");
    std::fs::create_dir_all(roxid_server::config::roxid_home()).ok();
    write_auth_file(&path, &serde_json::to_string_pretty(&auth).unwrap()).unwrap();
    println!("凭据已保存：{}", path.display());
    0
}

/// 写凭据文件并收紧权限 0600（M30 碴13：令牌敏感信息原随 umask 默认
/// 0644 可被同机其他用户读取）。
///
/// - 参数 path：auth.json 目标路径
/// - 参数 text：序列化后的凭据 JSON 文本
/// - 返回：io::Result<()>；调用方 unwrap 提示
fn write_auth_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// signout：清除凭据
async fn cmd_signout() -> i32 {
    let path = roxid_server::config::roxid_home().join("auth.json");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
        println!("已退出登录");
    } else {
        println!("当前未登录");
    }
    0
}

// M101（迭代32 碴2）：RFC3339 双形态解析下沉服务端 repo 模块（单实现
// 两处复用：CLI list 时间渲染 + /v1/models created 真实化），本地副本
// 委托删除——导入保持既有调用点零改动（days_from_civil 仅测试模块
// 引用，改由测试内自行导入；2026-09-10 21-34）
use roxid_server::repo::parse_rfc3339_secs;

/// M90（迭代28）：epoch 秒差 → 中文相对时间短语（分段阈值对齐官方
/// ollama humanize 语义；短语中文化为用户裁决 2026-09-10 04:41）。
/// 差值 ≤ 0（时钟偏差/未来时间）显示「刚刚」（计划书 D3）。
fn humanize_age(delta_secs: i64) -> String {
    if delta_secs <= 0 {
        return "刚刚".into();
    }
    let mins = delta_secs / 60;
    if mins < 1 {
        return format!("{delta_secs} 秒前");
    }
    let hours = mins / 60;
    if hours < 1 {
        return format!("{mins} 分钟前");
    }
    let days = hours / 24;
    if days < 1 {
        return format!("{hours} 小时前");
    }
    if days < 7 {
        return format!("{days} 天前");
    }
    if days < 30 {
        return format!("{} 周前", days / 7);
    }
    if days < 365 {
        return format!("{} 个月前", days / 30);
    }
    format!("{} 年前", days / 365)
}

/// M90（迭代28）：modified_at 原始串 → `YYYY-MM-DD HH:MM (N 单位前)`
/// 混合格式。绝对部分直取串前 16 位（T 换空格：偏移形态即本地时间，
/// Z 形态为 UTC 值——std 零依赖无本地时区 API，计划书 D2 如实取舍）；
/// 解析失败原样输出整串（D4 兜底，不吞数据）。
/// - 参数 s：/api/tags 返回的 modified_at 字符串
/// - 参数 now_secs：当前 epoch 秒（相对时间基准）
/// - 返回：混合格式渲染串
fn fmt_modified(s: &str, now_secs: i64) -> String {
    match parse_rfc3339_secs(s) {
        Some(t) => format!(
            "{} ({})",
            format!("{} {}", &s[..10], &s[11..16]),
            humanize_age(now_secs - t)
        ),
        None => s.to_string(),
    }
}

/// 仅相对时间短语（M112，迭代33 碴5）：窄终端（<60 列）MODIFIED 列形态
/// ——混合格式 25 列放不下时收敛为短语（40 列终端三列总宽 ≤40 的收敛
/// 承诺）；解析失败原样输出整串（D4 兜底语义同 [`fmt_modified`]）。
///
/// - 参数 s：/api/tags 返回的 modified_at 字符串
/// - 参数 now_secs：当前 epoch 秒（相对时间基准）
/// - 返回：相对短语（如「3 个月前」）
fn fmt_modified_rel(s: &str, now_secs: i64) -> String {
    match parse_rfc3339_secs(s) {
        Some(t) => humanize_age(now_secs - t),
        None => s.to_string(),
    }
}

/// list：模型表格（M90：MODIFIED 列混合格式；M112：列宽随终端收敛 +
/// SIZE 量纲自适应——原 NAME 固定宽不截断，超长模型名行撑至 114 字符、
/// 40 列终端不收敛；SIZE 恒 MB 整数无 GB 档）
async fn cmd_list() -> i32 {
    let v: Value = http()
        .get(format!("{}/api/tags", base_url()))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e))
        .json()
        .await
        .unwrap_or(Value::Null);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // M112：列宽分配——SIZE 右对齐 10 列；MODIFIED 常规 25 列（绝对 16
    // + 括注约 9），窄终端（<60 列）仅保留相对短语（40 列终端三列总宽
    // 收敛 ≤40）；NAME 取剩余宽度，超出 CJK 感知截尾 …（M109 基建复用）
    // M130（迭代37）：改两遍渲染——先收集全部行，NAME 统一列宽取本轮
    // 最长（含表头），截断后 pad_cols 补齐，三列起点固定全表对齐
    // （来源：用户确认 Q2「动态列宽（ollama 形态）」2026-09-11 06:51）
    let width = term_width();
    let narrow = width < 60;
    let modified_col = if narrow { 9 } else { 25 };
    let name_col = width.saturating_sub(10 + modified_col + 4).max(4);
    let rows: Vec<(String, String, String)> = v["models"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let modified = m["modified_at"].as_str().unwrap_or("?");
            let modified_cell = if narrow {
                fmt_modified_rel(modified, now_secs)
            } else {
                fmt_modified(modified, now_secs)
            };
            (
                m["name"].as_str().unwrap_or("?").to_string(),
                fmt_bytes(m["size"].as_u64().unwrap_or(0)),
                modified_cell,
            )
        })
        .collect();
    let nw = name_col_width(
        &rows.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>(),
        name_col,
    );
    let sc = stdout_color();
    println!(
        "{}  {:>10}  MODIFIED",
        paint(sc, "1", &pad_cols("NAME", nw)),
        "SIZE"
    );
    for (name, size, modified_cell) in &rows {
        println!(
            "{}  {:>10}  {}",
            pad_cols(&truncate_cols(name, nw), nw),
            size,
            modified_cell
        );
    }
    0
}

/// ps：运行中模型六列表格（迭代36 M127，Q2 裁决 2026-09-11 05:46：
/// NAME/ID/SIZE/PROCESSOR/CONTEXT/UNTIL；Q6 裁决六列不删、NAME 弹性截断）。
/// 各列口径：ID=digest 剥 sha256: 前缀取 12 位——HF 直引（hf.co/ 前缀）
/// 取末尾 12 位，主源取前 12 位（迭代38 M135）；PROCESSOR=层数百分比
/// `X%/Y% CPU/GPU`，解析失败直书原因不降级（Q3）；CONTEXT=仅窗口总量
/// `{N} token`，无使用率（Q4）；UNTIL=中文未来短语（Q5）。
async fn cmd_ps() -> i32 {
    let v: Value = http()
        .get(format!("{}/api/ps", base_url()))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e))
        .json()
        .await
        .unwrap_or(Value::Null);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // 列宽预算：固定列 ID12+SIZE10+CONTEXT12+UNTIL10=44 + 间隙 2×5=10，
    // PROCESSOR 列宽动态（M134）；NAME 取终端剩余宽（Q6：六列不删，
    // 窄终端 NAME 压缩）
    // M131（迭代37）：两遍渲染——NAME 统一列宽取本轮最长（含表头），
    // 截断后 pad_cols 补齐，六列起点固定全表对齐（来源：用户确认 Q1
    // 「list + ps 一并修」2026-09-11 06:51）
    // M134（迭代38）：PROCESSOR 列宽动态扩宽——取本轮最长单元格显示宽
    // （16 下限维持无 note 时的既有形态），pad_cols 显示宽补齐替换
    // `{:<16}` 字符数补齐，CJK 失败原因不再溢出挤压后列；NAME 预算由
    // 固定 70 联动为 54+pw（来源：用户确认 Q1「动态扩宽」2026-09-11 07:27）
    let rows: Vec<(String, String, String, String, String, String)> = v["models"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            (
                m["name"].as_str().unwrap_or("?").to_string(),
                short_id12(
                    m["digest"].as_str().unwrap_or(""),
                    m["name"].as_str().unwrap_or(""),
                )
                .to_string(),
                fmt_bytes(m["size"].as_u64().unwrap_or(0)),
                fmt_processor(&m),
                fmt_context_total(&m),
                fmt_until_rel(m["expires_at"].as_str().unwrap_or(""), now_secs),
            )
        })
        .collect();
    // M134：PROCESSOR 统一列宽——全行显示宽最大值（表头参与下限），
    // 16 为无 note 场景的既有形态下限（百分比形态实际 15 列）
    let pw = rows
        .iter()
        .map(|(_, _, _, p, _, _)| display_width(p))
        .chain(std::iter::once(display_width("PROCESSOR")))
        .max()
        .unwrap_or(16)
        .max(16);
    // NAME 预算随 pw 联动收缩（54 = ID12+SIZE10+CONTEXT12+UNTIL10+间隙10）
    let name_col = term_width().saturating_sub(54 + pw).max(4);
    let nw = name_col_width(
        &rows.iter().map(|(n, ..)| n.as_str()).collect::<Vec<_>>(),
        name_col,
    );
    let sc = stdout_color();
    println!(
        "{}  {:<12}  {:>10}  {}  {:<12}  UNTIL",
        paint(sc, "1", &pad_cols("NAME", nw)),
        "ID",
        "SIZE",
        pad_cols("PROCESSOR", pw),
        "CONTEXT"
    );
    for (name, id, size, processor, context, until) in &rows {
        println!(
            "{}  {:<12}  {:>10}  {}  {:<12}  {}",
            pad_cols(&truncate_cols(name, nw), nw),
            id,
            size,
            pad_cols(processor, pw),
            context,
            until
        );
    }
    0
}

/// ID 列：digest 剥 `sha256:` 前缀取 12 位（迭代36 M127，Q2 裁决）。
/// 迭代38 M135 双口径（来源：用户确认「有sha256，取哈希值后12位」
/// 2026-09-11 07:31 + 澄清「仅 HF 直引用末尾，主源保持前 12 位」07:33）：
/// HF 直引（模型名 hf.co/ 前缀）取摘要末尾 12 位，主源取前 12 位。
/// 与 short_digest（8 位，错误摘要用）区分——本处固定 12 位对齐官方 ps。
///
/// - 参数 digest：完整摘要串（空串/缺前缀均容忍）
/// - 参数 model_name：/api/ps 模型全名（hf.co/ 前缀判定两源口径）
/// - 返回：12 位十六进制片段；空串输入返回 "?"
fn short_id12(digest: &str, model_name: &str) -> String {
    let hex = digest.rsplit(':').next().unwrap_or(digest);
    if hex.is_empty() {
        return "?".to_string();
    }
    if model_name.starts_with("hf.co/") {
        hex.chars()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        hex.chars().take(12).collect()
    }
}

/// PROCESSOR 列：层数换算 `X%/Y% CPU/GPU`（迭代36 M127，Q3 裁决：
/// 精确口径 stderr 层数解析；失败直书原因不降级，来源 2026-09-11 05:52）。
///
/// - 参数 m：/api/ps 单条模型对象
/// - 返回：百分比形态或失败原因原文；两者皆缺返回 "?"
fn fmt_processor(m: &Value) -> String {
    match (m["gpu_layers"].as_u64(), m["total_layers"].as_u64()) {
        (Some(gpu), Some(total)) if total > 0 => {
            let gpu_pct = gpu * 100 / total;
            format!("{}%/{}% CPU/GPU", 100 - gpu_pct, gpu_pct)
        }
        _ => m["gpu_layers_note"].as_str().unwrap_or("?").to_string(),
    }
}

/// CONTEXT 列：仅窗口总量 `{N} token`（迭代36 M127，Q4 裁决
/// 2026-09-11 06:02：不查子进程、无使用率百分比）。
///
/// - 参数 m：/api/ps 单条模型对象
/// - 返回：如 `8192 token`；字段缺失返回 "?"
fn fmt_context_total(m: &Value) -> String {
    match m["context_length"].as_u64() {
        Some(ctx) => format!("{ctx} token"),
        None => "?".to_string(),
    }
}

/// UNTIL 列：expires_at（RFC3339）→ 中文未来短语（迭代36 M127，Q5 裁决
/// 2026-09-11 06:02：如「4 分钟后」）；已到期显示「即将卸载」；
/// 解析失败原样输出整串（不吞数据，D4 兜底语义）。
///
/// - 参数 s：/api/ps 返回的 expires_at 字符串
/// - 参数 now_secs：当前 epoch 秒（相对时间基准）
/// - 返回：未来短语或原串
fn fmt_until_rel(s: &str, now_secs: i64) -> String {
    match parse_rfc3339_secs(s) {
        Some(t) => humanize_remaining(t - now_secs),
        None => s.to_string(),
    }
}

/// 剩余时长 → 中文未来短语（UNTIL 列专用；与 humanize_age 的过去方向
/// 「N 前」对偶，本处为「N 后」；分段粒度同迭代28：秒/分/时/天）。
///
/// - 参数 secs：剩余秒数（到期时刻 - 当前时刻）
/// - 返回：如「4 分钟后」；≤0 返回「即将卸载」
fn humanize_remaining(secs: i64) -> String {
    if secs <= 0 {
        return "即将卸载".to_string();
    }
    let mins = secs / 60;
    if secs < 60 {
        return format!("{secs} 秒后");
    }
    if mins < 60 {
        return format!("{mins} 分钟后");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours} 小时后");
    }
    format!("{} 天后", hours / 24)
}

/// cp：复制模型（M117：成功回显——原成功零输出无确认；官方差异挂账：
/// 官方 ollama cp 静默，回显为用户实测报告 2026-09-10 需求）
async fn cmd_cp(source: &str, destination: &str) -> i32 {
    let resp = http()
        .post(format!("{}/api/copy", base_url()))
        .json(&json!({"source": source, "destination": destination}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if resp.status().is_success() {
        println!(
            "{}",
            paint(
                stdout_color(),
                "32",
                &format!("已复制 {source} → {destination}")
            )
        );
        0
    } else {
        print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
        1
    }
}

/// rm：删除模型（M117：成功回显 + 错误体整形——原 rm 失败把
/// `{"error":"..."}` 原始 JSON 包裹整段透出）
async fn cmd_rm(model: &str) -> i32 {
    let resp = http()
        .delete(format!("{}/api/delete", base_url()))
        .json(&json!({"model": model}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if resp.status().is_success() {
        println!(
            "{}",
            paint(stdout_color(), "32", &format!("已删除 {model}"))
        );
        0
    } else {
        print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
        1
    }
}

/// 通用单模型 POST（stop/push；M117：成功回显 + 错误体整形）
///
/// - 参数 path：API 路径
/// - 参数 model：模型名
/// - 参数 done_msg：成功回显文案（含模型名的完整句）
/// - 返回：进程退出码
async fn cmd_simple_post(path: &str, model: &str, done_msg: &str) -> i32 {
    let resp = http()
        .post(format!("{}{path}", base_url()))
        .json(&json!({"model": model}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if resp.status().is_success() {
        println!("{}", paint(stdout_color(), "32", done_msg));
        0
    } else {
        print_error(&fmt_api_error(&resp.text().await.unwrap_or_default()));
        1
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// M27（碴3）：NDJSON 事件 → assistant 文本增量提取三分支
    /// （正常增量 / 空增量 / done 统计末事件）
    #[test]
    fn assistant_content_extraction() {
        let delta = json!({"model": "m", "message": {"role": "assistant", "content": "你"}});
        assert_eq!(super::assistant_content_from_event(&delta), Some("你"));
        let empty = json!({"message": {"content": ""}});
        assert_eq!(super::assistant_content_from_event(&empty), None);
        let done = json!({"done": true, "eval_count": 3});
        assert_eq!(super::assistant_content_from_event(&done), None);
    }

    /// M29 碴10（R3-A）+ 迭代42 D5（N-7）：thinking 渲染经 M108 颜色层
    /// 门控——TTY 下 dim 包裹、非 TTY/NO_COLOR 下纯文本；空增量恒空串
    #[test]
    fn thinking_view_goes_through_color_gate() {
        // 等价性断言（环境无关）：thinking_view ≡ paint(stdout_color(), "2", ·)
        assert_eq!(
            super::thinking_view("推理中"),
            super::paint(super::stdout_color(), "2", "推理中")
        );
        assert_eq!(super::thinking_view(""), "", "空增量必须为空串");
        // TTY 形态锚定：dim 转义对（门控开启时 paint 的输出形态）
        assert_eq!(super::paint(true, "2", "推理中"), "\x1b[2m推理中\x1b[0m");
    }

    /// M30 碴13：凭据文件写盘后权限必须收紧 0600（含 umask 非 600 环境）
    #[test]
    #[cfg(unix)]
    fn auth_file_permissions_restricted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("roxid-auth-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        super::write_auth_file(&path, r#"{"username":"u","token":"t"}"#).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "凭据文件必须仅属主可读写");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M90（迭代28）：days_from_civil 与服务端 civil_from_days 权威锚点
    /// 互证（epoch-days 20689 = 2026-08-24 UTC，来源同 registry 单测）
    /// M101：实现下沉服务端 repo 模块，此处直引服务端单实现
    #[test]
    fn days_from_civil_anchor() {
        assert_eq!(roxid_server::repo::days_from_civil(2026, 8, 24), 20689);
        assert_eq!(roxid_server::repo::days_from_civil(1970, 1, 1), 0);
    }

    /// M90（迭代28）：RFC3339 双形态解析——roxid Z 秒级 / 官方纳秒偏移，
    /// 同一时刻三种写法换算必须一致；非法输入 None
    #[test]
    fn rfc3339_dual_form_parsing() {
        let z = super::parse_rfc3339_secs("2026-09-07T00:00:00Z").unwrap();
        // 官方 Ollama 形态：纳秒 + 正偏移（+08:00 同一时刻）
        let plus = super::parse_rfc3339_secs("2026-09-07T08:00:00.123456789+08:00").unwrap();
        // 纳秒 + 负偏移紧凑形态（-0800 同一时刻）
        let minus = super::parse_rfc3339_secs("2026-09-06T16:00:00.5-0800").unwrap();
        assert_eq!(z, plus);
        assert_eq!(z, minus);
        // 结构非法 → None（兜底原样输出路径）
        assert!(super::parse_rfc3339_secs("not-a-time").is_none());
        assert!(super::parse_rfc3339_secs("2026-09-07T00:00").is_none());
    }

    /// M105/M106（迭代32 碴6）：量纲与剩余时间短语边界
    #[test]
    fn byte_magnitude_and_eta_phrases() {
        assert_eq!(super::fmt_bytes(512 * 1024), "512.00 KB");
        assert_eq!(super::fmt_bytes(3 * 1024 * 1024), "3.00 MB");
        assert_eq!(super::fmt_bytes(5_500_000_000), "5.12 GB");
        assert_eq!(super::fmt_eta(0.5), "<1秒");
        assert_eq!(super::fmt_eta(45.0), "45秒");
        assert_eq!(super::fmt_eta(200.0), "3分20秒");
        assert_eq!(super::fmt_eta(3900.0), "1时05分");
    }

    /// M105/M106：SpeedTracker——单样本（时距不足）None；窗口首末差分正确
    #[test]
    fn speed_tracker_window_semantics() {
        let mut st = super::SpeedTracker::new(std::time::Duration::from_secs(5));
        assert!(st.push(1_000).is_none(), "首样本无差分基线");
        // 连续同刻 push：时距 <0.2s 防尖峰 None（不 panic、不溢出）
        assert!(st.push(2_000).is_none() || st.push(2_000).unwrap() >= 0.0);
    }

    /// M90（迭代28）：混合格式渲染与七段中文短语边界（固定基准
    /// now = 1_800_000_000 = 2027-01-15T08:00:00Z，锚点见 days 互证）
    #[test]
    fn fmt_modified_mixed_form_and_age_tiers() {
        let now = 1_800_000_000i64;
        // 45 秒 → 秒段；绝对部分 T 换空格直取前 16 位
        assert_eq!(
            super::fmt_modified("2027-01-15T07:59:15Z", now),
            "2027-01-15 07:59 (45 秒前)"
        );
        // 90 秒 → 分钟段（官方偏移形态 + 纳秒位忽略）
        assert_eq!(
            super::fmt_modified("2027-01-15T07:58:30.9+00:00", now),
            "2027-01-15 07:58 (1 分钟前)"
        );
        // 2 小时 → 小时段
        assert_eq!(
            super::fmt_modified("2027-01-15T06:00:00Z", now),
            "2027-01-15 06:00 (2 小时前)"
        );
        // 3 天 → 天段
        assert_eq!(
            super::fmt_modified("2027-01-12T08:00:00Z", now),
            "2027-01-12 08:00 (3 天前)"
        );
        // 14 天 → 周段
        assert_eq!(
            super::fmt_modified("2027-01-01T08:00:00Z", now),
            "2027-01-01 08:00 (2 周前)"
        );
        // 97 天 → 个月段（97/30 = 3）
        assert_eq!(
            super::fmt_modified("2026-10-10T08:00:00Z", now),
            "2026-10-10 08:00 (3 个月前)"
        );
        // 400 天 → 年段（400/365 = 1）
        assert_eq!(
            super::fmt_modified("2025-12-11T08:00:00Z", now),
            "2025-12-11 08:00 (1 年前)"
        );
        // 差值 0 / 未来时间 → 刚刚（计划书 D3）
        assert!(super::fmt_modified("2027-01-15T08:00:00Z", now).ends_with("(刚刚)"));
        // 解析失败兜底原样输出（计划书 D4，不吞数据）
        assert_eq!(super::fmt_modified("?", now), "?");
    }

    /// M108（迭代33 碴1）：颜色门控——非 TTY 必禁；paint 关闭原样/开启包裹
    #[test]
    fn color_gating_and_paint() {
        assert!(!super::color_ok(false), "非 TTY 必须禁色");
        assert_eq!(super::paint(false, "31", "x"), "x");
        assert_eq!(super::paint(true, "31", "x"), "\x1b[31mx\x1b[0m");
    }

    /// M109（迭代33 碴2）：CJK 感知宽度与截断——中文 2 列、… 尾缀
    /// 1 列、不超宽原样；82 字符长帧截到 38 列（40 列终端场景）
    #[test]
    fn cjk_truncate_semantics() {
        assert_eq!(super::display_width("ab"), 2);
        assert_eq!(super::display_width("中文"), 4);
        let cut = super::truncate_cols(&"p".repeat(50), 38);
        assert_eq!(super::display_width(&cut), 38, "截断结果必须恰为上限宽度");
        assert!(cut.ends_with('…'));
        assert_eq!(super::truncate_cols("abc", 38), "abc", "不超宽原样返回");
        assert_eq!(super::truncate_cols("中文字", 5), "中文…");
    }

    /// M110（迭代33 碴3）：status 中文映射——官方英文原文转中文渲染；
    /// 未知 status 原样（兼容服务端后续新增阶段）
    #[test]
    fn status_zh_mapping() {
        assert_eq!(
            super::status_zh("verifying sha256 digest"),
            "校验 sha256 摘要"
        );
        assert_eq!(super::status_zh("writing manifest"), "写入清单");
        assert_eq!(super::status_zh("pulling manifest"), "拉取清单");
        assert_eq!(super::status_zh("success"), "成功");
        assert_eq!(
            super::status_zh("retrying download (attempt 1/3)"),
            "下载中断，正在重试（第 1/3）"
        );
        assert_eq!(super::status_zh("future phase"), "future phase");
        // 纯 "pulling"（进度事件 status）无后缀内容——原样（进度行自组装）
        assert_eq!(super::status_zh("pulling"), "pulling");
    }

    /// M117（迭代33 碴10）：错误体整形——单层/嵌套 error 提取、非 JSON 原样
    #[test]
    fn api_error_body_extraction() {
        assert_eq!(
            super::fmt_api_error(r#"{"error":"model not found"}"#),
            "model not found"
        );
        assert_eq!(
            super::fmt_api_error(r#"{"error":{"message":"bad request"}}"#),
            "bad request"
        );
        assert_eq!(super::fmt_api_error("plain text"), "plain text");
        assert_eq!(super::fmt_api_error(r#"{"other":1}"#), r#"{"other":1}"#);
    }

    /// M112：窄终端 MODIFIED 相对短语形态；解析失败原样
    #[test]
    fn fmt_modified_rel_form() {
        let now = 1_800_000_000i64;
        assert_eq!(
            super::fmt_modified_rel("2027-01-15T07:59:15Z", now),
            "45 秒前"
        );
        assert_eq!(super::fmt_modified_rel("?", now), "?");
    }

    /// 迭代36 M127：ps 六列渲染族（ID/PROCESSOR/CONTEXT/UNTIL 各列口径）
    #[test]
    fn ps_six_column_rendering() {
        // ID：剥前缀取 12 位；空串容错（迭代38 M135：第二参模型名分流口径）
        assert_eq!(
            super::short_id12("sha256:6a4c9f1b2c3d4e5f", "smollm2:135m"),
            "6a4c9f1b2c3d"
        );
        assert_eq!(super::short_id12("6a4c9f1b", "m:latest"), "6a4c9f1b");
        assert_eq!(super::short_id12("", "m:latest"), "?");
        // 迭代38 M135 双口径：HF 直引（hf.co/ 前缀）取末尾 12 位，主源
        // 保持前 12 位（来源：用户澄清 2026-09-11 07:33，两源口径不同）
        let sha64 = "0123456789abcdef".repeat(4);
        assert_eq!(
            super::short_id12(&format!("sha256:{sha64}"), "hf.co/unsloth/X:Q4"),
            "456789abcdef"
        );
        assert_eq!(
            super::short_id12(&sha64, "hf.co/u/r:Q4_K_M"),
            "456789abcdef"
        );
        assert_eq!(super::short_id12(&sha64, "gemma4:latest"), "0123456789ab");
        // PROCESSOR：33/41 → 80% GPU → 「20%/80% CPU/GPU」；全 GPU；纯 CPU；
        // 失败直书原因（Q3）；双缺 "?"
        let m = json!({"gpu_layers": 33, "total_layers": 41});
        assert_eq!(super::fmt_processor(&m), "20%/80% CPU/GPU");
        assert_eq!(
            super::fmt_processor(&json!({"gpu_layers": 41, "total_layers": 41})),
            "0%/100% CPU/GPU"
        );
        assert_eq!(
            super::fmt_processor(&json!({"gpu_layers": 0, "total_layers": 41})),
            "100%/0% CPU/GPU"
        );
        assert_eq!(
            super::fmt_processor(&json!({"gpu_layers_note": "stderr 未匹配层卸载行"})),
            "stderr 未匹配层卸载行"
        );
        assert_eq!(super::fmt_processor(&json!({})), "?");
        // CONTEXT：仅窗口总量（Q4），无使用率
        assert_eq!(
            super::fmt_context_total(&json!({"context_length": 8192})),
            "8192 token"
        );
        assert_eq!(super::fmt_context_total(&json!({})), "?");
    }

    /// 迭代36 M127：UNTIL 未来短语（Q5 中文形态）+ 兜底
    #[test]
    fn until_relative_phrase() {
        let now = 1_800_000_000i64; // 2027-01-15T08:00:00Z
        assert_eq!(
            super::fmt_until_rel("2027-01-15T08:04:00Z", now),
            "4 分钟后"
        );
        assert_eq!(super::fmt_until_rel("2027-01-15T08:00:30Z", now), "30 秒后");
        // 23 小时仍在时段；24 小时整落入天段「1 天后」（分段同迭代28）
        assert_eq!(
            super::fmt_until_rel("2027-01-16T07:00:00Z", now),
            "23 小时后"
        );
        assert_eq!(super::fmt_until_rel("2027-01-16T08:00:00Z", now), "1 天后");
        assert_eq!(
            super::fmt_until_rel("2027-01-15T07:59:00Z", now),
            "即将卸载"
        );
        assert_eq!(super::fmt_until_rel("?", now), "?");
    }

    /// M111（迭代33 碴4）：非 TTY 层汇总行形态（digest 前 8 位 + 百分比
    /// + 总量）——层结束各落一行对齐官方
    #[test]
    fn layer_summary_form() {
        let l = super::LayerSummaryState {
            digest: "sha256:3c1c9dabcd".into(),
            total: 1024 * 1024,
            done: 1024 * 1024,
            started: std::time::Instant::now(),
        };
        let line = super::layer_summary_line(&l);
        assert!(
            line.starts_with("拉取 3c1c9dab：100% 1.00 MB"),
            "实际：{line}"
        );
    }

    /// M118（迭代33 碴11）：全局选项后置解析——`roxid run --verbose m`
    /// 与前置形态均合法且值透传（原后置报 unexpected argument）
    #[test]
    fn global_flags_parse_after_subcommand() {
        use clap::Parser as _;
        // Cli 未实现 Debug——映射为 Result<(), String>（错误文本可读）
        let after = super::Cli::try_parse_from(["roxid", "run", "--verbose", "m1"])
            .map(|cli| cli.verbose)
            .map_err(|e| e.to_string());
        assert!(
            matches!(after, Ok(true)),
            "子命令后置 --verbose 必须可解析且值透传：{after:?}"
        );
        let before = super::Cli::try_parse_from(["roxid", "--verbose", "run", "m1"])
            .map(|cli| cli.verbose)
            .map_err(|e| e.to_string());
        assert!(matches!(before, Ok(true)), "前置形态同样合法：{before:?}");
    }

    /// M130/M131（迭代37）：动态列宽对齐——pad_cols 补齐语义、
    /// name_col_width 最长取宽受 cap 约束、list/ps 行渲染列位一致性
    /// （用户实测样例：长短名混合行 SIZE 域终点与 MODIFIED/ID 起点一致）
    #[test]
    fn list_ps_column_alignment() {
        // pad_cols：短补齐 / CJK 按 2 列计补 / 已满与超宽原样（截断归 truncate_cols）
        assert_eq!(super::pad_cols("ab", 5), "ab   ");
        assert_eq!(super::pad_cols("模型", 6), "模型  ");
        assert_eq!(super::pad_cols("abc", 3), "abc");
        assert_eq!(super::pad_cols("abcdef", 5), "abcdef");
        // name_col_width：取最长 / 表头 4 下限 / cap 收敛
        assert_eq!(
            super::name_col_width(&["gemma4:latest", "smollm2:135m"], 41),
            13
        );
        assert_eq!(super::name_col_width(&[], 41), 4);
        assert_eq!(
            super::name_col_width(&["hf.co/unsloth/Qwen3.5-4B-GGUF:Q4_K_M"], 20),
            20
        );
        // list 对齐快照（用户样例三行）：SIZE 右端对齐（域终点一致）
        // + MODIFIED 起点逐行一致（SIZE 为 {:>10} 右对齐域）
        let names = [
            "gemma4:latest",
            "smollm2:135m",
            "hf.co/unsloth/Qwen3.5-4B-GGUF:Q4_K_M",
        ];
        let sizes = ["8.95 GB", "258.34 MB", "2.55 GB"];
        let nw = super::name_col_width(&names, 80);
        assert_eq!(nw, 36);
        let (mut size_ends, mut mod_starts) = (Vec::new(), Vec::new());
        for (n, s) in names.iter().zip(&sizes) {
            let line = format!(
                "{}  {:>10}  2026-09-09 22:53 (23 小时前)",
                super::pad_cols(&super::truncate_cols(n, nw), nw),
                s
            );
            size_ends.push(line.find(s).unwrap() + s.len());
            mod_starts.push(line.find("2026-09-09").unwrap());
        }
        assert!(
            size_ends.windows(2).all(|w| w[0] == w[1]),
            "SIZE 域终点不一致：{size_ends:?}"
        );
        assert_eq!(size_ends[0], 36 + 2 + 10);
        assert!(
            mod_starts.windows(2).all(|w| w[0] == w[1]),
            "MODIFIED 起点不一致：{mod_starts:?}"
        );
        assert_eq!(mod_starts[0], 36 + 2 + 10 + 2);
        // 截断与对齐共存（窄终端）：cap 20 下截断补齐后 SIZE 域起点固定
        let nw20 = super::name_col_width(&names, 20);
        assert_eq!(nw20, 20);
        let head = format!(
            "{}  {:>10}",
            super::pad_cols(&super::truncate_cols(names[0], nw20), nw20),
            "8.95 GB"
        );
        assert_eq!(head.find("8.95 GB"), Some(22 + 3));
        // ps 六列对齐快照：ID 起点逐行一致（与 NAME 长短无关）；
        // 迭代38 M134 回归：PROCESSOR 混排（15 列百分比 + 21 列 CJK note）
        // 时 CONTEXT 起点仍逐行一致（原 {：<16} 字符数补齐致漂移约 7 列）
        let ps_names = ["smollm2:135m", "hf.co/unsloth/Qwen3.5-4B-GGUF:Q4_K_M"];
        let ids = ["b0f58c4c1a3c", "ea35d2362372"];
        let procs = ["20%/80% CPU/GPU", "stderr 未匹配层卸载行"];
        let nw = super::name_col_width(&ps_names, 40);
        // M134：PROCESSOR 列宽 = 全行显示宽最大值（16 下限维持既有形态）
        let pcw = procs
            .iter()
            .map(|p| super::display_width(p))
            .chain(std::iter::once(super::display_width("PROCESSOR")))
            .max()
            .unwrap()
            .max(16);
        assert_eq!(pcw, 21);
        let (id_starts, ctx_starts): (Vec<usize>, Vec<usize>) = ps_names
            .iter()
            .zip(&ids)
            .zip(&procs)
            .map(|((n, id), p)| {
                let line = format!(
                    "{}  {:<12}  {:>10}  {}  {:<12}  {}",
                    super::pad_cols(&super::truncate_cols(n, nw), nw),
                    id,
                    "258.34 MB",
                    super::pad_cols(p, pcw),
                    "4096 token",
                    "5 分钟后"
                );
                // 迭代42 D1（2026-09-11）：列起点断言由字节位置改为显示宽
                // 口径——PROCESSOR CJK 行 pad 后字节宽 28 ≠ 显示宽 21，
                // 字节 find 起点必漂移（迭代38 引入潜伏），显示对齐才是目标
                (
                    super::display_width(&line[..line.find(id).unwrap()]),
                    super::display_width(&line[..line.find("4096").unwrap()]),
                )
            })
            .unzip();
        assert!(
            id_starts.windows(2).all(|w| w[0] == w[1]),
            "ID 起点不一致：{id_starts:?}"
        );
        assert_eq!(id_starts[0], 36 + 2);
        assert!(
            ctx_starts.windows(2).all(|w| w[0] == w[1]),
            "CONTEXT 起点不一致：{ctx_starts:?}"
        );
        assert_eq!(ctx_starts[0], 36 + 2 + 12 + 2 + 10 + 2 + pcw + 2);
    }
}
