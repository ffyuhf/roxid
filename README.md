# roxid

<div align="center">

**Ollama 的 Rust 复刻 · 单二进制本地大模型运行时 · llama.cpp 子进程后端**

A Rust reimplementation of Ollama · single-binary local LLM runtime · llama.cpp subprocess backend

[快速开始](#快速开始) | [文档](#文档) | [English](#english)

</div>

---

## 中文

### roxid 是什么

roxid 是 [Ollama](https://ollama.com) 的 Rust 复刻：单个静态链接的二进制 `roxid`，对外提供 **Ollama API**、**OpenAI 兼容 API** 与 **llama.cpp 原生端点直通** 三套接口；推理后端为 llama.cpp 的 `llama-server`，以子进程按需拉起、复用、卸载。

### 解决什么问题

- **本地运行大语言模型**：一条命令启动服务，拉取模型即可对话，数据不出本机；
- **生态无缝切换**：端口 `127.0.0.1:11434` 与 API 形态对齐原版 Ollama——现有 ollama 客户端、脚本、OpenAI SDK 无需改造即可指向 roxid；
- **纯 Rust + musl 静态单二进制**：无运行时依赖、无动态库，下载即用；后端与驱动由 roxid 运行时自管。

### 核心能力

- **三套 API（29 端点）**：`/api/*` Ollama 原生 15 端点（NDJSON 流式）；`/v1/*` OpenAI 兼容 6 端点（chat/completions、completions、embeddings、rerank、responses、models）；llama.cpp 原生 8 端点直通（tokenize/detokenize/infill/completion/embedding/props/slots/metrics）
- **19 命令形态 CLI**：15 个官方对齐子命令 + `setup --llama-url` 参数模式 + `runtime` 多版本管理族 + `completion` 补全族；help 文本跟随 locale 中英双语；bash/zsh/fish TAB 补全开箱即用
- **llama.cpp 后端多版本管理**：`runtime list/install/use/rm`；默认版本切换后首个请求即生效；GPU/CPU 分载依赖 llama.cpp `--fit` 自动适配
- **双源模型拉取**：Ollama 主源（断点续传、NDJSON 进度、重复拉取零重下）+ HuggingFace 直引（`roxid run -hf user/repo:quant` 形态）
- **Modelfile 全指令**：FROM / SYSTEM / TEMPLATE / PARAMETER / MESSAGE / ADAPTER / RUNTIME；`create -i` 交互式创建向导
- **think 语义双保险**：`think:false` 模板侧抑制 + 响应 `<think>` 块剥离；low/medium/high/max 四档透传
- **工程防护**：子进程孤儿防护（PDEATHSIG）、实例复用四键判定、宽松 JSON 解析（对齐官方裸 `curl -d` 生态）

### 支持平台与运行时

| 项 | 支持范围 |
|---|---|
| 操作系统 | Linux（amd64 / arm64） |
| 二进制形态 | musl 静态链接单二进制，无运行时依赖 |
| 推理后端 | llama.cpp `llama-server`（首次启动自动下载预编译包，或手动指定/多版本管理） |
| 源码构建 | Rust stable 工具链（`cargo build --release`） |

### 安装

一行安装（Release 页发布后可用）：

```sh
curl -fsSL https://github.com/ffyuhf/roxid/releases/latest/download/roxid_install.sh | sh
```

> 注：发布资产由 GitHub 工作流在创建 Release 时自动编译产出。首个 Release 发布前，请从源码构建：`cargo build --release`，产物位于 `target/release/roxid`。

详细安装方式（脚本双模式 / systemd 服务 / 哈希校验 / 卸载）见 [安装文档](docs/zh/installation.md)。

### 快速开始

CLI 一条命令（首次运行自动进入 setup 引导，自动下载 llama.cpp 后端）：

```sh
roxid serve            # 终端 1：启动 API 服务（127.0.0.1:11434）
roxid run llama3.2:3b  # 终端 2：拉取并对话（未安装时自动拉取）
```

API 一次请求：

```sh
curl http://127.0.0.1:11434/api/generate -d '{"model": "llama3.2:3b", "prompt": "为什么天空是蓝色的？", "stream": false}'
```

完整路径（setup 引导 → 拉模型 → 交互对话 → API 调用）见 [快速开始](docs/zh/quickstart.md)。

### 文档

| 文档 | 中文 | English |
|---|---|---|
| 安装与卸载 | [docs/zh/installation.md](docs/zh/installation.md) | [docs/installation.md](docs/installation.md) |
| 快速开始 | [docs/zh/quickstart.md](docs/zh/quickstart.md) | [docs/quickstart.md](docs/quickstart.md) |
| CLI 命令参考 | [docs/zh/cli.md](docs/zh/cli.md) | [docs/cli.md](docs/cli.md) |
| API 参考 | [docs/zh/api.md](docs/zh/api.md) | [docs/api.md](docs/api.md) |
| 配置 | [docs/zh/configuration.md](docs/zh/configuration.md) | [docs/configuration.md](docs/configuration.md) |
| 错误与状态码 | [docs/zh/errors.md](docs/zh/errors.md) | [docs/errors.md](docs/errors.md) |

### 许可证

[MIT](LICENSE)

### 版本与维护状态

- 当前版本：**0.1.0**（仓库基线）；Release 产物版本跟随发布 tag（发布工作流自动注入）
- 维护状态：**活跃开发中**（迭代 1–24 持续演进；已知边界与挂账项见各文档标注）

---

## English

### What is roxid

roxid is a Rust reimplementation of [Ollama](https://ollama.com): a single statically-linked binary that serves the **Ollama API**, an **OpenAI-compatible API**, and **native llama.cpp endpoint passthrough**. Inference runs on llama.cpp's `llama-server`, spawned as an on-demand subprocess that is reused and unloaded automatically.

### What problem does it solve

- **Run LLMs locally**: start the server with one command, pull a model, and chat — your data never leaves the machine;
- **Drop-in ecosystem compatibility**: listens on `127.0.0.1:11434` with Ollama-shaped APIs — existing ollama clients, scripts, and OpenAI SDKs work unmodified when pointed at roxid;
- **Pure Rust + musl static binary**: no runtime dependencies, no dynamic libraries; the backend and GPU dispatch are managed by roxid itself.

### Core capabilities

- **Three API surfaces (29 endpoints)**: `/api/*` — 15 native Ollama endpoints (NDJSON streaming); `/v1/*` — 6 OpenAI-compatible endpoints (chat/completions, completions, embeddings, rerank, responses, models); 8 native llama.cpp endpoints passed through (tokenize/detokenize/infill/completion/embedding/props/slots/metrics)
- **19-command CLI**: 15 ollama-aligned subcommands + `setup --llama-url` parameter mode + the `runtime` multi-version family + the `completion` family; help text switches between English and Chinese based on locale; bash/zsh/fish tab completion out of the box
- **llama.cpp multi-version backend management**: `runtime list/install/use/rm`; switching the default version takes effect on the very next request; GPU/CPU dispatch relies on llama.cpp `--fit`
- **Dual model sources**: the Ollama registry (resumable downloads, NDJSON progress, digest-level skip for repeated pulls) plus direct HuggingFace references (`roxid run -hf user/repo:quant`)
- **Full Modelfile support**: FROM / SYSTEM / TEMPLATE / PARAMETER / MESSAGE / ADAPTER / RUNTIME; interactive `create -i` wizard
- **think semantics with double insurance**: `think:false` suppression at the template layer plus `<think>`-block stripping in responses; low/medium/high/max tiers passed through
- **Engineering safeguards**: subprocess orphan protection (PDEATHSIG), four-key instance reuse, lenient JSON parsing (matches the official bare `curl -d` ecosystem)

### Supported platforms and runtime

| Item | Supported |
|---|---|
| OS | Linux (amd64 / arm64) |
| Binary | musl static single binary, no runtime dependencies |
| Inference backend | llama.cpp `llama-server` (prebuilt package auto-downloaded on first start, or manually supplied / multi-version managed) |
| Build from source | Rust stable toolchain (`cargo build --release`) |

### Install

One-liner (available once the first Release is published):

```sh
curl -fsSL https://github.com/ffyuhf/roxid/releases/latest/download/roxid_install.sh | sh
```

> Note: release assets are built automatically by a GitHub workflow whenever a Release is published. Before the first Release, build from source: `cargo build --release` (binary at `target/release/roxid`).

See the [installation guide](docs/installation.md) for details (script dual-mode / systemd services / hash verification / uninstall).

### Quick start

One CLI command (first run enters the setup wizard and auto-downloads the llama.cpp backend):

```sh
roxid serve            # terminal 1: start the API server (127.0.0.1:11434)
roxid run llama3.2:3b  # terminal 2: pull and chat (auto-pulls if not installed)
```

One API request:

```sh
curl http://127.0.0.1:11434/api/generate -d '{"model": "llama3.2:3b", "prompt": "Why is the sky blue?", "stream": false}'
```

Full walkthrough (setup wizard → pull → interactive chat → API calls): [quickstart](docs/quickstart.md).

### Documentation

| Document | English | 中文 |
|---|---|---|
| Installation & Uninstall | [docs/installation.md](docs/installation.md) | [docs/zh/installation.md](docs/zh/installation.md) |
| Quickstart | [docs/quickstart.md](docs/quickstart.md) | [docs/zh/quickstart.md](docs/zh/quickstart.md) |
| CLI Reference | [docs/cli.md](docs/cli.md) | [docs/zh/cli.md](docs/zh/cli.md) |
| API Reference | [docs/api.md](docs/api.md) | [docs/zh/api.md](docs/zh/api.md) |
| Configuration | [docs/configuration.md](docs/configuration.md) | [docs/zh/configuration.md](docs/zh/configuration.md) |
| Errors & Status Codes | [docs/errors.md](docs/errors.md) | [docs/zh/errors.md](docs/zh/errors.md) |

### License

[MIT](LICENSE)

### Version and maintenance status

- Current version: **0.1.0** (repository baseline); release artifacts carry the published tag version (injected by the release workflow)
- Maintenance: **actively developed** (iterations 1–24 and counting; known boundaries and deferred items are annotated in each document)
