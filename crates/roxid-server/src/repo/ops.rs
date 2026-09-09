//! 模型仓库操作：三目录聚合扫描、硬链接落位、复制、删除。
//!
//! 硬链接语义（来源：用户确认 2026-08-24 18:33 Q9）：
//! 衍生/复制模型拥有独立目录，大文件硬链接源模型——目录直观完整且零额外磁盘；
//! 同一 models 树内必为同一文件系统，硬链接必然成功；
//! 删除按 inode 引用计数天然安全：删任一目录不影响其他链接的文件可用性。
//!
//! 修改历史：M4 新增 2026-08-24 19:11；M20 补 find_blob 摘要定位 2026-08-24 22-41
//! M30 碴6（迭代10，R3-A）：copy_model 失败路径清理 dst 半成品——原
//! 先 save_meta 后硬链的失败残留带 model.json 无 gguf 空壳目录（tags
//! 列出 size=0 模型且重试报"已存在"）2026-09-06 22-20
//! M31（迭代11 F1）：list_models 三目录聚合（ollama/HF/derived）、
//! find_model 走 locate 顺序查找、copy dst 落 derived、delete 清理
//! HF 三级空父链（原因：存储三分离需求变更）2026-09-06 22-48
//! M32 碴6/碴7（迭代12）：cleanup_empty_parents/dir_in_source_root 升
//! pub(crate) 供 modelfile 覆盖路径复用；新增 list_models_with_dirs
//! 姊妹函数（聚合扫描携带真实持有目录），find_blob/role_to_file 改以
//! 扫描目录定位（原 ModelRef 重新 locate 丢弃来源，同名跨源共存时
//! 摘要定位错位）2026-09-07 01-10
//! M33 碴7（迭代13）：find_model 未命中分支对 hf 名缺 quant 形态增强
//! 报错（附已装量化列表或 pull 指引）——原缺省补 tag=latest 使报错
//! 「hf.co/u/r:latest not found」误导（用户以为缺 latest 实为缺 quant）
//! 2026-09-07 01-58

use std::path::Path;

use crate::error::{RoxidError, RoxidResult};

use super::model_json::{load_meta, save_meta, ModelMeta};
use super::ModelRef;

