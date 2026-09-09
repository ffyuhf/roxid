//! GGUF 文件头元数据解析（M14，迭代2计划 D1）。
//!
//! 只流式读取 header 与 metadata kv 区，tensor 数据零加载：
//! - 目标 key：general.architecture（family）、general.size_label /
//!   general.size（parameter_size，旧文件用旧 key）、general.file_type（量化）
//! - 非目标 key 按类型宽度计算跳过字节数，tokenizer 等大数组不进内存
//!
//! 版本兼容：v3（u64 长度）/ v2（u32 长度）/ v1（全部 u32 计数）。
//!
//! 修改历史：M14 新增 2026-08-24 20-02
//! M35 D13（迭代15）：新增目标 key {arch}.context_length（show 的
//! model_info 官方键名数据源）与 tensor info 区遍历累加参数元素总数
//! （general.parameter_count 官方数字口径）+ 量化名反查
//! quant_name_to_file_type 2026-09-07 21-42

use std::io::{BufReader, Read, Take};
use std::path::Path;

use crate::error::{RoxidError, RoxidResult};

/// GGUF magic（字节序 "GGUF"，小端 u32 读出值）
const GGUF_MAGIC: u32 = 0x4655_4747;

/// metadata value 类型枚举（GGUF spec）
const T_UINT8: u32 = 0;
const T_INT8: u32 = 1;
const T_UINT16: u32 = 2;
const T_INT16: u32 = 3;
const T_UINT32: u32 = 4;
const T_INT32: u32 = 5;
const T_FLOAT32: u32 = 6;
const T_BOOL: u32 = 7;
const T_STRING: u32 = 8;
const T_ARRAY: u32 = 9;
const T_UINT64: u32 = 10;
const T_INT64: u32 = 11;
const T_FLOAT64: u32 = 12;

/// 目标 key：架构名（→ family）
const KEY_ARCHITECTURE: &str = "general.architecture";
/// 目标 key：规模标签（→ parameter_size；旧文件为 general.size）
const KEY_SIZE_LABEL: &str = "general.size_label";
const KEY_SIZE_LEGACY: &str = "general.size";
/// 目标 key：文件量化类型（→ quantization_level）
const KEY_FILE_TYPE: &str = "general.file_type";

/// 从 GGUF header 解析出的模型元数据
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GgufMetadata {
    /// 模型架构名（如 qwen3 / llama / bert）
    pub architecture: Option<String>,
    /// 规模标签（如 0.6B / 3B / 8B）
    pub size_label: Option<String>,
    /// 文件量化类型原始枚举值（见 file_type_to_quant_name 映射）
    pub file_type: Option<u32>,
    /// 架构上下文长度（GGUF {arch}.context_length 标量；M35 D13）
    pub context_length: Option<u64>,
    /// 参数元素总数（tensor info 区 ne 累加；M35 D13 general.parameter_count）
    pub parameter_count: Option<u64>,
}

/// GGUF kv 区流式读取器（BufReader 包装：定宽读取 + 按字节跳过）
struct KvReader<R: Read> {
    inner: R,
}

impl<R: Read> KvReader<R> {
    fn new(inner: R) -> Self {
        Self { inner }
    }

    /// 读 1 字节
    fn read_u8(&mut self) -> std::io::Result<u8> {
        let mut b = [0u8; 1];
        self.inner.read_exact(&mut b)?;
        Ok(b[0])
    }

