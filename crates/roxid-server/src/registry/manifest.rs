//! Ollama registry manifest 与相关层类型定义。
//!
//! 协议实测（2026-08-24 19:12）：
//! - GET /v2/{repo}/manifests/{tag} + Accept manifest.v2 → 200（匿名可读）
//! - config：JSON 元数据（family 等）
//! - layers 按 mediaType 区分：model/params/template/license/
//!   system/messages/adapter/projection
//! - GET /v2/{repo}/blobs/sha256:{digest} → 307 重定向 R2 预签名 URL
//!
//! 修改历史：M5 新增 2026-08-24 19:14
//!；M200（迭代53）结构体级豁免 non_snake_case——字段名逐字对齐协议键
//! 禁改（用户确认 2026-09-12 21:36「不要改变任何功能，但是要消除警告」）
//! 2026-09-12 21-37

use serde::{Deserialize, Serialize};

/// registry v2 manifest 顶层结构
///
/// schemaVersion/mediaType 逐字对齐 Ollama registry v2 JSON 协议键
///（serde 反序列化契约，改名即破坏协议对齐），故结构体级豁免
/// non_snake_case（来源：用户确认 2026-09-12 21:36，M200）
#[allow(non_snake_case)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub schemaVersion: u32,
    pub mediaType: String,
    /// 模型配置层（小 JSON：family 等）
    pub config: LayerDescriptor,
    /// 内容层集合
    pub layers: Vec<LayerDescriptor>,
}

/// manifest 中的层描述
///
/// mediaType 同 Manifest：协议键直译命名，结构体级豁免 non_snake_case
///（来源：用户确认 2026-09-12 21:36，M200）
#[allow(non_snake_case)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LayerDescriptor {
    pub mediaType: String,
    /// 形如 sha256:eb2c714d...
    pub digest: String,
    pub size: u64,
}

/// Ollama 层 mediaType 常量（来源：manifest 实测与原版协议）
pub mod media_types {
    /// 主模型 GGUF
    pub const MODEL: &str = "application/vnd.ollama.image.model";
    /// 参数（stop 等）
    pub const PARAMS: &str = "application/vnd.ollama.image.params";
    /// 聊天模板
    pub const TEMPLATE: &str = "application/vnd.ollama.image.template";
    /// 许可证文本
    pub const LICENSE: &str = "application/vnd.ollama.image.license";
    /// 系统提示（create 产生）
    pub const SYSTEM: &str = "application/vnd.ollama.image.system";
    /// 预置对话（create 产生）
    pub const MESSAGES: &str = "application/vnd.ollama.image.messages";
    /// LoRA 适配器
    pub const ADAPTER: &str = "application/vnd.ollama.image.adapter";
    /// 多模态投影（mmproj）
    pub const PROJECTION: &str = "application/vnd.ollama.image.projection";
}

/// config 层 JSON 结构（模型家族等元数据）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageConfig {
    /// 模型家族（llama / qwen2 / ...）
    #[serde(default)]
    pub model_family: String,
    /// 家族链
    #[serde(default)]
    pub model_families: Option<Vec<String>>,
    /// 参数量标识（3.2B）
    #[serde(default)]
    pub model_type: String,
    /// 量化等级（Q4_K_M）
    #[serde(default)]
    pub file_type: String,
    /// 目录/来源信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootfs: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// manifest 实测报文必须完整反序列化（smollm:135m，2026-08-24 抓取）
    #[test]
    fn manifest_parses_real_payload() {
        let raw = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": {
                "mediaType": "application/vnd.docker.container.image.v1+json",
                "digest": "sha256:f590523c855b7d0f2741a9e076d4b663b1f128f2617b7fcd3fe7d7b57ce71d83",
                "size": 488
            },
            "layers": [
                {"mediaType": "application/vnd.ollama.image.model", "digest": "sha256:eb2c714d40d4b35ba4b8ee98475a06d51d8080a17d2d2a75a23665985c739b94", "size": 91727296},
                {"mediaType": "application/vnd.ollama.image.template", "digest": "sha256:62fbfd9ed093d6e5ac83190c86eec5369317919f4b149598d2dbb38900e9faef", "size": 182},
                {"mediaType": "application/vnd.ollama.image.license", "digest": "sha256:cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30", "size": 11358},
                {"mediaType": "application/vnd.ollama.image.params", "digest": "sha256:ca7a9654b5469dc2d638456f31a51a03367987c54135c089165752d9eeb08cd7", "size": 89}
            ]
        }"#;
        let m: Manifest = serde_json::from_str(raw).unwrap();
        assert_eq!(m.layers.len(), 4);
        assert_eq!(m.layers[0].mediaType, media_types::MODEL);
        assert_eq!(m.layers[0].size, 91727296);
    }
}
