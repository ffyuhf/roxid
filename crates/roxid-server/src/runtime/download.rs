//! llama.cpp 官方预编译包下载、解压与缓存。
//!
//! 实测事实（2026-08-24 18:54）：
//! - 资产为 tar.gz，顶层目录 llama-{tag}/，二进制与全部 .so 平铺同目录；
//! - llama-server 的 rpath 含 $ORIGIN，解压后可直接运行，无需 LD_LIBRARY_PATH。
//!
//! 代理机制（来源：用户确认 2026-08-24 19:04 Q11 及 19:05 澄清：
//! 只添加 ROXID_GH_PROXY 环境变量，代码中不写死任何默认代理网址）：
//! - 未设置 ROXID_GH_PROXY → 回退 config.toml [proxy].gh，仍未配置则直连 GitHub；
//! - 设置 ROXID_GH_PROXY（如 https://gh.jasonzeng.dev/）→ 前缀拼接代理下载。
//! 读取链 env 优先 → 文件回退：迭代4 裁决 Q2/Q3 2026-08-26 05:24。
//!
//! 修改历史：M2 新增 2026-08-24 18:56；M2 修正 2026-08-24 19:06
//! （原因：增加代理变量 + 下载重试退避 + 显式超时）；
//! M28 碴11 gh 代理前缀文件段 mtime 缓存（迭代8：消除重复读盘，
//! 与 hf.rs 同款策略）2026-08-30 02-10
//! M29 碴7（迭代9，R1-A）：安装互斥 + 临时目录 rename 原子落位——
//! 跨模型并行冷加载首次安装同变体时不再并发写 cache_dir（.part 交错
//! 损坏与 tar 解压竞争），半途失败不污染缓存 2026-09-05 12-58
//! M30 碴5（迭代10，R2-A）：安装段落盘/解压/清理全 async 化——原
//! std::fs 写 16MB 整包 + std Command 阻塞 wait tar + 同步 remove
//! 均在 tokio worker 线程执行（对齐 registry/downloader 的 tokio::fs
//! 先例）2026-09-06 22-18
//! M31（迭代11 F2，用户确认 2026-09-06 22:34）：setup --llama-url 手动
//! 安装链——install_manual 按 URL 下载（tar.gz 解压 / 裸二进制直接落位）
//! 至 llama.cpp/manual/；ensure_llama_server 优先级变为 env → manual →
//! b10605 自动链（原因：后端手动链接更新需求）2026-09-06 23-05
//! M36（迭代16，用户裁决 Q1-B 2026-09-09 04:20）：多版本管理——
//! asset_url/variant_cache_dir/install_version 全链 tag 参数化（b10605
//! 不再是唯一可装版本）；新增 list_installed/remove_version 供
//! `roxid runtime` 子命令族；ensure_llama_server 优先级变为
//! env → manual → default_version(config，已装校验) → b10605 锁定链
//!（未配置 default_version 时行为与旧版逐字节一致）2026-09-09 04-36
//! M54（迭代19 碴B）：新增 resolve_llama_server_path——ensure 链同序的
//! resolve-only 解析（零下载零网络），供 scheduler 复用第四键比对
//! runtime use 切换默认版本后应然路径与实例记录不一致触发重建
//! 2026-09-09 20-30
//! M32 碴8（迭代12）：download_all 响应体读取失败纳入 3 次重试（原
//! `?` 直接返回，body 中断绕过重试语义）+ ignored 替身测试挂档
//! 2026-09-07 01-15
//! M87（迭代27，用户确认 2026-09-10 04:15）：手动安装链代理拼接——
//! install_manual 对 https://github.com/ 开头的 URL 经 apply_gh_proxy
//! 前置拼接已配置代理前缀（空前缀=直连），非 GitHub 域名原样直用
//!（原因：用户确认代理后手动链仍裸连直下，代理配置未覆盖手动链）
//! 2026-09-10 04-20
//! M99（迭代31，用户裁决 Q3/Q4 2026-09-10 18:17/18:21）：ensure/resolve
//! 命中已装缓存时 ELF 架构校验（确认不匹配 ensure 报 ArchMismatch、
//! resolve 视为未命中终止——双链同序对齐）；新增 warn_installed_arch_
//! mismatch（serve 启动扫描 warn 不阻断）与 installed_variant_arch_
//! mismatch（runtime list 标注数据源）2026-09-10 18-30
//! M105（迭代32 碴6a）：下载进度回调链——download_all_with_progress
//! 流式累计（content-length 为总量）+ install_version/install_manual
//! 透传回调 + serve 自动下载场景 5s 周期日志（原全程静默无总量/速度/
//! 剩余时间，下大包形似卡死；用户实测报告）2026-09-10 21-38
//! M181（迭代48，Q2/Q3/Q4 裁决 2026-09-12 04:23/04:24/04:25）：
//! pick_prerelease_tags 泛化（limit 截断，install tag 补全数据源）+
//! latest_llama_cpp_tags 多 tag 查询（runtime update 与补全共用）+
//! 查询核心抽取 fetch_releases_with_retry（重试语义逐字不变），
//! config [runtime].tag_complete_limit 补全数量通道 2026-09-12 04-30
//! M189（迭代50，用户裁决链 15:47/15:55/15:58）：use 变体选择——
//! variant_dirs_of 单 tag 已装变体扫描 + resolve_variant_keyword
//! 变体词匹配（磁盘事实零硬编码）+ effective_variant 生效解析
//!（default_variant 优先、缺省回退探测）2026-09-12 16-25

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use tokio::process::Command;

use super::backend::diagnose_arch_mismatch;
use crate::config::llama_runtime_root;
use crate::error::{RoxidError, RoxidResult};

/// 预发布条目降序候选（M181 共享核心）：过滤 prerelease==true 且
/// draft!=false，按 created_at 降序——M173 pick_latest_prerelease_tag
/// 与 M181 pick_prerelease_tags 共用（提取自 M173 原内联实现，语义逐字
/// 保持：created_at 同格式 ISO8601 下字典序即时间序，GitHub 返回形态
/// 恒定前提成立——迭代46 注记②延续）。
fn sorted_prerelease_candidates(releases: &[serde_json::Value]) -> Vec<(&str, &str)> {
    let mut candidates: Vec<(&str, &str)> = releases
        .iter()
        .filter(|r| r["prerelease"].as_bool() == Some(true) && r["draft"].as_bool() != Some(true))
        .filter_map(|r| Some((r["tag_name"].as_str()?, r["created_at"].as_str()?)))
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(a.1));
    candidates
}

/// 从 Releases JSON 数组选取最新预发布 tag（迭代46 M173）：过滤
/// prerelease==true 且 draft==false，按 created_at 降序取首个 tag_name
///（用户 curl 命令 jq 管道的 Rust 等价实现；抽纯函数供单测锚定语义）。
/// 注意：首个候选**不做** is_valid_tag 过滤——非法形态由调用方报错
/// （ensure 兜底链 M173 语义），与 pick_prerelease_tags 的宽容剔除
/// 语义刻意不同（双链漂移防护：修改任一处须同步注释说明）。
///
/// - 参数 releases：GitHub Releases 列表（已解析 JSON）
/// - 返回：最新 tag_name（无候选 None）
fn pick_latest_prerelease_tag(releases: &[serde_json::Value]) -> Option<String> {
    sorted_prerelease_candidates(releases)
        .into_iter()
        .next()
        .map(|(tag, _)| tag.to_string())
}

/// 从 Releases JSON 数组选取预发布 tag 列表（迭代48 M181，Q3-B/Q4-A
/// 裁决 2026-09-12 04:24/04:25）：共享核心降序候选 → is_valid_tag 剔除
/// 非法形态 → 取前 limit 个——`runtime install` tag 补全候选数据源
///（宽容语义：补全列表剔除非法 tag，不因个别异常条目整体失败）。
///
/// - 参数 releases：GitHub Releases 列表（已解析 JSON）
/// - 参数 limit：最多返回的 tag 数（补全数量通道，调用方来自
///   config [runtime].tag_complete_limit，默认 10）
/// - 返回：降序 tag 列表（无合法候选时空 Vec）
pub fn pick_prerelease_tags(releases: &[serde_json::Value], limit: usize) -> Vec<String> {
    sorted_prerelease_candidates(releases)
        .into_iter()
        .map(|(tag, _)| tag.to_string())
        .filter(|tag| is_valid_tag(tag))
        .take(limit)
        .collect()
}

