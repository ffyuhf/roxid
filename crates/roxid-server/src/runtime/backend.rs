//! GPU 后端探测与宿主架构感知（来源：用户确认 2026-08-24 18:51 Q10 方案 B）。
//!
//! 实测事实（2026-08-24 18:50，b10605 全部 27 资产枚举）：
//! Linux 官方预编译无 CUDA 包（CUDA 仅 Windows），
//! 因此 NVIDIA/AMD/Intel GPU 统一经 Vulkan 后端加速；
//! 需要原生 CUDA 时用户可设 ROXID_LLAMA_SERVER 指向自编译产物（见 download 模块）。
//!
//! 架构感知（迭代31，用户裁决 Q1「动态探测：识别架构，检查GPU，下载」
//! 2026-09-10 18:15）：变体名 = ubuntu[-vulkan]-{x64|arm64}，架构片段取自
//! std::env::consts::ARCH（编译期常量，双架构原生构建下与宿主一致）；
//! android 变体不纳入自动链（Q2：Termux 用户经 setup --llama-url 手动装，
//! 2026-09-10 18:15）。修复前 asset_variant 硬编码 x64 两常量，arm64 宿主
//! 误装 ubuntu-x64 包（ELF e_machine=0x3e），执行即 ENOENT。
//!
//! 修改历史：M2 新增 2026-08-24 18:56（原因：运行时管理里程碑）；
//! M27 探测缓存（迭代7 P0碴2 组成）：OnceLock 一次性记忆探测结果，
//! 消除每次冷加载的重复外部命令阻塞 2026-08-26 21-35
//! M99 架构维度（迭代31）：asset_variant 拼接宿主架构片段（x86_64 产物与
//! 旧常量逐字一致，存量缓存零迁移）；新增 ELF e_machine 校验族
//! （elf_machine_of / arch_mismatch_static / diagnose_arch_mismatch，
//! Q3 命中校验 / Q3 list 标注 / Q4 spawn 诊断三处复用单实现）
//! 2026-09-10 18-30

use std::process::Command;

use crate::error::{RoxidError, RoxidResult};

/// llama.cpp 运行时后端（对应官方预编译包变体）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// GPU：经 Vulkan（NVIDIA 驱动 / AMD / Intel 均原生支持；
    /// arm64 同语义——Q1 裁决「与 x64 对齐的动态探测」2026-09-10 18:15）
    Vulkan,
    /// 纯 CPU 回退
    Cpu,
}

/// ELF e_machine 常量（M99，迭代31）：官方预编译包覆盖的两种 Linux 架构
const EM_X86_64: u16 = 0x3e;
const EM_AARCH64: u16 = 0xb7;

/// 宿主架构 → 官方资产名架构片段（M99，迭代31）。
/// x86_64 → "x64"、aarch64 → "arm64"；官方无预编译包的架构返回 None
///（调用方报错并引导手动链逃生口，不再误下 x64 包）。
///
/// - 返回：架构片段
pub fn host_arch_fragment() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// 后端 + 架构片段 → 资产变体名（纯函数注入，便于单测全组合覆盖）。
///
/// - 参数 backend：探测得到的后端
/// - 参数 arch：架构片段（host_arch_fragment 产物）
/// - 返回：变体名，如 "ubuntu-vulkan-arm64"
pub fn variant_fragment(backend: Backend, arch: &str) -> String {
    match backend {
        Backend::Vulkan => format!("ubuntu-vulkan-{arch}"),
        Backend::Cpu => format!("ubuntu-{arch}"),
    }
}

