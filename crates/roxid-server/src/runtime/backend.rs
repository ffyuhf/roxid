//! GPU 后端探测（来源：用户确认 2026-08-24 18:51 Q10 方案 B）。
//!
//! 实测事实（2026-08-24 18:50，b10605 全部 27 资产枚举）：
//! Linux 官方预编译无 CUDA 包（CUDA 仅 Windows），
//! 因此 NVIDIA/AMD/Intel GPU 统一经 Vulkan 后端加速；
//! 需要原生 CUDA 时用户可设 ROXID_LLAMA_SERVER 指向自编译产物（见 download 模块）。
//!
//! 修改历史：M2 新增 2026-08-24 18:56（原因：运行时管理里程碑）；
//! M27 探测缓存（迭代7 P0碴2 组成）：OnceLock 一次性记忆探测结果，
//! 消除每次冷加载的重复外部命令阻塞 2026-08-26 21-35

use std::process::Command;

/// llama.cpp 运行时后端（对应官方预编译包变体）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// GPU：经 Vulkan（NVIDIA 驱动 / AMD / Intel 均原生支持）
    Vulkan,
    /// 纯 CPU 回退
    Cpu,
}

impl Backend {
    /// 官方 release 资产名中的变体片段（不含 .tar.gz 后缀）
    ///
    /// - 返回：资产名片段，如 "ubuntu-vulkan-x64"
    pub fn asset_variant(self) -> &'static str {
        match self {
            Backend::Vulkan => "ubuntu-vulkan-x64",
            Backend::Cpu => "ubuntu-x64",
        }
    }
}

/// 探测本机 GPU（结果进程内一次性缓存）。
/// M27（碴2）：OnceLock 记忆首次探测结果——外部命令仅首问执行一次，
/// 后续冷加载零探测开销（与加载移出全局写锁叠加，锁持有时长收敛）。
///
/// - 返回：Backend，探测永不失败，最差回退 Cpu
pub fn detect_backend() -> Backend {
    static CACHED: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();
    *CACHED.get_or_init(probe_backend)
}

/// 真实探测（仅首次执行）：nvidia-smi 或 vulkaninfo 任一可运行即采用 Vulkan 包。
///
/// - 返回：Backend
fn probe_backend() -> Backend {
    if program_succeeds("nvidia-smi") || program_succeeds_with_summary("vulkaninfo") {
        Backend::Vulkan
    } else {
        Backend::Cpu
    }
}

/// 外部程序裸跑退出码 0 即视为可用（命令不存在同样返回 false）
fn program_succeeds(program: &str) -> bool {
    Command::new(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// vulkaninfo 用 --summary 探测，避免全量输出耗时
fn program_succeeds_with_summary(program: &str) -> bool {
    Command::new(program)
        .arg("--summary")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
