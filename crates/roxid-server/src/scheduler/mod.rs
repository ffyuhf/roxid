//! 推理调度层：llama-server Runner 生命周期与请求编排。
//!
//! - M3（本阶段）：Runner 原语——子进程拉起/关闭、动态端口、
//!   /health 就绪与探测、keep_alive 到期语义、输出转发
//! - M6（后续）：Runner 注册表、同模型并行 slots 复用、跨模型多实例、
//!   请求排队、定时卸载循环、/api/ps、stop
//!
//! 修改历史：占位 2026-08-24 18:35；M3 实装 2026-08-24 19:08；
//! M27 导出 RunnerLease（迭代7 P0碴1 在途保护租约）2026-08-26 21-24

mod registry;
mod runner;

pub use registry::{RunnerRegistry, RunningModel, DEFAULT_KEEP_ALIVE};
pub use runner::{
    gpu_process_vram_map, resolve_num_parallel, spawn_args, Runner, RunnerLease, SpawnSpec,
    DEFAULT_PARALLEL, ENV_NUM_PARALLEL, ENV_NUM_PARALLEL_FALLBACK,
};

use std::sync::atomic::{AtomicU16, Ordering};

/// 动态端口分配起始值（避开常用服务端口段；llama-server 绑定失败会以
/// "启动即退出"暴露，M6 注册表层做换端口重试）
pub const RUNNER_PORT_BASE: u16 = 32141;

/// 全局端口分配游标
static NEXT_PORT: AtomicU16 = AtomicU16::new(RUNNER_PORT_BASE);

/// 分配下一个可用 runner 端口：顺序游标 + 绑定探测跳过已占端口
/// （防进程重启后撞上残留实例，实测踩坑 2026-08-24 19:27）。
///
/// - 返回：经绑定验证可用的端口号
pub fn alloc_port() -> u16 {
    loop {
        let candidate = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        if candidate > 65000 {
            // 游标越界回卷到起始段（极多实例的极端场景）
            NEXT_PORT.store(RUNNER_PORT_BASE, Ordering::Relaxed);
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端口分配必须返回可绑定端口且不重复。
    /// bind 验证带短重试：探测 socket 关闭与再绑定之间存在偶发内核延迟窗口
    /// （2026-08-24 迭代2 回归实测，AddrInUse 偶发两现，重试后稳定）。
    #[test]
    fn port_allocation_returns_bindable_unique() {
        let a = alloc_port();
        let b = alloc_port();
        assert_ne!(a, b, "连续分配不得重复");
        let mut bindable = false;
        for _ in 0..50 {
            if std::net::TcpListener::bind(("127.0.0.1", a)).is_ok() {
                bindable = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(bindable, "分配端口必须可绑定（含重试）");
    }
}
