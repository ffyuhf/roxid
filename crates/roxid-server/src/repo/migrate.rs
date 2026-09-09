//! 存量模型布局迁移器（迭代11 F1，来源：用户确认 2026-09-06 22:33
//! 「存量启动时自动迁移」；R3/R6 裁决 2026-09-06 22:38）。
//!
//! 旧布局 `{models}/{model}/{tag}/` 按元数据 `source` 字段三值分类迁移：
//! - `huggingface:{repo}` → `models/HF/{user}/{repo}/{tag}/`，
//!   并将 `meta.name` 重写为新注册名 `hf.co/{user}/{repo}:{tag}`
//!   （repo 取自 source 字段原文，不反解目录名——旧目录名是 `/`→`--` 转义产物）
//! - `ollama-registry` → `models/ollama/{model}/{tag}/`
//! - `local-create`    → `models/derived/{model}/{tag}/`
//!
//! 语义边界：
//! - 幂等可重试：rename 前目标存在即跳过（warn），无 meta / 未知 source
//!   跳过留原地（warn，对齐 M29 残留自愈语义——新布局 list 不再扫旧目录，
//!   留存目录不删除，等下次同位 pull 自愈或用户手动处理）
//! - fail-fast（R6）：任一可分类目录 rename 失败即中止启动并报错，
//!   错误信息含源→目标完整路径；重试时已完成部分因目标存在自动跳过
//! - 进程启动单次执行（serve_main 路由挂载前同步调用），无并发面
//!
//! 修改历史：M31 新增 2026-09-06 22-50（原因：迭代11 F1 存储三分离）

use std::path::Path;

use crate::error::{RoxidError, RoxidResult};

use super::model_json::{load_meta, save_meta, ModelMeta};
use super::HF_NAME_PREFIX;

/// 旧布局下 ollama 主源拉取产物的 source 标记（迁移分类依据之一）
const SOURCE_OLLAMA: &str = "ollama-registry";
/// 旧布局下衍生模型（create/copy）的 source 标记（迁移分类依据之一）
const SOURCE_DERIVED: &str = "local-create";
/// 旧布局下 HF 直拉产物的 source 前缀（迁移分类依据之一，后接 {user}/{repo}）
const SOURCE_HF_PREFIX: &str = "huggingface:";

/// 新布局三个源目录名（迁移扫描旧布局时跳过，防二次迁移）
const NEW_SOURCE_DIRS: [&str; 3] = ["ollama", "HF", "derived"];

/// 执行存量迁移：扫描旧两级布局并按 source 分类 rename 到三目录。
///
/// - 参数 models_root：models 根目录（不存在视为全新安装，直接成功）
/// - 返回：Ok(()) 全部完成（含全部跳过）；任一 rename 失败返回 Err（fail-fast）
pub fn migrate_legacy_layout(models_root: &Path) -> RoxidResult<()> {
    let Ok(models) = std::fs::read_dir(models_root) else {
        return Ok(()); // 根目录不存在：全新安装，无事可做
    };
    let mut migrated = 0usize;
    for model_entry in models.flatten() {
        let model_dir = model_entry.path();
        if !model_dir.is_dir() {
            continue;
        }
        let Some(model_name) = model_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // 新布局源目录不参与旧布局扫描（幂等：已迁移部分天然不可见）
        if NEW_SOURCE_DIRS.contains(&model_name) {
            continue;
        }
        let Ok(tags) = std::fs::read_dir(&model_dir) else {
            continue;
        };
        for tag_entry in tags.flatten() {
            let tag_dir = tag_entry.path();
            if !tag_dir.is_dir() {
                continue;
            }
            if let Some(target) = classify_and_prepare(&tag_dir, model_name) {
                if target.exists() {
                    tracing::warn!(
                        "迁移跳过（目标已存在，保留原位）：{} -> {}",
                        tag_dir.display(),
                        target.display()
                    );
                    continue;
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Err(e) = std::fs::rename(&tag_dir, &target) {
                    // R6 fail-fast：半迁移状态可重试幂等恢复（已完成部分目标存在自动跳过）
                    return Err(RoxidError::RunnerFailure(format!(
                        "存量模型迁移失败：{} -> {}：{e}",
                        tag_dir.display(),
                        target.display()
                    )));
                }
                migrated += 1;
                tracing::info!(
                    "存量模型迁移：{} -> {}",
                    tag_dir.display(),
                    target.display()
                );
            }
        }
        // 旧 {model} 一级目录空则清理（未知/无 meta 残留会使其非空而保留）
        if model_dir
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&model_dir);
        }
    }
    if migrated > 0 {
        tracing::info!("存量迁移完成：共 {migrated} 个模型目录");
    }
    Ok(())
}

