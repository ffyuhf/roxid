//! 全局配置：roxid 主目录、目录约定与对外常量。
//!
//! 目录约定（来源：用户确认 2026-08-24 18:33 Q9；迭代11 F1 三分离
//! 2026-09-06 22:33）：模型仓库按来源三目录布局
//! ~/.roxid/models/{ollama|HF|derived}/...，衍生模型大文件硬链接基础
//! 模型，目录内即完整可运行文件。
//!
//! 配置持久化（来源：迭代4 裁决 Q1–Q5 2026-08-26 05:24–05:25）：
//! ~/.roxid/config.toml 承载初次引导完成标记与代理配置；
//! 下载读取链为「环境变量优先 → config.toml 回退」，未配置时行为与现状一致（直连）。
//!
//! 修改历史：M1 新增 2026-08-24 18:35；M24 配置持久化 2026-08-26 05:30；
//! M29 碴9 补 Clone 2026-09-05 13:02；M31 RuntimeSection 2026-09-06 23:00；
//! M32 碴4（迭代12）：头注释目录约定对齐三目录分离布局（原两级布局
//! 描述漂移）2026-09-07 01-15
//! M36（迭代16）：RuntimeSection 增加 default_version——roxid runtime use
//! 的持久化落地字段（env 临时覆盖 + config 持久默认两层并存，用户裁决
//! 2026-09-09 04:33）2026-09-09 04-35
//! M181（迭代48，Q4-A 裁决 2026-09-12 04:25）：RuntimeSection 增加
//! tag_complete_limit——runtime install tag 补全候选数量通道（默认 10，
//! 用户手改 config.toml 生效，无 CLI 写入命令）；顺带修正本段两条
//! 「b10605 锁定链」失真注释（迭代46 起兜底链为在线最新版，注释漏更）
//! 2026-09-12 04-32
//! M190（迭代50，用户裁决链 15:47/15:55/15:58）：RuntimeSection 增加
//! default_variant——`roxid runtime use <tag> <词>` 的变体持久化落地
//!（存真实变体目录名，use 时从该 tag 已装变体目录匹配取得，磁盘事实
//! 零硬编码）2026-09-12 16-20

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// roxid 版本号，编译期取自 Cargo.toml，保证与二进制一致
pub const ROXID_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 默认 API 监听地址（与原版 Ollama 的 11434 端口保持一致，
/// 便于现有 ollama 客户端与生态脚本无缝切换）
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:11434";

/// 获取 roxid 主目录，优先读 ROXID_HOME 环境变量，缺省为 ~/.roxid。
///
/// - 返回：主目录 PathBuf，调用方按需 join 子路径
pub fn roxid_home() -> PathBuf {
    if let Ok(custom) = std::env::var("ROXID_HOME") {
        return PathBuf::from(custom);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".roxid")
}

/// 模型仓库根目录：{roxid_home}/models。
/// 迭代11 F1（用户确认 2026-09-06 22:33）起根下按来源三分：
/// ollama/（主源拉取）、HF/（HF 直拉三级路径）、derived/（create/copy 衍生）。
/// 本函数保留给迁移器扫描旧布局与测试根目录构造使用。
pub fn models_root() -> PathBuf {
    roxid_home().join("models")
}

/// ollama 主源模型根目录：{roxid_home}/models/ollama（迭代11 F1）
pub fn models_ollama_root() -> PathBuf {
    models_root().join("ollama")
}

/// HF 直拉模型根目录：{roxid_home}/models/HF（迭代11 F1，
/// 内部三级布局 {user}/{repo}/{quant}/）
pub fn models_hf_root() -> PathBuf {
    models_root().join("HF")
}

/// 衍生模型根目录：{roxid_home}/models/derived（迭代11 F1，
/// create/copy 产物，大文件硬链接基础模型）
pub fn models_derived_root() -> PathBuf {
    models_root().join("derived")
}

/// llama.cpp 运行时缓存目录：{roxid_home}/llama.cpp（存放预编译包与版本锁定文件）
pub fn llama_runtime_root() -> PathBuf {
    roxid_home().join("llama.cpp")
}