    /// 读小端 u16
    fn read_u16(&mut self) -> std::io::Result<u16> {
        let mut b = [0u8; 2];
        self.inner.read_exact(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    /// 读小端 u32
    fn read_u32(&mut self) -> std::io::Result<u32> {
        let mut b = [0u8; 4];
        self.inner.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// 读小端 u64
    fn read_u64(&mut self) -> std::io::Result<u64> {
        let mut b = [0u8; 8];
        self.inner.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    /// 读一个 string 值（长度按版本定宽）
    fn read_string(&mut self, version: u32) -> std::io::Result<String> {
        let len = match version {
            3 => self.read_u64()? as usize,
            _ => self.read_u32()? as usize,
        };
        let mut buf = vec![0u8; len];
        self.inner.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf)
            .trim_matches('\0')
            .trim()
            .to_string())
    }

    /// 跳过 n 字节（大数组专用，不进内存）
    fn skip_bytes(&mut self, n: u64) -> std::io::Result<()> {
        let mut take: Take<&mut R> = (&mut self.inner).take(n);
        std::io::copy(&mut take, &mut std::io::sink())?;
        Ok(())
    }

    /// 跳过一个 string 值（长度按版本定宽，不加载内容）
    fn skip_string(&mut self, version: u32) -> std::io::Result<()> {
        let len = match version {
            3 => self.read_u64()?,
            _ => self.read_u32()? as u64,
        };
        self.skip_bytes(len)
    }

    /// 标量类型的字节宽度（非标量返回 None）
    fn scalar_width(vtype: u32) -> Option<u64> {
        match vtype {
            T_UINT8 | T_INT8 | T_BOOL => Some(1),
            T_UINT16 | T_INT16 => Some(2),
            T_UINT32 | T_INT32 | T_FLOAT32 => Some(4),
            T_UINT64 | T_INT64 | T_FLOAT64 => Some(8),
            _ => None,
        }
    }

    /// 跳过一个 array 值（元素类型 u32 + 个数；标量元素按宽度批量跳，
    /// string 元素逐长度跳过，嵌套 array 不存在于目标文件但做防御递归）
    fn skip_array(&mut self, version: u32, depth: u32) -> std::io::Result<()> {
        let elem_type = self.read_u32()?;
        let count = match version {
            3 => self.read_u64()?,
            _ => self.read_u32()? as u64,
        };
        if let Some(width) = Self::scalar_width(elem_type) {
            // 标量数组：宽度 × 个数一次性跳过（tokenizer 大数组走此路径）
            return self.skip_bytes(width.saturating_mul(count));
        }
        match elem_type {
            T_STRING => {
                for _ in 0..count {
                    self.skip_string(version)?;
                }
                Ok(())
            }
            T_ARRAY if depth < 4 => {
                for _ in 0..count {
                    self.skip_array(version, depth + 1)?;
                }
                Ok(())
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("无法跳过的数组元素类型：{elem_type}"),
            )),
        }
    }

    /// 读取或跳过一个值；仅目标 key 的目标类型被加载，其余全部跳过
    fn read_or_skip_value(&mut self, version: u32, vtype: u32) -> std::io::Result<Option<String>> {
        if let Some(_width) = Self::scalar_width(vtype) {
            // 标量：按宽度读出（u32 供 file_type 用，其余只跳）
            let raw = match _width {
                1 => u64::from(self.read_u8()?),
                2 => u64::from(self.read_u16()?),
                4 => u64::from(self.read_u32()?),
                _ => self.read_u64()?,
            };
            return Ok(Some(raw.to_string()));
        }
        match vtype {
            T_STRING => Ok(Some(self.read_string(version)?)),
            T_ARRAY => {
                self.skip_array(version, 0)?;
                Ok(None)
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("未知 metadata 类型：{vtype}"),
            )),
        }
    }
}

