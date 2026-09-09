//! Modelfile 引擎：解析与模型创建（M9）。
//!
//! 支持指令：FROM / SYSTEM / TEMPLATE / PARAMETER / MESSAGE / ADAPTER；
//! M38（迭代16 Q3 裁决 2026-09-09 04:25）新增 RUNTIME——llama-server
//! 启动参数字符串（roxid 扩展，官方 Modelfile 无此指令）。
//! 值形态：单行裸值或 """三引号块"""（与原版一致）。
//! 模板策略（用户确认 2026-08-24 18:26 透传原则）：TEMPLATE 仅存元数据，
//! 推理优先直通 GGUF 内建模板。
//!
//! 修改历史：占位 2026-08-24 18:35；M9 实装 2026-08-24 19:34；
//! M20 层摘要继承 2026-08-24 22:44；M22 license 继承 2026-08-24 23:12；
//! M31（迭代11 F1）：create 衍生一律落 models/derived/；自引用覆盖的
//! 旧目录经 locate 定位实际位置（可能在 ollama 源）再 .rebuilding 交换
//!（原因：存储三分离需求变更，用户确认 2026-09-06 22:33）2026-09-06 22-55
//! M29 两碴（迭代9）：碴1 create 自引用覆盖安全化（原名先删后链致基础
//! 权重丢失——先原子重命名 .rebuilding 再建新，失败回滚恢复）、碴8
//! PARAMETER 多值白名单 stop 累积成数组（原同名后写覆盖丢多值语义）
//! 2026-09-05 12-40
//! M30 两碴（迭代10）：碴7 衍生模型 adapter 摘要键重排（复制后文件
//! lora-{i} 重新编号而 layer_digests 原样继承 adapter-{全局层索引}，
//! find_blob 定位到不存在文件 → /api/blobs 404）、碴11 非自引用 create
//! 失败清理半成品目录（原残留无 gguf 空壳，窗口内 tags 显示 size=0）
//! 2026-09-06 22-22
//! M32 碴6（迭代12）：create 覆盖（非自引用删旧）与自引用交换（清理
//! .rebuilding）后，按旧目录所在源清理变空的 {model} 父目录链——对齐
//! delete_model 的 cleanup_empty_parents 先例 2026-09-07 01-10

use crate::error::{RoxidError, RoxidResult};
use crate::repo::{ChatMessage, ModelMeta, ModelRef};

/// 解析后的 Modelfile 指令集合
#[derive(Debug, Clone, Default)]
pub struct Modelfile {
    /// FROM：基础模型（本地已存在或远端引用）
    pub from: String,
    /// SYSTEM 系统提示
    pub system: Option<String>,
    /// TEMPLATE 聊天模板
    pub template: Option<String>,
    /// PARAMETER 键值集合（后写覆盖先写）
    pub parameters: std::collections::BTreeMap<String, serde_json::Value>,
    /// MESSAGE 预置对话
    pub messages: Vec<ChatMessage>,
    /// ADAPTER 文件路径列表
    pub adapters: Vec<String>,
    /// RUNTIME：llama-server 启动参数字符串（M38，迭代16 Q3 裁决；
    /// 多行 RUNTIME 空格拼接，spawn 时 shell 风格分词——支持
    /// --override-tensor "exps=CPU" 类含空格引号值）
    pub runtime: Option<String>,
}

/// 多值参数白名单（M29 碴8，R2-A 裁决 2026-09-05 11:43）：同名多笔累积为
/// 数组（对齐原版多条 `PARAMETER stop` 语义）；其余参数同名后写覆盖
/// （标量语义——避免 temperature 等意外成数组破坏下游采样参数映射）
const MULTI_VALUE_PARAMS: &[&str] = &["stop"];