// ==================== 持久化配置（迭代4 M24） ====================

/// 持久化配置根结构，序列化为 {roxid_home}/config.toml。
/// M29 碴9：补 Clone（setup 引导基线复用既有代理段）。
/// M31（迭代11 F2）：新增 runtime 段（setup --llama-url 持久化来源链接）。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PersistConfig {
    /// 引导是否已完成；true 后联网命令不再自动触发引导（触发判定：config.toml 是否存在）
    #[serde(default)]
    pub setup_done: bool,
    /// 代理配置段（环境变量缺省时的回退来源）
    #[serde(default)]
    pub proxy: ProxySection,
    /// 运行时段（迭代11 F2：手动指定 llama.cpp 下载链接的记录处）
    #[serde(default)]
    pub runtime: RuntimeSection,
}

/// 运行时段（迭代11 F2，用户确认 2026-09-06 22:34；迭代16 M36 扩展）：
/// `roxid setup --llama-url <url>` 安装成功后记录来源链接（展示/重装依据）；
/// `roxid runtime use <tag|manual>` 记录默认后端版本（M36 多版本管理）。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RuntimeSection {
    /// 手动指定的 llama.cpp 包下载链接（tar.gz 或裸 llama-server 二进制）；
    /// None 表示未配置（走在线最新版兜底自动链——M181 注释修正，原
    /// 「b10605 版本锁定」措辞为迭代46 前旧态）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llama_url: Option<String>,
    /// 默认后端版本 tag（M36，迭代16 用户裁决 Q1-B）：
    /// `roxid runtime use <tag>` 的持久化落地；ensure 链在 manual 未命中后
    /// 按此版本解析变体缓存；None 表示未设置（兜底在线查最新版，存量零感知）。
    /// 特殊值 "manual" 表示默认走手动版本目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_version: Option<String>,
    /// 默认变体目录名（迭代50 M190，用户裁决链 2026-09-12 15:47/15:55/15:58）：
    /// `roxid runtime use <tag> <词>` 写入的真实变体目录名（如
    /// ubuntu-cuda-12.4-x64——use 时从该 tag 已装变体目录匹配取得，
    /// 磁盘事实零硬编码，代码不含任何变体字符串）；effective_variant()
    /// 在其与 default_version 同时在位时优先使用；None 表示未设置
    ///（自动探测 vulkan/cpu，存量用户零感知）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_variant: Option<String>,
    /// runtime install tag 补全候选数量（迭代48 M181，Q4-A 裁决
    /// 2026-09-12 04:25）：`__complete` 在 install 值位联网查 GitHub
    /// Releases 时返回的最新预发布 tag 数；None 表示未配置（回退
    /// DEFAULT_TAG_COMPLETE_LIMIT=10）。用户手动编辑 config.toml 生效，
    /// 无 CLI 写入命令（Q4-A「同款持久化链」＝ serde 往返保全）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_complete_limit: Option<usize>,
}

/// install tag 补全候选数量的缺省值（迭代48 M181，Q4-A 裁决
/// 2026-09-12 04:25「默认 10」）。
pub const DEFAULT_TAG_COMPLETE_LIMIT: usize = 10;

/// 读取 install tag 补全候选数量：config.toml [runtime].tag_complete_limit
/// 缺省/未设置时回退默认 10；结果 clamp 至 1..=100（上限依据 GitHub
/// Releases API per_page=100 单页上限，超出无意义）。
///
/// - 返回：补全候选数量（恒在 1..=100 区间）
pub fn tag_complete_limit() -> usize {
    load_persist_config()
        .runtime
        .tag_complete_limit
        .unwrap_or(DEFAULT_TAG_COMPLETE_LIMIT)
        .clamp(1, 100)
}

/// 代理配置段：键名与既有环境变量语义一一对应（Q3 裁决：仅 GH/HF 两项，ollama 主源不设）。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProxySection {
    /// GitHub 代理前缀（语义同 ROXID_GH_PROXY：拼接在 GitHub 原始 URL 之前）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gh: Option<String>,
    /// HuggingFace 镜像基址（语义同 ROXID_HF_PROXY：整体替换官方域名）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hf: Option<String>,
}

