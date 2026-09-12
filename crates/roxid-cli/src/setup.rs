//! 初次运行引导（迭代4 M23）：中国用户检测 → 代理配置询问 → config.toml 持久化。
//!
//! 裁决来源（2026-08-26 05:24–05:25）：
//! - Q1：时区或 locale 任一命中即视为中国用户（纯本地检测，无网络依赖）
//! - Q2：持久化于 ~/.roxid/config.toml，读取链 env 优先 → 文件回退
//! - Q3：仅配置 ROXID_GH_PROXY/ROXID_HF_PROXY 语义两项，ollama 主源不设
//! - Q4：联网命令（serve/pull/create）首次运行触发；roxid setup 随时重开
//! - Q5：推荐值仅作预填建议（可改/清空），用户确认的值才落盘，
//!   未确认前代码行为与现状完全一致，不违反 Q11「不写死默认网址」
//!
//! 修改历史：M24 新增 2026-08-26 05-30
//! M29 碴9（迭代9）：写盘基线以既有代理配置为底——非 CN 分支与拒绝
//! 配置路径不再空覆盖（原 ProxySection::default() 使重开引导静默清除
//! 已保存代理；清空仅经显式输入 none 路径）2026-09-05 13-02
//! M31（迭代11 F2，用户确认 2026-09-06 22:34）：--llama-url 参数模式
//! 直接安装手动后端（无需 TTY）；交互引导追加可选 llama.cpp 链接询问
//! （原因：后端手动链接更新需求）2026-09-06 23-08
//! M32 碴1（迭代12）：交互引导「立即安装」成功后回写 cfg.runtime.llama_url
//! （原实现仅 install_manual_and_record 内部落盘，随后被引导末尾的基线
//! cfg 全量覆盖写抹掉——来源记录在交互路径丢失）2026-09-07 01-05
//! M58（迭代20 裁决 2026-09-09 21:30）：向导末尾新增「安装 shell 补全」
//! 一步（默认 Y，调用 completion::install_completion 同一内核）2026-09-09 21-58
//! M88（迭代27，用户确认 2026-09-10 04:15）：引导「立即下载安装」分支
//! 安装前先落盘本次 cfg——install_manual 对 GitHub 域名 URL 自动拼代理
//! 时经 gh_proxy_prefix 读 env 优先 → config 回退，原实现引导末尾才
//! save，安装时刻读取链两路皆空（原因：代理已确认却对手动链不生效）
//! 2026-09-10 04-21

use std::io::{BufRead, IsTerminal, Write};

use roxid_server::config::{self, PersistConfig};

/// GitHub 代理推荐预填值（Q5：仅引导建议，不选不落盘；同 runtime/download.rs 文档示例）
const RECOMMENDED_GH_PROXY: &str = "https://gh.jasonzeng.dev/";
/// HF 镜像推荐预填值（Q5：仅引导建议；M16 镜像链路 2026-08-24 20:06 实测可达）
const RECOMMENDED_HF_MIRROR: &str = "https://hf-mirror.com/";

/// 联网命令首次运行检测（Q4 触发语义）：config.toml 存在即视为已引导，直接放行；
/// 无 TTY（CI/脚本/管道）静默跳过并提示环境变量方式，保证自动化不被阻塞。
pub async fn maybe_run_first_use_wizard() {
    if config::config_file_path().exists() {
        return;
    }
    if !std::io::stdout().is_terminal() {
        eprintln!(
            "提示：首次使用可运行 `roxid setup` 配置下载代理（或设置 \
             ROXID_GH_PROXY / ROXID_HF_PROXY 环境变量）"
        );
        return;
    }
    run_wizard().await;
}

/// roxid setup 子命令（M31 F2 更新）：
/// - 携 --llama-url <url>：非交互直行安装手动后端（无需 TTY，自动化可用），
///   成功后将来源链接记入 config.toml [runtime]
/// - 无参数：手动重开交互引导（Q4 裁决），已保存值优先作为预填
///
/// - 参数 llama_url：手动指定的 llama.cpp 包下载链接（None 走交互引导）
/// - 返回：进程退出码（0 成功；1 为安装失败或非交互环境拒绝执行）
pub async fn run_setup_command(llama_url: Option<String>) -> i32 {
    if let Some(url) = llama_url {
        return install_manual_and_record(&url).await;
    }
    if !std::io::stdout().is_terminal() {
        eprintln!(
            "setup 需要交互式终端；非交互环境请设置 ROXID_GH_PROXY / ROXID_HF_PROXY 环境变量，\
             或使用 `roxid setup --llama-url <url>` 安装自定义后端"
        );
        return 1;
    }
    run_wizard().await;
    0
}