/// 解析 Modelfile 文本。
///
/// - 参数 text：Modelfile 原文
/// - 返回：指令集合；FROM 缺失时报错
pub fn parse(text: &str) -> RoxidResult<Modelfile> {
    let mut mf = Modelfile::default();
    // 指令名 → (单行值 or 三引号块)；逐行扫描，三引号块吞噬到闭合
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (directive, rest) = split_directive(trimmed)?;
        // M38：RUNTIME 值原样保留（不剥引号不走三引号块）——通用路径的
        // trim_matches('"') 会剥掉 "exps=CPU" 类含空格值的尾部引号，
        // 破坏下游 shell 风格分词语义
        let value = if directive == "RUNTIME" {
            rest.trim().to_string()
        } else if let Some(open_rest) = rest.strip_prefix("\"\"\"") {
            if let Some(close) = open_rest.strip_suffix("\"\"\"") {
                close.trim().to_string() // 单行三引号
            } else {
                let mut block = vec![open_rest.to_string()];
                loop {
                    let Some(l) = lines.next() else {
                        return Err(RoxidError::InvalidRequest("三引号块未闭合".into()));
                    };
                    if let Some(v) = l.strip_suffix("\"\"\"") {
                        block.push(v.to_string());
                        break;
                    }
                    block.push(l.to_string());
                }
                block.join("\n").trim().to_string()
            }
        } else {
            rest.trim().trim_matches('"').to_string()
        };

        match directive {
            "FROM" => mf.from = value,
            "SYSTEM" => mf.system = Some(value),
            "TEMPLATE" => mf.template = Some(value),
            "PARAMETER" => {
                let (k, v) = value.split_once(char::is_whitespace).ok_or_else(|| {
                    RoxidError::InvalidRequest(format!("PARAMETER 缺值：{value}"))
                })?;
                // 参数值可为引号包裹字符串（如 stop "<|end|>"），剥离后类型推断
                let coerced = coerce_value(v.trim().trim_matches('"'));
                let key = k.to_string();
                // M29 碴8：多值参数（白名单）同名累积为数组，其余后写覆盖
                if MULTI_VALUE_PARAMS.contains(&key.as_str()) {
                    match mf.parameters.remove(&key) {
                        Some(serde_json::Value::Array(mut arr)) => {
                            arr.push(coerced);
                            mf.parameters.insert(key, serde_json::Value::Array(arr));
                        }
                        Some(prev) => {
                            mf.parameters
                                .insert(key, serde_json::json!([prev, coerced]));
                        }
                        None => {
                            mf.parameters.insert(key, coerced);
                        }
                    }
                } else {
                    mf.parameters.insert(key, coerced);
                }
            }
            "MESSAGE" => {
                let (role, content) = value.split_once(char::is_whitespace).ok_or_else(|| {
                    RoxidError::InvalidRequest(format!("MESSAGE 缺内容：{value}"))
                })?;
                mf.messages.push(ChatMessage {
                    role: role.to_string(),
                    content: content.to_string(),
                });
            }
            "ADAPTER" => mf.adapters.push(value),
            // M38：RUNTIME 启动参数——多行 RUNTIME 空格拼接为单串；
            // 空 flag 报错（语法错误早暴露优于 spawn 时拒启）
            "RUNTIME" => {
                if value.is_empty() {
                    return Err(RoxidError::InvalidRequest("RUNTIME 缺值".into()));
                }
                mf.runtime = Some(match mf.runtime.take() {
                    Some(prev) => format!("{prev} {value}"),
                    None => value,
                });
            }
            other => {
                tracing::warn!("忽略未知 Modelfile 指令：{other}");
            }
        }
    }
    if mf.from.is_empty() {
        return Err(RoxidError::InvalidRequest("Modelfile 缺少 FROM".into()));
    }
    Ok(mf)
}

/// 指令行切分："SYSTEM xxx" → ("SYSTEM", "xxx")
fn split_directive(line: &str) -> RoxidResult<(&str, &str)> {
    line.split_once(char::is_whitespace)
        .ok_or_else(|| RoxidError::InvalidRequest(format!("非法指令行：{line}")))
}

/// 参数值类型推断："0.7"→数字，"true"→布尔，其余字符串
fn coerce_value(v: &str) -> serde_json::Value {
    if let Ok(n) = v.parse::<i64>() {
        return serde_json::json!(n);
    }
    if let Ok(f) = v.parse::<f64>() {
        return serde_json::json!(f);
    }
    if v == "true" || v == "false" {
        return serde_json::json!(v == "true");
    }
    serde_json::json!(v)
}

