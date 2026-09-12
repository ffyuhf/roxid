//! 单个 llama-server 子进程实例的生命周期管理（M3）。
//!
//! 职责：
//! - 拉起 llama-server（SpawnSpec：GGUF/mmproj/LoRA/ctx 全参数）
//! - /health 就绪轮询（进程启动即退出时立即失败，不空等）
//! - 子进程 stdout/stderr 后台转发到 tracing（防管道满阻塞）
//! - keep_alive 到期判定与刷新（与原版 Ollama 语义一致：请求到达即续期）
//! - 优雅关闭：kill + wait 确保进程资源回收
//!
//! 修改历史：M3 新增 2026-08-24 19:08；M6 扩展 SpawnSpec 与 size 字段 2026-08-24 19:20
//! （原因：注册表需要元数据驱动的启动参数与 /api/ps 尺寸信息）；
//! M15 显存统计：PID 记录 + nvidia-smi 进程显存查询 2026-08-24 20-09
//! （来源：用户确认 R2-1A 2026-08-24 19:59）；
//! M25 移除未使用 import Path（迭代5 M12 遗留警告清偿）2026-08-26 05:48
//! M26 死进程安全回收（迭代6 P0碴1 前置）：shutdown_mut 对已退出进程跳过
//! kill 仅 wait 回收，消除 InvalidInput 报错 2026-08-26 06-44
//! M27 在途保护（迭代7 P0碴1）：Runner 增 in_flight 计数与 keep_alive 窗口记忆，
//! 新增 RunnerLease 租约（R1 全调用点 / R3 响应完成时续期，用户裁决
//! 2026-08-26 21:11）——推理中的实例不再被 keep_alive 到期误杀 2026-08-26 21-18
//! M30 碴8（迭代10，R4-A）：RunnerLease::drop 慢路径在非 runtime 上下文
//! 经 blocking_lock 兜底完成 leave_request——原跳过使 in_flight 永不归
//! 零，reaper 永不卸载该实例（注释声称"仅影响时机"实为泄漏至进程退出）
//! 2026-09-06 22-25
//! M46（迭代18 BUG-10）：spawn_with 注入 prctl(PR_SET_PDEATHSIG,
//! SIGKILL) 孤儿防护——serve 被 kill -9/panic/OOM 时无 drop 机会，
//! kill_on_drop 失效，runner 永久泄漏（历史孤儿 PPID=1 实证）；内核
//! 在父进程死亡时向子进程投递 SIGKILL 2026-09-09 06-02
//! M54（迭代19 碴B）：Runner 记录实际二进制路径 llama_server_bin——
//! runtime use 切换默认版本后应然路径与实例记录不一致时触发重建
//! （碴B：原仅 ctx/RUNTIME 双键，后端版本切换运行实例无感知）2026-09-09 20-25
//! M97（迭代30 碴A增强）：stderr 尾部环形缓存（12 行）——「启动即退出」
//! 报错附带 stderr 摘要；gemma4 案实证裸文案无法定位真实失败原因
//! （wrong number of tensors 湮没在统一 404 与无效重拉噪音中）2026-09-10 07-20
//! M99（迭代31，Q4）：spawn 失败 ENOENT 且二进制在位时读 ELF 头诊断
//! 架构不匹配——原裸报「No such file or directory (os error 2)」无从
//! 定位（x64 包误装 arm64 宿主即此形态，用户实测 2026-09-10）
//! 2026-09-10 18-30
//! M100（迭代32 碴1）：spawn_args 增 --alias 规范化模型名——透传层
//! 剥离请求 model 字段后 llama-server 以加载路径兜底回显（用户实测
//! model 字段返回 /root/.roxid/...gguf）；--alias 使全部回显点统一为
//! model:tag（llama-server 官方 API 层命名参数，README 实证）
//! 2026-09-10 21-26；
//! M170（迭代45 Q5-A 裁决 2026-09-12 01:45）：expires_at() getter
//! 公开——registry 空闲实例 LRU 卸载（evict_one_idle）按到期时刻
//! 最早优先的比较键 2026-09-12 01-56
//! M196（迭代51 Q4-A/Q5-A/Q6-A 裁决 2026-09-12 18:03/18:11）：spawn_args
//! 推理优化自动注入——生成类注入 --spec-type（MTP 头在位则
//! draft-mtp,ngram-mod + n-max 2，否则 ngram-mod）+ --spec-autotune，
//! RUNTIME 显式接管时整段跳过 2026-09-12 18-17

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::{RoxidError, RoxidResult};

/// /health 就绪轮询间隔
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// /health 就绪等待总超时（大模型加载耗时较长，给足余量）
const HEALTH_READY_TIMEOUT: Duration = Duration::from_secs(300);

/// stderr 尾部环形缓存行数（M97：启动失败报错附 stderr 摘要的诊断上限；
/// 取 12 覆盖 llama-server 加载错误及其上下文行，满额滚动淘汰最早行）
const STDERR_TAIL_LINES: usize = 12;

/// 默认 slot 并行数（对齐原版 Ollama OLLAMA_NUM_PARALLEL 默认值；
/// 来源：用户确认 Q2 2026-08-24 22:10「补齐并默认开启」）
pub const DEFAULT_PARALLEL: u32 = 4;

/// slot 并行数环境变量：本项目命名惯例（优先）
pub const ENV_NUM_PARALLEL: &str = "ROXID_NUM_PARALLEL";
/// slot 并行数环境变量：原版 Ollama 惯例（回退）
pub const ENV_NUM_PARALLEL_FALLBACK: &str = "OLLAMA_NUM_PARALLEL";

/// 读取 slot 并行数配置。
/// 优先级：ROXID_NUM_PARALLEL → OLLAMA_NUM_PARALLEL → DEFAULT_PARALLEL(4)。
/// （来源：用户确认 M19-R1 2026-08-24 22:33「双变量，对齐 CLI 已兼容
/// OLLAMA_HOST 的先例，原版用户零改动」）
///
/// - 参数 roxid / ollama：两变量的当前取值（纯函数便于单测）
/// - 返回：生效的并行数（≥1）
pub fn resolve_num_parallel(roxid: Option<String>, ollama: Option<String>) -> u32 {
    roxid
        .or(ollama)
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(DEFAULT_PARALLEL)
}