/// 手动后端安装 + 来源记录（M31 F2）：安装成功才写 [runtime].llama_url
///（失败不记录，避免 config 指向不可用来源）；基线保留既有全部配置段。
///
/// - 参数 url：包下载链接（tar.gz 或裸 llama-server 二进制）
/// - 返回：进程退出码（0 成功；1 安装失败）
async fn install_manual_and_record(url: &str) -> i32 {
    // M105（迭代32 碴6a）：下载进度可见（量纲/速度/剩余时间——渲染器
    // 复用 main.rs 双路径实现：TTY spinner / 非 TTY 周期文本行）
    let (bar, on_progress) = crate::runtime_download_progress("下载运行时");
    let result = roxid_server::runtime::install_manual(url, on_progress).await;
    bar.finish_and_clear();
    match result {
        Ok(path) => {
            let mut cfg = config::load_persist_config();
            cfg.setup_done = true;
            cfg.runtime.llama_url = Some(url.to_string());
            match config::save_persist_config(&cfg) {
                Ok(()) => println!("llama.cpp 已安装：{}\n来源已记录：{url}", path.display()),
                Err(e) => eprintln!("安装成功但配置记录失败：{e}（llama-server 可用，来源未记录）"),
            }
            0
        }
        Err(e) => {
            eprintln!("llama.cpp 安装失败：{e}");
            1
        }
    }
}

/// 引导主流程：CN 检测 → 询问 → 写盘。任何输入异常（EOF 等）按保守默认处理，
/// 且始终落盘 setup_done=true，避免反复打扰（重开仅经 roxid setup）。
/// M31 F2：代理询问后追加可选 llama.cpp 自定义链接询问（默认跳过）。
async fn run_wizard() {
    println!("=== roxid 初次运行引导 ===");
    let existing = config::load_persist_config();
    let mut cfg = wizard_baseline(&existing);
    if config::is_cn_user() {
        println!("检测到中国网络环境（时区/locale 命中）。");
        if ask_yes_no("是否配置下载代理以加速模型与运行时获取？") {
            cfg.proxy.gh = ask_prefilled_value(
                "GitHub 代理前缀（拼接于 GitHub URL 之前）",
                existing.proxy.gh.as_deref().unwrap_or(RECOMMENDED_GH_PROXY),
            );
            cfg.proxy.hf = ask_prefilled_value(
                "HuggingFace 镜像基址（整体替换官方域名）",
                existing
                    .proxy
                    .hf
                    .as_deref()
                    .unwrap_or(RECOMMENDED_HF_MIRROR),
            );
        }
    } else {
        println!("未检测到中国网络环境，跳过代理配置；如需代理可随时运行 `roxid setup`。");
    }
    // M31 F2：可选自定义 llama.cpp 下载链接（默认跳过；确认输入后可选择
    // 立即安装或仅记录配置）
    if ask_yes_no("是否配置自定义 llama.cpp 下载链接（手动更新后端）？") {
        let prefill = existing.runtime.llama_url.as_deref().unwrap_or("");
        if let Some(url) =
            ask_prefilled_value("llama.cpp 包链接（tar.gz 或裸 llama-server）", prefill)
        {
            if ask_yes_no("是否立即下载安装？") {
                // M88（迭代27）：安装前先落盘本次 cfg——install_manual 对
                // GitHub 域名 URL 自动拼代理（gh_proxy_prefix：env 优先 →
                // config 回退），原实现引导末尾才 save，此刻读取链两路皆
                // 空、本次确认的代理读不到。save 失败仅警告不中止（下载
                // 退回直连，安装本身不依赖 config；引导末尾仍有全量 save
                // 兜底，成功路径的 llama_url 回写链 M32 碴1 语义不变）。
                if let Err(e) = config::save_persist_config(&cfg) {
                    eprintln!("配置提前落盘失败：{e}（本次下载可能不经代理直连）");
                }
                // 返回码仅作屏显（失败已在内部打印）；引导不因安装失败中止。
                // M32 碴1：安装成功必须回写 cfg——install_manual_and_record 的
                // 内部落盘会被引导末尾的基线 cfg 全量覆盖写抹掉（原实现仅
                // 「仅记录」分支写 cfg，立即安装路径来源记录丢失）
                if install_manual_and_record(&url).await == 0 {
                    cfg.runtime.llama_url = Some(url);
                }
            } else {
                cfg.runtime.llama_url = Some(url); // 仅记录，下次 serve 仍走自动链
            }
        }
    }
    // 迭代20 M58：安装 shell 补全（默认 Y；装完重开终端即可 TAB 补全；
    // 失败不中断引导——completion::install_step_for_wizard 内部已兜底提示）
    if ask_yes_no("是否安装 shell 命令补全（bash/zsh/fish，装完重开终端即可 TAB 补全）？")
    {
        crate::completion::install_step_for_wizard();
    }
    match config::save_persist_config(&cfg) {
        Ok(()) => println!("配置已保存：{}", config::config_file_path().display()),
        Err(e) => eprintln!("配置保存失败：{e}（本次代理未生效，可用环境变量替代）"),
    }
}

