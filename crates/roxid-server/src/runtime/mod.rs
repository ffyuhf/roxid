//! llama.cpp 运行时管理器。
//!
//! 职责（来源：用户确认 2026-08-24 18:25 Q4 / 18:49 版本 / 18:51 Q10 方案 B）：
//! - GPU 探测：nvidia-smi / vulkaninfo 任一可用 → Vulkan 预编译包；否则 CPU 包
//!   （实测 Linux 官方预编译无 CUDA 包，NVIDIA 经 Vulkan 加速；
//!    需原生 CUDA 时设 ROXID_LLAMA_SERVER 复用用户自编译产物）
//! - 从 llama.cpp 官方 GitHub Releases 下载 tar.gz（迭代46 M173 起
//!   兜底链在线查最新预发布版本并落 default_version——硬编码锁定 tag
//!   已删除，用户裁决 2026-09-12 02:11/02:17/02:41；历史：b10605
//!   （2026-08-24）→ b10883（M95 2026-09-10）；
//!   M36 起支持任意 tag 多版本并存 + config default_version 持久默认），
//!   解压平铺缓存于 {roxid_home}/llama.cpp/{tag}/{variant}/，
//!   llama-server 的 rpath 含 $ORIGIN，直接运行即可
//!
//! 实现排期：M2（本模块）/ M3（子进程编排，见 scheduler）。
//! 修改历史：占位 2026-08-24 18:35；M2 实装 2026-08-24 18:56；
//! M36（迭代16）多版本管理：install_version/list_installed/remove_version
//! 公开导出供 `roxid runtime` 子命令族调用 2026-09-09 04-37
//! M54（迭代19 碴B）：resolve_llama_server_path 导出——scheduler 复用
//! 第四键的 resolve-only 解析（runtime use 切换感知）2026-09-09 20-30
//! M173/M174（迭代46）：LOCKED_LLAMA_CPP_TAG 常量删除（兜底链改在线查
//! 最新版 + resolve 第4步 None 放行）2026-09-12 02-45
//! M181（迭代48）：latest_llama_cpp_tags / pick_prerelease_tags 导出
//! ——runtime update 子命令与 install tag 补全的共用数据源
//! （Q2-A/Q3-B 裁决 2026-09-12 04:23/04:24）2026-09-12 04-32

mod backend;
mod download;

// M99（迭代31）：backend 新增架构感知与 ELF 校验族导出（host_arch_fragment
// 供 CLI 提示、diagnose_arch_mismatch 供 spawn 诊断）；download 新增启动
// 扫描与 list 标注数据源导出 2026-09-10 18-30
pub use backend::{
    detect_backend, diagnose_arch_mismatch, elf_machine_of, host_arch_fragment, variant_fragment,
    Backend,
};
pub use download::{
    asset_url, ensure_llama_server, install_manual, install_version,
    installed_variant_arch_mismatch, is_valid_tag, latest_llama_cpp_tags, list_installed,
    manual_dir, manual_server_path, pick_prerelease_tags, remove_version,
    resolve_llama_server_path, url_is_archive, variant_cache_dir, warn_installed_arch_mismatch,
    ENV_LLAMA_SERVER_OVERRIDE,
};