/// Releases 列表查询共享核心（M181 抽取自 M173 原内联实现，查询参数/
/// 重试节奏/错误文案逐字保持）：GET api.github.com releases?per_page=100，
/// 3 次指数退避重试；4xx 确定性失败（429 限流除外）快速报错——重试
/// 语义对齐 M32/M49；无任何写死兜底值（用户裁决 2026-09-12 02:11）。
///
/// - 返回：已解析的 Releases JSON 列表
async fn fetch_releases_with_retry() -> RoxidResult<Vec<serde_json::Value>> {
    const API_URL: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=100";
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| RoxidError::RegistryRequest(format!("构建 HTTP 客户端失败：{e}")))?;
    let mut last_error = String::new();
    for attempt in 1..=3 {
        match client
            .get(API_URL)
            .header("User-Agent", "roxid")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let releases: Vec<serde_json::Value> = resp.json().await.map_err(|e| {
                    RoxidError::RegistryRequest(format!("解析 Releases 响应失败：{e}"))
                })?;
                return Ok(releases);
            }
            // M49 语义：4xx 确定性失败快速报错；429 限流属瞬态保留重试
            Ok(resp) if resp.status().is_client_error() && resp.status().as_u16() != 429 => {
                return Err(RoxidError::RegistryRequest(format!(
                    "查询 GitHub Releases 失败：HTTP {}（第 {attempt}/3 次）",
                    resp.status().as_u16()
                )));
            }
            Ok(resp) => last_error = format!("HTTP {}", resp.status().as_u16()),
            Err(e) => last_error = format!("{e}"),
        }
        if attempt < 3 {
            let backoff = Duration::from_secs(1u64 << (attempt - 1));
            tracing::warn!(
                "查询 llama.cpp 最新版本失败（{last_error}），{backoff:?} 后重试（{attempt}/3）"
            );
            tokio::time::sleep(backoff).await;
        }
    }
    Err(RoxidError::RegistryRequest(format!(
        "查询 llama.cpp 最新版本失败（{last_error}）。可手动安装：roxid runtime install \
         --llama-url <包地址>；或设置 gh 代理（ROXID_GH_PROXY env / config [proxy].gh）后重试"
    )))
}

/// 查询 llama.cpp 官方 GitHub Releases 最新预发布版本 tag（迭代46 M173，
/// 用户裁决 2026-09-12 02:06 提供 curl 命令 / 02:17 下载时才查 / 02:41
/// 完整逻辑确认）：GET releases 列表，过滤 prerelease==true 且
/// draft==false，按 created_at 降序取首个 tag_name——curl+jq 管道的
/// Rust 等价实现。重试语义对齐 M32/M49：3 次指数退避，4xx 确定性失败
/// （429 限流除外）快速报错；无任何写死兜底值（用户裁决 02:11）。
/// M181 起查询体走 fetch_releases_with_retry 共享核心。
///
/// - 返回：最新预发布 tag（如 "b10909"，经 is_valid_tag 校验）
async fn latest_llama_cpp_tag() -> RoxidResult<String> {
    let releases = fetch_releases_with_retry().await?;
    match pick_latest_prerelease_tag(&releases) {
        Some(tag) if is_valid_tag(&tag) => Ok(tag),
        Some(tag) => Err(RoxidError::RegistryRequest(format!(
            "最新版本 tag 非法：{tag}"
        ))),
        None => Err(RoxidError::RegistryRequest(
            "GitHub Releases 无预发布版本（prerelease 均为 false）".into(),
        )),
    }
}

/// 查询 llama.cpp 官方 GitHub Releases 最新预发布版本 tag 列表（迭代48
/// M181，Q2-A/Q3-B/Q4-A 裁决链 2026-09-12 04:23-04:25）：与
/// latest_llama_cpp_tag 同款查询与重试语义（共享核心），返回按
/// created_at 降序的前 limit 个合法 tag——`runtime update`（limit=1
/// 取首）与 `runtime install` tag 补全（limit 来自 config
/// [runtime].tag_complete_limit）共用数据源。
///
/// - 参数 limit：最多返回的 tag 数
/// - 返回：降序 tag 列表（全量剔除后无合法候选时报错——与 M173
///   「无预发布版本」同语义）
pub async fn latest_llama_cpp_tags(limit: usize) -> RoxidResult<Vec<String>> {
    let releases = fetch_releases_with_retry().await?;
    let tags = pick_prerelease_tags(&releases, limit);
    if tags.is_empty() {
        return Err(RoxidError::RegistryRequest(
            "GitHub Releases 无预发布版本（prerelease 均为 false）".into(),
        ));
    }
    Ok(tags)
}

/// 覆盖运行时二进制的环境变量（来源：用户确认 2026-08-24 18:51 Q10 方案 B：
/// 用户自编译 CUDA 版 llama-server 时直接复用）
pub const ENV_LLAMA_SERVER_OVERRIDE: &str = "ROXID_LLAMA_SERVER";

/// GitHub 代理前缀环境变量（来源：用户确认 2026-08-24 19:04/19:05 Q11：
/// 仅提供变量，不设默认值；未设置时直连 GitHub）
pub const ENV_GH_PROXY: &str = "ROXID_GH_PROXY";

/// 读取 GitHub 代理前缀：ROXID_GH_PROXY 环境变量优先（非空即用），
/// 回退 config.toml [proxy].gh；均未配置时为空串直连
///（Q11 语义不变：代码不写死任何默认代理网址）。
/// M28 碴11：文件段经 mtime 缓存读取（原实现每次调用读盘解析）。
///
/// - 返回：代理前缀（直连时为空串）
pub fn gh_proxy_prefix() -> String {
    if let Ok(prefix) = std::env::var(ENV_GH_PROXY) {
        if !prefix.is_empty() {
            return prefix;
        }
    }
    cached_file_proxy_gh().unwrap_or_default()
}

/// config.toml [proxy].gh 的 mtime 缓存读取（M28 碴11，与 hf.rs 同款策略）。
/// 缓存键 =（文件路径, mtime）：路径变化（ROXID_HOME 切换）或 mtime 变更才
/// 重读；文件不存在不缓存、每次走 load 的快速默认路径。env 不缓存（进程内
/// 读取开销可忽略），保持 Q11/Q2 动态语义与既有单测兼容。
///
/// - 返回：文件段代理前缀（未配置或空为 None）
fn cached_file_proxy_gh() -> Option<String> {
    static CACHE: std::sync::Mutex<
        Option<((std::path::PathBuf, std::time::SystemTime), Option<String>)>,
    > = std::sync::Mutex::new(None);
    let path = crate::config::config_file_path();
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let (Some((key, value)), Some(current)) = (&*guard, mtime) {
        if *key == (path.clone(), current) {
            return value.clone();
        }
    }
    let value = crate::config::load_persist_config()
        .proxy
        .gh
        .filter(|s| !s.is_empty());
    if let Some(current) = mtime {
        *guard = Some(((path, current), value.clone()));
    }
    value
}

/// GitHub 原始资产 URL（不经代理）
///
/// - 参数 backend_variant：变体片段，如 "ubuntu-vulkan-x64"
/// - 参数 tag：版本 tag，如 "b10883"（M36 起参数化，不再锁死）
/// - 返回：完整 GitHub URL
fn github_raw_url(backend_variant: &str, tag: &str) -> String {
    format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{tag}/llama-{tag}-bin-{variant}.tar.gz",
        tag = tag,
        variant = backend_variant,
    )
}

/// 实际下载 URL = 代理前缀（可为空）+ GitHub 原始 URL
///
/// - 参数 backend_variant：变体片段，如 "ubuntu-vulkan-x64"
/// - 参数 tag：版本 tag（M36 参数化）
/// - 返回：完整下载 URL
pub fn asset_url(backend_variant: &str, tag: &str) -> String {
    format!(
        "{}{}",
        gh_proxy_prefix(),
        github_raw_url(backend_variant, tag)
    )
}

/// 某后端变体的缓存目录（解压落位处，平铺含 llama-server 与全部 .so）
///
/// - 参数 backend_variant：变体片段
/// - 参数 tag：版本 tag（M36 参数化：{root}/{tag}/{variant} 多版本并存）
/// - 返回：缓存目录路径
pub fn variant_cache_dir(backend_variant: &str, tag: &str) -> PathBuf {
    llama_runtime_root().join(tag).join(backend_variant)
}