/// llama-server 启动参数（由模型元数据驱动）
#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    /// llama-server 二进制路径
    pub llama_server_bin: PathBuf,
    /// 主模型 GGUF
    pub gguf: PathBuf,
    /// 多模态投影（vision 模型）
    pub mmproj: Option<PathBuf>,
    /// LoRA 适配器列表
    pub lora: Vec<PathBuf>,
    /// 每 slot 上下文窗口（下发总量 = ctx_size × parallel；默认 4096）
    pub ctx_size: u32,
    /// slot 并行数（--parallel；M19 默认 DEFAULT_PARALLEL）
    pub parallel: u32,
    /// RUNTIME 启动参数原始串（M39：modelfile RUNTIME 指令或请求级覆盖；
    /// None 表示未设置——不干预 llama-server 默认行为，--fit 自动分载生效）。
    /// 同时作为实例重建比较键（请求级 runtime 变化触发重建，对齐 D4c）
    pub runtime_flags: Option<String>,
    /// 模型元数据 RUNTIME 指令串（迭代42 D2，N-1 清偿 2026-09-11）：
    /// 实例的「默认形态」基准——与 runtime_flags 组合区分「模型自带指令」
    /// 与「请求级临时覆盖」，复用判定据此隔离覆盖实例（R1-A 裁决）
    pub model_runtime: Option<String>,
    /// 服务类别（迭代51 M193）：同类互斥换载（M194）判定键；推理优化
    /// 注入（M196）仅生成类启用。spawn_spec_from_meta 经 service_class()
    /// 单一事实源填充
    pub service_class: super::ServiceClass,
    /// GGUF 内嵌 MTP 头在位（迭代51 M196：tensor 名含 nextn.eh_proj，
    /// 对齐 llama.cpp common/speculative.cpp auto-detect 判定标志）；
    /// 投机参数组合选择的依据；读 GGUF 失败宽容 false
    pub has_mtp: bool,
}

/// 构造 llama-server 完整参数列表（纯函数，便于单测断言）。
///
/// - 参数 spec：启动参数
/// - 参数 port：监听端口
/// - 参数 alias：API 层模型别名（规范化完整名 model:tag）——llama-server
///   响应与流式分片的 model 回显点统一以此填充（官方 --alias 参数语义）
/// - 返回：argv 参数向量（不含程序名）
pub fn spawn_args(spec: &SpawnSpec, port: u16, alias: &str) -> Vec<String> {
    let per_slot_ctx = spec.ctx_size.max(512);
    let total_ctx = per_slot_ctx.saturating_mul(spec.parallel.max(1));
    let mut args: Vec<String> = vec![
        "-m".into(),
        spec.gguf.display().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        total_ctx.to_string(),
        "--parallel".into(),
        spec.parallel.max(1).to_string(),
        "--embeddings".into(),
        "--metrics".into(),
        // M100（迭代32 碴1）：API 层模型别名——透传层剥离请求 model 字段后
        // llama-server 以加载路径兜底回显（oaicompat_model），--alias 使
        // 非流式响应/流式分片/直通层 /v1/models 的 model 字段统一为
        // 规范化模型名（2026-09-10 21:22）
        "--alias".into(),
        alias.to_string(),
        // M186（迭代49）：固定启用 --jinja——llama-server 改用 GGUF 内嵌
        // 官方 Jinja 模板渲染工具调用。内建模板按家族硬编码（Qwen3 为
        // JSON 风格），与 Qwen3.5 等新模型训练分布（XML 风格
        // <function=...><parameter=...>）错位，模型输出残缺工具调用 →
        // 上游 500「Failed to parse tool call arguments as JSON」。
        // RUNTIME flags 追加段在其后，用户仍可后写覆盖（2026-09-12 07:04）
        "--jinja".into(),
    ];
    // M196（迭代51，Q4-A/Q5-A/Q6-A 裁决 2026-09-12 18:03/18:11）：
    // 推理优化自动注入——仅生成类；llama-server --spec-type 默认 none，
    // 不注入则 MTP/ngram 加速全部旁置。组合：MTP 头在位 →
    // draft-mtp,ngram-mod 并存（逗号多选）+ --spec-draft-n-max 2
    // （官方 PR #22673 推荐值）；否则 ngram-mod（官方 --spec-default 同款，
    // 无 draft 模型依赖）；均附 --spec-autotune 自动调优 tokens/sec。
    // RUNTIME 显式含 spec 类参数（--spec-type/-md/--spec-draft-model）时
    // 整段跳过——用户接管；注入段位于 RUNTIME 追加段之前，后写覆盖
    // 语义保持用户最终控制权。embedding/TTS 类零注入（投机仅对生成有意义）
    let user_takes_over_spec = match spec.runtime_flags.as_deref() {
        Some(rt) => tokenize_flags(rt).iter().any(|t| {
            t == "--spec-type"
                || t == "-md"
                || t == "--spec-draft-model"
                || t.starts_with("--spec-type=")
        }),
        None => false,
    };
    if spec.service_class == super::ServiceClass::Generation && !user_takes_over_spec {
        args.push("--spec-type".into());
        if spec.has_mtp {
            args.push("draft-mtp,ngram-mod".into());
            args.push("--spec-draft-n-max".into());
            args.push("2".into());
        } else {
            args.push("ngram-mod".into());
        }
        args.push("--spec-autotune".into());
    }
    if let Some(mmproj) = &spec.mmproj {
        args.push("--mmproj".into());
        args.push(mmproj.display().to_string());
    }
    for lora in &spec.lora {
        args.push("--lora".into());
        args.push(lora.display().to_string());
    }
    // M39：RUNTIME flags 经 shell 风格分词后追加在固定参数之后——
    // llama.cpp 后写覆盖先写，用户可覆盖 roxid 默认的 -c/--parallel 等；
    // 未设置时不追加任何 GPU 层参数（-ngl 交 llama-server/--fit 自动分载）
    if let Some(rt) = &spec.runtime_flags {
        args.extend(tokenize_flags(rt));
    }
    args
}