/// 由 Modelfile 创建模型：基础模型大文件硬链接（Q9），叠加元数据。
///
/// - 参数 mf：解析后的 Modelfile
/// - 参数 name：新模型名
/// - 参数 models_root：仓库根目录
/// - 返回：新模型引用
pub fn create_model(
    mf: &Modelfile,
    name: &str,
    models_root: &std::path::Path,
) -> RoxidResult<ModelRef> {
    let base = ModelRef::parse(&mf.from)?;
    let base_meta = crate::repo::model_json::load_meta(&base.dir(models_root))
        .map_err(|_| RoxidError::ModelNotFound(format!("FROM 基础模型未安装：{}", mf.from)))?;
    let new_ref = ModelRef::parse(name)?;

    // M29 碴1：同名覆盖安全化——自引用（FROM 与目标同名）时旧目录先原子
    // 重命名为 .rebuilding 再建新，失败回滚恢复（原实现 remove_dir_all 先删，
    // 硬链接源随目录消失 → 基础权重不可恢复丢失）
    // M31（迭代11 F1）：新模型一律落 derived 源；旧同名模型经 locate 定位
    // 实际目录（可能在 ollama——如覆盖拉取的基础模型），rename 回滚以旧位置
    // 为锚点（旧布局两者同路径的假设不再成立）
    let dir = new_ref.dir_in(models_root, crate::repo::ModelSource::Derived);
    let self_overwrite = base == new_ref;
    // M32 碴6：保留 (source, dir) 完整定位——删除/交换后按源清理空父链
    let old_loc = crate::repo::locate_model_dir(models_root, &new_ref);
    let old_dir = old_loc.as_ref().map(|(_, d)| d.clone());
    // 自引用必须存在旧目录（base_meta 已读成功，locate 不可能落空；防御式兜错）
    if self_overwrite && old_dir.is_none() {
        return Err(RoxidError::ModelNotFound(base.full_name()));
    }
    let rebuilding = rebuilding_dir_of(old_dir.as_ref().unwrap_or(&dir));
    // 遗留 .rebuilding 为极端崩溃的未完成事务残留，入口清理
    if rebuilding.exists() {
        std::fs::remove_dir_all(&rebuilding)?;
    }
    let source_dir = if self_overwrite {
        std::fs::rename(old_dir.as_ref().unwrap(), &rebuilding)?;
        rebuilding.clone()
    } else {
        // 非自引用覆盖更新（与原版一致）：旧同名目录无论位于哪个源都清掉；
        // M32 碴6：删除后清理因此变空的 {model} 父目录链（delete 先例语义）
        if let Some((source, old)) = &old_loc {
            std::fs::remove_dir_all(old)?;
            cleanup_empty_source_parents(models_root, *source, old);
        }
        base.dir(models_root)
    };

    match build_derived(mf, &base_meta, &source_dir, &dir, &new_ref) {
        Ok(()) => {
            if self_overwrite {
                std::fs::remove_dir_all(&rebuilding)?;
                // M32 碴6：交换完成旧源目录已空，清理其空父链
                if let Some((source, old)) = &old_loc {
                    cleanup_empty_source_parents(models_root, *source, old);
                }
            }
            Ok(new_ref)
        }
        Err(e) => {
            if self_overwrite {
                // 回滚：清半成品后恢复原目录（权重零丢失）
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::rename(&rebuilding, old_dir.as_ref().unwrap())?;
            } else {
                // M30 碴11（R3-A）：非自引用失败清理半成品——原残留带
                // model.json 无 gguf 的空壳目录（tags 显示 size=0 模型；
                // 下次同名 create 靠 dir.exists 先删自愈，不如当场清理直接）
                let _ = std::fs::remove_dir_all(&dir);
            }
            Err(e)
        }
    }
}