/// tag 是否为合法 llama.cpp 版本形态（M36，R5：`b` + 纯数字，如 b10700）。
/// "manual" 为保留字（手动版本目录），不按 tag 形态校验。
///
/// - 参数 tag：待校验字符串
/// - 返回：true 表示可作为目录名与下载 tag
pub fn is_valid_tag(tag: &str) -> bool {
    match tag.strip_prefix('b') {
        Some(digits) => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// M99（迭代31，Q3 校验 2，用户裁决 2026-09-10 18:17）：ELF 架构校验——
/// ensure 链命中已装缓存时防止架构不匹配放行（x64 包误装 arm64 宿主即
/// ENOENT，原实现放行到 spawn 阶段才裸报 os error 2）。宽容语义：ELF
/// 解析失败 / 未知 e_machine 跳过校验（不误伤自备非常规二进制，
/// android bionic 包 e_machine 同为 0xb7 不受影响）。
///
/// - 参数 bin：已装 llama-server 路径
/// - 参数 ctx：报错文案上下文（如 "最新版 b10909 变体 ubuntu-arm64"）
/// - 返回：Ok(()) 放行；Err 确认不匹配（含 rm 重装 / env 逃生口指引）
fn ensure_arch_matches(bin: &std::path::Path, ctx: &str) -> RoxidResult<()> {
    if let Some((bin_arch, host_arch)) = diagnose_arch_mismatch(bin) {
        return Err(RoxidError::ArchMismatch(format!(
            "{ctx} 的 llama-server 为 {bin_arch} 构建，与宿主 {host_arch} 不匹配，无法执行；\
             可 `roxid runtime rm <tag>` 清理后重装本机架构版本，或设置 {ENV_LLAMA_SERVER_OVERRIDE} 指向自编译产物"
        )));
    }
    Ok(())
}

/// M99（迭代31）：resolve 侧同款判定——确认不匹配视为不可用（与 ensure
/// 的 Err 终止对齐：resolve None → 调用方保守放行，诊断报错由 ensure 承担）。
///
/// - 参数 bin：已装 llama-server 路径
/// - 返回：true 表示可用（匹配 / ELF 无法判定宽容）；false 确认不匹配
fn arch_ok(bin: &std::path::Path) -> bool {
    diagnose_arch_mismatch(bin).is_none()
}

/// 确保 llama-server 可用并返回其路径。
/// 优先级（M36 更新）：ROXID_LLAMA_SERVER 环境变量 → manual 手动版本
/// （setup --llama-url 安装）→ config default_version 指定版本的变体缓存
/// （未安装则告警回退）→ 在线最新版兜底链（查 GitHub API 最新预发布
/// tag；已装复用/未装下载，落定写 default_version 设默认。迭代46 M173
/// 起替代原锁定链；历史：b10605 → b10883（M95）→ 删除）。
/// M99（迭代31）：各分支命中已装缓存时 ELF 架构校验，不匹配报 ArchMismatch。
///
/// - 参数 backend_variant：探测得到的变体片段（Backend::asset_variant()）
/// - 返回：llama-server 二进制路径
pub async fn ensure_llama_server(backend_variant: &str) -> RoxidResult<PathBuf> {
    // 1) 环境变量逃生口：用户自编译产物直接复用（仅校验存在性，不校验版本；
    //    M99：架构不匹配在此即报诊断，不再放行到 spawn 裸 ENOENT）
    if let Ok(custom) = std::env::var(ENV_LLAMA_SERVER_OVERRIDE) {
        let path = PathBuf::from(&custom);
        if path.is_file() {
            ensure_arch_matches(&path, &format!("{ENV_LLAMA_SERVER_OVERRIDE}={custom}"))?;
            return Ok(path);
        }
        return Err(RoxidError::RunnerFailure(format!(
            "{ENV_LLAMA_SERVER_OVERRIDE}={custom} 指向的文件不存在"
        )));
    }
    // 2) 手动版本优先（M31 F2）：setup --llama-url 安装的产物直接复用，
    //    跳过变体探测（手动包即用户指定的一种构建，无 vulkan/cpu 之分）；
    //    default_version == "manual" 时同样落到此处（特殊保留字）
    let manual = manual_server_path();
    if manual.is_file() {
        ensure_arch_matches(&manual, "manual 手动版本")?;
        return Ok(manual);
    }
    // 3) M36：config default_version 指定版本优先于兜底链（use 命令的生效点）
    if let Some(tag) = crate::config::load_persist_config()
        .runtime
        .default_version
        .filter(|t| is_valid_tag(t))
    {
        let server = variant_cache_dir(backend_variant, &tag).join("llama-server");
        if server.is_file() {
            ensure_arch_matches(&server, &format!("默认版本 {tag} 变体 {backend_variant}"))?;
            return Ok(server);
        }
        // 指定版本未安装（如被手删目录）：告警回退在线兜底链，不硬失败
        tracing::warn!("默认后端版本 {tag} 的变体 {backend_variant} 未安装，回退在线最新版兜底链");
    }
    // 4) 兜底链（迭代46 M173：删除硬编码锁定 tag，用户裁决 2026-09-12
    //    02:06「不再硬编码」/02:11「下载时才查」/02:17「下载最新版并设
    //    置为默认」/02:41 完整逻辑确认）——查 GitHub API 最新预发布版本；
    //    已装该版本复用，未装下载；落定后写 default_version 设为默认，
    //    下次启动走第3层命中不再联网
    let tag = latest_llama_cpp_tag().await?;
    let server = variant_cache_dir(backend_variant, &tag).join("llama-server");
    if server.is_file() {
        ensure_arch_matches(&server, &format!("最新版 {tag} 变体 {backend_variant}"))?;
    } else {
        // M105（迭代32 碴6a）：serve 自动下载场景进度——5s 周期日志（无
        // TTY 无 spinner，原全程静默）；CLI 显式安装场景由调用方传渲染回调
        install_version(&tag, backend_variant, periodic_log_progress()).await?;
    }
    // 设为默认版本（用户裁决 02:17「设置为默认」）：写 config 让后续启动
    // 直接走第3层命中；写失败仅告警不阻断——缓存已就位，无行为差异
    let mut cfg = crate::config::load_persist_config();
    if cfg.runtime.default_version.as_deref() != Some(tag.as_str()) {
        cfg.runtime.default_version = Some(tag.clone());
        if let Err(e) = crate::config::save_persist_config(&cfg) {
            tracing::warn!("设置默认版本 {tag} 失败（非致命）：{e}");
        }
    }
    Ok(server)
}

/// resolve-only 路径解析（M54b 碴B）：按 ensure_llama_server 同序链
/// （env → manual → default_version 已装 → 兜底 None 放行——迭代46
/// M174 起原锁定链缓存检查移除，零网络函数不查在线 tag）解析当前
/// 应然二进制路径，仅存在性检查、零下载零网络。scheduler 复用判定用它
/// 与实例记录路径比对，不一致（runtime use 切换默认版本 / env 改指向）
/// 时触发重建——碴B修复：原仅 ctx/RUNTIME 双键，后端版本切换后运行
/// 实例无感知，keep_alive 窗口内一直用旧版本跑。
/// 与 ensure 的差异面（调用方语义）：env 指向不存在或兜底链未落定时
/// 返回 None 而非报错/下载——调用方按「不可比对」保守放行，加载路径
/// 仍由 ensure 完整链兜底；链序与命中判定必须与 ensure 保持逐字一致
/// （双链漂移防护：修改任一处须同步另一处）。
///
/// - 参数 backend_variant：探测得到的变体片段（Backend::asset_variant()）
/// - 返回：Some(路径) 表示已存在可用；None 表示全链未命中
pub fn resolve_llama_server_path(backend_variant: &str) -> Option<PathBuf> {
    // 1) 环境变量逃生口（不校验报错面：指向不存在时 None 放行，
    //    显式报错由加载路径的 ensure 完整链承担；
    //    M99：命中但架构不匹配同样 None——对齐 ensure 该分支的 Err 终止）
    if let Ok(custom) = std::env::var(ENV_LLAMA_SERVER_OVERRIDE) {
        let path = PathBuf::from(&custom);
        return path.is_file().then_some(path).filter(|p| arch_ok(p));
    }
    // 2) 手动版本（default_version == "manual" 特殊保留字同落此处；
    //    M99：命中但不匹配 → None 终止，不落后续分支——ensure 同分支
    //    在此 Err 终止，双链命中面必须一致）
    let manual = manual_server_path();
    if manual.is_file() {
        return arch_ok(&manual).then_some(manual);
    }
    // 3) config default_version 指定版本（仅已装命中；未装落第4步 None
    //    放行，不打 warn——本函数每请求调用，告警刷屏；留痕由 ensure 承担；
    //    M99：已装但不匹配 → None 终止（ensure 同分支 Err 终止，对齐））
    if let Some(tag) = crate::config::load_persist_config()
        .runtime
        .default_version
        .filter(|t| is_valid_tag(t))
    {
        let server = variant_cache_dir(backend_variant, &tag).join("llama-server");
        if server.is_file() {
            return arch_ok(&server).then_some(server);
        }
    }
    // 4) 兜底链（迭代46 M174：硬编码锁定 tag 已删除；本函数设计上零网络
    //    零下载，无本地可判定的兜底版本——返回 None 保守放行，加载路径
    //    由 ensure 完整链查最新版并落 default_version；用户裁决 2026-09-12
    //    02:41「resolve 第4步找不到就返回『没有』，真正下载由 ensure 负责」）
    None
}

/// 手动版本缓存目录：{roxid_home}/llama.cpp/manual（R2 裁决：单目录不分变体）
pub fn manual_dir() -> PathBuf {
    llama_runtime_root().join("manual")
}

/// 手动版本 llama-server 路径。
pub fn manual_server_path() -> PathBuf {
    manual_dir().join("llama-server")
}

/// URL 是否为压缩包形态（剥 query/fragment 后判后缀）。
/// tar.gz/.tgz → 解压流程；其余视为裸 llama-server 二进制直接落位。
///
/// - 参数 url：用户提供的下载链接
/// - 返回：true 表示按 tar.gz 解压
pub fn url_is_archive(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

/// GitHub 域名 URL 的代理前置拼接（M87，迭代27）：仅 `https://github.com/`
/// 开头的 URL 前置拼接 gh_proxy_prefix()（env 优先 → config 回退，与
/// asset_url() 拼接语义逐字一致；空前缀 = 直连原样返回）。非 GitHub
/// 域名（自建镜像 / HF / file:// / 已是代理前缀形态的完整地址）原样
/// 直用——「完整地址即最终地址」，天然防止对已拼代理的 URL 二次拼接。
///（来源：用户确认 2026-09-10 04:15「我都确认代理了，那我就是想用代理」）
///
/// - 参数 url：用户提供的下载链接
/// - 返回：实际下载用的 URL
fn apply_gh_proxy(url: &str) -> String {
    if url.starts_with("https://github.com/") {
        format!("{}{}", gh_proxy_prefix(), url)
    } else {
        url.to_string()
    }
}

/// 手动安装（迭代11 F2）：下载指定 URL 的包并落位为 manual 版本。
/// tar.gz 走解压流程（strip 顶层目录，$ORIGIN 布局），裸二进制直接落位；
/// 经 INSTALL_LOCK 互斥 + .installing 暂存原子落位（复用 M29 碴7 先例）。
/// M87（迭代27）：GitHub 域名 URL 自动前置拼接已配置代理前缀（用户确认
/// 代理即想用代理，无需手动拼地址）；archive 形态判定与来源记录
///（[runtime].llama_url）均以用户输入的原始 URL 为准，不受拼接影响。
///
/// - 参数 url：包下载链接（GitHub Releases 形态 tar.gz 或裸 llama-server）
/// - 参数 on_progress：进度回调（已下载字节, 总量 Option——HTTP 头缺失
///   时 None；M105 迭代32 碴6a）
/// - 返回：落位后的 llama-server 路径
pub async fn install_manual<F>(url: &str, mut on_progress: F) -> RoxidResult<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    let _guard = INSTALL_LOCK.lock().await;
    let dir = manual_dir();
    let download_url = apply_gh_proxy(url);
    if download_url != url {
        tracing::info!("手动安装 llama.cpp 运行时（经 GitHub 代理）：{download_url}");
    } else {
        tracing::info!("手动安装 llama.cpp 运行时：{download_url}");
    }
    let bytes = download_all_with_progress(&download_url, on_progress).await?;
    place_runtime(&bytes, url_is_archive(url), &dir).await?;
    Ok(manual_server_path())
}

/// 安装互斥锁（M29 碴7，R1-A）：跨模型并行冷加载（M27 起加载在全局写锁外）
/// 首次安装同变体时串行化——消除 cache_dir 的并发写与解压竞争
///（tokio Mutex::new 非 const，经 LazyLock 惰性初始化）
static INSTALL_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// 下载官方 tar.gz 并解压到缓存目录（下载完整字节后一次性落盘）。
/// M29 碴7（R1-A）：进程内互斥串行 + 解压落位至 {cache_dir}.installing
/// 临时目录、成功后 rename 原子替换——半途失败不污染缓存目录（无残缺
/// llama-server，下次调用自动重试）；锁内重查缓存使并发等待者直接复用。
/// M36：tag 参数化（`roxid runtime install <tag>` 公开入口）。
///
/// - 参数 tag：版本 tag（b\d+ 形态，调用方校验）
/// - 参数 backend_variant：变体片段
/// - 参数 on_progress：进度回调（已下载字节, 总量 Option——HTTP 头缺失
///   时 None；M105 迭代32 碴6a）
/// - 返回：落位后的 llama-server 路径
pub async fn install_version<F>(
    tag: &str,
    backend_variant: &str,
    mut on_progress: F,
) -> RoxidResult<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    let _guard = INSTALL_LOCK.lock().await;
    let cache_dir = variant_cache_dir(backend_variant, tag);
    let server = cache_dir.join("llama-server");
    // 锁内重查：等锁期间前序并发任务可能已完成安装，直接复用
    if server.is_file() {
        return Ok(server);
    }

    let url = asset_url(backend_variant, tag);
    tracing::info!("下载 llama.cpp 运行时：{url}");
    let bytes = download_all_with_progress(&url, on_progress).await?;
    place_runtime(&bytes, true, &cache_dir).await?;
    Ok(server)
}

/// 已安装运行时版本清单（M36：`roxid runtime list` 数据源）。
/// 扫描 {root}/{tag}/{variant} 两级目录；manual 与 .installing 残留不计入。
///
/// - 返回：按 tag 字典序排列的已装条目（每条含该 tag 下全部已装变体）
pub fn list_installed() -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let Ok(tags) = std::fs::read_dir(llama_runtime_root()) else {
        return out;
    };
    for entry in tags.flatten() {
        let tag = entry.file_name().to_string_lossy().into_owned();
        if !is_valid_tag(&tag) {
            continue; // manual / 暂存残留 / 未知目录跳过
        }
        let mut variants = Vec::new();
        if let Ok(subs) = std::fs::read_dir(entry.path()) {
            for sub in subs.flatten() {
                if sub.path().is_dir() && sub.file_name().to_string_lossy().ends_with(".installing")
                {
                    continue;
                }
                let name = sub.file_name().to_string_lossy().into_owned();
                if sub.path().is_dir() && !name.starts_with('.') {
                    variants.push(name);
                }
            }
        }
        if !variants.is_empty() {
            out.push((tag, variants));
        }
    }
    out.sort();
    out
}

