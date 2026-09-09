//! 统一错误类型：全部模块共用，避免各层自定义错误互相转换失真。
//!
//! 修改历史：新增于 2026-08-24 18:35（M1 脚手架，原因：项目启动）
//! M34 BUG-5（迭代14）：新增 ServeStartup 变体——serve 端口绑定/退出失败
//! 原归 RunnerFailure 误报「llama-server process error」误导排查方向
//! 2026-09-07 19-20

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