/// 组装衍生模型目录（M29 碴1 重构抽出）：从 source_dir 硬链基础大文件——
/// source_dir 在自引用场景为重命名后的 .rebuilding 目录（原模型内容所在地）。
///
/// - 参数 mf：解析后的 Modelfile
/// - 参数 base_meta：基础模型元数据（rename 前已读入内存）
/// - 参数 source_dir：基础大文件来源目录
/// - 参数 dir：新模型目标目录
/// - 参数 new_ref：新模型引用
/// - 返回：Ok(()) 表示目录组装与元数据落盘完成
fn build_derived(
    mf: &Modelfile,
    base_meta: &ModelMeta,
    source_dir: &std::path::Path,
    dir: &std::path::Path,
    new_ref: &ModelRef,
) -> RoxidResult<()> {
    std::fs::create_dir_all(dir)?;

    // 大文件硬链接基础模型
    crate::repo::hardlink_or_copy(
        &source_dir.join(&base_meta.files.model),
        &dir.join(&base_meta.files.model),
    )?;
    if let Some(mm) = &base_meta.files.mmproj {
        crate::repo::hardlink_or_copy(&source_dir.join(mm), &dir.join(mm))?;
    }

    // ADAPTER 复制到新目录（外部文件，非仓库内）
    let mut adapters = Vec::new();
    for (i, path) in mf.adapters.iter().enumerate() {
        let src = std::path::Path::new(path);
        let file = format!("lora-{i}.gguf");
        std::fs::copy(src, dir.join(&file))?;
        adapters.push(file);
    }

    // 元数据合并：基础继承 + Modelfile 覆盖
    let mut parameters = base_meta.parameters.clone();
    for (k, v) in &mf.parameters {
        parameters.insert(k.clone(), v.clone());
    }
    let meta = ModelMeta {
        name: new_ref.full_name(),
        family: base_meta.family.clone(),
        families: base_meta.families.clone(),
        parameter_size: base_meta.parameter_size.clone(),
        quantization_level: base_meta.quantization_level.clone(),
        system: mf.system.clone().unwrap_or(base_meta.system.clone()),
        template: mf.template.clone().or(base_meta.template.clone()),
        parameters,
        messages: if mf.messages.is_empty() {
            base_meta.messages.clone()
        } else {
            mf.messages.clone()
        },
        // M30 碴7：剥离继承的基础 adapter 死键（见函数注释）
        layer_digests: without_base_adapter_digests(base_meta),
        adapters,
        files: base_meta.files.clone(),
        digest: base_meta.digest.clone(),
        license: base_meta.license.clone(),
        // M38：RUNTIME 显式覆盖优先，未设置继承基础（对齐 system/template
        // 继承先例——自引用覆盖更新未带 RUNTIME 时保留原参数）
        runtime: mf.runtime.clone().or(base_meta.runtime.clone()),
        source: "local-create".into(),
        created_at: crate::registry::now_rfc3339(),
    };
    crate::repo::model_json::save_meta(dir, &meta)?;
    Ok(())
}