/// 硬链接源文件到目标；异常时（跨设备等）回退为物理复制并记录日志。
///
/// - 参数 src / dst：源与目标绝对路径
/// - 返回：Ok(()) 表示目标已就位
pub fn hardlink_or_copy(src: &Path, dst: &Path) -> RoxidResult<()> {
    if let Err(e) = std::fs::hard_link(src, dst) {
        tracing::warn!(
            "硬链接失败（{e}），回退为复制：{} -> {}",
            src.display(),
            dst.display()
        );
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// 复制模型：新建目标目录（落 derived 源，迭代11 F1），元数据改名，
/// 全部大文件硬链接（src 可为三目录任一模型，跨源硬链接同文件系统不变）。
/// M30 碴6（R3-A）：大文件落位任一步失败时清理 dst 半成品（原残留空壳
/// 目录：有 model.json 无 gguf，tags 列出 size=0 模型且重试 copy 报
/// "模型已存在"；失败即回到未创建态，语义与覆盖重建预期一致）。
///
/// - 参数 root：models 根目录
/// - 参数 src：已存在的源模型引用
/// - 参数 dst：目标模型引用（不得已存在）
/// - 返回：Ok(()) 表示复制完成
pub fn copy_model(root: &Path, src: &ModelRef, dst: &ModelRef) -> RoxidResult<()> {
    let src_dir = src.dir(root);
    if !src_dir.is_dir() {
        return Err(RoxidError::ModelNotFound(src.full_name()));
    }
    let dst_dir = dst.dir_in(root, super::ModelSource::Derived);
    if dst_dir.exists() {
        return Err(RoxidError::InvalidRequest(format!(
            "模型已存在：{}",
            dst.full_name()
        )));
    }
    let mut meta = load_meta(&src_dir)?;
    meta.name = dst.full_name();
    std::fs::create_dir_all(&dst_dir)?;
    save_meta(&dst_dir, &meta)?;

    // 主模型与可选多模态投影均硬链接；LoRA 适配器同样硬链接。
    // 任一步失败清理半成品后透出错误（M30 碴6）。
    let linking = || -> RoxidResult<()> {
        hardlink_or_copy(
            &src_dir.join(&meta.files.model),
            &dst_dir.join(&meta.files.model),
        )?;
        if let Some(mmproj) = &meta.files.mmproj {
            hardlink_or_copy(&src_dir.join(mmproj), &dst_dir.join(mmproj))?;
        }
        for adapter in &meta.adapters {
            hardlink_or_copy(&src_dir.join(adapter), &dst_dir.join(adapter))?;
        }
        Ok(())
    };
    if let Err(e) = linking() {
        let _ = std::fs::remove_dir_all(&dst_dir);
        return Err(e);
    }
    Ok(())
}

/// 删除模型：locate 定位实际目录后移除整个模型目录；
/// 父目录因此变空时逐级清理（HF 三级路径 user/repo/tag 删后清空 repo、
/// user；普通名清理 {model}），到达源根目录即停，保持仓库树整洁。
/// 硬链接的其他副本不受影响（inode 引用计数语义）。
///
/// - 参数 root：models 根目录
/// - 参数 target：待删除模型引用
/// - 返回：Ok(()) 表示删除完成
pub fn delete_model(root: &Path, target: &ModelRef) -> RoxidResult<()> {
    let Some((source, dir)) = super::locate_model_dir(root, target) else {
        return Err(RoxidError::ModelNotFound(target.full_name()));
    };
    std::fs::remove_dir_all(&dir)?;
    let source_root = dir_in_source_root(root, source);
    cleanup_empty_parents(&dir, &source_root);
    Ok(())
}

/// 源枚举 → 该源根目录路径（{root}/ollama|HF|derived）。
/// M32 碴6：升 pub(crate) 供 modelfile 覆盖路径计算空父链清理上限。
///
/// - 参数 root：models 根目录
/// - 参数 source：模型来源
/// - 返回：源根目录路径
pub(crate) fn dir_in_source_root(root: &Path, source: super::ModelSource) -> std::path::PathBuf {
    use super::ModelSource as S;
    match source {
        S::Ollama => root.join("ollama"),
        S::Hf => root.join("HF"),
        S::Derived => root.join("derived"),
    }
}

/// 自被删目录向上清理空父目录链，抵达 stop（源根目录）即停。
/// M32 碴6：升 pub(crate) 供 modelfile create 覆盖/交换路径复用
///（原仅 delete_model 使用，同模块族语义对齐）。
///
/// - 参数 dir：已删除的模型目录路径
/// - 参数 stop：清理上限目录（源根目录，自身不删）
pub(crate) fn cleanup_empty_parents(dir: &Path, stop: &Path) {
    let mut cur = dir.parent();
    while let Some(p) = cur {
        if p == stop {
            break;
        }
        let empty = p
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !empty || std::fs::remove_dir(p).is_err() {
            break;
        }
        cur = p.parent();
    }
}

/// 罗列仓库全部模型（迭代11 F1：ollama/HF/derived 三目录聚合扫描，
/// HF 为 user/repo/tag 三级布局，读取每个 model.json）。
/// 单个模型元数据损坏时跳过并记录，不影响整体列举。
/// M29 碴12（R4-A）：无元数据目录（pull 失败残留的 .parts 断点目录等）
/// 降 debug——原 warn 使每次 tags/ps 刷告警日志；残留目录在下次 pull
/// 成功时自愈，断点续传语义保留。
/// M32 碴7：内部委托 list_models_with_dirs（单一扫描逻辑，map 掉目录）。
///
/// - 参数 root：models 根目录
/// - 返回：全部可解析的模型元数据
pub fn list_models(root: &Path) -> Vec<ModelMeta> {
    list_models_with_dirs(root)
        .into_iter()
        .map(|(_, m)| m)
        .collect()
}

/// 聚合扫描并携带每个模型的真实持有目录（M32 碴7，list_models 姊妹函数）。
/// find_blob 与 tags 的 model_size 等按目录定位文件的消费方必须使用
/// (dir, meta) 对：原实现经 `ModelRef::parse(meta.name).dir()` 重新
/// locate（R5 ollama 优先），同名跨源共存（如 cp 同名自复制使 ollama
/// 与 derived 各持一份同名模型）时丢弃真实来源，size 显示与摘要定位
/// 错位到另一源的文件。
///
/// - 参数 root：models 根目录
/// - 返回：(模型目录, 元数据) 对列表（目录为扫描时的真实持有者）
pub fn list_models_with_dirs(root: &Path) -> Vec<(std::path::PathBuf, ModelMeta)> {
    let mut out = Vec::new();
    for source in [
        super::ModelSource::Ollama,
        super::ModelSource::Hf,
        super::ModelSource::Derived,
    ] {
        let source_root = dir_in_source_root(root, source);
        if source == super::ModelSource::Hf {
            collect_hf_three_level(&source_root, &mut out);
        } else {
            collect_two_level(&source_root, &mut out);
        }
    }
    out
}

/// 两级布局目录收集：{source_root}/{model}/{tag}/ 逐个读 model.json。
///
/// - 参数 source_root：单源根目录
/// - 参数 out：聚合输出
fn collect_two_level(source_root: &Path, out: &mut Vec<(std::path::PathBuf, ModelMeta)>) {
    let Ok(models) = std::fs::read_dir(source_root) else {
        return; // 源根目录尚不存在视为空
    };
    for model_entry in models.flatten() {
        collect_tag_dirs(&model_entry.path(), out);
    }
}

/// HF 三级布局收集：{HF}/{user}/{repo}/{quant}/ 逐个读 model.json。
///
/// - 参数 hf_root：HF 源根目录
/// - 参数 out：聚合输出
fn collect_hf_three_level(hf_root: &Path, out: &mut Vec<(std::path::PathBuf, ModelMeta)>) {
    let Ok(users) = std::fs::read_dir(hf_root) else {
        return;
    };
    for user_entry in users.flatten() {
        let Ok(repos) = std::fs::read_dir(user_entry.path()) else {
            continue;
        };
        for repo_entry in repos.flatten() {
            collect_tag_dirs(&repo_entry.path(), out);
        }
    }
}

/// 单个 {model 或 repo} 目录下的全部 tag/quant 子目录收集。
///
/// - 参数 model_dir：一级模型（或 HF repo）目录
/// - 参数 out：聚合输出
fn collect_tag_dirs(model_dir: &Path, out: &mut Vec<(std::path::PathBuf, ModelMeta)>) {
    if !model_dir.is_dir() {
        return;
    }
    let Ok(tags) = std::fs::read_dir(model_dir) else {
        return;
    };
    for tag_entry in tags.flatten() {
        let tag_dir = tag_entry.path();
        if !tag_dir.is_dir() {
            continue;
        }
        match load_meta(&tag_dir) {
            // M32 碴7：携带真实持有目录（消费方免于 ModelRef 重新 locate）
            Ok(meta) => out.push((tag_dir, meta)),
            // M29 碴12：降 debug（pull 失败残留目录为常态存在——.parts
            // 断点续传受益；真正损坏的 model.json 也有 debug 痕迹）
            Err(e) => tracing::debug!("跳过无元数据/损坏模型目录 {}：{e}", tag_dir.display()),
        }
    }
}

/// 按完整名（model:tag）精确查找模型（迭代11 F1：locate 顺序路由，
/// 普通名 ollama 优先于 derived，HF 名直达 HF 三级路径）。
///
/// - 参数 root：models 根目录
/// - 参数 name：完整模型名
/// - 返回：命中的元数据；未命中返回 ModelNotFound
pub fn find_model(root: &Path, name: &str) -> RoxidResult<ModelMeta> {
    let r = ModelRef::parse(name)?;
    match super::locate_model_dir(root, &r) {
        Some((_, dir)) => load_meta(&dir),
        // M33 碴7：hf 名缺 quant 的报错增强（附已装量化或 pull 指引）
        None => Err(RoxidError::ModelNotFound(hf_missing_hint(root, &r))),
    }
}

/// hf 名未命中的报错文本增强（M33 碴7）：扫描 HF/{user}/{repo}/ 一级
/// 子目录（quant 目录名即候选），非空则附「该仓库已装量化」排序列表；
/// 空/不存在则附「pull 时请带量化 tag」指引。普通名与显式带 tag 的
/// hf 名语义自明，原样返回 full_name 不增强。
///
/// - 参数 root：models 根目录
/// - 参数 r：未命中的模型引用
/// - 返回：报错文本（ModelNotFound 的载荷）
fn hf_missing_hint(root: &Path, r: &ModelRef) -> String {
    let full = r.full_name();
    if !super::is_hf_name(&r.model) || r.tag != super::DEFAULT_TAG {
        return full;
    }
    let repo_dir = root.join("HF").join(
        r.model
            .strip_prefix(super::HF_NAME_PREFIX)
            .unwrap_or(&r.model),
    );
    let mut quants: Vec<String> = std::fs::read_dir(&repo_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    if quants.is_empty() {
        format!(
            "{full}（未安装；pull 时请带量化 tag，如 {}:Q4_K_M）",
            r.model
        )
    } else {
        quants.sort();
        format!(
            "{full}（该仓库已装量化：{}，请显式指定）",
            quants.join("、")
        )
    }
}

/// /api/blobs 命中结果（M20，Q1 裁决：从本地已拉取模型目录定位）
#[derive(Debug, Clone)]
pub struct BlobLocation {
    /// 命中文件绝对路径
    pub path: std::path::PathBuf,
    /// 文件字节大小
    pub size: u64,
}

/// 按摘要定位本地 blob 文件：扫描仓库全部模型的层摘要索引。
/// role → 文件映射：config → model.json（动态代表）；model/projection/adapter-N →
/// 对应 GGUF；template/params/system/messages 层未独立落盘（直观布局裁决），不可定位。
/// M32 碴7：经 list_models_with_dirs 以扫描目录定位文件（原 ModelRef
/// 重新 locate 丢弃真实来源，同名跨源共存时摘要错位到另一源文件）。
///
/// - 参数 root：models 根目录
/// - 参数 digest：请求摘要（sha256:hex 或裸 hex）
/// - 返回：命中位置；未命中 None（由调用方返回 404）
pub fn find_blob(root: &Path, digest: &str) -> Option<BlobLocation> {
    let norm = digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .to_ascii_lowercase();
    if norm.is_empty() {
        return None;
    }
    for (dir, meta) in list_models_with_dirs(root) {
        let hit = meta.layer_digests.iter().find(|(_, d)| {
            d.strip_prefix("sha256:")
                .unwrap_or(d)
                .eq_ignore_ascii_case(&norm)
        });
        if let Some((role, _)) = hit {
            if let Some(p) = role_to_file(&dir, &meta, role) {
                if let Ok(m) = std::fs::metadata(&p) {
                    return Some(BlobLocation {
                        path: p,
                        size: m.len(),
                    });
                }
            }
        }
    }
    None
}

/// role → 该模型目录内对应文件。
/// M32 碴7：dir 由调用方传入（聚合扫描携带的真实持有目录），不再经
/// ModelRef::parse(meta.name).dir() 重新 locate。
///
/// - 参数 dir：模型真实持有目录（扫描所得）
/// - 参数 meta：模型元数据
/// - 参数 role：层角色（config/model/projection/adapter-N）
/// - 返回：该层对应的文件路径
fn role_to_file(dir: &Path, meta: &ModelMeta, role: &str) -> Option<std::path::PathBuf> {
    match role {
        "config" => Some(dir.join(super::model_json::META_FILE_NAME)),
        "model" => Some(dir.join(&meta.files.model)),
        "projection" => meta.files.mmproj.as_ref().map(|m| dir.join(m)),
        t if t.starts_with("adapter-") => {
            let idx = t.strip_prefix("adapter-")?;
            Some(dir.join(format!("lora-{idx}.gguf")))
        }
        // 文本层（template/params/system/messages）按直观布局未独立落盘
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M33 碴7：hf 名缺 quant 未命中报错附已装量化列表（排序呈现）；
    /// 未安装附 pull 带 quant 指引；普通名报错原文不变（回归锚定）
    #[test]
    fn hf_missing_hint_lists_installed_quants() {
        let root = std::env::temp_dir().join(format!("roxid-hfhint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // seed HF 仓库两个量化目录（仅目录名参与提示，无需 meta）
        let repo_dir = root.join("HF/uu/rr");
        std::fs::create_dir_all(repo_dir.join("Q4_K_M")).unwrap();
        std::fs::create_dir_all(repo_dir.join("IQ2_M")).unwrap();

        // 缺 quant（缺省补 latest 未命中）：前缀保持 full_name + 排序量化列表
        let r = ModelRef::parse("hf.co/uu/rr").unwrap();
        let msg = hf_missing_hint(&root, &r);
        assert!(
            msg.starts_with("hf.co/uu/rr:latest"),
            "前缀保持 full_name：{msg}"
        );
        assert!(
            msg.contains("该仓库已装量化：IQ2_M、Q4_K_M"),
            "已装量化排序呈现：{msg}"
        );

        // 仓库存在但无任何量化目录：pull 指引（示例带量化 tag）
        let r2 = ModelRef::parse("hf.co/uu/none").unwrap();
        let msg2 = hf_missing_hint(&root, &r2);
        assert!(
            msg2.contains("如 hf.co/uu/none:Q4_K_M"),
            "指引示例带量化 tag：{msg2}"
        );

        // 完全未出现过的仓库：同 pull 指引形态
        let r3 = ModelRef::parse("hf.co/nobody/void").unwrap();
        assert!(hf_missing_hint(&root, &r3).contains("pull 时请带量化 tag"));

        // 普通名：原文返回不增强
        let plain = ModelRef::parse("plainm").unwrap();
        assert_eq!(hf_missing_hint(&root, &plain), "plainm:latest");

        // find_model 集成：hf 名报错携带增强提示；普通名报错原文不变
        let err = find_model(&root, "hf.co/uu/rr").unwrap_err();
        assert!(
            err.to_string().contains("已装量化"),
            "find_model 报错经增强：{err}"
        );
        assert_eq!(
            find_model(&root, "plainm").unwrap_err().to_string(),
            "model 'plainm:latest' not found",
            "普通名报错语义零变化（回归锚定）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