/// shell 风格分词（M39）：空白分隔 + 双/单引号保留含空格单值——
/// `--override-tensor "exps=CPU" -ngl 30` → 3 个 token（引号剥除、
/// exps=CPU 为单 token）。未闭合引号宽容处理（余量并入当前 token）。
///
/// - 参数 s：RUNTIME 参数串
/// - 返回：token 向量（可直接作为 argv 追加段）
pub fn tokenize_flags(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None => match c {
                '"' | '\'' => quote = Some(c),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// llama-server 子进程实例（一个活跃模型对应一个实例）
pub struct Runner {
    /// 完整模型名（model:tag），注册键与 /api/ps 展示用
    pub model_name: String,
    /// 本实例监听端口（仅绑定 127.0.0.1）
    pub port: u16,
    /// 模型占用字节数（GGUF 大小，/api/ps 展示）
    pub size: u64,
    /// 子进程 PID（nvidia-smi 进程显存查询用；进程结束后置 None）
    pid: Option<u32>,
    /// 每 slot 上下文窗口（D4c：请求级 num_ctx 与实例不一致时触发重建的比较键；
    /// 测试替身为 0）
    ctx_per_slot: u32,
    /// 生效的 RUNTIME 启动参数原始串（M39：请求级 runtime 与实例不一致时
    /// 触发重建的比较键；测试替身为 None）
    runtime_flags: Option<String>,
    /// 模型级 RUNTIME 默认串（迭代42 D2）：复用判定的「未被覆盖」基准
    model_runtime: Option<String>,
    /// 实际使用的 llama-server 二进制路径（M54b：resolve 链应然路径与实例
    /// 记录不一致时触发重建的比较键——runtime use 切换默认版本的生效点；
    /// 测试替身为空路径，比对方按「不可比对」放行）
    llama_server_bin: PathBuf,
    /// 服务类别（迭代51 M193）：同类互斥换载判定键（M194）；替身默认
    /// 生成桶（Generation），真实实例经 spawn_llama_server 记录 spec 值
    service_class: super::ServiceClass,
    /// 子进程句柄
    child: Child,
    /// keep_alive 到期时刻（绝对时间）；每次请求到达时刷新
    expires_at: Instant,
    /// 当前生效的 keep_alive 窗口（租约释放时重算起点的依据，M27 R3）
    keep_alive: Duration,
    /// 在途请求数（M27 碴1：非零表示有请求未完成，reaper 不得卸载）
    in_flight: u32,
    /// stderr 尾部环形缓存（M97：启动即退出报错携带 stderr 摘要的数据源；
    /// std Mutex——转发任务与 wait_until_healthy 均短临界区、锁内无 await）
    stderr_tail: Arc<StdMutex<VecDeque<String>>>,
    /// GPU/总层数（迭代36 M126，Q3 裁决精确口径：日志转发路径逐行解析命中即存
    /// ——环形缓存仅留 12 行，运行期日志会把层卸载行滚出，事后再读必漏；
    /// std Mutex 短临界区锁内无 await；替身进程恒 None，原因见 gpu_layers_note）
    gpu_layers: Arc<StdMutex<Option<(u32, u32)>>>,
    /// 健康检查复用的 HTTP 客户端（2s 超时）
    http: reqwest::Client,
}

impl Runner {
    /// 按启动参数拉起 llama-server 并等待 /health 就绪。
    ///
    /// - 参数 model_name：完整模型名（model:tag）
    /// - 参数 port：已分配的监听端口
    /// - 参数 keep_alive：初始空闲存活时长
    /// - 参数 spec：元数据驱动的启动参数
    /// - 返回：就绪的 Runner
    pub async fn spawn_llama_server(
        model_name: impl Into<String>,
        port: u16,
        keep_alive: Duration,
        spec: &SpawnSpec,
    ) -> RoxidResult<Self> {
        let size = std::fs::metadata(&spec.gguf)?.len();
        // M100：规范化名先行物化——同时作 spawn_with 的实例名与 --alias
        // 回显值（所有权隔离，避免 move 顺序约束）
        let name: String = model_name.into();
        let alias = name.clone();
        let mut cmd = Command::new(&spec.llama_server_bin);
        // 参数统一经 spawn_args 构造（M19 抽取为纯函数；含 M18 实测必需的
        // --embeddings/--metrics 与 M19 的 --parallel/-c 总量换算；M100 增
        // --alias API 层模型名回显）
        cmd.args(spawn_args(spec, port, &alias));
        let mut runner = Self::spawn_with(name, port, keep_alive, &mut cmd).await?;
        // 等待 /health 就绪后再交付（进程即退或超时在此暴露）
        runner.wait_until_healthy().await?;
        runner.ctx_per_slot = spec.ctx_size.max(512); // D4c：记录实例每 slot 窗口（重建比较键）
        runner.runtime_flags = spec.runtime_flags.clone(); // M39：RUNTIME 重建比较键
        runner.model_runtime = spec.model_runtime.clone(); // 迭代42 D2：默认形态基准
        runner.llama_server_bin = spec.llama_server_bin.clone(); // M54b：后端版本重建比较键
        runner.service_class = spec.service_class; // 迭代51 M193：同类换载判定键
        runner.size = size;
        Ok(runner)
    }

    /// 通用拉起：执行给定命令并立即返回（不做健康等待），
    /// 由调用方决定是否 wait_until_healthy；测试用它构造替身进程。
    ///
    /// - 参数 model_name / port / keep_alive：语义同 spawn_llama_server
    /// - 参数 cmd：已配置好参数的待执行命令
    /// - 返回：持有子进程句柄的 Runner
    pub async fn spawn_with(
        model_name: impl Into<String>,
        port: u16,
        keep_alive: Duration,
        cmd: &mut Command,
    ) -> RoxidResult<Self> {
        // 迭代18 BUG-10（M46）：孤儿防护——serve 主进程被 kill -9/panic/
        // OOM 时无 drop 机会，kill_on_drop 失效；PDEATHSIG 让内核在父进程
        // 死亡时向子进程投递 SIGKILL（Linux 标准机制）。pre_exec 在 fork
        // 后、exec 前的子进程上下文执行：仅 prctl 单调用，无分配无锁
        // （async-signal-safety 合规）；失败 best-effort 不阻断 spawn。
        // 来源：用户裁决 Q1-A（2026-09-09 05:49）；libc 依赖本次引入。
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(
                    libc::PR_SET_PDEATHSIG,
                    libc::SIGKILL as libc::c_ulong,
                    0,
                    0,
                    0,
                );
                Ok(())
            });
        }
        // stdin 必须保持打开的管道：llama-server 监听 stdin EOF 触发优雅退出，
        // Stdio::null() 会立即 EOF 导致进程秒退（实测 b10605，2026-08-24 19:25）。
        // 管道写端由 Child 持有，Runner 存活期间无 EOF；drop 时关闭即优雅退出。
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 兜底防泄漏：Runner 被 drop（如宿主异常退出路径）时自动终止子进程
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                // M99（迭代31，Q4）：ENOENT 且二进制文件在位 → 内核 execve
                // 拒载错误架构 ELF 的典型形态，读 ELF 头给出架构诊断
                //（ELF 无法判定时维持原文案，不冒进）
                if e.kind() == std::io::ErrorKind::NotFound {
                    let bin = cmd.as_std().get_program();
                    if let Some((bin_arch, host_arch)) =
                        crate::runtime::diagnose_arch_mismatch(std::path::Path::new(bin))
                    {
                        return RoxidError::ArchMismatch(format!(
                            "llama-server 为 {bin_arch} 构建，与宿主 {host_arch} 不匹配，\
                             无法执行；可 `roxid runtime rm <tag>` 清理后重装本机架构版本，\
                             或设置 ROXID_LLAMA_SERVER 指向自编译产物"
                        ));
                    }
                }
                RoxidError::RunnerFailure(format!("拉起子进程失败：{e}"))
            })?;
        // PID 在进程存活期间恒有效，spawn 后立即记录（显存查询键）；
        // child.id() 返回 Option<u32>：进程句柄存在即恒为 Some

        // 后台逐行转发子进程输出到日志，防止管道写满导致子进程阻塞
        if let Some(stdout) = child.stdout.take() {
            spawn_log_forwarder(stdout, "stdout", None, None);
        }
        // M97：stderr 同时入尾部缓存（启动失败诊断摘要数据源）
        let stderr_tail = Arc::new(StdMutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        // 迭代36 M126：stderr 层卸载行解析结果槽（转发任务命中即写入）
        let gpu_layers: Arc<StdMutex<Option<(u32, u32)>>> = Arc::new(StdMutex::new(None));
        if let Some(stderr) = child.stderr.take() {
            spawn_log_forwarder(
                stderr,
                "stderr",
                Some(stderr_tail.clone()),
                Some(gpu_layers.clone()),
            );
        }

        Ok(Self {
            model_name: model_name.into(),
            port,
            size: 0,
            pid: child.id(),
            ctx_per_slot: 0,
            runtime_flags: None, // M39：替身无 RUNTIME（真实实例经 spawn_llama_server 记录）
            model_runtime: None, // 迭代42 D2：替身无默认串（真实实例经 spawn_llama_server 记录）
            llama_server_bin: PathBuf::new(), // M54b：替身空路径（比对时视为不可比对放行）
            service_class: super::ServiceClass::Generation, // 迭代51 M193：替身默认生成桶
            child,
            expires_at: Instant::now() + keep_alive,
            keep_alive,
            in_flight: 0,
            stderr_tail,
            gpu_layers,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(|e| RoxidError::RunnerFailure(format!("构建健康检查客户端失败：{e}")))?,
        })
    }

    /// 轮询 /health 直到就绪；子进程启动即退出时立即报错。
    ///
    /// - 返回：Ok(()) 表示 /health 返回 2xx
    async fn wait_until_healthy(&mut self) -> RoxidResult<()> {
        let deadline = Instant::now() + HEALTH_READY_TIMEOUT;
        loop {
            if self.is_healthy().await {
                return Ok(());
            }
            if self.is_process_dead() {
                // M97：附带 stderr 尾部摘要——gemma4 案实证裸「启动即退出」
                // 文案无法定位真实失败原因（锁内无 await，短临界区安全）
                let tail = self
                    .stderr_tail
                    .lock()
                    .map(|q| q.iter().cloned().collect::<Vec<_>>().join(" | "))
                    .unwrap_or_default();
                return Err(RoxidError::RunnerFailure(if tail.is_empty() {
                    format!("llama-server 启动即退出：model={}", self.model_name)
                } else {
                    format!(
                        "llama-server 启动即退出：model={}；stderr 尾部：{tail}",
                        self.model_name
                    )
                }));
            }
            if Instant::now() >= deadline {
                return Err(RoxidError::RunnerFailure(format!(
                    "健康检查超时 {}s：model={}",
                    HEALTH_READY_TIMEOUT.as_secs(),
                    self.model_name
                )));
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    /// 子进程 PID（显存查询键；进程结束后为 None）。
    ///
    /// - 返回：存活子进程的 PID
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// 实例的每 slot 上下文窗口（D4c：请求级 num_ctx 重建比较键；替身为 0）。
    ///
    /// - 返回：拉起时下发的 per-slot ctx
    pub fn ctx_per_slot(&self) -> u32 {
        self.ctx_per_slot
    }

    /// GPU/总层数（迭代36 M126：/api/ps PROCESSOR 列数据源；替身恒 None）。
    ///
    /// - 返回：Some((gpu 层数, 总层数))；None = stderr 未匹配层卸载行
    pub fn gpu_layer_split(&self) -> Option<(u32, u32)> {
        self.gpu_layers.lock().ok().and_then(|g| *g)
    }

    /// 生效的 RUNTIME 启动参数串（M39：请求级 runtime 重建比较键；替身 None）。
    ///
    /// - 返回：拉起时生效的 RUNTIME 原始串
    pub fn runtime_flags(&self) -> Option<&str> {
        self.runtime_flags.as_deref()
    }

    /// 模型级 RUNTIME 默认串（迭代42 D2，N-1 清偿）——复用判定基准。
    ///
    /// - 返回：模型元数据 RUNTIME 指令串（未被请求覆盖时的生效串）
    pub fn model_runtime(&self) -> Option<&str> {
        self.model_runtime.as_deref()
    }

    /// 实例实际使用的 llama-server 二进制路径（M54b：runtime use 切换默认
    /// 版本后的重建比较键；替身为空路径，比对方按「不可比对」放行）。
    ///
    /// - 返回：拉起时的二进制路径
    pub fn llama_server_bin(&self) -> &std::path::Path {
        &self.llama_server_bin
    }

    /// 测试专用：覆写记录的二进制路径（M54b registry 单测构造路径比对
    /// 场景；生产路径经 spawn_llama_server 从 SpawnSpec 记录）。
    #[cfg(test)]
    pub fn set_llama_server_bin(&mut self, bin: PathBuf) {
        self.llama_server_bin = bin;
    }

    /// 探测 /health 是否返回 2xx。
    ///
    /// - 返回：true 表示实例健康
    pub async fn is_healthy(&self) -> bool {
        self.http
            .get(format!("http://127.0.0.1:{}/health", self.port))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// 子进程是否已退出（非阻塞探测）。
    ///
    /// - 返回：true 表示进程已结束（崩溃或自行退出）
    pub fn is_process_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// 刷新 keep_alive：请求到达时调用，续期空闲存活窗口。
    /// M27（R3）：同步记忆窗口时长，供租约释放时自响应完成时刻重算。
    ///
    /// - 参数 keep_alive：新的空闲存活时长
    pub fn refresh_keep_alive(&mut self, keep_alive: Duration) {
        self.keep_alive = keep_alive;
        self.expires_at = Instant::now() + keep_alive;
    }

    /// 在途请求数（M27 碴1：reaper 卸载前置条件——零才可卸载）。
    ///
    /// - 返回：当前持有租约的请求数
    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// 请求进入：在途计数加一（acquire 发放租约时调用）。
    pub(crate) fn enter_request(&mut self) {
        self.in_flight += 1;
    }

    /// 请求离开：在途计数减一并自当前时刻重计空闲窗口。
    /// M27（碴1，R3 裁决 2026-08-26 21:11）：keep_alive 自响应完成时刻起算，
    /// 对齐原版 Ollama「请求完成后才计空闲」语义。
    pub(crate) fn leave_request(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.expires_at = Instant::now() + self.keep_alive;
    }

    /// 服务类别（迭代51 M193：registry evict_same_class_idle 同类互斥
    /// 换载的判定键）。
    pub(crate) fn service_class(&self) -> super::ServiceClass {
        self.service_class
    }

    /// 测试辅助：替身实例覆写服务类别（真实实例经 spawn_llama_server
    /// 从 spec 记录，测试替身默认生成桶，构造异类/在途场景用）。
    #[cfg(test)]
    pub(crate) fn override_service_class_for_test(&mut self, class: super::ServiceClass) {
        self.service_class = class;
    }

    /// keep_alive 到期时刻（M170：registry evict_one_idle 的 LRU 比较键——
    /// 到期时刻最早 = 空闲起点最早 = 最久未使用）。
    ///
    /// - 返回：到期绝对时刻
    pub(crate) fn expires_at(&self) -> Instant {
        self.expires_at
    }

    /// 空闲是否已到期（到期即可被卸载）。
    ///
    /// - 返回：true 表示已超过 keep_alive 窗口
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    /// keep_alive 剩余时长（/api/ps expires_at 换算用）。
    ///
    /// - 返回：剩余时长（已到期则为 0）
    pub fn keep_alive_remaining(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }

    /// 关闭实例：kill 后 wait，确保进程资源回收（消耗 self 的便捷版本）。
    ///
    /// - 返回：Ok(()) 表示进程已终止
    pub async fn shutdown(mut self) -> RoxidResult<()> {
        self.shutdown_mut().await
    }

    /// 关闭实例的借用版本（注册表经 MutexGuard 调用时使用）。
    /// M26（迭代6 P0碴1 前置）：进程已自行退出（崩溃）时跳过 kill——对已回收
    /// 进程 kill 必报 InvalidInput；仅 wait 回收 zombie 后正常返回。
    ///
    /// - 返回：Ok(()) 表示进程已终止并回收
    pub async fn shutdown_mut(&mut self) -> RoxidResult<()> {
        if !self.is_process_dead() {
            self.child.kill().await.map_err(|e| {
                RoxidError::RunnerFailure(format!(
                    "杀掉子进程失败（model={}）：{e}",
                    self.model_name
                ))
            })?;
        }
        self.child.wait().await.map_err(|e| {
            RoxidError::RunnerFailure(format!("回收子进程失败（model={}）：{e}", self.model_name))
        })?;
        self.pid = None; // 进程已回收，PID 不再可用于显存查询
        Ok(())
    }
}

/// 实例租约（M27 碴1，R1 裁决：三套 API 全部 acquire 调用点统一持有）。
///
/// RAII 语义：acquire 发放时在途计数已加一；Drop 时计数减一并自当前时刻
/// 重计 keep_alive 窗口（R3：响应完成时刻起算，对齐原版「请求完成后才计空闲」）。
/// 非流式 handler 持有至函数返回；流式响应将租约移入 body 流状态/闭包，
/// 流耗尽或客户端断开丢弃 body 时释放——请求全程实例受保护。
pub struct RunnerLease {
    /// 目标实例句柄（与注册表同源）
    runner: Arc<Mutex<Runner>>,
}

impl std::fmt::Debug for RunnerLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerLease").finish_non_exhaustive()
    }
}

impl RunnerLease {
    /// 构造租约（在途计数加一已由 acquire 侧完成）。
    ///
    /// - 参数 runner：实例句柄
    pub fn new(runner: Arc<Mutex<Runner>>) -> Self {
        Self { runner }
    }

    /// 实例句柄（测试与需直接访问实例状态的调用方使用）。
    ///
    /// - 返回：底层 Arc<Mutex<Runner>> 引用
    pub fn runner(&self) -> &Arc<Mutex<Runner>> {
        &self.runner
    }

    /// 本实例监听端口（请求转发目标）。
    ///
    /// - 返回：127.0.0.1 端口号
    pub async fn port(&self) -> u16 {
        self.runner.lock().await.port
    }
}

impl Drop for RunnerLease {
    fn drop(&mut self) {
        // 快路径：锁空闲则同步完成释放与窗口重算（绝大多数场景）
        if let Ok(mut guard) = self.runner.try_lock() {
            guard.leave_request();
            return;
        }
        // 慢路径：锁被在途访问占用。有 runtime 上下文（axum/hyper 任务内，
        // 绝大多数）交还运行时延迟释放（Drop 内禁止阻塞 runtime 线程）；
        // M30 碴8（R4-A）：无 runtime 上下文（独立 std 线程等罕见路径）经
        // blocking_lock 兜底完成释放——原跳过使 in_flight 永不归零、
        // reaper 永不卸载该实例。blocking_lock 仅允许在非 runtime 线程调用，
        // 分支条件（try_current 失败）即其合法前置。
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let runner = self.runner.clone();
                handle.spawn(async move {
                    runner.lock().await.leave_request();
                });
            }
            Err(_) => {
                self.runner.blocking_lock().leave_request();
            }
        }
    }
}

