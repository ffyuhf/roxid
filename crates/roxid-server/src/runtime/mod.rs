//! llama.cpp 运行时管理器。
//!
//! 职责（来源：用户确认 2026-08-24 18:25 Q4 / 18:49 版本 / 18:51 Q10 方案 B）：
//! - GPU 探测：nvidia-smi / vulkaninfo 任一可用 → Vulkan 预编译包；否则 CPU 包
//!   （实测 Linux 官方预编译无 CUDA 包，NVIDIA 经 Vulkan 加速；
//!    需原生 CUDA 时设 ROXID_LLAMA_SERVER 复用用户自编译产物）
//! - 从 llama.cpp 官方 GitHub Releases 下载 tar.gz（锁定链 b10605 兜底；
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

mod backend;
mod download;

pub use backend::{detect_backend, Backend};
pub use download::{
    asset_url, ensure_llama_server, install_manual, install_version, is_valid_tag, list_installed,
    manual_dir, manual_server_path, remove_version, resolve_llama_server_path, url_is_archive,
    variant_cache_dir, ENV_LLAMA_SERVER_OVERRIDE, LOCKED_LLAMA_CPP_TAG,
};
