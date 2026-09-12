//! 推理调度层：llama-server Runner 生命周期与请求编排。
//!
//! - M3（本阶段）：Runner 原语——子进程拉起/关闭、动态端口、
//!   /health 就绪与探测、keep_alive 到期语义、输出转发
//! - M6（后续）：Runner 注册表、同模型并行 slots 复用、跨模型多实例、
//!   请求排队、定时卸载循环、/api/ps、stop
//!
//! 修改历史：占位 2026-08-24 18:35；M3 实装 2026-08-24 19:08；
//! M27 导出 RunnerLease（迭代7 P0碴1 在途保护租约）2026-08-26 21-24；
//! M193 ServiceClass 服务类别判定层（迭代51 同类换载，词根规则动态
//! 归组——用户裁决 2026-09-12 18:03「llama.cpp支持多少就分多少类，
//! 动态」）2026-09-12 18-16

mod registry;
mod runner;

pub use registry::{RunnerRegistry, RunningModel, DEFAULT_KEEP_ALIVE};
pub use runner::{
    gpu_process_vram_map, resolve_num_parallel, spawn_args, Runner, RunnerLease, SpawnSpec,
    DEFAULT_PARALLEL, ENV_NUM_PARALLEL, ENV_NUM_PARALLEL_FALLBACK,
};

use std::sync::atomic::{AtomicU16, Ordering};

/// 服务类别（迭代51 M193）：按 GGUF architecture 词根动态归组——类别
/// 集合经词根判定开放扩展（llama.cpp 新增能力类别时增补词根即扩展，
/// 不设类别数上限）；未命中任何词根的家族默认落生成桶。同类互斥换载
/// （M194）与推理优化注入（M196，仅生成类）的判定键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServiceClass {
    /// 文本生成（chat/generate/completions——默认桶）
    #[default]
    Generation,
    /// 向量/embedding（bert/bge/embed 词根族）
    Embedding,
    /// 语音合成 TTS（kokoro/piper/vits/tts 词根族）
    Tts,
}

/// embedding 模型家族判定（M28 碴9 词根集；迭代51 M193 迁入调度层共享
/// ——api capabilities 与 scheduler 同类换载的单一事实源，原 api 层
/// 私有实现删除防漂移）。
///
/// - 参数 family：GGUF general.architecture 家族名
/// - 返回：true 表示 embedding 模型
pub fn is_embedding_family(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    f.contains("embed")
        || f == "bert"
        || f.starts_with("bert-")
        || f.contains("-bert")
        || f.starts_with("bge")
        || f.contains("-bge")
}

/// TTS 模型家族判定（迭代51 M193）：kokoro（llama.cpp 现役 TTS 架构）
/// 以及 piper/vits/tts 词根（前瞻覆盖 llama.cpp 后续 TTS 架构命名——
/// 动态归组语义，新词根即新覆盖面）；纯生成与 embedding 家族不含这些
/// 词根。
///
/// - 参数 family：GGUF general.architecture 家族名
/// - 返回：true 表示 TTS 模型
pub fn is_tts_family(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    f.contains("kokoro") || f.contains("piper") || f.contains("vits") || f.contains("tts")
}

/// 家族 → 服务类别（迭代51 M193 单一事实源：M194 同类换载与 M196
/// 推理优化注入均经此判定，禁止旁路另写词根）。
///
/// - 参数 family：GGUF general.architecture 家族名（model.json family）
/// - 返回：服务类别（未知家族默认生成桶）
pub fn service_class(family: &str) -> ServiceClass {
    if is_embedding_family(family) {
        ServiceClass::Embedding
    } else if is_tts_family(family) {
        ServiceClass::Tts
    } else {
        ServiceClass::Generation
    }
}

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

    /// 迭代51 M193：服务类别判定词根矩阵——embedding 词根族（M28 碴9
    /// 原矩阵迁入）、TTS 词根族、生成默认桶边界（未知/纯生成家族）。
    /// 来源：用户裁决 2026-09-12 18:03「llama.cpp支持多少就分多少类，动态」。
    #[test]
    fn service_class_word_root_matrix() {
        // embedding 词根族（含 bert 系漏判修复语义迁入）
        for f in [
            "nomic-embed-text",
            "nomic-bert",
            "jina-bert-v2",
            "bert",
            "bert-uncased",
            "bge-m3",
            "x-bge",
        ] {
            assert_eq!(
                service_class(f),
                ServiceClass::Embedding,
                "{f} 须归 embedding"
            );
        }
        // TTS 词根族（kokoro 现役 + piper/vits/tts 前瞻）
        for f in ["kokoro", "piper", "vits", "f5-tts", "kokoro-82m"] {
            assert_eq!(service_class(f), ServiceClass::Tts, "{f} 须归 TTS");
        }
        // 生成默认桶（未知/纯生成家族零误判）
        for f in [
            "llama",
            "qwen2",
            "gemma",
            "glm4",
            "gptoss",
            "deepseek2",
            "totally-unknown",
        ] {
            assert_eq!(service_class(f), ServiceClass::Generation, "{f} 须归生成");
        }
    }
}
