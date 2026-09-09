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

mod complete;
mod completion;
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

/// 服务不可达的统一提示
fn conn_hint(e: reqwest::Error) -> ! {
    eprintln!("无法连接 roxid 服务（{}）：{e}", base_url());
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
    #[arg(long)]
    nowordwrap: bool,
    /// Show timings for response
    #[arg(long)]
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
        Commands::Stop { model } => cmd_simple_post("/api/stop", &model).await,
        Commands::Pull { model, hf } => {
            let model = normalize_hf_arg(model, hf);
            cmd_pull(&model).await
        }
        Commands::Push { model } => cmd_simple_post("/api/push", &model).await,
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
        // 迭代20：补全族与候选源均为本地操作（同步执行，不经 serve）
        Commands::Completion { cmd } => completion::cmd_completion(cmd),
        Commands::Complete { words } => {
            complete::complete(&words);
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
                println!("  {tag}  {}{mark}", variants.join(", "));
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
                return match rt::install_manual(&url).await {
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
            let variant = rt::detect_backend().asset_variant();
            println!("探测后端变体：{variant}（GPU → vulkan / 无 GPU → cpu）");
            match rt::install_version(&tag, variant).await {
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
                eprintln!("无法读取 Modelfile {f}：{e}");
                return 1;
            }
        },
        None => {
            if interactive {
                return create_wizard(model).await;
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
        eprintln!("创建失败：{}", resp.text().await.unwrap_or_default());
        return 1;
    }
    match consume_ndjson_progress(resp).await {
        Ok(()) => {
            println!("已创建：{model}");
            0
        }
        Err(e) => {
            eprintln!("创建失败：{e}");
            1
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
        eprintln!("错误：{}", resp.text().await.unwrap_or_default());
        return 1;
    }
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    println!(
        "  Model:      {}",
        v["details"]["family"].as_str().unwrap_or("?")
    );
    println!(
        "  Parameters: {} ({})",
        v["details"]["parameter_size"].as_str().unwrap_or("?"),
        v["details"]["quantization_level"].as_str().unwrap_or("?")
    );
    if let Some(sys) = v["system"].as_str() {
        if !sys.is_empty() {
            println!("  System:     {sys}");
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
    let mut rl = rustyline::DefaultEditor::new().expect("初始化 REPL 失败");
    let mut messages: Vec<Value> = Vec::new();
    let single = !first_prompt.is_empty();
    let mut prompt = first_prompt;
    // M33 碴8：本会话是否已自动拉取过（防 404 循环拉取，最多一次）
    let mut pulled = false;

    loop {
        if !single {
            println!(">>> 提示词送出，/bye 退出，/clear 清空对话 <<<");
            match rl.readline(format!("{model}> ").as_str()) {
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
                            println!("（已清空对话）");
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
                    eprintln!("请求失败：{e}");
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
            eprintln!("错误：{}", resp.text().await.unwrap_or_default());
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
                        timing_info = format!("\n(输入 {pcount} tok / 输出 {count} tok)");
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
/// - 返回：带暗色转义的渲染串（空增量为空串，不输出转义对）
fn thinking_view(thinking: &str) -> String {
    if thinking.is_empty() {
        String::new()
    } else {
        format!("\x1b[2m{thinking}\x1b[0m")
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
///
/// - 参数 model：模型名（hf.co/ 形态直通服务端辅源分支）
/// - 返回：0 成功；1 失败（错误已打印）
async fn pull_and_render(model: &str) -> i32 {
    let resp = match http()
        .post(format!("{}/api/pull", base_url()))
        .json(&json!({"model": model}))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => conn_hint(e),
    };
    if !resp.status().is_success() {
        eprintln!("拉取失败：{}", resp.text().await.unwrap_or_default());
        return 1;
    }
    match consume_ndjson_progress(resp).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("拉取失败：{e}");
            1
        }
    }
}

/// 消费 NDJSON 进度/状态流并渲染（M33 碴8 自 cmd_pull 抽取；M34 O-1 双
/// 路径渲染）：TTY 下 spinner 带百分比/字节（对齐原版实时进度条）；非
/// TTY（管道/重定向——indicatif 默认隐藏，实测全程静默）降级为 ≥1s 周期
/// 文本行（对齐官方 Go CLI 非 TTY 打印行为）；error 事件即时终止；见
/// success 判成功；流耗尽未见 success 判中断。
///
/// - 参数 resp：上游 2xx 响应（/api/pull 或 /api/create）
/// - 返回：Ok(()) 见 success；Err(错误文本) 见 error 事件或流中断
async fn consume_ndjson_progress(resp: reqwest::Response) -> Result<(), String> {
    // M34 O-1：stdout 非 TTY 时 indicatif spinner 完全隐藏——降级文本行
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let bar = indicatif::ProgressBar::new_spinner();
    bar.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut ok = false;
    let mut last_print = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let Ok(bytes) = chunk else { continue };
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            if let Ok(ev) = serde_json::from_str::<Value>(line.trim()) {
                if let Some(err) = ev["error"].as_str() {
                    bar.finish_and_clear();
                    return Err(err.to_string());
                }
                let status = ev["status"].as_str().unwrap_or_default();
                if status == "success" {
                    ok = true;
                    continue;
                }
                if let (Some(done), Some(total)) = (ev["completed"].as_u64(), ev["total"].as_u64())
                {
                    let pct = if total > 0 {
                        (done as f64 / total as f64 * 100.0) as u64
                    } else {
                        100
                    };
                    if tty {
                        bar.set_message(format!(
                            "{status} {}/{} MB ({}%)",
                            done / 1048576,
                            total / 1048576,
                            pct
                        ));
                    } else if last_print.elapsed() >= std::time::Duration::from_secs(1) {
                        // 非 TTY 周期文本行（1s 节流——进度事件 200ms 粒度直达会刷屏）
                        println!(
                            "{status} {}/{} MB ({}%)",
                            done / 1048576,
                            total / 1048576,
                            pct
                        );
                        last_print = std::time::Instant::now();
                    }
                } else if !status.is_empty() {
                    // 状态事件（pulling manifest 等）：频率低，两路径即时显示
                    if tty {
                        bar.set_message(status.to_string());
                    } else {
                        println!("{status}");
                    }
                }
            }
        }
    }
    bar.finish_and_clear();
    if ok {
        Ok(())
    } else {
        Err("拉取中断".to_string())
    }
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

/// list：模型表格
async fn cmd_list() -> i32 {
    let v: Value = http()
        .get(format!("{}/api/tags", base_url()))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e))
        .json()
        .await
        .unwrap_or(Value::Null);
    println!("{:<32} {:>12}  {}", "NAME", "SIZE", "MODIFIED");
    for m in v["models"].as_array().cloned().unwrap_or_default() {
        let size = m["size"].as_u64().unwrap_or(0);
        println!(
            "{:<32} {:>10} MB  {}",
            m["name"].as_str().unwrap_or("?"),
            size / 1048576,
            m["modified_at"].as_str().unwrap_or("?")
        );
    }
    0
}

/// ps：运行中模型
async fn cmd_ps() -> i32 {
    let v: Value = http()
        .get(format!("{}/api/ps", base_url()))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e))
        .json()
        .await
        .unwrap_or(Value::Null);
    println!("{:<32} {:>12}", "NAME", "SIZE");
    for m in v["models"].as_array().cloned().unwrap_or_default() {
        println!(
            "{:<32} {:>10} MB",
            m["name"].as_str().unwrap_or("?"),
            m["size"].as_u64().unwrap_or(0) / 1048576
        );
    }
    0
}

/// cp：复制模型
async fn cmd_cp(source: &str, destination: &str) -> i32 {
    let resp = http()
        .post(format!("{}/api/copy", base_url()))
        .json(&json!({"source": source, "destination": destination}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if resp.status().is_success() {
        0
    } else {
        eprintln!("复制失败：{}", resp.text().await.unwrap_or_default());
        1
    }
}

/// rm：删除模型
async fn cmd_rm(model: &str) -> i32 {
    let resp = http()
        .delete(format!("{}/api/delete", base_url()))
        .json(&json!({"model": model}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if resp.status().is_success() {
        0
    } else {
        eprintln!("删除失败：{}", resp.text().await.unwrap_or_default());
        1
    }
}

/// 通用单模型 POST（stop/push）
async fn cmd_simple_post(path: &str, model: &str) -> i32 {
    let resp = http()
        .post(format!("{}{path}", base_url()))
        .json(&json!({"model": model}))
        .send()
        .await
        .unwrap_or_else(|e| conn_hint(e));
    if resp.status().is_success() {
        0
    } else {
        eprintln!("失败：{}", resp.text().await.unwrap_or_default());
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

    /// M29 碴10（R3-A）：thinking 渲染形态——非空增量带 ANSI dim 包裹，
    /// 空增量不产出转义对（避免污染输出）
    #[test]
    fn thinking_view_wraps_with_ansi_dim() {
        assert_eq!(super::thinking_view("推理中"), "\x1b[2m推理中\x1b[0m");
        assert_eq!(super::thinking_view(""), "", "空增量必须为空串");
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
}