/// 删除已安装版本（M36：`roxid runtime rm <tag>` 数据源）。
/// tag 目录整体删除（含其下全部变体）；"manual" 保留字删除手动版本目录。
///
/// - 参数 tag：版本 tag 或 "manual"
/// - 返回：Ok(()) 删除完成；目录不存在时报错（CLI 提示先 runtime list 查看）
pub fn remove_version(tag: &str) -> RoxidResult<()> {
    let dir = if tag == "manual" {
        manual_dir()
    } else if is_valid_tag(tag) {
        llama_runtime_root().join(tag)
    } else {
        return Err(RoxidError::RunnerFailure(format!(
            "非法版本标识：{tag}（期望 b\\d+ 形态或 manual）"
        )));
    };
    if !dir.is_dir() {
        return Err(RoxidError::RunnerFailure(format!("版本未安装：{tag}")));
    }
    tokio::task::block_in_place(|| std::fs::remove_dir_all(dir))
        .map_err(|e| RoxidError::RunnerFailure(format!("删除 {tag} 失败：{e}")))
}

/// 某 tag 下已装变体目录名清单（迭代50 M189）：扫描 {root}/{tag}/ 子目录，
/// 过滤口径与 list_installed 同款（.installing 暂存与隐藏目录排除）——
/// use 变体词匹配与补全候选的数据源（磁盘事实零硬编码）。
///
/// - 参数 tag：版本 tag
/// - 返回：变体目录名列表（字典序；目录不存在为空）
pub fn variant_dirs_of(tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(subs) = std::fs::read_dir(llama_runtime_root().join(tag)) else {
        return out;
    };
    for sub in subs.flatten() {
        let name = sub.file_name().to_string_lossy().into_owned();
        if sub.path().is_dir() && !name.starts_with('.') && !name.ends_with(".installing") {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// use 变体词合法短词集（迭代50 M189，用户裁决 2026-09-12 15:55
/// 「roxid runtime use b10917 cuda」命令形态）：匹配语义见
/// resolve_variant_keyword；导出供 CLI 补全候选单一事实源。
pub const VARIANT_KEYWORDS: &[&str] = &["cuda", "vulkan", "cpu"];

/// use 变体词解析（迭代50 M189，用户裁决链 15:47/15:55/15:58）：
/// 词在该 tag 已装变体目录名（磁盘事实）中匹配，零硬编码——CUDA 版本号
/// 等一切细节只存在于用户落位的目录名中（裁决 15:58「新增一个硬编码
/// 有什么用」）：
/// ① 完整目录名精确命中优先（多版本并存时的精确指定通道）；
/// ② 短词按命名惯例匹配：cuda=目录名含 "cuda" 子串、vulkan=含
///    "vulkan"、cpu=不含两者的 ubuntu-* 目录。
///
/// - 参数 tag：版本 tag
/// - 参数 keyword：完整变体目录名或短词（VARIANT_KEYWORDS 成员）
/// - 返回：Ok(真实变体目录名)；Err 非法词/多命中/零命中（文案含候选清单）
pub fn resolve_variant_keyword(tag: &str, keyword: &str) -> RoxidResult<String> {
    let dirs = variant_dirs_of(tag);
    if dirs.iter().any(|d| d == keyword) {
        return Ok(keyword.to_string());
    }
    let matches: Vec<&String> = match keyword {
        "cuda" | "vulkan" => dirs.iter().filter(|d| d.contains(keyword)).collect(),
        "cpu" => dirs
            .iter()
            .filter(|d| !d.contains("cuda") && !d.contains("vulkan"))
            .collect(),
        _ => {
            return Err(RoxidError::RunnerFailure(format!(
                "非法变体：{keyword}（期望 {} 或完整变体目录名）",
                VARIANT_KEYWORDS.join(" / ")
            )));
        }
    };
    let joined = |list: &[&String]| {
        list.iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match matches.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(RoxidError::RunnerFailure(format!(
            "版本 {tag} 下没有匹配「{keyword}」的已装变体；已装：{}",
            if dirs.is_empty() {
                "（无）".to_string()
            } else {
                dirs.join(", ")
            }
        ))),
        many => Err(RoxidError::RunnerFailure(format!(
            "版本 {tag} 下有多个匹配「{keyword}」的已装变体：{}；请用完整目录名指定其一",
            joined(many)
        ))),
    }
}

/// 变体目录名形态安全校验（迭代50 M189）：default_variant 直接拼接缓存
/// 目录路径，防 config 手误写入路径穿越形态（is_valid_tag 同款防护思路）。
fn is_safe_variant_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
}

/// 生效变体解析（迭代50 M189，用户裁决链 15:47/15:55/15:58）：
/// config [runtime].default_variant（use 写入的真实目录名）与
/// default_version 同时在位且该目录 llama-server 在位时直接返回——
/// 零拼接零映射；缺省 / 形态非法 / 不在位（warn）一律回退探测
///（detect_backend().asset_variant()，未配置用户行为逐字节不变）。
/// 调用面：scheduler acquire / 复用第四键 / CLI install / update。
///
/// - 返回：生效变体目录名
pub fn effective_variant() -> RoxidResult<String> {
    let cfg = crate::config::load_persist_config();
    if let Some(variant) = cfg.runtime.default_variant {
        if !is_safe_variant_name(&variant) {
            tracing::warn!("默认变体 {variant} 形态非法，回退自动探测");
        } else {
            let in_place = cfg
                .runtime
                .default_version
                .as_deref()
                .filter(|t| is_valid_tag(t))
                .map(|t| {
                    variant_cache_dir(&variant, t)
                        .join("llama-server")
                        .is_file()
                })
                .unwrap_or(false);
            if in_place {
                return Ok(variant);
            }
            tracing::warn!(
                "默认变体 {variant} 未在位（default_version 未设或目录缺失），回退自动探测"
            );
        }
    }
    super::backend::detect_backend().asset_variant()
}

/// serve 启动扫描已装变体架构健康度（M99，迭代31，Q4 第一项：用户裁决
/// 「三项全做：启动扫描 warn（不阻断）」2026-09-10 18:21）——确认不匹配
/// 打 warn 提前暴露（原实现静默，用户要到首次推理 ENOENT 才发现）；
/// 不阻断启动、不自动删除（Q3：存量误装目录保留）。本地读文件毫秒级。
pub fn warn_installed_arch_mismatch() {
    for (tag, variants) in list_installed() {
        for variant in variants {
            let bin = llama_runtime_root()
                .join(&tag)
                .join(&variant)
                .join("llama-server");
            if let Some((bin_arch, host_arch)) = diagnose_arch_mismatch(&bin) {
                tracing::warn!(
                    "已装运行时 {tag}/{variant} 的 llama-server 为 {bin_arch}，\
                     与宿主 {host_arch} 不匹配，推理时将报错；\
                     可 `roxid runtime rm {tag}` 清理后重装"
                );
            }
        }
    }
}

/// 已装变体架构匹配性（M99，迭代31，Q3 校验 3：`roxid runtime list`
/// 标注数据源——不匹配变体名旁标「与宿主架构不匹配」警示）。
///
/// - 参数 tag / variant：list_installed 条目
/// - 返回：true 表示确认不匹配（ELF 解析失败 / 未知 e_machine 按 false 宽容）
pub fn installed_variant_arch_mismatch(tag: &str, variant: &str) -> bool {
    let bin = llama_runtime_root()
        .join(tag)
        .join(variant)
        .join("llama-server");
    diagnose_arch_mismatch(&bin).is_some()
}

/// 通用落位段（M31 抽取）：bytes → staging 解压或直接落位 → 校验
/// llama-server 存在 + 可执行位 → 原子 rename 到 dest_dir。
/// M30 碴5：全部 tokio 化（原 std 同步调用阻塞 worker 线程）。
///
/// - 参数 bytes：完整包字节（tar.gz 或裸二进制）
/// - 参数 is_archive：true 走 tar 解压（strip 顶层）；false 直接落为 llama-server
/// - 参数 dest_dir：最终落位目录
/// - 返回：Ok(()) 表示落位完成
async fn place_runtime(
    bytes: &[u8],
    is_archive: bool,
    dest_dir: &std::path::Path,
) -> RoxidResult<()> {
    // 解压暂存目录（与目标同父目录保证 rename 原子）；遗留为上次失败残留，先清理
    let staging = installing_dir_of(dest_dir);
    if staging.exists() {
        tokio::fs::remove_dir_all(&staging).await?;
    }
    tokio::fs::create_dir_all(&staging).await?;

    if is_archive {
        let archive = staging.join("runtime.tar.gz.part");
        tokio::fs::write(&archive, bytes).await?;
        // 调系统 tar 解压（Linux 标配），strip 顶层目录使产物平铺于暂存目录
        // （tokio Command 的 status().await 等待期间不占用 worker 线程）
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging)
            .arg("--strip-components=1")
            .status()
            .await?;
        tokio::fs::remove_file(&archive).await?; // 压缩包用后即删，节约磁盘
        if !status.success() {
            tokio::fs::remove_dir_all(&staging).await.ok();
            return Err(RoxidError::RunnerFailure(format!(
                "解压失败：{}",
                staging.display()
            )));
        }
    } else {
        // 裸二进制：字节即 llama-server 本体
        tokio::fs::write(staging.join("llama-server"), bytes).await?;
    }

    // 校验产物存在（压缩包顶层布局异常时无 llama-server → 显式报错不污染缓存）
    let server = staging.join("llama-server");
    if !server.is_file() {
        tokio::fs::remove_dir_all(&staging).await.ok();
        return Err(RoxidError::RunnerFailure(format!(
            "包内缺少 llama-server：{}",
            staging.display()
        )));
    }
    // 确保可执行位（官方包通常已带 x，此处双保险；裸二进制路径必须补）
    let mut perm = tokio::fs::metadata(&server).await?.permissions();
    perm.set_mode(0o755);
    tokio::fs::set_permissions(&server, perm).await?;

    // 原子落位：清掉空/残缺的旧目标后 rename（互斥锁内安全）
    if dest_dir.exists() {
        tokio::fs::remove_dir_all(dest_dir).await?;
    }
    tokio::fs::rename(&staging, dest_dir).await?;
    Ok(())
}

/// 安装暂存目录路径：{cache_dir}.installing（与目标同父目录，rename 原子）。
///
/// - 参数 cache_dir：正式缓存目录
/// - 返回：暂存目录路径
fn installing_dir_of(cache_dir: &std::path::Path) -> std::path::PathBuf {
    let mut s = cache_dir.as_os_str().to_os_string();
    s.push(".installing");
    std::path::PathBuf::from(s)
}

/// 整包下载：显式超时（连接 30s / 总量 600s）+ 3 次指数退避重试。
/// 运行时包实测约 16MB，无需分块续传；
/// 模型 blob 的分块并发与断点续传引擎在 registry 模块实现（M5 落地）。
/// M32 碴8：响应体读取失败同样计入重试——原实现 `?` 直接返回，连接
/// 中断/慢速下载的 body 中断绕过「3 次重试」语义直接 fail。
/// 字节量纲自适应（M105：周期日志共用；服务端仅日志用途保持简单两位精度）。
///
/// - 参数 n：字节数
/// - 返回：人类可读量纲串（MB/GB）
fn fmt_bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let v = n as f64;
    if v >= 1024.0 * MB {
        format!("{:.2} GB", v / (1024.0 * MB))
    } else {
        format!("{:.2} MB", v / MB)
    }
}

