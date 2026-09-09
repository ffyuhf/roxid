//! 模型仓库：按来源三分的一模型一目录布局
//! （来源：用户确认 2026-08-24 18:30 Q8 / 18:33 Q9；迭代11 F1 三分离 2026-09-06 22:33）。
//!
//! ```text
//! {root}/models/
//! ├─ ollama/{model}/{tag}/            # 主源拉取
//! ├─ HF/{user}/{repo}/{quant}/        # HF 直拉（量化 tag 即目录名）
//! └─ derived/{model}/{tag}/           # create/copy 衍生（大文件硬链接基础）
//!    ├─ model.gguf    # 完整可运行量化文件（可读文件名）
//!    ├─ mmproj.gguf   # 多模态投影（如有）
//!    └─ model.json    # 元数据（见 model_json.rs）
//! ```
//! HF 模型注册名 `hf.co/{user}/{repo}:{quant}`（与原版 ollama 一致，
//! 迭代11 Q3 裁决 2026-09-06 22:35）；目录路由：注册名去 hf.co/ 前缀即 HF 子路径。
//!
//! 修改历史：占位 2026-08-24 18:35；M4 实装 2026-08-24 19:11
//! M31 三目录分离路由与 hf.co 名解析（原因：迭代11 F1 需求变更）2026-09-06 22-45

pub mod migrate;
pub mod model_json;
pub mod ops;

pub use model_json::{ChatMessage, ModelFiles, ModelMeta};
pub use ops::{
    copy_model, delete_model, find_blob, find_model, hardlink_or_copy, list_models,
    list_models_with_dirs, BlobLocation,
};

use std::path::{Path, PathBuf};

use crate::error::{RoxidError, RoxidResult};

/// 默认 tag（与原版 Ollama 一致：不带 tag 的模型名即 latest）
pub const DEFAULT_TAG: &str = "latest";

/// 解析后的模型引用 {model}:{tag}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    /// 模型名：普通名为一级目录名；HF 名为 `hf.co/{user}/{repo}`（路由依据）
    pub model: String,
    /// 二级目录名（普通名变体 tag / HF 名量化 quant）
    pub tag: String,
}

/// 模型来源（迭代11 F1 三目录分离，用户确认 2026-09-06 22:33）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    /// ollama registry 拉取产物 → models/ollama/
    Ollama,
    /// HF 直拉产物 → models/HF/
    Hf,
    /// create/copy 衍生模型 → models/derived/
    Derived,
}

/// HF 注册名前缀（迭代11 Q3 裁决：hf.co/{user}/{repo}:{quant}，对齐原版 ollama）
pub const HF_NAME_PREFIX: &str = "hf.co/";

/// 模型名是否为 HF 形态（hf.co/ 前缀）
pub fn is_hf_name(model: &str) -> bool {
    model.starts_with(HF_NAME_PREFIX)
}