/// 衍生模型的层摘要索引（M30 碴7）：剥离继承的基础 adapter 摘要键。
/// 衍生模型的 adapter 实体仅来自 Modelfile `ADAPTER` 指令（外部文件复制，
/// 编号 lora-0..n，无 registry digest）——不继承基础模型的 adapter 文件；
/// 原样继承的 `adapter-{全局层索引}` 键指向衍生目录不存在的文件，是
/// find_blob 定位不到的 404 死键。外部 ADAPTER 无摘要可建键，故全部剥离
/// （digest 仍可经基础模型自身命中其实体文件）。
/// 注：计划书 D7 原述「按新序号重排」，实施中被代码事实修正——build_derived
/// 的 adapters 生成自 Modelfile 指令而非基础继承，无旧 digest 可重排。
///
/// - 参数 base：基础模型元数据（摘要索引来源）
/// - 返回：剥离 adapter 死键后的层摘要索引（其余键原样保留）
fn without_base_adapter_digests(base: &ModelMeta) -> std::collections::BTreeMap<String, String> {
    base.layer_digests
        .iter()
        .filter(|(k, _)| !k.starts_with("adapter-"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// 同名覆盖事务的旧目录暂存名：{dir}.rebuilding（与目标同父目录，
/// 同分区 rename 原子性保证交换过程无中间丢失态）。
///
/// - 参数 dir：新模型目标目录
/// - 返回：暂存目录路径
fn rebuilding_dir_of(dir: &std::path::Path) -> std::path::PathBuf {
    let mut s = dir.as_os_str().to_os_string();
    s.push(".rebuilding");
    std::path::PathBuf::from(s)
}

/// 清理旧目录所在源中因此变空的父目录链（M32 碴6）：自 create 的
/// 非自引用覆盖删除与自引用交换收尾两处调用；stop 为该源根目录，
/// 仅空目录可删（复用 repo::ops::cleanup_empty_parents，delete 先例）。
///
/// - 参数 root：models 根目录
/// - 参数 source：旧目录实际所在源
/// - 参数 old：已删除/已交换走的旧模型目录路径
fn cleanup_empty_source_parents(
    root: &std::path::Path,
    source: crate::repo::ModelSource,
    old: &std::path::Path,
) {
    let source_root = crate::repo::ops::dir_in_source_root(root, source);
    crate::repo::ops::cleanup_empty_parents(old, &source_root);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M38：RUNTIME 指令——单行原样保留（含引号）、多行空格拼接、
    /// 空 flag 报错、未写 RUNTIME 时为 None
    #[test]
    fn parse_runtime_directive() {
        let single = parse(
            r#"FROM m
RUNTIME --flash-attn --override-tensor "exps=CPU" -ngl 30"#,
        )
        .unwrap();
        assert_eq!(
            single.runtime.as_deref(),
            Some(r#"--flash-attn --override-tensor "exps=CPU" -ngl 30"#),
            "RUNTIME 值必须原样保留（引号不剥——下游 shell 风格分词）"
        );

        let multi = parse("FROM m\nRUNTIME --flash-attn\nRUNTIME -ngl 30 --jinja").unwrap();
        assert_eq!(
            multi.runtime.as_deref(),
            Some("--flash-attn -ngl 30 --jinja"),
            "多行 RUNTIME 必须空格拼接"
        );

        assert!(
            parse("FROM m\nRUNTIME   ").is_err(),
            "空 RUNTIME 值必须报错（语法错误早暴露）"
        );

        assert!(parse("FROM m").unwrap().runtime.is_none());
    }

    /// M38：create 继承——Modelfile 未写 RUNTIME 时继承基础模型；
    /// 显式写时覆盖（对齐 system/template 继承先例）
    #[test]
    fn create_inherits_and_overrides_runtime() {
        let root = std::env::temp_dir().join(format!("roxid-mf-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = ModelRef::parse("basem:1").unwrap();
        let base_dir = base.dir(&root);
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: Some("--inherited-flag".into()),
                name: "basem:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4_K_M".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-09-09T04:42:00Z".into(),
            },
        )
        .unwrap();

        let inherit = parse("FROM basem:1").unwrap();
        let r1 = create_model(&inherit, "child:a", &root).unwrap();
        assert_eq!(
            crate::repo::model_json::load_meta(&r1.dir(&root))
                .unwrap()
                .runtime
                .as_deref(),
            Some("--inherited-flag")
        );

        let override_ = parse("FROM basem:1\nRUNTIME --my-flag").unwrap();
        let r2 = create_model(&override_, "child:b", &root).unwrap();
        assert_eq!(
            crate::repo::model_json::load_meta(&r2.dir(&root))
                .unwrap()
                .runtime
                .as_deref(),
            Some("--my-flag")
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 全指令解析（含三引号块与参数类型推断）
    #[test]
    fn parse_full_modelfile() {
        let mf = parse(
            r#"
# 注释行
FROM llama3.2
SYSTEM """You are a pirate.
Reply in pirate speak."""
PARAMETER temperature 0.7
PARAMETER num_ctx 8192
PARAMETER stop "<|end|>"
MESSAGE user hello
MESSAGE assistant ahoy
TEMPLATE """{{ .Prompt }}"""
"#,
        )
        .unwrap();
        assert_eq!(mf.from, "llama3.2");
        assert!(
            mf.system
                .unwrap()
                .contains("You are a pirate.\nReply in pirate speak."),
            "三引号块必须保留内部换行"
        );
        assert_eq!(mf.parameters["temperature"], 0.7);
        assert_eq!(mf.parameters["num_ctx"], 8192);
        assert_eq!(mf.parameters["stop"], "<|end|>");
        assert_eq!(mf.messages.len(), 2);
        assert_eq!(mf.messages[0].role, "user");
    }

    /// FROM 缺失必须报错
    #[test]
    fn parse_requires_from() {
        assert!(parse("SYSTEM x").is_err());
    }

    /// 创建模型：硬链接 + 元数据合并覆盖
    #[test]
    fn create_derived_model() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(format!("roxid-mf-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // 造基础模型
        let base_dir = root.join("ollama/llama3.2/3b");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"gguf-bytes").unwrap();
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: None,
                name: "llama3.2:3b".into(),
                family: "llama".into(),
                families: vec!["llama".into()],
                parameter_size: "3.2B".into(),
                quantization_level: "Q4_K_M".into(),
                system: "base-system".into(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: "sha256:base".into(),
                license: "MIT".into(),
                source: "ollama-registry".into(),
                created_at: "2026-08-24T19:00:00Z".into(),
            },
        )
        .unwrap();

        let mf = parse("FROM llama3.2:3b\nSYSTEM my-system\nPARAMETER temperature 0.9").unwrap();
        let r = create_model(&mf, "my-pirate:v1", &root).unwrap();
        assert_eq!(r.full_name(), "my-pirate:v1");
        let meta = crate::repo::model_json::load_meta(&r.dir(&root)).unwrap();
        assert_eq!(meta.system, "my-system", "SYSTEM 必须覆盖基础模型");
        assert_eq!(meta.parameter_size, "3.2B", "家族信息必须继承");
        assert_eq!(meta.license, "MIT", "许可证必须继承（M22）");
        // 硬链接同 inode
        let i1 = std::fs::metadata(base_dir.join("model.gguf"))
            .unwrap()
            .ino();
        let i2 = std::fs::metadata(r.dir(&root).join("model.gguf"))
            .unwrap()
            .ino();
        assert_eq!(i1, i2);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M29 碴1：自引用覆盖（FROM 与目标同名）成功——旧权重经 .rebuilding
    /// 安全交换，新元数据生效且暂存目录清理（原实现先删后链致权重丢失）
    #[test]
    fn self_overwrite_rebuild_succeeds() {
        let root = std::env::temp_dir().join(format!("roxid-mf-selfok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // 造基础模型 llama3.2:3b（gguf 带内容标记）
        let base_dir = root.join("ollama/llama3.2/3b");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"weight-bytes").unwrap();
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: None,
                name: "llama3.2:3b".into(),
                family: "llama".into(),
                families: vec!["llama".into()],
                parameter_size: "3.2B".into(),
                quantization_level: "Q4_K_M".into(),
                system: "old-system".into(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: "sha256:base".into(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-08-24T19:00:00Z".into(),
            },
        )
        .unwrap();

        let mf = parse("FROM llama3.2:3b\nSYSTEM new-system").unwrap();
        let r = create_model(&mf, "llama3.2:3b", &root).unwrap();
        assert_eq!(r.full_name(), "llama3.2:3b");
        // M31（迭代11 F1）：自引用覆盖后新模型（衍生）落 derived 源；
        // 权重零丢失语义不变：gguf 内容原样（rename 交换 + 硬链接同 inode）
        let new_dir = root.join("derived/llama3.2/3b");
        assert_eq!(
            std::fs::read(new_dir.join("model.gguf")).unwrap(),
            b"weight-bytes"
        );
        let meta = crate::repo::model_json::load_meta(&new_dir).unwrap();
        assert_eq!(meta.system, "new-system", "覆盖后元数据必须生效");
        assert!(
            !rebuilding_dir_of(&base_dir).exists(),
            "成功后 .rebuilding 必须清理"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M29 碴1：自引用覆盖失败必须回滚——ADAPTER 指向不存在文件时原模型完好
    #[test]
    fn self_overwrite_failure_rolls_back() {
        let root = std::env::temp_dir().join(format!("roxid-mf-selfrb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base_dir = root.join("ollama/llama3.2/3b");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"weight-bytes").unwrap();
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: None,
                name: "llama3.2:3b".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "3.2B".into(),
                quantization_level: "Q4_K_M".into(),
                system: "old-system".into(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-08-24T19:00:00Z".into(),
            },
        )
        .unwrap();

        let mf = parse("FROM llama3.2:3b\nADAPTER /nonexistent/lora.gguf").unwrap();
        assert!(
            create_model(&mf, "llama3.2:3b", &root).is_err(),
            "构造失败路径"
        );
        // 原模型完好：元数据可读、权重在、暂存目录已回收
        let meta = crate::repo::model_json::load_meta(&base_dir).unwrap();
        assert_eq!(meta.system, "old-system", "回滚后原元数据不变");
        assert_eq!(
            std::fs::read(base_dir.join("model.gguf")).unwrap(),
            b"weight-bytes",
            "回滚后权重必须完整"
        );
        assert!(
            !rebuilding_dir_of(&base_dir).exists(),
            "回滚后暂存目录必须回收"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M29 碴1：遗留 .rebuilding（极端崩溃的事务残留）被入口清理
    #[test]
    fn stale_rebuilding_cleaned_on_entry() {
        let root = std::env::temp_dir().join(format!("roxid-mf-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base_dir = root.join("ollama/llama3.2/3b");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"weight-bytes").unwrap();
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: None,
                name: "llama3.2:3b".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "3.2B".into(),
                quantization_level: "Q4_K_M".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-08-24T19:00:00Z".into(),
            },
        )
        .unwrap();
        // 预置崩溃残留（同名目标自身的暂存目录——入口清理的作用域）
        let stale = rebuilding_dir_of(&base_dir);
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("junk"), b"junk").unwrap();

        let mf = parse("FROM llama3.2:3b\nSYSTEM s2").unwrap();
        create_model(&mf, "llama3.2:3b", &root).unwrap();
        assert!(!stale.exists(), "入口必须清理遗留 .rebuilding");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M29 碴8（R2-A）：stop 多笔累积成数组；标量参数后写覆盖
    #[test]
    fn multi_value_stop_accumulates() {
        let mf = parse(
            "FROM m\nPARAMETER stop \"<|a|>\"\nPARAMETER stop <|b|>\nPARAMETER temperature 0.5\nPARAMETER temperature 0.9",
        )
        .unwrap();
        assert_eq!(
            mf.parameters["stop"],
            serde_json::json!(["<|a|>", "<|b|>"]),
            "白名单多值参数必须累积成数组"
        );
        assert_eq!(
            mf.parameters["temperature"], 0.9,
            "非白名单参数维持后写覆盖"
        );
    }

    /// M30 碴7：衍生模型 adapter 摘要键按复制序号重排——基础模型单 adapter
    /// 处于 layers 全局索引 3（文件 lora-3.gguf、键 adapter-3），衍生复制后
    /// 文件为 lora-0.gguf，摘要键必须重排为 adapter-0（原键定位不存在文件）
    #[test]
    fn derived_adapter_digests_reindexed() {
        let root = std::env::temp_dir().join(format!("roxid-mf-adap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base_dir = root.join("ollama/basem/1");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"gguf").unwrap();
        std::fs::write(base_dir.join("lora-3.gguf"), b"lora-bytes").unwrap();
        let mut layer_digests = std::collections::BTreeMap::new();
        layer_digests.insert("model".to_string(), "sha256:mm".to_string());
        layer_digests.insert("adapter-3".to_string(), "sha256:ad3".to_string());
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: None,
                name: "basem:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests,
                adapters: vec!["lora-3.gguf".into()],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-09-06T00:00:00Z".into(),
            },
        )
        .unwrap();

        let mf = parse("FROM basem:1").unwrap();
        let r = create_model(&mf, "derm:1", &root).unwrap();
        let meta = crate::repo::model_json::load_meta(&r.dir(&root)).unwrap();
        // 衍生模型不继承基础 adapter 实体（ADAPTER 指令显式声明外部文件）
        assert!(
            meta.adapters.is_empty(),
            "无 ADAPTER 指令则衍生无 adapter 文件"
        );
        assert!(
            !r.dir(&root).join("lora-3.gguf").exists(),
            "基础 adapter 实体不得被复制进衍生目录"
        );
        assert!(
            meta.layer_digests.get("adapter-3").is_none(),
            "M30 碴7：继承的基础 adapter 死键必须剥离（原键定位衍生目录不存在的文件）"
        );
        assert_eq!(
            meta.layer_digests.get("model"),
            Some(&"sha256:mm".to_string()),
            "非 adapter 键原样保留"
        );
        // /api/blobs 语义锚定：digest 仍命中真实持有者（基础模型自身实体）
        let hit = crate::repo::find_blob(&root, "sha256:ad3").expect("摘要必须可定位");
        assert!(
            hit.path.ends_with("lora-3.gguf"),
            "命中基础模型的实际文件：{}",
            hit.path.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M32 碴6：非自引用覆盖删除旧目录后，其所在源的空 {model} 父目录
    /// 必须被清理（原残留空目录，与 delete_model 语义不一致）
    #[test]
    fn nonself_overwrite_cleans_empty_old_parent() {
        let root = std::env::temp_dir().join(format!("roxid-mf-emp1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // 造两个基础模型：basem:1（保留）与 oldm:1（待覆盖删除）
        for name in ["basem", "oldm"] {
            let d = root.join(format!("ollama/{name}/1"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("model.gguf"), b"gguf").unwrap();
            crate::repo::model_json::save_meta(
                &d,
                &ModelMeta {
                    runtime: None,
                    name: format!("{name}:1"),
                    family: "llama".into(),
                    families: vec![],
                    parameter_size: "1B".into(),
                    quantization_level: "Q4".into(),
                    system: String::new(),
                    template: None,
                    parameters: Default::default(),
                    messages: vec![],
                    layer_digests: Default::default(),
                    adapters: vec![],
                    files: crate::repo::ModelFiles {
                        model: "model.gguf".into(),
                        mmproj: None,
                    },
                    digest: String::new(),
                    license: String::new(),
                    source: "ollama-registry".into(),
                    created_at: "2026-09-07T00:00:00Z".into(),
                },
            )
            .unwrap();
        }

        // create oldm:1 FROM basem:1（非自引用覆盖：旧 oldm:1 在 ollama 删除）
        let mf = parse("FROM basem:1\nSYSTEM s").unwrap();
        create_model(&mf, "oldm:1", &root).unwrap();

        assert!(
            !root.join("ollama/oldm").exists(),
            "M32 碴6：覆盖删除后旧源空 {{model}} 父目录必须清理"
        );
        assert!(
            root.join("ollama/basem/1/model.gguf").is_file(),
            "非同名基础模型不受影响"
        );
        assert!(
            root.join("derived/oldm/1/model.gguf").is_file(),
            "新模型落 derived"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M32 碴6：自引用交换（旧目录 rename .rebuilding 后删除）收尾同样
    /// 清理旧源空父目录
    #[test]
    fn self_overwrite_cleans_empty_old_parent() {
        let root = std::env::temp_dir().join(format!("roxid-mf-emp2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join("ollama/mym/1");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("model.gguf"), b"weight").unwrap();
        crate::repo::model_json::save_meta(
            &d,
            &ModelMeta {
                runtime: None,
                name: "mym:1".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-09-07T00:00:00Z".into(),
            },
        )
        .unwrap();

        // 自引用覆盖：旧 ollama/mym/1 交换为 derived/mym/1
        let mf = parse("FROM mym:1\nSYSTEM s2").unwrap();
        create_model(&mf, "mym:1", &root).unwrap();

        assert!(
            !root.join("ollama/mym").exists(),
            "M32 碴6：交换后旧源空 {{model}} 父目录必须清理"
        );
        assert!(
            root.join("derived/mym/1/model.gguf").is_file(),
            "新模型落 derived"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M30 碴11：非自引用 create 失败必须清理半成品目录（原残留带
    /// model.json 无 gguf 空壳，窗口内 tags 显示 size=0 模型）
    #[test]
    fn nonself_create_failure_cleans_directory() {
        let root = std::env::temp_dir().join(format!("roxid-mf-nonself-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base_dir = root.join("ollama/llama3.2/3b");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::write(base_dir.join("model.gguf"), b"weight-bytes").unwrap();
        crate::repo::model_json::save_meta(
            &base_dir,
            &ModelMeta {
                runtime: None,
                name: "llama3.2:3b".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "3.2B".into(),
                quantization_level: "Q4_K_M".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "ollama-registry".into(),
                created_at: "2026-09-06T00:00:00Z".into(),
            },
        )
        .unwrap();

        let mf = parse("FROM llama3.2:3b\nADAPTER /nonexistent/lora.gguf").unwrap();
        assert!(
            create_model(&mf, "freshm:v9", &root).is_err(),
            "构造失败路径"
        );
        assert!(
            !ModelRef::parse("freshm:v9").unwrap().dir(&root).exists(),
            "M30 碴11：失败后不得残留半成品目录"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