/// 配置文件路径：{roxid_home}/config.toml。
///
/// - 返回：完整路径；文件存在与否即「已引导/未引导」判定依据（Q4 触发语义）
pub fn config_file_path() -> PathBuf {
    roxid_home().join("config.toml")
}

/// 读取持久化配置；文件不存在或内容损坏一律返回默认值（未引导、无代理），
/// 保证损坏文件不阻断启动，下载链自动回退直连（与现状一致）。
///
/// - 返回：反序列化后的 PersistConfig，失败时为全默认
pub fn load_persist_config() -> PersistConfig {
    let Ok(text) = std::fs::read_to_string(config_file_path()) else {
        return PersistConfig::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

/// 持久化写入配置（自动创建主目录，逐字段覆盖式全量写）。
///
/// - 参数 cfg：待写入的完整配置
/// - 返回：io::Result<()>；调用方决定失败提示方式（引导内为非致命告警）
pub fn save_persist_config(cfg: &PersistConfig) -> std::io::Result<()> {
    let path = config_file_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, text)
}

// ==================== 中国用户检测（迭代4 M24，Q1 裁决） ====================

/// 时区字符串是否命中中国大陆时区标识。
/// 命中集合：Asia/Shanghai、Asia/Chongqing、Asia/Urumqi、Asia/Harbin、PRC
///（兼容 TZ 带前导冒号写法与 /etc/localtime 链接目标的 zoneinfo 路径片段）。
///
/// - 参数 tz：任意形态的时区描述字符串
/// - 返回：true 表示属于中国大陆时区
pub fn timezone_indicates_cn(tz: &str) -> bool {
    const CN_TZ_MARKERS: [&str; 5] = [
        "Asia/Shanghai",
        "Asia/Chongqing",
        "Asia/Urumqi",
        "Asia/Harbin",
        "PRC",
    ];
    let tz = tz.trim().trim_start_matches(':');
    CN_TZ_MARKERS.iter().any(|marker| tz.contains(marker))
}

/// locale 字符串是否为简体中文中国区（zh_CN 前缀，如 zh_CN.UTF-8；zh_TW/zh_HK 不命中）。
///
/// - 参数 locale：locale 描述字符串
/// - 返回：true 表示属于中国大陆 locale
pub fn locale_indicates_cn(locale: &str) -> bool {
    locale.trim().to_ascii_lowercase().starts_with("zh_cn")
}

/// 中国用户综合判定（Q1 裁决 2026-08-26 05:24）：时区或 locale 任一命中即真，
/// 纯本地检测、无网络依赖。信号优先级：locale（LC_ALL/LC_MESSAGES/LANG）→ TZ 变量
/// → /etc/localtime 符号链接目标中的 zoneinfo 名称。
///
/// - 返回：true 视为中国用户（引导时据此询问代理配置）
pub fn is_cn_user() -> bool {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .unwrap_or_default();
    if locale_indicates_cn(&locale) {
        return true;
    }
    if timezone_indicates_cn(&std::env::var("TZ").unwrap_or_default()) {
        return true;
    }
    // /etc/localtime 通常为指向 /usr/share/zoneinfo/{时区名} 的符号链接
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        if let Some(tz_name) = target.to_string_lossy().split("zoneinfo/").nth(1) {
            return timezone_indicates_cn(tz_name);
        }
    }
    false
}

