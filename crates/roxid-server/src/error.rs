//! 统一错误类型：全部模块共用，避免各层自定义错误互相转换失真。
//!
//! 修改历史：新增于 2026-08-24 18:35（M1 脚手架，原因：项目启动）
//! M34 BUG-5（迭代14）：新增 ServeStartup 变体——serve 端口绑定/退出失败
//! 原归 RunnerFailure 误报「llama-server process error」误导排查方向
//! 2026-09-07 19-20
//! M99（迭代31）：新增 ArchMismatch 变体——llama-server 二进制架构与宿主
//! 不匹配的专用确定性错误（原 RunnerFailure 裸透 os error 2 无从定位，
//! 且被换端口盲目重试 3 次；acquire 归 5xx 加载失败通道，
//! 用户裁决 Q3/Q4 2026-09-10 18:17/18:21）2026-09-10 18-30

use thiserror::Error;

/// roxid 全局错误类型
#[derive(Debug, Error)]
pub enum RoxidError {
    /// 模型未找到：携带用户请求的模型名
    #[error("model '{0}' not found")]
    ModelNotFound(String),
    /// llama-server 子进程异常：携带进程上下文描述
    #[error("llama-server process error: {0}")]
    RunnerFailure(String),
    /// llama-server 二进制架构与宿主不匹配（M99，迭代31）：确定性失败——
    /// 换端口重试无意义（spawn_with_port_retry 对本变体直接透出不重试）；
    /// 携带诊断文案（二进制架构 vs 宿主架构 + rm 重装 / env 逃生口指引）
    #[error("llama-server arch mismatch: {0}")]
    ArchMismatch(String),
    /// 服务进程自身启动/运行失败（端口绑定、serve 循环退出——M34 BUG-5：
    /// 区别于推理子进程，端口占用不再误导排查 llama.cpp）
    #[error("serve error: {0}")]
    ServeStartup(String),
    /// 上游 registry（ollama/HF）请求失败
    #[error("registry request failed: {0}")]
    RegistryRequest(String),
    /// 请求参数不合法：携带字段级描述
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// 本地文件系统操作失败
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// 统一 Result 别名，全库签名统一使用
pub type RoxidResult<T> = Result<T, RoxidError>;