/// serve 自动下载场景的 5s 周期进度日志回调（M105，迭代32 碴6a）。
/// serve 进程无 TTY 无法渲染 spinner，以周期 info 行呈现总量/百分比；
/// 首 chunk 即打一行（下载开始可见），此后每 5s 一行。
///
/// - 返回：进度回调闭包（已下载字节, 总量 Option）
fn periodic_log_progress() -> impl FnMut(u64, Option<u64>) {
    let mut last_log: Option<std::time::Instant> = None;
    move |done: u64, total: Option<u64>| {
        let now = std::time::Instant::now();
        let due = last_log
            .map(|t| now.duration_since(t) >= Duration::from_secs(5))
            .unwrap_or(true);
        if !due {
            return;
        }
        match total {
            Some(t) if t > 0 => tracing::info!(
                "运行时下载进度：{}/{}（{}%）",
                fmt_bytes(done),
                fmt_bytes(t),
                done * 100 / t
            ),
            _ => tracing::info!("运行时下载进度：{}", fmt_bytes(done)),
        }
        last_log = Some(now);
    }
}

/// M49（迭代18 BUG-13）：4xx 确定性失败（404 tag 不存在等）立即报错
/// 不重试——重试必然同果，徒增 3 次等待与日志噪音；429（限流）属
/// 瞬态保留重试，与 5xx/网络错误/超时同通道。
/// M105（迭代32 碴6a）：body 改流式累计并逐 chunk 回调进度
///（content-length 为总量；头缺失时 None）——原 resp.bytes() 一次性
/// 读取全程静默。重试语义不变（body 中断落 last_error 重试）。
///
/// - 参数 url：下载地址
/// - 参数 on_progress：进度回调（已下载字节, 总量 Option）
/// - 返回：完整字节
async fn download_all_with_progress<F>(url: &str, mut on_progress: F) -> RoxidResult<Vec<u8>>
where
    F: FnMut(u64, Option<u64>),
{
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| RoxidError::RegistryRequest(format!("构建 HTTP 客户端失败：{e}")))?;

    let mut last_error = String::new();
    for attempt in 1..=3 {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let total = resp.content_length();
                let mut buf: Vec<u8> =
                    Vec::with_capacity(total.unwrap_or(0).min(64 << 20) as usize);
                let mut stream = resp.bytes_stream();
                // M32 碴8：body 读取失败记入 last_error 落入重试（不直接返回）
                let mut body_failed = false;
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(bytes) => {
                            buf.extend_from_slice(&bytes);
                            on_progress(buf.len() as u64, total);
                        }
                        Err(e) => {
                            last_error = format!("读取响应体失败：{e}");
                            body_failed = true;
                            break;
                        }
                    }
                }
                if !body_failed {
                    return Ok(buf);
                }
            }
            Ok(resp) => {
                // 迭代18 BUG-13（M49）：确定性失败快速退出（429 除外）
                if resp.status().as_u16() != 429 && resp.status().is_client_error() {
                    return Err(RoxidError::RegistryRequest(format!(
                        "下载失败（确定性错误，不重试）{url}：HTTP {}",
                        resp.status()
                    )));
                }
                last_error = format!("HTTP {}", resp.status());
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }
        tracing::warn!("下载失败（第 {attempt}/3 次）{url}：{last_error}");
        if attempt < 3 {
            tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await;
            // 1s/2s 退避
        }
    }
    Err(RoxidError::RegistryRequest(format!(
        "重试 3 次仍失败 {url}：{last_error}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 代理前缀生效：设置变量走前缀拼接，未设置直连（Q11：无默认网址）。
    /// ROXID_HOME 隔离：排除开发机真实 config.toml 对「未设置直连」断言的干扰。
    #[test]
    fn asset_url_respects_gh_proxy_env() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::set_var("ROXID_HOME", "/tmp/roxid-gh-env-test");
        std::env::set_var(ENV_GH_PROXY, "https://my.mirror.dev/");
        assert_eq!(
            asset_url("ubuntu-x64", "b10605"),
            "https://my.mirror.dev/https://github.com/ggml-org/llama.cpp/releases/download/b10605/llama-b10605-bin-ubuntu-x64.tar.gz"
        );
        std::env::remove_var(ENV_GH_PROXY);
        // 未设置 env 且无 config.toml 时必须直连（禁止代码内默认代理）
        assert_eq!(
            asset_url("ubuntu-x64", "b10605"),
            github_raw_url("ubuntu-x64", "b10605")
        );
        // M36：任意 tag 的 URL 拼接同样走代理前缀链
        assert_eq!(
            asset_url("ubuntu-vulkan-x64", "b99999"),
            "https://github.com/ggml-org/llama.cpp/releases/download/b99999/llama-b99999-bin-ubuntu-vulkan-x64.tar.gz"
        );
        std::env::remove_var("ROXID_HOME");
    }

    /// 读取链回退：env 未设置时取 config.toml [proxy].gh；env 存在时优先于文件
    ///（迭代4 裁决 Q2：env 优先 → 文件回退）
    #[test]
    fn gh_proxy_prefix_falls_back_to_config_file() {
        use crate::config::{
            save_persist_config, PersistConfig, ProxySection, ROXID_HOME_TEST_LOCK,
        };
        let _guard = ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-gh-fb-{}", std::process::id()));
        std::env::set_var("ROXID_HOME", &dir);
        std::env::remove_var(ENV_GH_PROXY);
        save_persist_config(&PersistConfig {
            setup_done: true,
            proxy: ProxySection {
                gh: Some("https://file.mirror.dev/".into()),
                hf: None,
            },
            runtime: Default::default(),
        })
        .expect("写盘必须成功");
        assert_eq!(
            gh_proxy_prefix(),
            "https://file.mirror.dev/",
            "文件回退必须生效"
        );
        std::env::set_var(ENV_GH_PROXY, "https://env.mirror.dev/");
        assert_eq!(
            gh_proxy_prefix(),
            "https://env.mirror.dev/",
            "env 必须优先于文件"
        );
        std::env::remove_var(ENV_GH_PROXY);
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// M87（迭代27）：手动链代理拼接四情形——GitHub 域名 + 代理前置拼接；
    /// GitHub 域名 + 无代理原样直连；非 GitHub 域名（自建镜像 / file://）
    /// 原样直用；已是代理前缀形态的完整地址不以 github.com 开头、原样
    /// 直用（天然防二次拼接）。ROXID_HOME 隔离排除开发机真实 config 干扰。
    #[test]
    fn apply_gh_proxy_manual_chain() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::set_var("ROXID_HOME", "/tmp/roxid-gh-manual-test");
        std::env::set_var(ENV_GH_PROXY, "https://my.mirror.dev/");
        // 情形1：GitHub 域名 URL 前置拼接代理前缀（与 asset_url 同语义）
        assert_eq!(
            apply_gh_proxy("https://github.com/ggml-org/llama.cpp/releases/download/b10883/llama-b10883-bin-ubuntu-vulkan-x64.tar.gz"),
            "https://my.mirror.dev/https://github.com/ggml-org/llama.cpp/releases/download/b10883/llama-b10883-bin-ubuntu-vulkan-x64.tar.gz"
        );
        // 情形4：已是代理前缀形态的完整地址原样直用，不二次拼接
        assert_eq!(
            apply_gh_proxy("https://my.mirror.dev/https://github.com/ggml-org/llama.cpp/releases/download/b10883/pkg.tar.gz"),
            "https://my.mirror.dev/https://github.com/ggml-org/llama.cpp/releases/download/b10883/pkg.tar.gz"
        );
        // 情形3：非 GitHub 域名（自建镜像 / file://）原样直用
        assert_eq!(
            apply_gh_proxy("https://files.example.com/llama-server"),
            "https://files.example.com/llama-server"
        );
        assert_eq!(
            apply_gh_proxy("file:///tmp/pkg.tar.gz"),
            "file:///tmp/pkg.tar.gz"
        );
        // 情形2：GitHub 域名但未配置代理（env 清除且无 config）→ 原样直连
        std::env::remove_var(ENV_GH_PROXY);
        assert_eq!(
            apply_gh_proxy(
                "https://github.com/ggml-org/llama.cpp/releases/download/b10883/pkg.tar.gz"
            ),
            "https://github.com/ggml-org/llama.cpp/releases/download/b10883/pkg.tar.gz"
        );
        std::env::remove_var("ROXID_HOME");
    }

    /// M29 碴7：安装暂存目录命名必须紧贴目标目录（同父目录保证 rename 原子）
    #[test]
    fn installing_dir_naming() {
        assert_eq!(
            installing_dir_of(&std::path::Path::new("/c/llama.cpp/b10605/ubuntu-x64")),
            std::path::PathBuf::from("/c/llama.cpp/b10605/ubuntu-x64.installing")
        );
    }

    /// GitHub 原始 URL 必须与官方实测命名逐字一致（b10605 实测 2026-08-24 18:41）
    #[test]
    fn raw_url_matches_official_naming() {
        assert_eq!(
            github_raw_url("ubuntu-vulkan-x64", "b10605"),
            "https://github.com/ggml-org/llama.cpp/releases/download/b10605/llama-b10605-bin-ubuntu-vulkan-x64.tar.gz"
        );
    }

    /// 缓存目录必须落在 {ROXID_HOME}/llama.cpp/{tag}/{variant}/（M36：任意 tag）
    #[test]
    fn cache_dir_layout() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::set_var("ROXID_HOME", "/tmp/roxid-rt-layout");
        assert_eq!(
            variant_cache_dir("ubuntu-x64", "b10605"),
            PathBuf::from("/tmp/roxid-rt-layout/llama.cpp/b10605/ubuntu-x64")
        );
        assert_eq!(
            variant_cache_dir("ubuntu-vulkan-x64", "b99999"),
            PathBuf::from("/tmp/roxid-rt-layout/llama.cpp/b99999/ubuntu-vulkan-x64")
        );
        std::env::remove_var("ROXID_HOME");
    }

    /// M36（R5）：tag 形态校验——b+纯数字通过；manual/b 缺数字/其他词拒绝
    #[test]
    fn tag_validation() {
        assert!(is_valid_tag("b10605"));
        assert!(is_valid_tag("b0"));
        assert!(!is_valid_tag("manual"));
        assert!(!is_valid_tag("b"));
        assert!(!is_valid_tag("10605"));
        assert!(!is_valid_tag("bx"));
        assert!(!is_valid_tag(""));
    }

    /// 迭代46 M173：GitHub Releases 最新预发布 tag 选取——prerelease
    /// 过滤 / draft 排除 / created_at 降序 / 空候选 None（用户 curl 命令
    /// jq 管道语义锚定，样本含晚于目标但非预发布与草稿的干扰项）
    #[test]
    fn pick_latest_prerelease_tag_semantics() {
        let releases: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
              {"tag_name":"b10909","prerelease":true,"draft":false,"created_at":"2026-09-11T10:00:00Z"},
              {"tag_name":"b10883","prerelease":true,"draft":false,"created_at":"2026-09-10T08:00:00Z"},
              {"tag_name":"v1.0.0","prerelease":false,"draft":false,"created_at":"2026-09-11T12:00:00Z"},
              {"tag_name":"b10910","prerelease":true,"draft":true,"created_at":"2026-09-11T11:00:00Z"}
            ]"#,
        )
        .unwrap();
        assert_eq!(
            pick_latest_prerelease_tag(&releases).as_deref(),
            Some("b10909"),
            "v1.0.0 非预发布、b10910 为草稿，均排除；b10909 created_at 最新的合法候选"
        );
        assert_eq!(pick_latest_prerelease_tag(&[]), None, "空列表无候选");
    }

    /// 迭代48 M181：pick_prerelease_tags 语义——prerelease 过滤 / draft
    /// 排除 / created_at 降序 / is_valid_tag 剔除 / limit 截断（Q3-B/Q4-A
    /// 裁决 2026-09-12 04:24/04:25，install tag 补全候选数据源锚定）
    #[test]
    fn pick_prerelease_tags_semantics() {
        let releases: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
              {"tag_name":"b10909","prerelease":true,"draft":false,"created_at":"2026-09-11T10:00:00Z"},
              {"tag_name":"b10883","prerelease":true,"draft":false,"created_at":"2026-09-10T08:00:00Z"},
              {"tag_name":"v1.0.0","prerelease":false,"draft":false,"created_at":"2026-09-11T12:00:00Z"},
              {"tag_name":"not-a-tag","prerelease":true,"draft":false,"created_at":"2026-09-11T11:00:00Z"},
              {"tag_name":"b10910","prerelease":true,"draft":true,"created_at":"2026-09-11T11:30:00Z"}
            ]"#,
        )
        .unwrap();
        // 宽容语义：非法形态剔除（not-a-tag 不出现）、非预发布与草稿排除、降序
        assert_eq!(
            pick_prerelease_tags(&releases, 10),
            vec!["b10909".to_string(), "b10883".to_string()],
            "not-a-tag 剔除、v1.0.0/b10910 干扰排除、created_at 降序"
        );
        // limit 截断：只取最新一个
        assert_eq!(
            pick_prerelease_tags(&releases, 1),
            vec!["b10909".to_string()]
        );
        // 空列表零候选
        assert!(pick_prerelease_tags(&[], 10).is_empty());
    }

    /// M36：list_installed 扫描 {tag}/{variant} 两级目录——manual 与
    /// .installing 残留不计入；空 tag 目录（无变体）不计入；结果按 tag 排序
    #[test]
    fn list_installed_scans_tag_dirs() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-rt-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        let root = llama_runtime_root();
        std::fs::create_dir_all(root.join("b99999").join("ubuntu-vulkan-x64")).unwrap();
        std::fs::create_dir_all(root.join("b10605").join("ubuntu-x64")).unwrap();
        std::fs::create_dir_all(root.join("manual")).unwrap();
        std::fs::create_dir_all(root.join("b88888.installing").join("ubuntu-x64")).unwrap();
        std::fs::create_dir_all(root.join("b77777")).unwrap(); // 空版本目录不计入

        let list = list_installed();
        assert_eq!(
            list,
            vec![
                ("b10605".to_string(), vec!["ubuntu-x64".to_string()]),
                ("b99999".to_string(), vec!["ubuntu-vulkan-x64".to_string()]),
            ],
            "manual/暂存/空版本目录必须排除，按 tag 排序"
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// M36：remove_version——tag 目录整体删除；未安装报错；非法标识报错；
    /// manual 保留字删手动版本目录
    #[test]
    fn remove_version_deletes_and_rejects() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-rt-rm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        let root = llama_runtime_root();
        std::fs::create_dir_all(root.join("b99999").join("ubuntu-x64")).unwrap();
        std::fs::create_dir_all(root.join("manual")).unwrap();

        remove_version("b99999").unwrap();
        assert!(!root.join("b99999").exists(), "tag 目录必须整体删除");
        assert!(remove_version("b99999").is_err(), "重复删除必须报错");
        assert!(remove_version("b12ab").is_err(), "非法 tag 必须报错");
        remove_version("manual").unwrap();
        assert!(!root.join("manual").exists(), "manual 保留字删除手动目录");
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// M31 F2：URL 形态判定——压缩包后缀（剥 query/fragment）走解压，其余裸二进制
    #[test]
    fn url_archive_form_judgement() {
        assert!(url_is_archive(
            "https://example.dev/llama-b9999-bin-ubuntu-vulkan-x64.tar.gz"
        ));
        assert!(url_is_archive("https://example.dev/pkg.tgz"));
        assert!(url_is_archive(
            "https://example.dev/pkg.tar.gz?token=abc#frag"
        ));
        assert!(!url_is_archive("https://example.dev/llama-server"));
        assert!(!url_is_archive(
            "https://example.dev/download?file=llama-server"
        ));
    }

    /// M31 F2（R2）：manual 版本单目录落位路径
    #[test]
    fn manual_dir_layout() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::set_var("ROXID_HOME", "/tmp/roxid-rt-manual");
        assert_eq!(
            manual_server_path(),
            PathBuf::from("/tmp/roxid-rt-manual/llama.cpp/manual/llama-server")
        );
        std::env::remove_var("ROXID_HOME");
    }

    /// M31 F2 + M36：ensure_llama_server 优先级——manual 存在时优先于变体缓存
    ///（不触发自动下载链；env 逃生口语义维持在前）；
    /// M36 扩展：default_version 已装时优先于兜底链缓存；
    /// 迭代46 M173：原「未装回退兜底链缓存」断言删除（第4步起先联网查
    /// tag，API 返回值不定，离线不可锚定——在线回退由实机验收覆盖）
    #[tokio::test]
    async fn ensure_prefers_manual_over_variant_cache() {
        use crate::config::{save_persist_config, PersistConfig};
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-rt-prio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        std::env::remove_var("ROXID_LLAMA_SERVER");

        // 无任何缓存：不触发下载的断言不可行（会真联网），改为直接构造两处缓存
        let manual = manual_dir();
        std::fs::create_dir_all(&manual).unwrap();
        std::fs::write(manual.join("llama-server"), b"manual-build").unwrap();
        // 迭代46 M173：锁定常量已删除，改历史字面量锚定（本测试语义为
        // manual 优先于变体缓存，tag 值无关紧要，不触第4步网络路径）
        let variant = variant_cache_dir("ubuntu-vulkan-x64", "b10883");
        std::fs::create_dir_all(&variant).unwrap();
        std::fs::write(variant.join("llama-server"), b"variant-build").unwrap();

        let got = ensure_llama_server("ubuntu-vulkan-x64").await.unwrap();
        assert_eq!(
            got,
            manual.join("llama-server"),
            "manual 必须优先于变体缓存"
        );

        // 迭代46 M173：原「删 manual 后回退兜底缓存」断言删除（第4步联网
        // 查 tag 离线不可锚定，见函数 doc 注记）；删 manual 动作保留——
        // 段3 需越过第2层验证 default_version 优先级
        std::fs::remove_dir_all(&manual).unwrap();

        // M36：default_version 指向已装 tag 时优先于兜底缓存
        let custom = variant_cache_dir("ubuntu-vulkan-x64", "b99999");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("llama-server"), b"b99999-build").unwrap();
        save_persist_config(&PersistConfig {
            setup_done: true,
            proxy: Default::default(),
            runtime: crate::config::RuntimeSection {
                llama_url: None,
                default_version: Some("b99999".into()),
                // M181（迭代48）：新字段补 None（本测试只锚定 default_version 优先级）；
                // M190（迭代50）：default_variant 同款补 None
                default_variant: None,
                tag_complete_limit: None,
            },
        })
        .unwrap();
        let by_config = ensure_llama_server("ubuntu-vulkan-x64").await.unwrap();
        assert_eq!(
            by_config,
            custom.join("llama-server"),
            "default_version 已装时必须优先于兜底缓存"
        );

        // 迭代46 M173：原「default_version 未装回退兜底缓存」段删除
        //（同上：第4步联网查 tag 离线不可锚定）
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// M31 F2：place_runtime 裸二进制形态——字节直接落为 llama-server + 755 + 原子落位
    #[tokio::test]
    async fn place_runtime_bare_binary_form() {
        let dir = std::env::temp_dir().join(format!("roxid-rt-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dest = dir.join("manual");
        place_runtime(b"#!/bin/sh\necho fake-llama-server\n", false, &dest)
            .await
            .unwrap();
        let server = dest.join("llama-server");
        assert!(server.is_file(), "裸二进制必须直接落位");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&server).unwrap().permissions().mode() & 0o111,
            0o111,
            "可执行位必须补齐"
        );
        assert!(!installing_dir_of(&dest).exists(), "暂存目录必须已回收");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// M32 碴8：body 中断纳入重试——坏源 200 后虚报 Content-Length（声明
    /// 1024 实写 10 字节即断连），3 次重试耗尽后显式报错且信息含读取失败
    ///（原实现 body 读取失败直接返回，绕过重试循环）
    #[tokio::test]
    #[ignore = "需要 python3 替身服务，验收时手动执行"]
    async fn body_interrupt_errors_after_retries() {
        let port = 38217u16;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(format!(
            r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        # 坏源行为：200 + 虚报 Content-Length（1024）但只写 10 字节即断连
        self.send_response(200)
        self.send_header('Content-Length', '1024')
        self.end_headers()
        self.wfile.write(b'0123456789')
        self.connection.close()
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', {port}), H).serve_forever()
"#
        ));
        let _stub =
            crate::scheduler::Runner::spawn_with("stub", port, Duration::from_secs(300), &mut cmd)
                .await
                .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let err = download_all_with_progress(&format!("http://127.0.0.1:{port}/pkg"), |_, _| {})
            .await
            .expect_err("body 中断必须重试耗尽后报错");
        assert!(
            err.to_string().contains("读取响应体失败"),
            "M32 碴8：错误信息必须含 body 读取失败原因：{err}"
        );
    }

    /// M49（迭代18 BUG-13）：404 确定性失败必须快速报错——单次请求即
    /// 终止（原实现盲重试 3 次约 4s）。替身统计请求数，断言仅 1 次。
    #[tokio::test]
    #[ignore = "需要 python3 替身服务，验收时手动执行"]
    async fn not_found_fails_fast_without_retry() {
        let port = 38219u16;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(format!(
            r#"
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    count = 0
    def do_GET(self):
        H.count += 1
        self.send_response(404)
        self.end_headers()
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', {port}), H).serve_forever()
"#
        ));
        let _stub = crate::scheduler::Runner::spawn_with(
            "stub404",
            port,
            Duration::from_secs(300),
            &mut cmd,
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let start = std::time::Instant::now();
        let err = download_all_with_progress(&format!("http://127.0.0.1:{port}/pkg"), |_, _| {})
            .await
            .expect_err("404 必须报错");
        assert!(
            err.to_string().contains("确定性错误"),
            "错误文案必须标明不重试语义：{err}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "必须单次快速失败（无 1s/2s 退避等待）：实际 {:?}",
            start.elapsed()
        );
    }

    /// 真实下载验收（需网络，代理经 ROXID_GH_PROXY 注入，M2 验收手动执行）：
    /// ROXID_GH_PROXY=... cargo test -p roxid-server runtime -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "真实网络下载，验收时手动执行"]
    #[allow(clippy::await_holding_lock)]
    async fn ensure_llama_server_downloads_and_caches() {
        // 迭代42 D11（注记B 清偿 2026-09-11）：--include-ignored 全并行时
        // 与读 ROXID_HOME env 的常规测试（gh_proxy_prefix_falls_back_to_
        // config_file 等）互扰——env 全局态须持锁串行（同文件既有模式；
        // 锁跨 await 持有为该测试族既定语义）
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::set_var("ROXID_HOME", "/tmp/roxid-rt-e2e");
        let path = ensure_llama_server("ubuntu-x64").await.unwrap();
        assert!(path.is_file(), "llama-server 未落位：{path:?}");
        // 二次调用必须命中缓存且路径一致（无重复下载）
        let again = ensure_llama_server("ubuntu-x64").await.unwrap();
        assert_eq!(path, again);
        std::env::remove_var("ROXID_HOME");
    }

    /// 构造与宿主交叉架构的 ELF64LE 假头（M99 测试工具：x64 宿主产 arm64
    /// 头、arm64 宿主产 x64 头——测试宿主无关）
    fn cross_arch_elf_header() -> [u8; 20] {
        let cross_machine: u16 = if super::super::backend::host_arch_fragment() == Some("arm64") {
            0x3e
        } else {
            0xb7
        };
        let mut h = [0u8; 20];
        h[..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        h[4] = 2;
        h[5] = 1;
        h[18..20].copy_from_slice(&cross_machine.to_le_bytes());
        h
    }

    /// M99（迭代31，Q3 校验 2）：ensure 命中已装版本缓存时 ELF 架构校验——
    /// 交叉架构假 llama-server 必须报 ArchMismatch（原实现放行到 spawn
    /// 阶段才裸报 os error 2），且报错即返回不触发下载。
    /// 迭代46 M173：原「锁定链缓存」场景改经 default_version 通道构造
    ///（锁定常量已删除、兜底链第4步先联网查 tag 无法离线构造）——
    /// ensure 第3层同走 ensure_arch_matches，ELF 校验逻辑等价覆盖。
    /// allow：锁须跨 await 持有——ROXID_HOME 全局环境隔离的完整语义
    ///（与 registry.rs 存量测试同模式，清偿标准：不新增即挂账容忍）
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ensure_rejects_cached_arch_mismatch() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-arch-ensure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        std::env::remove_var(ENV_LLAMA_SERVER_OVERRIDE);
        crate::config::save_persist_config(&crate::config::PersistConfig {
            runtime: crate::config::RuntimeSection {
                default_version: Some("b99999".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .expect("测试 config 写盘必须成功");
        let variant = variant_cache_dir("ubuntu-x64", "b99999");
        std::fs::create_dir_all(&variant).unwrap();
        std::fs::write(variant.join("llama-server"), cross_arch_elf_header()).unwrap();

        let err = ensure_llama_server("ubuntu-x64")
            .await
            .expect_err("交叉架构缓存必须报错而非放行");
        assert!(
            matches!(err, RoxidError::ArchMismatch(_)),
            "必须为 ArchMismatch 诊断错误：{err}"
        );
        assert!(
            err.to_string().contains("不匹配"),
            "文案必须含架构诊断结论：{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// M99（迭代31）：resolve 双链同步——命中但交叉架构视为不可得（None），
    /// 调用方保守放行、诊断由 ensure 承担（既有分工维持）；
    /// 非 ELF 假二进制宽容面照常命中（不误伤自备非常规产物）。
    /// 迭代46 M174：原「锁定链缓存」场景改经 default_version 通道构造
    ///（resolve 第4步已改 None 放行——兜底链缓存不再被 resolve 检查，
    /// 第3层 default_version 命中面维持 ELF 校验语义等价覆盖）。
    #[test]
    fn resolve_treats_arch_mismatch_as_unavailable() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-arch-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        std::env::remove_var(ENV_LLAMA_SERVER_OVERRIDE);
        crate::config::save_persist_config(&crate::config::PersistConfig {
            runtime: crate::config::RuntimeSection {
                default_version: Some("b99999".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .expect("测试 config 写盘必须成功");
        let variant = variant_cache_dir("ubuntu-x64", "b99999");
        std::fs::create_dir_all(&variant).unwrap();

        std::fs::write(variant.join("llama-server"), cross_arch_elf_header()).unwrap();
        assert_eq!(
            resolve_llama_server_path("ubuntu-x64"),
            None,
            "交叉架构命中必须视为不可得"
        );

        std::fs::write(variant.join("llama-server"), b"#!/bin/sh\necho fake\n").unwrap();
        assert!(
            resolve_llama_server_path("ubuntu-x64").is_some(),
            "非 ELF 宽容面必须照常命中"
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// 迭代50 M189：use 变体词解析——完整目录名精确命中优先、短词
    /// 子串/排除匹配、多命中/零命中/非法词报错文案含候选清单
    ///（磁盘事实零硬编码，用户裁决链 15:47/15:55/15:58）
    #[test]
    fn resolve_variant_keyword_matrix() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-var-kw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        for v in ["ubuntu-cuda-12.4-x64", "ubuntu-vulkan-x64", "ubuntu-x64"] {
            std::fs::create_dir_all(llama_runtime_root().join("b99001").join(v)).unwrap();
        }
        // 完整目录名精确命中（多版本并存时的精确指定通道）
        assert_eq!(
            resolve_variant_keyword("b99001", "ubuntu-cuda-12.4-x64").unwrap(),
            "ubuntu-cuda-12.4-x64"
        );
        // 短词匹配：cuda 子串 / vulkan 子串 / cpu 排除（不含 cuda 与 vulkan）
        assert_eq!(
            resolve_variant_keyword("b99001", "cuda").unwrap(),
            "ubuntu-cuda-12.4-x64"
        );
        assert_eq!(
            resolve_variant_keyword("b99001", "vulkan").unwrap(),
            "ubuntu-vulkan-x64"
        );
        assert_eq!(
            resolve_variant_keyword("b99001", "cpu").unwrap(),
            "ubuntu-x64"
        );
        // 多命中：报错列出全部候选引导完整目录名（不隐式择新）
        std::fs::create_dir_all(
            llama_runtime_root()
                .join("b99001")
                .join("ubuntu-cuda-12.6-x64"),
        )
        .unwrap();
        let err = resolve_variant_keyword("b99001", "cuda").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ubuntu-cuda-12.4-x64") && msg.contains("ubuntu-cuda-12.6-x64"));
        // 零命中：报错列出该 tag 已装变体清单；非法词：报错期望形态
        assert!(resolve_variant_keyword("b99002", "cuda")
            .unwrap_err()
            .to_string()
            .contains("没有匹配"));
        assert!(resolve_variant_keyword("b99001", "foo")
            .unwrap_err()
            .to_string()
            .contains("非法变体"));
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// 迭代50 M189：生效变体解析——default_variant 与 default_version
    /// 同时在位且目录在位时直用（原样返回真实目录名）；目录缺失/
    /// 形态非法/缺省一律回退探测（未配置用户零变化锚定）
    #[test]
    fn effective_variant_priority_matrix() {
        let _guard = crate::config::ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-eff-var-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("ROXID_HOME", &dir);
        std::env::remove_var(ENV_LLAMA_SERVER_OVERRIDE);
        // 未配置：回退探测（与 asset_variant 等价——未配置用户零变化锚定）
        let probed = crate::runtime::detect_backend()
            .asset_variant()
            .expect("发布矩阵内宿主探测不应报错");
        assert_eq!(effective_variant().unwrap(), probed);
        // 在位：直用 default_variant（零拼接零映射——原样返回真实目录名）
        let v = variant_cache_dir("ubuntu-cuda-12.4-x64", "b99003");
        std::fs::create_dir_all(&v).unwrap();
        std::fs::write(v.join("llama-server"), b"fake").unwrap();
        crate::config::save_persist_config(&crate::config::PersistConfig {
            runtime: crate::config::RuntimeSection {
                default_version: Some("b99003".into()),
                default_variant: Some("ubuntu-cuda-12.4-x64".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            effective_variant().unwrap(),
            "ubuntu-cuda-12.4-x64",
            "在位时必须直用 default_variant（use 写入的真实目录名）"
        );
        // 目录被删：warn 回退探测
        std::fs::remove_dir_all(&v).unwrap();
        assert_eq!(
            effective_variant().unwrap(),
            probed,
            "目录不在位必须回退探测"
        );
        // 形态非法（路径穿越防护）：回退探测
        crate::config::save_persist_config(&crate::config::PersistConfig {
            runtime: crate::config::RuntimeSection {
                default_version: Some("b99003".into()),
                default_variant: Some("../evil".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            effective_variant().unwrap(),
            probed,
            "非法形态必须回退探测（路径穿越防护）"
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }
}