/// 测试辅助：凡 set/remove ROXID_HOME 的单测必须先持此锁（src 内跨模块共享，
/// 防止 cargo test 默认并行下环境变量竞争）；毒锁直接复用不致命。
#[cfg(test)]
pub static ROXID_HOME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// ROXID_HOME 环境变量必须优先于默认 ~/.roxid
    #[test]
    fn roxid_home_env_overrides_default() {
        let _guard = ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::set_var("ROXID_HOME", "/tmp/roxid-test-home");
        assert_eq!(roxid_home(), PathBuf::from("/tmp/roxid-test-home"));
        std::env::remove_var("ROXID_HOME");
    }

    /// config.toml 往返：save 后 load 字段一致（setup_done 与两代理键）
    #[test]
    fn persist_config_roundtrip() {
        let _guard = ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-cfg-rt-{}", std::process::id()));
        std::env::set_var("ROXID_HOME", &dir);
        let cfg = PersistConfig {
            setup_done: true,
            proxy: ProxySection {
                gh: Some("https://gh.example.dev/".into()),
                hf: Some("https://hf.example.com".into()),
            },
            // M31：runtime 段一并往返（迭代11 F2）；M36：default_version
            // 一并往返；M181：tag_complete_limit 一并往返；
            // M190（迭代50）：default_variant 一并往返
            runtime: RuntimeSection {
                llama_url: Some("https://example.dev/llama-custom.tar.gz".into()),
                default_version: Some("b10700".into()),
                default_variant: Some("ubuntu-cuda-12.4-x64".into()),
                tag_complete_limit: Some(12),
            },
        };
        save_persist_config(&cfg).expect("写盘必须成功");
        assert_eq!(load_persist_config().setup_done, true);
        assert_eq!(
            load_persist_config().proxy.gh.as_deref(),
            Some("https://gh.example.dev/")
        );
        assert_eq!(
            load_persist_config().runtime.llama_url.as_deref(),
            Some("https://example.dev/llama-custom.tar.gz"),
            "M31：runtime.llama_url 必须完整往返"
        );
        assert_eq!(
            load_persist_config().runtime.default_version.as_deref(),
            Some("b10700"),
            "M36：runtime.default_version 必须完整往返"
        );
        assert_eq!(
            load_persist_config().runtime.tag_complete_limit,
            Some(12),
            "M181：runtime.tag_complete_limit 必须完整往返"
        );
        assert_eq!(
            load_persist_config().runtime.default_variant.as_deref(),
            Some("ubuntu-cuda-12.4-x64"),
            "M190：runtime.default_variant 必须完整往返"
        );
        assert_eq!(
            load_persist_config().proxy.hf.as_deref(),
            Some("https://hf.example.com")
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// 文件不存在 → 全默认（未引导、无代理）；损坏内容同样回退默认而非报错
    #[test]
    fn persist_config_missing_or_broken_falls_back_to_default() {
        let _guard = ROXID_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("roxid-cfg-dflt-{}", std::process::id()));
        std::env::set_var("ROXID_HOME", &dir);
        assert_eq!(load_persist_config().setup_done, false);
        assert!(load_persist_config().proxy.gh.is_none());

        save_persist_config(&PersistConfig::default()).expect("写盘必须成功");
        std::fs::write(config_file_path(), "not a valid toml {{{").expect("注入损坏内容");
        assert_eq!(
            load_persist_config().setup_done,
            false,
            "损坏文件必须回退默认"
        );
        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("ROXID_HOME");
    }

    /// 中国时区标识命中集合（含冒号前缀与链接路径形态）与排除项
    #[test]
    fn cn_timezone_detection() {
        assert!(timezone_indicates_cn("Asia/Shanghai"));
        assert!(timezone_indicates_cn(":Asia/Chongqing"));
        assert!(timezone_indicates_cn("/usr/share/zoneinfo/Asia/Urumqi"));
        assert!(timezone_indicates_cn("PRC"));
        assert!(timezone_indicates_cn("Asia/Harbin"));
        assert!(!timezone_indicates_cn("America/New_York"));
        assert!(!timezone_indicates_cn("Asia/Taipei"));
        assert!(!timezone_indicates_cn(""));
    }

    /// zh_CN 前缀命中（大小写不敏感）；zh_TW/zh_HK/en_US 排除
    #[test]
    fn cn_locale_detection() {
        assert!(locale_indicates_cn("zh_CN.UTF-8"));
        assert!(locale_indicates_cn("ZH_CN"));
        assert!(!locale_indicates_cn("zh_TW"));
        assert!(!locale_indicates_cn("zh_HK.UTF-8"));
        assert!(!locale_indicates_cn("en_US.UTF-8"));
        assert!(!locale_indicates_cn(""));
    }
}
