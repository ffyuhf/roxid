//! roxid-server：Ollama 全功能复刻的服务端核心库。
//!
//! 架构原则（来源：用户确认 2026-08-24 18:26）：
//! 凡 llama.cpp 原生自带的能力（多模态、采样参数、投机解码、
//! Responses API、rerank 等）一律透传给 llama-server 子进程，
//! 本库只做协议格式映射、模型管理与子进程编排，不实现推理侧管线。
//!
//! 模块总览（与 项目文档/03-设计/01-架构设计 总览文档一一对应）：
//! - config:    全局配置与目录约定
//! - error:     统一错误类型
//! - api:       API 网关层（Ollama /api/*、OpenAI /v1/*、Responses API）
//! - adapter:   协议适配层（Ollama 格式与 OpenAI 格式互转）
//! - scheduler: 推理调度层（Runner 注册表、keep_alive、请求排队）
//! - runtime:   llama.cpp 运行时管理器（GPU 探测、预编译包下载缓存）
//! - repo:      模型仓库（三目录分离布局：ollama/HF/derived，迭代11 F1）
//! - registry:  Registry 客户端（Ollama v2 协议 + HuggingFace GGUF）
//! - modelfile: Modelfile 引擎（解析与模型创建）
//!
//! 修改历史：新增于 2026-08-24 18:35（M1 脚手架，原因：项目启动）
//! M32 碴4（迭代12）：模块总览 repo 行对齐三目录分离布局（原两级布局
//! 描述漂移）2026-09-07 01-15

pub mod adapter;
pub mod api;
pub mod config;
pub mod error;
pub mod modelfile;
pub mod registry;
pub mod repo;
pub mod runtime;
pub mod scheduler;