/// nvidia-smi 进程显存查询总超时（防驱动卡顿拖慢 /api/ps）
const VRAM_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// 查询全部 GPU 进程显存：nvidia-smi 一次性输出 → {PID → 显存字节} 映射。
/// 单次调用一次外部命令，多实例共享同一次查询结果。
///
/// - 返回：pid → 显存字节映射；无 nvidia-smi / 超时 / 查询失败返回空映射
///   （空映射使 size_vram 落 0，与原版 Ollama CPU 加载语义一致）
pub async fn gpu_process_vram_map() -> HashMap<u32, u64> {
    let queried = tokio::time::timeout(VRAM_QUERY_TIMEOUT, async {
        tokio::process::Command::new("nvidia-smi")
            .args([
                "--query-compute-apps=pid,used_memory",
                "--format=csv,noheader,nounits",
            ])
            .output()
            .await
    })
    .await;
    match queried {
        Ok(Ok(out)) if out.status.success() => {
            parse_vram_csv(&String::from_utf8_lossy(&out.stdout))
        }
        _ => HashMap::new(),
    }
}

/// nvidia-smi csv 输出解析（"  pid, used_mib" 行 → pid→字节；脏行跳过）。
///
/// - 参数 text：nvidia-smi stdout 全文
/// - 返回：pid → 显存字节（MiB×1048576）映射
fn parse_vram_csv(text: &str) -> HashMap<u32, u64> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let Some((pid, mib)) = line.split_once(',') else {
            continue;
        };
        let (Ok(pid), Ok(mib)) = (pid.trim().parse::<u32>(), mib.trim().parse::<u64>()) else {
            continue;
        };
        map.insert(pid, mib * 1024 * 1024);
    }
    map
}