impl ModelRef {
    /// 解析用户输入的模型名：`model` → `model:latest`；`model:tag` 原样。
    /// 普通名仅接受字母/数字/点/下划线/连字符，拒绝路径穿越与非法字符；
    /// HF 名（`hf.co/{user}/{repo}` 或 `hf.co/{user}/{repo}:{quant}`）各路径段
    /// 走同套字符校验（迭代11 F1：注册名扩展，斜杠仅 hf.co/ 形态合法）。
    ///
    /// - 参数 name：用户输入模型名
    /// - 返回：解析后的 ModelRef；非法输入返回 InvalidRequest
    pub fn parse(name: &str) -> RoxidResult<Self> {
        let (model, tag) = match name.split_once(':') {
            Some((m, t)) => (m, t),
            None => (name, DEFAULT_TAG),
        };
        let valid = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        };
        if is_hf_name(model) {
            // hf.co/ 后至少一段仓库路径，各段同普通名校验（拒绝空段与 // 穿越）
            let repo = model.strip_prefix(HF_NAME_PREFIX).unwrap_or(model);
            if repo.is_empty() || !repo.split('/').all(valid) {
                return Err(RoxidError::InvalidRequest(format!(
                    "非法 HF 模型名：{name}"
                )));
            }
        } else if !valid(model) {
            return Err(RoxidError::InvalidRequest(format!("非法模型名：{name}")));
        }
        if !valid(tag) {
            return Err(RoxidError::InvalidRequest(format!("非法 tag：{name}")));
        }
        Ok(Self {
            model: model.to_string(),
            tag: tag.to_string(),
        })
    }

    /// 完整模型名（model:tag；HF 名即注册名 hf.co/{user}/{repo}:{quant}）
    pub fn full_name(&self) -> String {
        format!("{}:{}", self.model, self.tag)
    }

    /// 落位目录（显式来源，迭代11 F1）：写入类调用的唯一路由入口。
    /// - HF 名 + Hf 源：{root}/HF/{user}/{repo}/{tag}
    /// - 普通名 + Ollama/Derived 源：{root}/{ollama|derived}/{model}/{tag}
    ///
    /// - 参数 root：models 根目录
    /// - 参数 source：模型来源（落位目标源）
    /// - 返回：该源下的模型目录路径
    pub fn dir_in(&self, root: &Path, source: ModelSource) -> PathBuf {
        let source_dir = match source {
            ModelSource::Ollama => root.join("ollama"),
            ModelSource::Hf => root.join("HF"),
            ModelSource::Derived => root.join("derived"),
        };
        if is_hf_name(&self.model) {
            let repo = self
                .model
                .strip_prefix(HF_NAME_PREFIX)
                .unwrap_or(&self.model);
            source_dir.join(repo).join(&self.tag)
        } else {
            source_dir.join(&self.model).join(&self.tag)
        }
    }

    /// 模型实际目录定位（查找语义，迭代11 F1）：
    /// - HF 名：HF 三级路径（不存在即该路径，调用方以 is_dir 判定）
    /// - 普通名：ollama 存在用 ollama，否则 derived（R5 裁决：ollama 优先）；
    ///   均未命中回退 ollama 默认路径（供 exists 判定与报错语境）
    ///
    /// - 参数 root：models 根目录
    /// - 返回：命中或回退的目录路径
    pub fn dir(&self, root: &Path) -> PathBuf {
        match locate_model_dir(root, self) {
            Some((_, d)) => d,
            None => self.dir_in(root, ModelSource::Ollama),
        }
    }
}