/// 读取旧 tag 目录元数据并分类：返回迁移目标路径（含 meta.name 重写落盘）。
/// 无 meta / 未知 source / HF repo 路径异常 → None（跳过留原地，warn 留痕）。
///
/// - 参数 tag_dir：旧布局 {model}/{tag} 目录
/// - 参数 model_name：旧布局一级目录名（HF 场景不用——repo 以 source 字段为准）
/// - 返回：Some(目标目录) 表示可迁移；None 表示跳过
fn classify_and_prepare(tag_dir: &Path, model_name: &str) -> Option<std::path::PathBuf> {
    let models_root = tag_dir
        .parent()
        .and_then(|m| m.parent())
        .expect("迁移调用方保证 tag_dir 位于 {root}/{model}/{tag} 深度");
    let meta = match load_meta(tag_dir) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                "迁移跳过（无元数据或损坏，保留原位）：{}：{e}",
                tag_dir.display()
            );
            return None;
        }
    };
    // clone 后再 strip：避免 strip 的借用阻塞后续 meta 按值传参
    let source = meta.source.clone();
    if let Some(repo) = source.strip_prefix(SOURCE_HF_PREFIX) {
        return prepare_hf_target(models_root, tag_dir, meta, repo);
    }
    let sub = if meta.source == SOURCE_OLLAMA {
        "ollama"
    } else if meta.source == SOURCE_DERIVED {
        "derived"
    } else {
        tracing::warn!(
            "迁移跳过（未知 source=\"{}\"，保留原位）：{}",
            meta.source,
            tag_dir.display()
        );
        return None;
    };
    let tag = tag_dir.file_name()?.to_str()?;
    Some(models_root.join(sub).join(model_name).join(tag))
}

