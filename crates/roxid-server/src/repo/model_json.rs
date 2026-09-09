//! 模型目录元数据（model.json）结构定义与读写。
//!
//! 布局（来源：用户确认 2026-08-24 18:30/18:33 Q8/Q9；迭代11 F1 三分离）：
//! models/{ollama|HF|derived}/.../model.json + model.gguf
//! [+ mmproj.gguf + lora-*.gguf]
//!
//! 修改历史：M4 新增 2026-08-24 19:11；M20 层摘要索引 2026-08-24 22:41；
//! M22 license 字段 2026-08-24 23:10
//! M32 碴4（迭代12）：头注释布局行对齐三目录分离（原两级布局描述漂移）
//! 2026-09-07 01-15
//! M33 碴6（迭代13）：source 字段注释对齐 huggingface:{user}/{repo}
//! 前缀形态（原列举裸 huggingface 与 migrate/hf 落盘事实不符）
//! 2026-09-07 01-58

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{RoxidError, RoxidResult};

/// 聊天消息（Modelfile MESSAGE 与 /api/chat 共用结构）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    /// 角色：system / user / assistant / tool
    pub role: String,
    /// 文本内容
    pub content: String,
}

/// 模型目录内文件布局描述
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFiles {
    /// 主 GGUF 文件名（固定 model.gguf）
    pub model: String,
    /// 多模态投影文件名（无则 None）
    #[serde(default)]
    pub mmproj: Option<String>,
}

/// 模型元数据，落盘于 {模型目录}/model.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMeta {
    /// 完整模型名（model:tag）
    pub name: String,
    /// 模型家族（gguf general.architecture，如 llama / qwen2 / gptoss）
    pub family: String,
    /// 家族链（原版 families 字段）
    pub families: Vec<String>,
    /// 参数量标识（如 3.2B）
    pub parameter_size: String,
    /// 量化等级（如 Q4_K_M）
    pub quantization_level: String,
    /// Modelfile SYSTEM 指令（空串表示未设置）
    #[serde(default)]
    pub system: String,
    /// Modelfile TEMPLATE 指令（None 表示直通 GGUF 内建模板）
    #[serde(default)]
    pub template: Option<String>,
    /// Modelfile PARAMETER 指令集合
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_json::Value>,
    /// Modelfile MESSAGE 预置对话
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// 各内容层的 sha256 摘要（role → digest；M20 /api/blobs 定位用）。
    /// role 集：config / model / projection / adapter-{i} / template / params /
    /// system / messages；旧 model.json 无此字段时为空映射（serde 兼容）。
    #[serde(default)]
    pub layer_digests: BTreeMap<String, String>,
    /// ADAPTER LoRA 文件名列表（与主模型同目录）
    #[serde(default)]
    pub adapters: Vec<String>,
    /// Modelfile RUNTIME 指令（M38，迭代16 Q3 裁决：llama-server 启动参数
    /// 字符串，spawn 时 shell 风格分词透传）；None 表示未设置（旧 model.json
    /// serde 兼容，行为与既有完全一致）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// 文件布局
    pub files: ModelFiles,
    /// 内容摘要（下载时记录；/api/tags digest 字段）
    pub digest: String,
    /// 许可证文本（M22：pull 时保存 license 层，/api/show 返回；旧元数据为空串）
    #[serde(default)]
    pub license: String,
    /// 来源标识：ollama-registry / huggingface:{user}/{repo}（前缀形态，
    /// HF 直拉落盘实况见 migrate.rs/hf.rs）/ local-create
    pub source: String,
    /// 创建时间（RFC3339）
    pub created_at: String,
}

/// 元数据固定文件名
pub const META_FILE_NAME: &str = "model.json";

/// 从模型目录读取元数据。
///
/// - 参数 dir：模型 {model}/{tag} 目录
/// - 返回：解析后的 ModelMeta；文件缺失或格式错误时返回错误
pub fn load_meta(dir: &Path) -> RoxidResult<ModelMeta> {
    let path = dir.join(META_FILE_NAME);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| RoxidError::ModelNotFound(format!("读取 {} 失败：{e}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|e| RoxidError::InvalidRequest(format!("{} 解析失败：{e}", path.display())))
}

/// 将元数据写入模型目录（pretty JSON，便于人工检视）。
///
/// - 参数 dir：模型 {model}/{tag} 目录
/// - 参数 meta：待写入的元数据
pub fn save_meta(dir: &Path, meta: &ModelMeta) -> RoxidResult<()> {
    let path = dir.join(META_FILE_NAME);
    let pretty = serde_json::to_string_pretty(meta)
        .map_err(|e| RoxidError::InvalidRequest(format!("序列化元数据失败：{e}")))?;
    std::fs::write(path, pretty)?;
    Ok(())
}