/// 名字 → 实际目录顺序查找（迭代11 F1）：HF 名直接 HF 路径；
/// 普通名按 ollama → derived 顺序（R5：同名 ollama 优先）。
///
/// - 参数 root：models 根目录
/// - 参数 r：待定位的模型引用
/// - 返回：命中源与目录；未命中 None
pub fn locate_model_dir(root: &Path, r: &ModelRef) -> Option<(ModelSource, PathBuf)> {
    let candidates: &[ModelSource] = if is_hf_name(&r.model) {
        &[ModelSource::Hf]
    } else {
        &[ModelSource::Ollama, ModelSource::Derived]
    };
    candidates
        .iter()
        .map(|s| (*s, r.dir_in(root, *s)))
        .find(|(_, d)| d.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    /// 构造一次性测试仓库根目录（进程内唯一，避免并行测试互踩）
    fn test_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("roxid-repo-test-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// 造一个最小模型目录：model.gguf 占位 + model.json 元数据
    fn seed_model(root: &Path, model: &str, tag: &str) -> ModelRef {
        let r = ModelRef::parse(&format!("{model}:{tag}")).unwrap();
        let dir = r.dir(root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"fake-gguf-bytes").unwrap();
        let meta = ModelMeta {
            runtime: None,
            name: r.full_name(),
            family: "llama".into(),
            families: vec!["llama".into()],
            parameter_size: "3.2B".into(),
            quantization_level: "Q4_K_M".into(),
            system: String::new(),
            template: None,
            parameters: Default::default(),
            messages: vec![],
            adapters: vec![],
            files: ModelFiles {
                model: "model.gguf".into(),
                mmproj: None,
            },
            digest: "sha256:deadbeef".into(),
            layer_digests: Default::default(),
            license: String::new(),
            source: "local-create".into(),
            created_at: "2026-08-24T19:11:00Z".into(),
        };
        model_json::save_meta(&dir, &meta).unwrap();
        r
    }

    /// M31（迭代11 F1）：hf.co 名解析——前缀、多段、tag、非法形态
    #[test]
    fn hf_name_parse_branches() {
        let r = ModelRef::parse("hf.co/ISTA-DASLab/Qwen3.8-27B-GSQ-RCO-GGUF:IQ2_S").unwrap();
        assert_eq!(r.model, "hf.co/ISTA-DASLab/Qwen3.8-27B-GSQ-RCO-GGUF");
        assert_eq!(r.tag, "IQ2_S");
        // 缺省 tag 补 latest（HF 缺省量化选择在 api 层 hf 分支，见 M28 碴5）
        assert_eq!(ModelRef::parse("hf.co/user/repo").unwrap().tag, DEFAULT_TAG);
        assert!(ModelRef::parse("hf.co/").is_err(), "空仓库路径拒绝");
        assert!(ModelRef::parse("hf.co/user//repo").is_err(), "空段拒绝");
        assert!(
            ModelRef::parse("hf.co/user/repo:bad tag").is_err(),
            "非法 tag 拒绝"
        );
    }

    /// M31（迭代11 F1，R5）：同名模型查找 ollama 优先于 derived；ollama 缺位时 derived 接管
    #[test]
    fn locate_prefers_ollama_over_derived() {
        let root = test_root("r5order");
        let r = seed_model(&root, "dupm", "1"); // seed 经 dir() 回退落 ollama
        let derived_dir = r.dir_in(&root, ModelSource::Derived);
        std::fs::create_dir_all(&derived_dir).unwrap();
        std::fs::write(derived_dir.join("model.gguf"), b"derived").unwrap();

        let (src, dir) = locate_model_dir(&root, &r).unwrap();
        assert_eq!(src, ModelSource::Ollama, "同名必须 ollama 优先");
        assert_eq!(dir, r.dir_in(&root, ModelSource::Ollama));

        // 删 ollama 后 derived 接管
        std::fs::remove_dir_all(r.dir_in(&root, ModelSource::Ollama)).unwrap();
        let (src2, dir2) = locate_model_dir(&root, &r).unwrap();
        assert_eq!(src2, ModelSource::Derived);
        assert_eq!(dir2, derived_dir);
    }

    /// M31（迭代11 F1）：list 三目录聚合与 HF 注册名显示
    #[test]
    fn list_aggregates_three_sources() {
        let root = test_root("agg");
        seed_model(&root, "ollam", "1"); // ollama（经 dir() 回退落位）
        seed_model(&root, "deriv", "1");
        // 把 deriv 挪到 derived（seed 落在 ollama 回退路径）
        let d = ModelRef::parse("deriv:1").unwrap();
        let from = d.dir(&root);
        let to = d.dir_in(&root, ModelSource::Derived);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::rename(&from, &to).unwrap();
        // HF 三级目录手工 seed
        let hfr = ModelRef::parse("hf.co/uu/rr:Q8_0").unwrap();
        let hdir = hfr.dir_in(&root, ModelSource::Hf);
        std::fs::create_dir_all(&hdir).unwrap();
        std::fs::write(hdir.join("model.gguf"), b"hf").unwrap();
        model_json::save_meta(
            &hdir,
            &ModelMeta {
                runtime: None,
                name: hfr.full_name(),
                family: "qwen".into(),
                families: vec!["qwen".into()],
                parameter_size: "8B".into(),
                quantization_level: "Q8_0".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                adapters: vec![],
                files: ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                layer_digests: Default::default(),
                license: String::new(),
                source: "huggingface:uu/rr".into(),
                created_at: "2026-09-06T00:00:00Z".into(),
            },
        )
        .unwrap();

        let all = list_models(&root);
        assert_eq!(all.len(), 3, "三目录各一必须全部聚合列出");
        assert!(
            all.iter().any(|m| m.name == "hf.co/uu/rr:Q8_0"),
            "HF 注册名必须以 hf.co 形态展示"
        );
        // find_model 以 hf.co 名命中
        assert_eq!(
            find_model(&root, "hf.co/uu/rr:Q8_0").unwrap().name,
            "hf.co/uu/rr:Q8_0"
        );
    }

    /// 名字解析：默认 tag、显式 tag、非法输入全分支
    #[test]
    fn model_ref_parse_branches() {
        assert_eq!(
            ModelRef::parse("llama3.2").unwrap(),
            ModelRef {
                model: "llama3.2".into(),
                tag: "latest".into()
            }
        );
        assert_eq!(
            ModelRef::parse("llama3.2:3b").unwrap().full_name(),
            "llama3.2:3b"
        );
        assert!(ModelRef::parse("../escape").is_err(), "路径穿越必须拒绝");
        assert!(ModelRef::parse("").is_err(), "空名必须拒绝");
        assert!(ModelRef::parse("a/b").is_err(), "斜杠必须拒绝");
    }

    /// 列举与精确查找
    #[test]
    fn list_and_find_models() {
        let root = test_root("list");
        seed_model(&root, "llama3.2", "3b");
        seed_model(&root, "llama3.2", "1b");
        seed_model(&root, "qwen3", "latest");

        let all = list_models(&root);
        assert_eq!(all.len(), 3, "三个模型必须全部列出");
        let hit = find_model(&root, "llama3.2:3b").unwrap();
        assert_eq!(hit.name, "llama3.2:3b");
        assert!(
            find_model(&root, "llama3.2:9b").is_err(),
            "未安装变体必须报不存在"
        );
    }

    /// M30 碴6：copy 失败（硬链源缺失）必须清理 dst 半成品——原残留带
    /// model.json 无 gguf 的空壳目录（tags 列出 size=0 模型且重试报"已存在"）
    #[test]
    fn copy_failure_cleans_destination() {
        let root = test_root("copyfail");
        let src = seed_model(&root, "srcm", "1");
        // 删除 src 的 gguf 制造硬链源缺失（meta 仍指向 model.gguf）
        std::fs::remove_file(src.dir(&root).join("model.gguf")).unwrap();
        let dst = ModelRef::parse("dstm:1").unwrap();
        assert!(copy_model(&root, &src, &dst).is_err(), "硬链源缺失必须失败");
        assert!(
            !dst.dir(&root).exists(),
            "M30 碴6：失败后不得残留空壳 dst 目录"
        );
    }

    /// 复制必须硬链接（inode 一致）且元数据改名
    #[test]
    fn copy_model_uses_hardlink() {
        let root = test_root("copy");
        let src = seed_model(&root, "llama3.2", "3b");
        let dst = ModelRef::parse("my-llama:custom").unwrap();
        copy_model(&root, &src, &dst).unwrap();

        let src_inode = std::fs::metadata(src.dir(&root).join("model.gguf"))
            .unwrap()
            .ino();
        let dst_inode = std::fs::metadata(dst.dir(&root).join("model.gguf"))
            .unwrap()
            .ino();
        assert_eq!(
            src_inode, dst_inode,
            "复制出的 GGUF 必须与源同 inode（硬链接）"
        );
        assert_eq!(
            find_model(&root, "my-llama:custom").unwrap().name,
            "my-llama:custom"
        );
        assert!(copy_model(&root, &src, &dst).is_err(), "目标已存在必须拒绝");
    }

    /// M20：find_blob 摘要定位（model/projection 层命中文件；config 命中 model.json；
    /// 未落盘文本层与未知摘要不可定位）
    #[test]
    fn find_blob_by_digest() {
        let root = test_root("blobs");
        let r = seed_model(&root, "qwen3", "latest");
        // 补层摘要索引（seed_model 造的是空索引）
        let dir = r.dir(&root);
        let mut meta = model_json::load_meta(&dir).unwrap();
        meta.layer_digests
            .insert("model".into(), "sha256:aa11bb22".into());
        meta.layer_digests
            .insert("config".into(), "sha256:cc33dd44".into());
        meta.files.mmproj = Some("mmproj.gguf".into());
        std::fs::write(dir.join("mmproj.gguf"), b"proj-bytes").unwrap();
        meta.layer_digests
            .insert("projection".into(), "ee55ff66".into());
        model_json::save_meta(&dir, &meta).unwrap();

        // model 层 → GGUF 文件（带与不带 sha256: 前缀均可命中，大小正确）
        let hit = find_blob(&root, "sha256:aa11bb22").unwrap();
        assert!(hit.path.ends_with("model.gguf"));
        assert_eq!(hit.size, b"fake-gguf-bytes".len() as u64);
        assert!(find_blob(&root, "AA11BB22").is_some(), "大小写不敏感");

        // config 层 → model.json
        let hit = find_blob(&root, "sha256:cc33dd44").unwrap();
        assert!(hit.path.ends_with("model.json"));

        // projection 层 → mmproj.gguf
        let hit = find_blob(&root, "ee55ff66").unwrap();
        assert!(hit.path.ends_with("mmproj.gguf"));

        // 未命中与空摘要
        assert!(find_blob(&root, "sha256:00000000").is_none());
        assert!(find_blob(&root, "").is_none());
    }

    /// M32 碴7：同名跨源共存（ollama 与 derived 各持一份同名模型）——
    /// 聚合扫描携带真实持有目录，find_blob 摘要定位命中真实持有者的文件
    ///（原 ModelRef 重新 locate 丢弃来源，derived 条目的摘要错位到
    /// locate 优先命中的 ollama 目录文件）
    #[test]
    fn same_name_cross_source_blob_locates_real_holder() {
        let root = test_root("crosssrc");
        // ollama 源：8KB gguf（seed 经 dir() 回退落 ollama）
        let o = seed_model(&root, "dup3", "1");
        let odir = o.dir(&root);
        std::fs::write(odir.join("model.gguf"), vec![0u8; 8000]).unwrap();
        // 手工构造 derived 源同名模型：1 字节 gguf + 独有 model 摘要
        let d = ModelRef::parse("dup3:1").unwrap();
        let ddir = d.dir_in(&root, ModelSource::Derived);
        std::fs::create_dir_all(&ddir).unwrap();
        std::fs::write(ddir.join("model.gguf"), b"x").unwrap();
        let mut layer_digests = std::collections::BTreeMap::new();
        layer_digests.insert("model".to_string(), "sha256:dd11ee22".to_string());
        model_json::save_meta(
            &ddir,
            &ModelMeta {
                runtime: None,
                name: "dup3:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests,
                adapters: vec![],
                files: ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-07T00:00:00Z".into(),
            },
        )
        .unwrap();

        // 聚合两条同名条目、目录各归其源
        let pairs = list_models_with_dirs(&root);
        assert_eq!(pairs.len(), 2, "同名跨源两条必须全部聚合");
        assert!(
            pairs.iter().any(|(d, _)| d == &ddir),
            "derived 条目必须携带 derived 目录"
        );
        assert!(
            pairs.iter().any(|(d, _)| d == &odir),
            "ollama 条目必须携带 ollama 目录"
        );

        // derived 摘要必须命中 derived 目录的 1 字节文件（原缺陷：locate
        // 优先 ollama，错位返回 ollama 目录 8KB 文件）
        let hit = find_blob(&root, "sha256:dd11ee22").unwrap();
        assert_eq!(hit.path, ddir.join("model.gguf"), "摘要必须命中真实持有者");
        assert_eq!(hit.size, 1, "大小必须为 derived 文件的真实大小");
    }

    /// 删除语义：目录移除、父目录空时清理、硬链接副本不受影响
    #[test]
    fn delete_model_semantics() {
        let root = test_root("delete");
        let src = seed_model(&root, "llama3.2", "3b");
        let dst = ModelRef::parse("backup:copy").unwrap();
        copy_model(&root, &src, &dst).unwrap();

        delete_model(&root, &src).unwrap();
        assert!(!src.dir(&root).exists(), "源目录必须移除");
        // 剩余 backup 目录仍在（父目录不为空）
        assert!(
            dst.dir(&root).join("model.gguf").is_file(),
            "硬链接副本必须仍然可读"
        );
        delete_model(&root, &dst).unwrap();
        assert!(!root.join("backup").exists(), "父目录空后必须一并清理");
        assert!(delete_model(&root, &src).is_err(), "重复删除必须报不存在");
    }
}