/// 解析 GGUF 文件头元数据。
///
/// - 参数 path：GGUF 文件路径
/// - 返回：GgufMetadata（目标 key 缺失时对应字段为 None）
/// - 错误：非 GGUF 文件（magic 不符）、版本不支持、kv 区截断
pub fn parse_metadata(path: &Path) -> RoxidResult<GgufMetadata> {
    let file = std::fs::File::open(path).map_err(|e| {
        RoxidError::RegistryRequest(format!("打开 GGUF 失败 {}: {e}", path.display()))
    })?;
    let mut reader = KvReader::new(BufReader::with_capacity(64 * 1024, file));

    // header：magic + version + tensor_count + kv_count（v1 计数为 u32，v2+ 为 u64）
    let magic = reader.read_u32()?;
    if magic != GGUF_MAGIC {
        return Err(RoxidError::RegistryRequest(format!(
            "非 GGUF 文件（magic=0x{magic:08x}）：{}",
            path.display()
        )));
    }
    let version = reader.read_u32()?;
    if !(1..=3).contains(&version) {
        return Err(RoxidError::RegistryRequest(format!(
            "不支持的 GGUF 版本：v{version}"
        )));
    }
    // 计数字段宽度随版本：v1 全 u32，v2+ 为 u64
    let read_count = if version == 1 {
        u64::from(reader.read_u32()?)
    } else {
        reader.read_u64()?
    };
    let _tensor_count = read_count;
    let kv_count = if version == 1 {
        u64::from(reader.read_u32()?)
    } else {
        reader.read_u64()?
    };

    // kv 区逐条读取：目标 key 加载，其余跳过
    let mut meta = GgufMetadata::default();
    for _ in 0..kv_count {
        let key = reader.read_string(version)?;
        let vtype = reader.read_u32()?;
        let value = reader.read_or_skip_value(version, vtype)?;

        match key.as_str() {
            KEY_ARCHITECTURE if vtype == T_STRING => meta.architecture = value,
            KEY_SIZE_LABEL | KEY_SIZE_LEGACY if vtype == T_STRING => {
                // 新旧 key 都认：先到先得（正常文件只有其一）
                if meta.size_label.is_none() {
                    meta.size_label = value.filter(|s| !s.is_empty());
                }
            }
            KEY_FILE_TYPE if vtype == T_UINT32 => {
                meta.file_type = value.and_then(|s| s.parse::<u32>().ok());
            }
            // M35 D13：{arch}.context_length（标量整数）收集——show 的
            // model_info 官方键名数据源（如 llama.context_length）；
            // 先到先得（多架构文件取首个）
            _ if key.ends_with(".context_length")
                && vtype != T_STRING
                && vtype != T_ARRAY
                && vtype <= T_FLOAT64 =>
            {
                if meta.context_length.is_none() {
                    meta.context_length = value.and_then(|s| s.parse::<u64>().ok());
                }
            }
            _ => {}
        }
    }
    // M35 D13：tensor info 区遍历累加参数元素总数（general.parameter_count
    // 官方数字口径）。每 tensor 五字段（GGUF spec ggml/gguf.h）：name +
    // n_dims(u32) + ne[n_dims] + type(u32) + offset(u64，tensor 数据区偏移，
    // 必读否则后续项错位)；v1 的 ne 为 u32（v2/v3 为 u64）。tensor 数据区
    //（对齐 padding 之后）零读取
    for _ in 0.._tensor_count {
        let _name = reader.read_string(version)?;
        let n_dims = reader.read_u32()? as usize;
        let mut elements: u64 = 1;
        for _ in 0..n_dims {
            let ne = if version == 1 {
                u64::from(reader.read_u32()?)
            } else {
                reader.read_u64()?
            };
            elements = elements.saturating_mul(ne);
        }
        let _ty = reader.read_u32()?;
        let _offset = reader.read_u64()?;
        meta.parameter_count = Some(meta.parameter_count.unwrap_or(0).saturating_add(elements));
    }
    Ok(meta)
}

/// general.file_type 枚举 → 量化名（与 llama.cpp LlamaFType 对齐）。
///
/// - 参数 code：GGUF general.file_type 值
/// - 返回：量化名字符串（未知值兜底 "Q{code}"）
pub fn file_type_to_quant_name(code: u32) -> String {
    match code {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        4 => "Q4_1_SOME_F16",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        other => return format!("Q{other}"),
    }
    .to_string()
}