/// 后台逐行转发子进程输出到日志，防止管道写满导致子进程阻塞。
/// M97：tail 非 None 时逐行同步入环形缓存（stderr 诊断摘要数据源；
/// 满额滚动淘汰最早行）。
/// 迭代36 M126：gpu_layers 非 None 时逐行解析层卸载行命中即存（Q3 精确口径）。
///
/// - 参数 stream：stdout / stderr 任意一方
/// - 参数 stream_name：日志标注用的流名
/// - 参数 tail：尾部缓存（stdout 传 None，stderr 传 Some）
/// - 参数 gpu_layers：层卸载解析结果槽（stdout 传 None，stderr 传 Some）
fn spawn_log_forwarder<R>(
    stream: R,
    stream_name: &'static str,
    tail: Option<Arc<StdMutex<VecDeque<String>>>>,
    gpu_layers: Option<Arc<StdMutex<Option<(u32, u32)>>>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // 迭代42 D3（N-5 清偿，R2-A 裁决 2026-09-11 21:00）：stderr
            // 提级 info——GPU 分载（--fit）详情与异常诊断默认终端可见
            //（原全量 debug 不可见，排查两眼一抹黑）；stdout 推理内容流
            // 维持 debug（噪音大，无诊断价值）
            if stream_name == "stderr" {
                tracing::info!("[llama-server:{stream_name}] {line}");
            } else {
                tracing::debug!("[llama-server:{stream_name}] {line}");
            }
            if let Some(t) = &tail {
                if let Ok(mut q) = t.lock() {
                    if q.len() == STDERR_TAIL_LINES {
                        q.pop_front();
                    }
                    q.push_back(line.clone());
                }
            }
            // 迭代36 M126：命中层卸载行即写入结果槽（首条生效，后续重复行幂等）
            if let Some(g) = &gpu_layers {
                if let Some(split) = parse_gpu_layer_line(&line) {
                    if let Ok(mut slot) = g.lock() {
                        *slot = Some(split);
                    }
                }
            }
        }
    });
}