/// 引导写盘基线（M29 碴9）：以既有代理配置为基线——CN 检测未命中或用户
/// 拒绝配置时保留存量（原空默认覆盖会静默清除已保存代理）；清空仅经
/// ask_prefilled_value 显式输入 none 的路径发生（Q5 预填保留语义补全）。
///
/// - 参数 existing：既有持久化配置（无文件/损坏时为全默认）
/// - 返回：本次引导的写盘基线（setup_done 恒置 true 防反复打扰）
fn wizard_baseline(existing: &PersistConfig) -> PersistConfig {
    PersistConfig {
        setup_done: true,
        proxy: existing.proxy.clone(),
        runtime: existing.runtime.clone(),
    }
}

/// 是/否询问：回车默认「是」，仅显式否定词判否（简化交互，引导内部专用）。
///
/// - 参数 question：提示文案（不带选项后缀）
/// - 返回：true 表示确认
fn ask_yes_no(question: &str) -> bool {
    print!("{question} [Y/n] ");
    flush_stdout();
    let answer = read_line().trim().to_ascii_lowercase();
    !(answer == "n" || answer == "no" || answer == "否")
}

/// 预填询问（Q5 语义）：回车采用预填值；输入 none 清除；其他输入原样替换。
/// 空串结果统一归一为 None（与读取链「空值视为未设置」语义对齐）。
/// M116（迭代33 碴9）：两行式排版——原 label + 选项后缀单行 print 超
/// 60 列，窄终端折行使确认问句与下一问句挤行混排；label 独占一行、
/// 输入提示精简另起一行，窄终端不再折行。
///
/// - 参数 label：配置项说明
/// - 参数 prefill：预填建议值（推荐值或 setup 重开时的既有值）
/// - 返回：Option<String>，用户最终确认的值
fn ask_prefilled_value(label: &str, prefill: &str) -> Option<String> {
    println!("{label}");
    print!("[回车={prefill} | 新值替换 | none 清除]: ");
    flush_stdout();
    let answer = read_line().trim().to_string();
    let value = if answer.is_empty() {
        prefill.to_string()
    } else if answer.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        answer
    };
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// 读取单行 stdin；EOF/读失败返回空串（按保守默认走完引导，不中断主命令）。
fn read_line() -> String {
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line
}

/// 刷新 stdout，保证 print! 提示在等待输入前可见（无缓冲提示会卡到输入完成后）。
fn flush_stdout() {
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxid_server::config::ProxySection;

    /// M29 碴9：引导基线必须保留既有代理配置（原空默认覆盖清除存量）；
    /// M31：runtime 段同款保留语义
    #[test]
    fn wizard_baseline_preserves_existing_proxy() {
        let existing = PersistConfig {
            setup_done: true,
            proxy: ProxySection {
                gh: Some("https://keep-gh.mirror/".into()),
                hf: Some("https://keep-hf.mirror/".into()),
            },
            runtime: roxid_server::config::RuntimeSection {
                llama_url: Some("https://keep-llama.example/pkg.tar.gz".into()),
                // M36：新增字段补 None（保持「已有段保留」断言语义不变）；
                // M181（迭代48）：tag_complete_limit 同款补 None；
                // M190（迭代50）：default_variant 同款补 None
                default_version: None,
                default_variant: None,
                tag_complete_limit: None,
            },
        };
        let base = wizard_baseline(&existing);
        assert_eq!(base.proxy.gh.as_deref(), Some("https://keep-gh.mirror/"));
        assert_eq!(base.proxy.hf.as_deref(), Some("https://keep-hf.mirror/"));
        assert!(base.setup_done, "setup_done 恒置 true（防反复打扰语义）");
        assert_eq!(
            base.runtime.llama_url.as_deref(),
            Some("https://keep-llama.example/pkg.tar.gz"),
            "M31：runtime.llama_url 基线保留"
        );

        // 无存量（首次/损坏回退）时基线为空——与旧行为一致
        let fresh = wizard_baseline(&PersistConfig::default());
        assert!(fresh.proxy.gh.is_none() && fresh.proxy.hf.is_none());
    }
}