/// 量化名 → general.file_type 枚举（file_type_to_quant_name 逆向；M35 D13
/// show 的 model_info 数字口径兜底数据源——GGUF 实时解析缺位时使用）。
///
/// - 参数 name：量化名（如 Q4_K_M）
/// - 返回：枚举值；未知名 None
pub fn quant_name_to_file_type(name: &str) -> Option<u32> {
    (0..=u32::from(u16::MAX)).find(|&c| file_type_to_quant_name(c) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用 GGUF 字节构造辅助
    struct GgufBuilder {
        buf: Vec<u8>,
    }

    impl GgufBuilder {
        /// v3 头：magic + version + tensor_count=0 + kv_count（先占位后回填）
        fn v3() -> Self {
            let mut b = Self { buf: Vec::new() };
            b.buf.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
            b.buf.extend_from_slice(&3u32.to_le_bytes());
            b.buf.extend_from_slice(&0u64.to_le_bytes());
            b.buf.extend_from_slice(&0u64.to_le_bytes()); // kv_count 占位
            b
        }

        /// 追加 string kv（v3：key/value 长度均为 u64）
        fn str_kv(mut self, key: &str, val: &str) -> Self {
            self.buf
                .extend_from_slice(&(key.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(key.as_bytes());
            self.buf.extend_from_slice(&T_STRING.to_le_bytes());
            self.buf
                .extend_from_slice(&(val.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(val.as_bytes());
            self
        }

        /// 追加 u32 kv（v3）
        fn u32_kv(mut self, key: &str, val: u32) -> Self {
            self.buf
                .extend_from_slice(&(key.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(key.as_bytes());
            self.buf.extend_from_slice(&T_UINT32.to_le_bytes());
            self.buf.extend_from_slice(&val.to_le_bytes());
            self
        }

        /// 追加 u64 kv（v3；M35：context_length 收集路径）
        fn u64_kv(mut self, key: &str, val: u64) -> Self {
            self.buf
                .extend_from_slice(&(key.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(key.as_bytes());
            self.buf.extend_from_slice(&T_UINT64.to_le_bytes());
            self.buf.extend_from_slice(&val.to_le_bytes());
            self
        }

        /// 追加 tensor info（v3 五字段：name + n_dims(u32) + ne(u64×n) +
        /// type(u32) + offset(u64)——GGUF spec ggml/gguf.h，M35 修复实证）
        fn tensor(mut self, name: &str, ne: &[u64]) -> Self {
            self.buf
                .extend_from_slice(&(name.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(name.as_bytes());
            self.buf.extend_from_slice(&(ne.len() as u32).to_le_bytes());
            for &d in ne {
                self.buf.extend_from_slice(&d.to_le_bytes());
            }
            self.buf.extend_from_slice(&0u32.to_le_bytes());
            // tensor 数据区偏移（本测试恒 0；字段必须存在否则后续项错位）
            self.buf.extend_from_slice(&0u64.to_le_bytes());
            self
        }

        /// 回填 tensor_count（v3 header 布局 8..16）
        fn tensor_count(mut self, n: u64) -> Self {
            self.buf[8..16].copy_from_slice(&n.to_le_bytes());
            self
        }

        /// 追加标量大数组 kv（跳过路径验证）
        fn u32_array_kv(mut self, key: &str, elems: u32) -> Self {
            self.buf
                .extend_from_slice(&(key.len() as u64).to_le_bytes());
            self.buf.extend_from_slice(key.as_bytes());
            self.buf.extend_from_slice(&T_ARRAY.to_le_bytes());
            self.buf.extend_from_slice(&T_UINT32.to_le_bytes());
            self.buf.extend_from_slice(&(elems as u64).to_le_bytes());
            self.buf
                .extend(std::iter::repeat(0u8).take(4 * elems as usize));
            self
        }
    }

    /// 最小 v3 头：三个目标 key 全命中
    #[test]
    fn minimal_header_parses() {
        // kv_count=3 回填：构造顺序 architecture → size_label → file_type
        let mut b = GgufBuilder::v3().str_kv(KEY_ARCHITECTURE, "qwen3");
        b = b.str_kv(KEY_SIZE_LABEL, "0.6B").u32_kv(KEY_FILE_TYPE, 15);
        // 手工回填 kv_count（header 布局 magic(4)+version(4)+tensor_count(8) → 16..24）
        let meta = {
            let mut buf = b.buf.clone();
            buf[16..24].copy_from_slice(&3u64.to_le_bytes());
            let path = std::env::temp_dir().join("roxid-gguf-min.gguf");
            std::fs::write(&path, buf).unwrap();
            parse_metadata(&path).unwrap()
        };
        assert_eq!(meta.architecture.as_deref(), Some("qwen3"));
        assert_eq!(meta.size_label.as_deref(), Some("0.6B"));
        assert_eq!(meta.file_type, Some(15));
    }

    /// 大数组跳过：目标 key 排在大数组之后仍可正确读取
    #[test]
    fn large_array_skipped() {
        let mut b = GgufBuilder::v3().u32_array_kv("tokenizer.ids", 50_000);
        b = b.str_kv(KEY_ARCHITECTURE, "llama");
        let meta = {
            let mut buf = b.buf.clone();
            buf[16..24].copy_from_slice(&2u64.to_le_bytes());
            let path = std::env::temp_dir().join("roxid-gguf-arr.gguf");
            std::fs::write(&path, buf).unwrap();
            parse_metadata(&path).unwrap()
        };
        assert_eq!(meta.architecture.as_deref(), Some("llama"));
    }

    /// 坏 magic 必须报错
    #[test]
    fn bad_magic_errors() {
        let path = std::env::temp_dir().join("roxid-gguf-bad.gguf");
        std::fs::write(&path, b"NOPE....").unwrap();
        assert!(parse_metadata(&path).is_err());
    }

    /// 旧 key general.size 兼容
    #[test]
    fn legacy_size_key_accepted() {
        let mut b = GgufBuilder::v3().str_kv(KEY_SIZE_LEGACY, "8B");
        let meta = {
            let mut buf = b.buf.clone();
            buf[16..24].copy_from_slice(&1u64.to_le_bytes());
            let path = std::env::temp_dir().join("roxid-gguf-legacy.gguf");
            std::fs::write(&path, buf).unwrap();
            parse_metadata(&path).unwrap()
        };
        assert_eq!(meta.size_label.as_deref(), Some("8B"));
    }

    /// file_type → 量化名映射（含兜底）
    #[test]
    fn file_type_mapping() {
        assert_eq!(file_type_to_quant_name(0), "F32");
        assert_eq!(file_type_to_quant_name(15), "Q4_K_M");
        assert_eq!(file_type_to_quant_name(18), "Q6_K");
        assert_eq!(file_type_to_quant_name(32), "BF16");
        assert_eq!(file_type_to_quant_name(999), "Q999");
    }

    /// M35 D13：量化名反查（file_type_to_quant_name 逆向往返）
    #[test]
    fn quant_name_reverse_lookup() {
        assert_eq!(quant_name_to_file_type("Q4_K_M"), Some(15));
        assert_eq!(quant_name_to_file_type("F16"), Some(1));
        assert_eq!(quant_name_to_file_type("BF16"), Some(32));
        assert_eq!(quant_name_to_file_type("IQ4_XS"), Some(30));
        assert_eq!(quant_name_to_file_type("不存在的量化"), None);
    }

    /// M35 D13：context_length 收集 + tensor 区参数累加
    #[test]
    fn context_length_and_parameter_count_collected() {
        // kv：arch + {arch}.context_length(u64)；tensor：2×3 + 4（参数 10）
        let b = GgufBuilder::v3()
            .str_kv(KEY_ARCHITECTURE, "llama")
            .u64_kv("llama.context_length", 131072)
            .tensor("token_embd.weight", &[2, 3])
            .tensor("output_norm.weight", &[4])
            .tensor_count(2);
        let meta = {
            let mut buf = b.buf.clone();
            buf[16..24].copy_from_slice(&2u64.to_le_bytes()); // kv_count=2
            let path = std::env::temp_dir().join("roxid-gguf-m35.gguf");
            std::fs::write(&path, buf).unwrap();
            parse_metadata(&path).unwrap()
        };
        assert_eq!(meta.architecture.as_deref(), Some("llama"));
        assert_eq!(meta.context_length, Some(131072));
        assert_eq!(meta.parameter_count, Some(10), "2×3 + 4 = 10 元素累加");
    }

    /// M35 D13：无 context_length kv / 无 tensor 时两字段 None（兜底链数据源）
    #[test]
    fn missing_keys_yield_none() {
        let b = GgufBuilder::v3().str_kv(KEY_ARCHITECTURE, "llama");
        let meta = {
            let mut buf = b.buf.clone();
            buf[16..24].copy_from_slice(&1u64.to_le_bytes());
            let path = std::env::temp_dir().join("roxid-gguf-m35b.gguf");
            std::fs::write(&path, buf).unwrap();
            parse_metadata(&path).unwrap()
        };
        assert_eq!(meta.context_length, None);
        assert_eq!(meta.parameter_count, None);
    }

    /// 真实 GGUF 头部解析（ignored：样本需网络下载，手动验收）。
    /// 样本来源：hf-mirror.com bartowski/Qwen2.5-0.5B-Instruct-GGUF Q4_K_M 前 2MB（Range）。
    #[test]
    #[ignore = "真实文件验收：样本 /tmp/roxid-real-gguf-head.gguf 需镜像 Range 下载"]
    fn real_gguf_header_parses() {
        let path = std::path::PathBuf::from("/tmp/roxid-real-gguf-head.gguf");
        if !path.is_file() {
            eprintln!("真实样本不存在，跳过（下载方式见本测试文档注释）");
            return;
        }
        let meta = parse_metadata(&path).expect("真实 GGUF header 必须解析成功");
        assert_eq!(meta.architecture.as_deref(), Some("qwen2"));
        assert_eq!(meta.size_label.as_deref(), Some("0.5B"));
        assert_eq!(meta.file_type, Some(15), "Q4_K_M 的 file_type 枚举值");
    }
}