/// 解析 stderr 行中的 llama-server 层卸载信息（迭代36 M126，Q3 裁决）。
/// 目标形态（b10xxx 族）：`load_tensors: offloaded 33/41 layers to GPU`
/// ——含 GPU 数与总数，可直接换算百分比；老形态 `offloading 60 layers`
/// 无总数不可换算，不采纳（按 Q3「失败直书原因」由调用方输出 note）。
/// 手写定位解析，不引入 regex 依赖；任一环节不匹配返回 None。
///
/// - 参数 line：子进程 stderr 单行
/// - 返回：Some((gpu 层数, 总层数))；total 为 0 或 gpu > total 的畸形行返回 None
pub(crate) fn parse_gpu_layer_line(line: &str) -> Option<(u32, u32)> {
    let idx = line.find("offloaded ")?;
    let rest = &line[idx + "offloaded ".len()..];
    let end = rest.find(" layers")?;
    let (gpu, total) = rest[..end].split_once('/')?;
    let gpu: u32 = gpu.trim().parse().ok()?;
    let total: u32 = total.trim().parse().ok()?;
    (total > 0 && gpu <= total).then_some((gpu, total))
}

/// 测试辅助：健康替身返回 200 的 python 单行服务
#[cfg(test)]
pub(crate) const HEALTH_STUB: &str = r#"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 迭代36 M126：层卸载行解析（Q3 精确口径）——b10xxx 新形态含总数
    /// 可换算百分比；老形态无总数、畸形行、无关行均拒绝
    #[test]
    fn parse_gpu_layer_line_forms() {
        assert_eq!(
            parse_gpu_layer_line("load_tensors: offloaded 33/41 layers to GPU"),
            Some((33, 41))
        );
        assert_eq!(
            parse_gpu_layer_line("load_tensors: offloaded 41/41 layers to GPU"),
            Some((41, 41))
        );
        // 老形态无总数：不可换算百分比 → 拒绝（由调用方输出 note）
        assert_eq!(parse_gpu_layer_line("offloading 60 layers to GPU"), None);
        // 畸形与无关行
        assert_eq!(parse_gpu_layer_line("offloaded 45/41 layers to GPU"), None);
        assert_eq!(parse_gpu_layer_line("offloaded x/41 layers to GPU"), None);
        assert_eq!(
            parse_gpu_layer_line("llm_load_print_meta: n_layer = 36"),
            None
        );
        assert_eq!(parse_gpu_layer_line(""), None);
    }

    /// 进程管理：sleep 替身的存活与关闭
    #[tokio::test]
    async fn spawn_and_shutdown_sleep_process() {
        let mut cmd = Command::new("sleep");
        cmd.arg("300");
        let mut runner = Runner::spawn_with("test:sleep", 0, Duration::from_secs(60), &mut cmd)
            .await
            .unwrap();
        assert!(!runner.is_process_dead());
        runner.shutdown().await.unwrap();
    }

    /// keep_alive 语义：到期判定与请求续期
    #[tokio::test]
    async fn keep_alive_expiry_and_refresh() {
        let mut cmd = Command::new("sleep");
        cmd.arg("300");
        let mut runner = Runner::spawn_with("test:ka", 0, Duration::from_millis(50), &mut cmd)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(runner.is_expired(), "空闲超过 50ms 必须判定到期");
        runner.refresh_keep_alive(Duration::from_secs(60));
        assert!(!runner.is_expired(), "续期后必须未到期");
        assert!(runner.keep_alive_remaining() > Duration::from_secs(50));
        runner.shutdown().await.unwrap();
    }

    /// M27（碴1）：租约 RAII——Drop 后在途归零且窗口自释放时刻重算（R3）
    #[tokio::test]
    async fn lease_drop_releases_inflight_and_renews_window() {
        let mut cmd = Command::new("sleep");
        cmd.arg("300");
        let runner = Runner::spawn_with("test:lease", 0, Duration::from_millis(60), &mut cmd)
            .await
            .unwrap();
        let arc = Arc::new(Mutex::new(runner));
        {
            arc.lock().await.enter_request();
            let _lease = RunnerLease::new(arc.clone());
            assert_eq!(arc.lock().await.in_flight(), 1);
        } // 租约 Drop：计数减一 + 窗口自当前时刻重算
        assert_eq!(arc.lock().await.in_flight(), 0, "释放后计数必须归零");
        assert!(!arc.lock().await.is_expired(), "窗口自释放时刻重算（R3）");
        arc.lock().await.shutdown_mut().await.unwrap();
    }

    /// M30 碴8（R4-A）：非 runtime 上下文的 Drop 慢路径经 blocking_lock 兜底
    /// ——锁被占用场景下 in_flight 仍必须归零（原跳过使 reaper 永不卸载）
    #[tokio::test]
    async fn lease_drop_off_runtime_releases_inflight() {
        let mut cmd = Command::new("sleep");
        cmd.arg("300");
        let runner = Runner::spawn_with("t:offrt", 0, Duration::from_secs(60), &mut cmd)
            .await
            .unwrap();
        let arc = Arc::new(Mutex::new(runner));
        arc.lock().await.enter_request();

        // 线程 A（std 线程持锁 200ms 制造 try_lock 失败）与线程 B（std 线程
        // Drop 租约：try_lock 失败 → try_current 失败 → blocking_lock 等待 A
        // 释放后完成 leave_request）
        let holder = arc.clone();
        let a = std::thread::spawn(move || {
            let guard = holder.blocking_lock();
            std::thread::sleep(Duration::from_millis(200));
            drop(guard);
        });
        std::thread::sleep(Duration::from_millis(50)); // 等 A 持有锁
        let dropper = arc.clone();
        let b = std::thread::spawn(move || {
            drop(RunnerLease::new(dropper));
        });
        b.join().unwrap();
        a.join().unwrap();
        assert_eq!(
            arc.lock().await.in_flight(),
            0,
            "M30 碴8：非 runtime Drop 必须完成释放（不得泄漏在途计数）"
        );
        arc.lock().await.shutdown_mut().await.unwrap();
    }

    /// 健康检查：HTTP 200 替身 → true；关闭后 → false
    #[tokio::test]
    async fn health_check_against_http_stub() {
        let port = 38099u16;
        let mut cmd = Command::new("python3");
        cmd.arg("-c").arg(HEALTH_STUB).arg(port.to_string());
        let runner = Runner::spawn_with("test:health", port, Duration::from_secs(60), &mut cmd)
            .await
            .unwrap();
        let mut ok = false;
        for _ in 0..20 {
            if runner.is_healthy().await {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ok, "HTTP 200 替身必须探测为健康");
        runner.shutdown().await.unwrap();
    }

    /// 进程退出探测：立即退出的进程（true 命令）必须被识别为 dead
    #[tokio::test]
    async fn dead_process_detection() {
        let mut cmd = Command::new("true");
        let mut runner = Runner::spawn_with("test:dead", 0, Duration::from_secs(60), &mut cmd)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(runner.is_process_dead(), "已退出进程必须判定为 dead");
    }

    /// M26（碴1 前置）：对已退出进程 shutdown_mut 必须成功（跳过 kill 仅回收），
    /// 且 PID 清空——死实例清理路径的前提条件
    #[tokio::test]
    async fn shutdown_succeeds_on_already_dead_process() {
        let mut cmd = Command::new("true");
        let mut runner =
            Runner::spawn_with("test:dead-shutdown", 0, Duration::from_secs(60), &mut cmd)
                .await
                .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(runner.is_process_dead(), "前置：进程必须已退出");
        runner
            .shutdown_mut()
            .await
            .expect("对已退出进程关闭必须成功（不触发 kill 报错）");
        assert_eq!(runner.pid(), None, "回收后 PID 必须清空");
    }

    /// PID 生命周期：拉起后可查、关闭后清空
    #[tokio::test]
    async fn pid_lifecycle() {
        let mut cmd = Command::new("sleep");
        cmd.arg("300");
        let mut runner = Runner::spawn_with("test:pid", 0, Duration::from_secs(60), &mut cmd)
            .await
            .unwrap();
        let pid = runner.pid().expect("存活进程必须有 PID");
        assert!(pid > 0);
        // shutdown_mut 为借用版本（shutdown 消耗 self，无法在关闭后继续断言）
        runner.shutdown_mut().await.unwrap();
        assert_eq!(runner.pid(), None, "进程回收后 PID 必须清空");
    }

    /// nvidia-smi csv 解析：常规行（含空白缩进）、脏行与 N/A 跳过、MiB→字节换算
    #[test]
    fn vram_csv_parsing() {
        let m = parse_vram_csv("  1234, 2048\n\t5678,\t\t64\nbad,line\n9999, N/A\n");
        assert_eq!(m.get(&1234), Some(&(2048u64 * 1024 * 1024)));
        assert_eq!(m.get(&5678), Some(&(64u64 * 1024 * 1024)));
        assert!(!m.contains_key(&9999), "N/A 行必须跳过");
    }

    /// 真实 nvidia-smi 查询（ignored：依赖 NVIDIA 驱动环境，手动验收）
    #[tokio::test]
    #[ignore = "真实环境验收：nvidia-smi 输出全量 GPU 进程映射"]
    async fn real_vram_map_queries() {
        let m = gpu_process_vram_map().await;
        eprintln!("当前 GPU 计算进程数：{}", m.len());
    }

    /// M19：并行数解析优先级（ROXID 优先 → OLLAMA 回退 → 默认 4 → 非法回退）
    #[test]
    fn num_parallel_resolution_priority() {
        assert_eq!(
            resolve_num_parallel(Some("8".into()), Some("2".into())),
            8,
            "ROXID 优先"
        );
        assert_eq!(
            resolve_num_parallel(None, Some("2".into())),
            2,
            "回退 OLLAMA"
        );
        assert_eq!(resolve_num_parallel(None, None), DEFAULT_PARALLEL, "缺省 4");
        assert_eq!(
            resolve_num_parallel(Some(" 16 ".into()), None),
            16,
            "容忍空白"
        );
        assert_eq!(
            resolve_num_parallel(Some("abc".into()), None),
            DEFAULT_PARALLEL,
            "非法回退默认"
        );
        assert_eq!(
            resolve_num_parallel(Some("0".into()), None),
            DEFAULT_PARALLEL,
            "0 拒绝（≥1）"
        );
    }

    /// M19：spawn_args 必须含 --parallel 且 -c 为 per-slot × parallel 总量
    #[test]
    fn spawn_args_parallel_and_total_ctx() {
        let spec = SpawnSpec {
            runtime_flags: None, // M39：本用例不覆盖 RUNTIME（追加段另测）
            model_runtime: None, // 迭代42 D2：本用例无模型默认串
            llama_server_bin: PathBuf::from("/bin/llama-server"),
            gguf: PathBuf::from("/models/m.gguf"),
            mmproj: None,
            lora: vec![],
            ctx_size: 2048,
            parallel: 4,
            // 迭代51 M196：默认生成类 + 无 MTP（注入段断言见注入矩阵用例）
            service_class: crate::scheduler::ServiceClass::Generation,
            has_mtp: false,
        };
        let args = spawn_args(&spec, 32141, "m:latest");
        let idx = |k: &str| args.iter().position(|a| a == k).unwrap();
        assert_eq!(args[idx("-c") + 1], "8192", "-c 必须为 2048×4 总量");
        assert_eq!(args[idx("--parallel") + 1], "4");
        assert!(args.contains(&"--embeddings".to_string()));
        assert!(args.contains(&"--metrics".to_string()));
        // M100：--alias API 层模型名回显（值 = 规范化完整名）
        assert_eq!(args[idx("--alias") + 1], "m:latest");
        // M186（迭代49）：--jinja 固定启用（GGUF 内嵌官方模板渲染工具调用）
        assert!(args.contains(&"--jinja".to_string()));
        // ctx 下限保护：0 → 512×parallel
        let mut low = spec.clone();
        low.ctx_size = 0;
        assert_eq!(
            spawn_args(&low, 1, "m:latest")[idx("-c") + 1],
            "2048",
            "512×4 下限"
        );
    }

    /// M39：tokenize_flags——双/单引号值保留为单 token、空白分隔、
    /// 连续空白跳过、未闭合引号宽容并入
    #[test]
    fn runtime_flags_tokenization() {
        assert_eq!(
            tokenize_flags(r#"--flash-attn --override-tensor "exps=CPU" -ngl 30"#),
            vec![
                "--flash-attn",
                "--override-tensor",
                "exps=CPU",
                "-ngl",
                "30"
            ]
        );
        assert_eq!(
            tokenize_flags("--ctk 'q8_0'   --ctv   q8_0"),
            vec!["--ctk", "q8_0", "--ctv", "q8_0"],
            "连续空白压缩，单引号同语义"
        );
        assert!(tokenize_flags("   ").is_empty());
        assert_eq!(
            tokenize_flags("--unclosed \"still one"),
            vec!["--unclosed", "still one"],
            "未闭合引号宽容处理"
        );
    }

    /// M39：RUNTIME flags 追加在固定参数之后（后写覆盖语义的用户控制面）；
    /// 未设置时零追加（--fit 自动分载不干预）
    #[test]
    fn spawn_args_appends_runtime_flags_last() {
        let spec = SpawnSpec {
            llama_server_bin: PathBuf::from("/bin/llama-server"),
            gguf: PathBuf::from("/models/m.gguf"),
            mmproj: None,
            lora: vec![],
            ctx_size: 2048,
            parallel: 4,
            runtime_flags: Some(r#"-ngl 30 --override-tensor "exps=CPU" --no-mmap"#.into()),
            model_runtime: None, // 迭代42 D2：请求级覆盖场景（默认串不参与 spawn_args）
            service_class: crate::scheduler::ServiceClass::Generation, // 迭代51 M196 补齐
            has_mtp: false,
        };
        let args = spawn_args(&spec, 32141, "m:latest");
        // 追加段必须位于末尾（llama.cpp 后写覆盖先写——用户可覆盖 -c/--parallel；
        // M196 兼容：生成类注入段在其之前，尾部 5 token 仍为 RUNTIME 段）
        assert_eq!(
            args[args.len() - 5..],
            vec!["-ngl", "30", "--override-tensor", "exps=CPU", "--no-mmap"]
        );
        // 固定段不受影响（-m 与 -c 仍在）
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-m" && w[1] == "/models/m.gguf"));
    }

    /// M196（迭代51）：投机参数注入矩阵——无 MTP 生成类 / MTP 组合 /
    /// RUNTIME 接管跳过 / embedding 与 TTS 零注入。
    /// 来源：Q4-A/Q5-A/Q6-A 裁决 2026-09-12 18:03/18:11。
    #[test]
    fn spawn_args_speculative_injection_matrix() {
        let base = || SpawnSpec {
            llama_server_bin: PathBuf::from("/bin/llama-server"),
            gguf: PathBuf::from("/models/m.gguf"),
            mmproj: None,
            lora: vec![],
            ctx_size: 2048,
            parallel: 4,
            runtime_flags: None,
            model_runtime: None,
            service_class: crate::scheduler::ServiceClass::Generation,
            has_mtp: false,
        };
        let idx = |args: &[String], k: &str| args.iter().position(|a| a == k).unwrap();

        // 无 MTP 生成类：ngram-mod + autotune，无 draft-mtp 段
        let args = spawn_args(&base(), 1, "m:latest");
        assert_eq!(args[idx(&args, "--spec-type") + 1], "ngram-mod");
        assert!(args.contains(&"--spec-autotune".to_string()));
        assert!(!args.contains(&"draft-mtp".to_string()));
        assert!(!args.contains(&"--spec-draft-n-max".to_string()));

        // 有 MTP：draft-mtp,ngram-mod 并存 + n-max 2 + autotune
        let mut mtp = base();
        mtp.has_mtp = true;
        let args = spawn_args(&mtp, 1, "m:latest");
        assert_eq!(args[idx(&args, "--spec-type") + 1], "draft-mtp,ngram-mod");
        assert_eq!(args[idx(&args, "--spec-draft-n-max") + 1], "2");
        assert!(args.contains(&"--spec-autotune".to_string()));

        // RUNTIME 显式 --spec-type：自动段整段跳过（用户接管）
        let mut takeover = base();
        takeover.runtime_flags = Some("--spec-type none".into());
        let args = spawn_args(&takeover, 1, "m:latest");
        assert_eq!(
            args.iter().filter(|a| *a == "--spec-type").count(),
            1,
            "仅用户那一份 --spec-type"
        );
        assert_eq!(args[idx(&args, "--spec-type") + 1], "none", "用户值生效");
        assert!(!args.contains(&"--spec-autotune".to_string()));

        // RUNTIME 显式 -md（draft 模型接管）：同样跳过
        let mut draft = base();
        draft.runtime_flags = Some("-md /draft.gguf".into());
        assert!(!spawn_args(&draft, 1, "m:latest").contains(&"--spec-autotune".to_string()));

        // embedding / TTS 类：零投机注入
        let mut embed = base();
        embed.service_class = crate::scheduler::ServiceClass::Embedding;
        let args = spawn_args(&embed, 1, "m:latest");
        assert!(!args.contains(&"--spec-type".to_string()));
        assert!(!args.contains(&"--spec-autotune".to_string()));
        let mut tts = base();
        tts.service_class = crate::scheduler::ServiceClass::Tts;
        assert!(!spawn_args(&tts, 1, "m:latest").contains(&"--spec-type".to_string()));
    }
}
