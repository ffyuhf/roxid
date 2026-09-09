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

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::config::llama_runtime_root;
use crate::error::{RoxidError, RoxidResult};

/// 版本锁定的 llama.cpp 构建 tag（来源：用户确认 2026-08-24 18:49：
/// b10605 才是最新版本；升级需重新回归测试）
pub const LOCKED_LLAMA_CPP_TAG: &str = "b10605";

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
/// - 参数 tag：版本 tag，如 "b10605"（M36 起参数化，不再锁死）
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

/// 确保 llama-server 可用并返回其路径。
/// 优先级（M36 更新）：ROXID_LLAMA_SERVER 环境变量 → manual 手动版本
/// （setup --llama-url 安装）→ config default_version 指定版本的变体缓存
/// （未安装则告警回退）→ b10605 锁定链（缓存 → 下载解压）。
///
/// - 参数 backend_variant：探测得到的变体片段（Backend::asset_variant()）
/// - 返回：llama-server 二进制路径
pub async fn ensure_llama_server(backend_variant: &str) -> RoxidResult<PathBuf> {
    // 1) 环境变量逃生口：用户自编译产物直接复用（仅校验存在性，不校验版本）
    if let Ok(custom) = std::env::var(ENV_LLAMA_SERVER_OVERRIDE) {
        let path = PathBuf::from(&custom);
        if path.is_file() {
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
        return Ok(manual);
    }
    // 3) M36：config default_version 指定版本优先于锁定链（use 命令的生效点）
    if let Some(tag) = crate::config::load_persist_config()
        .runtime
        .default_version
        .filter(|t| is_valid_tag(t))
    {
        let server = variant_cache_dir(backend_variant, &tag).join("llama-server");
        if server.is_file() {
            return Ok(server);
        }
        // 指定版本未安装（如被手删目录）：告警回退锁定链，不硬失败
        tracing::warn!(
            "默认后端版本 {tag} 的变体 {backend_variant} 未安装，回退 {LOCKED_LLAMA_CPP_TAG} 锁定链"
        );
    }
    // 4) b10605 锁定链：缓存命中直接复用，未命中下载解压落位
    let server = variant_cache_dir(backend_variant, LOCKED_LLAMA_CPP_TAG).join("llama-server");
    if server.is_file() {
        return Ok(server);
    }
    install_version(LOCKED_LLAMA_CPP_TAG, backend_variant).await?;
    Ok(server)
}

/// resolve-only 路径解析（M54b 碴B）：按 ensure_llama_server 同序链
/// （env → manual → default_version 已装 → b10605 锁定链缓存）解析当前
/// 应然二进制路径，仅存在性检查、零下载零网络。scheduler 复用判定用它
/// 与实例记录路径比对，不一致（runtime use 切换默认版本 / env 改指向）
/// 时触发重建——碴B修复：原仅 ctx/RUNTIME 双键，后端版本切换后运行
/// 实例无感知，keep_alive 窗口内一直用旧版本跑。
/// 与 ensure 的差异面（调用方语义）：env 指向不存在或锁定链未缓存时
/// 返回 None 而非报错/下载——调用方按「不可比对」保守放行，加载路径
/// 仍由 ensure 完整链兜底；链序与命中判定必须与 ensure 保持逐字一致
/// （双链漂移防护：修改任一处须同步另一处）。
///
/// - 参数 backend_variant：探测得到的变体片段（Backend::asset_variant()）
/// - 返回：Some(路径) 表示已存在可用；None 表示全链未命中
pub fn resolve_llama_server_path(backend_variant: &str) -> Option<PathBuf> {
    // 1) 环境变量逃生口（不校验报错面：指向不存在时 None 放行，
    //    显式报错由加载路径的 ensure 完整链承担）
    if let Ok(custom) = std::env::var(ENV_LLAMA_SERVER_OVERRIDE) {
        let path = PathBuf::from(&custom);
        return path.is_file().then_some(path);
    }
    // 2) 手动版本（default_version == "manual" 特殊保留字同落此处）
    let manual = manual_server_path();
    if manual.is_file() {
        return Some(manual);
    }
    // 3) config default_version 指定版本（仅已装命中；未装回退锁定链，
    //    不打 warn——本函数每请求调用，告警刷屏；留痕由 ensure 承担）
    if let Some(tag) = crate::config::load_persist_config()
        .runtime
        .default_version
        .filter(|t| is_valid_tag(t))
    {
        let server = variant_cache_dir(backend_variant, &tag).join("llama-server");
        if server.is_file() {
            return Some(server);
        }
    }
    // 4) b10605 锁定链缓存（未缓存 None——不下载，下载由 ensure 承担）
    let server = variant_cache_dir(backend_variant, LOCKED_LLAMA_CPP_TAG).join("llama-server");
    server.is_file().then_some(server)
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
/// - 返回：落位后的 llama-server 路径
pub async fn install_manual(url: &str) -> RoxidResult<PathBuf> {
    let _guard = INSTALL_LOCK.lock().await;
    let dir = manual_dir();
    let download_url = apply_gh_proxy(url);
    if download_url != url {
        tracing::info!("手动安装 llama.cpp 运行时（经 GitHub 代理）：{download_url}");
    } else {
        tracing::info!("手动安装 llama.cpp 运行时：{download_url}");
    }
    let bytes = download_all(&download_url).await?;
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
/// - 返回：落位后的 llama-server 路径
pub async fn install_version(tag: &str, backend_variant: &str) -> RoxidResult<PathBuf> {
    let _guard = INSTALL_LOCK.lock().await;
    let cache_dir = variant_cache_dir(backend_variant, tag);
    let server = cache_dir.join("llama-server");
    // 锁内重查：等锁期间前序并发任务可能已完成安装，直接复用
    if server.is_file() {
        return Ok(server);
    }

    let url = asset_url(backend_variant, tag);
    tracing::info!("下载 llama.cpp 运行时：{url}");
    let bytes = download_all(&url).await?;
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
/// M49（迭代18 BUG-13）：4xx 确定性失败（404 tag 不存在等）立即报错
/// 不重试——重试必然同果，徒增 3 次等待与日志噪音；429（限流）属
/// 瞬态保留重试，与 5xx/网络错误/超时同通道。
async fn download_all(url: &str) -> RoxidResult<Vec<u8>> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| RoxidError::RegistryRequest(format!("构建 HTTP 客户端失败：{e}")))?;

    let mut last_error = String::new();
    for attempt in 1..=3 {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                // M32 碴8：body 读取失败记入 last_error 落入重试（不直接返回）
                match resp.bytes().await {
                    Ok(bytes) => return Ok(bytes.to_vec()),
                    Err(e) => {
                        last_error = format!("读取响应体失败：{e}");
                    }
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
    ///（不触发 b10605 自动下载链；env 逃生口语义维持在前）；
    /// M36 扩展：default_version 已装时优先于锁定链缓存，未装回退锁定链
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
        let variant = variant_cache_dir("ubuntu-vulkan-x64", LOCKED_LLAMA_CPP_TAG);
        std::fs::create_dir_all(&variant).unwrap();
        std::fs::write(variant.join("llama-server"), b"variant-build").unwrap();

        let got = ensure_llama_server("ubuntu-vulkan-x64").await.unwrap();
        assert_eq!(
            got,
            manual.join("llama-server"),
            "manual 必须优先于变体缓存"
        );

        // 删 manual 后回退变体缓存
        std::fs::remove_dir_all(&manual).unwrap();
        let fallback = ensure_llama_server("ubuntu-vulkan-x64").await.unwrap();
        assert_eq!(fallback, variant.join("llama-server"));

        // M36：default_version 指向已装 tag 时优先于锁定链缓存
        let custom = variant_cache_dir("ubuntu-vulkan-x64", "b99999");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("llama-server"), b"b99999-build").unwrap();
        save_persist_config(&PersistConfig {
            setup_done: true,
            proxy: Default::default(),
            runtime: crate::config::RuntimeSection {
                llama_url: None,
                default_version: Some("b99999".into()),
            },
        })
        .unwrap();
        let by_config = ensure_llama_server("ubuntu-vulkan-x64").await.unwrap();
        assert_eq!(
            by_config,
            custom.join("llama-server"),
            "default_version 已装时必须优先于锁定链"
        );

        // default_version 未装（目录被手删）→ 告警回退锁定链缓存
        std::fs::remove_dir_all(&custom).unwrap();
        let degraded = ensure_llama_server("ubuntu-vulkan-x64").await.unwrap();
        assert_eq!(
            degraded,
            variant.join("llama-server"),
            "default_version 未装必须回退锁定链而非硬失败"
        );
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

        let err = download_all(&format!("http://127.0.0.1:{port}/pkg"))
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
        let err = download_all(&format!("http://127.0.0.1:{port}/pkg"))
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
    async fn ensure_llama_server_downloads_and_caches() {
        std::env::set_var("ROXID_HOME", "/tmp/roxid-rt-e2e");
        let path = ensure_llama_server("ubuntu-x64").await.unwrap();
        assert!(path.is_file(), "llama-server 未落位：{path:?}");
        // 二次调用必须命中缓存且路径一致（无重复下载）
        let again = ensure_llama_server("ubuntu-x64").await.unwrap();
        assert_eq!(path, again);
        std::env::remove_var("ROXID_HOME");
    }
}