impl Backend {
    /// 官方 release 资产名中的变体片段（不含 .tar.gz 后缀）
    ///
    /// M99（迭代31）：拼接宿主架构片段——x86_64 产物与旧硬编码常量逐字一致
    ///（存量缓存目录名零迁移、命中路径零变化），aarch64 产物改拉
    /// ubuntu[-vulkan]-arm64 官方资产（此前误拉 ubuntu-x64 执行即 ENOENT）。
    ///
    /// - 返回：变体名，如 "ubuntu-vulkan-x64"；宿主架构无官方预编译包时报错
    pub fn asset_variant(self) -> RoxidResult<String> {
        let arch = host_arch_fragment().ok_or_else(|| {
            RoxidError::RunnerFailure(format!(
                "宿主架构 {} 无 llama.cpp 官方预编译包，请经 setup --llama-url 安装手动版本或设置 {} 指向自编译产物",
                std::env::consts::ARCH,
                super::download::ENV_LLAMA_SERVER_OVERRIDE
            ))
        })?;
        Ok(variant_fragment(self, arch))
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

/// 读取 ELF64 头的 e_machine（偏移 0x12，两字节小端）。
/// 宽容语义：非 ELF / 非 ELF64 / 非小端 / 读取失败返回 None——不误伤用户
/// 自备的非常规二进制（校验方跳过放行，诊断方降级原错误文案）。
///（官方预编译包均为 ELF64 小端，实测 2026-08-24 18:54）
///
/// - 参数 path：二进制文件路径
/// - 返回：e_machine 值（如 0x3e=x86-64、0xb7=aarch64）
pub fn elf_machine_of(path: &std::path::Path) -> Option<u16> {
    use std::io::Read;
    let mut header = [0u8; 20];
    std::fs::File::open(path)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    if header[..4] != [0x7f, b'E', b'L', b'F'] || header[4] != 2 || header[5] != 1 {
        return None; // 非 ELF / 非 ELF64（EI_CLASS≠2）/ 非小端（EI_DATA≠1）
    }
    Some(u16::from_le_bytes([header[18], header[19]]))
}

/// e_machine → 架构片段名（与 host_arch_fragment 同名空间，可直接比对）。
///
/// - 参数 machine：e_machine 值
/// - 返回：架构名；官方预编译未覆盖的架构 None（无法判定）
pub fn elf_arch_name(machine: u16) -> Option<&'static str> {
    match machine {
        EM_X86_64 => Some("x64"),
        EM_AARCH64 => Some("arm64"),
        _ => None,
    }
}

/// e_machine 与宿主架构比对（纯函数注入 host，单测全组合覆盖）。
///
/// - 参数 machine：二进制 e_machine
/// - 参数 host：宿主架构片段
/// - 返回：Some((二进制架构, 宿主架构)) 表示确认不匹配；
///   None 表示匹配 / e_machine 未知（无法判定不冒进）
fn arch_mismatch_static(machine: u16, host: &'static str) -> Option<(&'static str, &'static str)> {
    let bin_arch = elf_arch_name(machine)?;
    (bin_arch != host).then_some((bin_arch, host))
}

/// 诊断已落位二进制与宿主架构不匹配（IO 包装：Q3 命中校验 / Q3 list 标注 /
/// Q4 spawn 诊断三处复用的单实现）。
///
/// - 参数 bin：llama-server 二进制路径
/// - 返回：Some((二进制架构, 宿主架构)) 表示确认不匹配；
///   None 表示匹配 / 文件缺失 / 非 ELF（宽容不冒进）
pub fn diagnose_arch_mismatch(bin: &std::path::Path) -> Option<(&'static str, &'static str)> {
    arch_mismatch_static(elf_machine_of(bin)?, host_arch_fragment()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 变体拼接全组合：x64 产物与旧硬编码常量逐字一致（存量缓存零迁移保障），
    /// arm64 产物对齐官方资产命名（用户实测 b10883 资产清单 2026-09-10）
    #[test]
    fn variant_fragment_full_matrix() {
        assert_eq!(
            variant_fragment(Backend::Vulkan, "x64"),
            "ubuntu-vulkan-x64"
        );
        assert_eq!(variant_fragment(Backend::Cpu, "x64"), "ubuntu-x64");
        assert_eq!(
            variant_fragment(Backend::Vulkan, "arm64"),
            "ubuntu-vulkan-arm64"
        );
        assert_eq!(variant_fragment(Backend::Cpu, "arm64"), "ubuntu-arm64");
    }

    /// 本项目发布矩阵架构（x86_64/aarch64，迭代23）必有片段映射
    #[test]
    fn host_arch_fragment_supported_on_release_matrix() {
        let frag = host_arch_fragment();
        assert!(
            frag == Some("x64") || frag == Some("arm64"),
            "发布矩阵内宿主必须有片段映射，实际：{frag:?}"
        );
    }

    /// asset_variant 在发布矩阵宿主上必成功且尾部嵌宿主架构片段
    #[test]
    fn asset_variant_embeds_host_arch() {
        let variant = Backend::Cpu
            .asset_variant()
            .expect("发布矩阵内宿主不应报错");
        assert!(
            variant.ends_with(host_arch_fragment().unwrap()),
            "变体名必须嵌宿主架构片段：{variant}"
        );
    }

    /// 构造 20 字节 ELF64LE 假头（e_machine 偏移 0x12 可注入）
    fn fake_elf_header(machine: u16) -> [u8; 20] {
        let mut h = [0u8; 20];
        h[..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        h[4] = 2; // EI_CLASS：ELF64
        h[5] = 1; // EI_DATA：小端
        h[18..20].copy_from_slice(&machine.to_le_bytes());
        h
    }

    /// e_machine 解析：魔数 / 类 / 端序校验 + 偏移 0x12 提取；非 ELF 宽容 None
    #[test]
    fn elf_machine_of_parses_header() {
        let dir = std::env::temp_dir().join(format!("roxid-elf-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("llama-server");
        std::fs::write(&f, fake_elf_header(EM_X86_64)).unwrap();
        assert_eq!(
            elf_machine_of(&f),
            Some(EM_X86_64),
            "x86-64 头必须解析出 0x3e"
        );
        std::fs::write(&f, fake_elf_header(EM_AARCH64)).unwrap();
        assert_eq!(
            elf_machine_of(&f),
            Some(EM_AARCH64),
            "aarch64 头必须解析出 0xb7"
        );
        // 非 ELF（脚本解释器）与不足 20 字节的截断文件：宽容 None
        std::fs::write(&f, b"#!/bin/sh\necho hi\n").unwrap();
        assert_eq!(elf_machine_of(&f), None, "非 ELF 必须宽容 None");
        std::fs::write(&f, b"\x7fELF").unwrap();
        assert_eq!(elf_machine_of(&f), None, "截断 ELF 必须宽容 None");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 比对矩阵：交叉架构确认不匹配（含双方名称）/ 同架构匹配 / 未知 e_machine
    /// 无法判定（RISC-V 等官方未覆盖架构不冒进）
    #[test]
    fn arch_mismatch_matrix() {
        assert_eq!(
            arch_mismatch_static(EM_X86_64, "arm64"),
            Some(("x64", "arm64"))
        );
        assert_eq!(
            arch_mismatch_static(EM_AARCH64, "x64"),
            Some(("arm64", "x64"))
        );
        assert_eq!(arch_mismatch_static(EM_X86_64, "x64"), None, "同架构匹配");
        assert_eq!(
            arch_mismatch_static(EM_AARCH64, "arm64"),
            None,
            "同架构匹配"
        );
        assert_eq!(
            arch_mismatch_static(0xf3, "x64"),
            None,
            "未知 e_machine 无法判定"
        );
    }

    /// 诊断 IO 包装：文件缺失宽容 None（调用方各有降级路径，不误伤）
    #[test]
    fn diagnose_arch_mismatch_missing_file_is_none() {
        assert_eq!(
            diagnose_arch_mismatch(std::path::Path::new(
                "/nonexistent/roxid-arch-test/llama-server"
            )),
            None
        );
    }
}