/// HF 分类目标构造：HF/{user}/{repo}/{tag} + meta.name 重写为 hf.co 注册名。
/// repo 非 user/model 两段（异常数据）跳过留原地。
///
/// - 参数 models_root：models 根目录
/// - 参数 tag_dir：旧布局 tag 目录（meta.name 重写后在此落盘，随 rename 迁走）
/// - 参数 meta：已读入的元数据
/// - 参数 repo：source 字段中的 {user}/{repo} 原文
/// - 返回：Some(目标目录)；异常形态 None
fn prepare_hf_target(
    models_root: &Path,
    tag_dir: &Path,
    mut meta: ModelMeta,
    repo: &str,
) -> Option<std::path::PathBuf> {
    // HF 仓库路径必为 {user}/{model} 两段；异常数据不猜（R3 同源语义）
    if repo.split('/').count() != 2 || repo.split('/').any(|s| s.is_empty()) {
        tracing::warn!(
            "迁移跳过（HF source=\"{}\" 非两段仓库路径，保留原位）：{}",
            repo,
            tag_dir.display()
        );
        return None;
    }
    let tag = tag_dir.file_name()?.to_str()?;
    // 注册名重写为 hf.co/{user}/{repo}:{tag}（迭代11 Q3 裁决；旧值是 -- 转义名）
    let new_name = format!("{HF_NAME_PREFIX}{repo}:{tag}");
    if meta.name != new_name {
        meta.name = new_name;
        if let Err(e) = save_meta(tag_dir, &meta) {
            tracing::warn!(
                "迁移跳过（meta.name 重写失败，保留原位）：{}：{e}",
                tag_dir.display()
            );
            return None;
        }
    }
    Some(models_root.join("HF").join(repo).join(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一次性测试根目录
    fn test_root(label: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("roxid-migrate-test-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// 在旧布局位置 seed 一个模型目录（指定 source 与 tag）
    fn seed_legacy(root: &Path, model: &str, tag: &str, source: &str) -> std::path::PathBuf {
        let dir = root.join(model).join(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"fake-gguf").unwrap();
        let meta = ModelMeta {
            runtime: None,
            name: format!("{model}:{tag}"),
            family: "llama".into(),
            families: vec!["llama".into()],
            parameter_size: "3B".into(),
            quantization_level: "Q4_K_M".into(),
            system: String::new(),
            template: None,
            parameters: Default::default(),
            messages: vec![],
            adapters: vec![],
            files: super::super::ModelFiles {
                model: "model.gguf".into(),
                mmproj: None,
            },
            digest: String::new(),
            layer_digests: Default::default(),
            license: String::new(),
            source: source.into(),
            created_at: "2026-09-06T00:00:00Z".into(),
        };
        save_meta(&dir, &meta).unwrap();
        dir
    }

    /// 三类 source 全部按分类落位，HF 注册名重写
    #[test]
    fn migrate_classifies_three_sources() {
        let root = test_root("classify");
        seed_legacy(&root, "llama3.2", "3b", SOURCE_OLLAMA);
        seed_legacy(&root, "user--repo", "IQ2_S", "huggingface:user/repo");
        seed_legacy(&root, "mymodel", "v1", SOURCE_DERIVED);

        migrate_legacy_layout(&root).unwrap();

        assert!(root.join("ollama/llama3.2/3b/model.json").is_file());
        assert!(root.join("HF/user/repo/IQ2_S/model.json").is_file());
        assert!(root.join("derived/mymodel/v1/model.json").is_file());
        // HF 注册名重写为 hf.co 形态
        let hf_meta = load_meta(&root.join("HF/user/repo/IQ2_S")).unwrap();
        assert_eq!(hf_meta.name, "hf.co/user/repo:IQ2_S");
        // 旧一级目录全部清理
        assert!(!root.join("llama3.2").exists());
        assert!(!root.join("user--repo").exists());
        assert!(!root.join("mymodel").exists());
    }

    /// 无 meta 与未知 source 跳过留原地
    #[test]
    fn migrate_skips_unknown_and_missing_meta() {
        let root = test_root("skip");
        seed_legacy(&root, "kept", "1", SOURCE_OLLAMA);
        // 无 meta 目录（伪造 pull 残留形态）
        let no_meta = root.join("residue").join("parts");
        std::fs::create_dir_all(&no_meta).unwrap();
        // 未知 source
        seed_legacy(&root, "weird", "1", "some-future-source");

        migrate_legacy_layout(&root).unwrap();

        assert!(root.join("ollama/kept/1").is_file() || root.join("ollama/kept/1").is_dir());
        assert!(no_meta.exists(), "无 meta 目录必须留原地");
        assert!(
            root.join("weird/1/model.json").is_file(),
            "未知 source 必须留原地"
        );
    }

    /// 幂等：二次执行零动作、零报错
    #[test]
    fn migrate_is_idempotent() {
        let root = test_root("idempotent");
        seed_legacy(&root, "m", "1", SOURCE_OLLAMA);
        migrate_legacy_layout(&root).unwrap();
        // 二次执行（模拟上次失败后重试）：目标已存在场景不存在于本次输入，直接成功
        migrate_legacy_layout(&root).unwrap();
        assert!(root.join("ollama/m/1/model.json").is_file());
    }

    /// 目标已存在（冲突）跳过不覆盖
    #[test]
    fn migrate_target_conflict_skips() {
        let root = test_root("conflict");
        seed_legacy(&root, "m", "1", SOURCE_OLLAMA);
        // 预置目标（模拟半迁移后重试）
        let target = root.join("ollama/m/1");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("sentinel"), b"keep").unwrap();

        migrate_legacy_layout(&root).unwrap();

        assert!(target.join("sentinel").is_file(), "目标内容不得被覆盖");
        // 旧目录因冲突跳过而保留
        assert!(root.join("m/1/model.json").is_file());
    }

    /// 根目录不存在：全新安装直接成功
    #[test]
    fn migrate_missing_root_ok() {
        let root = std::env::temp_dir().join(format!("roxid-migrate-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(migrate_legacy_layout(&root).is_ok());
    }

    /// 迁移后 HF 模型可被 find_model 以 hf.co 注册名定位
    #[test]
    fn migrated_hf_locatable_by_new_name() {
        let root = test_root("locate");
        seed_legacy(&root, "user--repo", "IQ2_S", "huggingface:user/repo");
        migrate_legacy_layout(&root).unwrap();
        let meta = crate::repo::find_model(&root, "hf.co/user/repo:IQ2_S").unwrap();
        assert_eq!(meta.source, "huggingface:user/repo");
        // 旧转义名不再命中（新注册名语义）
        assert!(crate::repo::find_model(&root, "user--repo:IQ2_S").is_err());
    }
}
