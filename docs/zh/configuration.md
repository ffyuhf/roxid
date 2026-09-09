# 配置

[English](../configuration.md) | 中文

---

## 配置文件

| 项 | 值 |
|---|---|
| 路径 | `{ROXID_HOME}/config.toml`，缺省 `~/.roxid/config.toml` |
| 格式 | TOML |
| 查找顺序 | `ROXID_HOME` 环境变量优先 → 缺省 `~/.roxid` |
| 存在性语义 | 文件不存在 = 尚未完成首次引导（联网命令 `serve` / `pull` / `create` / `runtime install` 首次运行时自动触发 setup 向导） |
| 校验与容错 | 文件损坏（非法 TOML）**不阻断启动**——回退全默认值（未引导、无代理），下载链自动直连；保存为逐字段全量覆盖写 |

## 全部字段

```toml
# ~/.roxid/config.toml —— roxid 自动生成与维护（setup 向导 / runtime use 等写入）

setup_done = true            # 引导完成标记；true 后联网命令不再自动触发引导

[proxy]                      # 下载代理（环境变量缺省时的回退来源）
gh = "https://gh.jasonzeng.dev/"   # GitHub 代理前缀（拼接于 GitHub 原始 URL 之前）；可省略
hf = "https://hf-mirror.com/"      # HuggingFace 镜像基址（整体替换官方域名）；可省略

[runtime]                    # llama.cpp 后端
llama_url = "https://example.com/llama.tar.gz"  # 手动后端来源链接（setup --llama-url 记录）；可省略
default_version = "b10700"   # 默认后端版本（runtime use 写入；特殊值 "manual" 走手动版本）；可省略
```

| 字段 | 类型 | 默认 | 取值约束 | 写入方 |
|---|---|---|---|---|
| `setup_done` | bool | `false` | — | setup 向导（恒置 true） |
| `[proxy].gh` | string? | 未设置 | URL 前缀，语义同 `ROXID_GH_PROXY` | setup 向导 |
| `[proxy].hf` | string? | 未设置 | 镜像基址，语义同 `ROXID_HF_PROXY` | setup 向导 |
| `[runtime].llama_url` | string? | 未设置 | tar.gz 或裸二进制链接；`https://github.com/` 开头的链接下载时自动前置拼接已配置的 GitHub 代理前缀（记录值保持用户输入原样） | `setup --llama-url` |
| `[runtime].default_version` | string? | 未设置 | `b\d+` 形态 tag 或 `"manual"` | `runtime use` |

## 环境变量全表

### 服务端

| 变量 | 覆盖优先级 | 说明 |
|---|---|---|
| `ROXID_GH_PROXY` | env > `[proxy].gh` > 直连 | GitHub 代理前缀（后端自动/手动下载、HF 镜像回退链等 GitHub 资源；手动链 `https://github.com/` 开头自动拼接，其余链接原样直用） |
| `ROXID_HF_PROXY` | env > `HF_ENDPOINT` > `[proxy].hf` > 官方 | HuggingFace 镜像基址（整体替换官方域名） |
| `ROXID_LLAMA_SERVER` | **后端解析链最高**：env → manual → `default_version` → 锁定链 `b10605` | 直指 llama-server 二进制路径（自编译 CUDA 版逃生口；不可被配置覆盖） |
| `ROXID_HOME` | env > `~/.roxid` | roxid 数据根目录（config.toml / models / llama.cpp / auth.json 全部随之迁移） |
| `ROXID_ORIGINS` | env > `OLLAMA_ORIGINS` > 默认 localhost 系 | CORS 放行来源（逗号分隔，支持 `*` 通配） |
| `OLLAMA_ORIGINS` | 同上回退位 | 兼容原版 Ollama 惯例 |
| `ROXID_NUM_PARALLEL` | 模型元数据 `num_parallel` > env > `OLLAMA_NUM_PARALLEL` > `4` | 每 slot 并行请求数 |
| `OLLAMA_NUM_PARALLEL` | 同上回退位 | 兼容原版 Ollama 惯例 |
| `RUST_LOG` | — | 日志级别（tracing EnvFilter；缺省 `info`） |

### CLI 侧

| 变量 | 覆盖优先级 | 说明 |
|---|---|---|
| `ROXID_HOST` | env > `OLLAMA_HOST` > `http://127.0.0.1:11434` | CLI 连接的服务地址 |

### 安装脚本侧（仅安装过程读取，详见 [安装文档](installation.md)）

`ROXID_INSTALL_SCOPE` / `ROXID_INSTALL_SOURCE` / `ROXID_LOCAL_BIN` / `ROXID_DOWNLOAD_URL` / `ROXID_VERSION` / `ROXID_GH_PROXY`（与服务端共用同名变量——前缀拼接语义，仅作用于脚本内置基地址）/ `ROXID_RELEASE_BASE`

## 配置文件 / 环境变量 / 命令行参数覆盖关系

| 配置域 | 优先级（高 → 低） |
|---|---|
| 后端版本选择 | `ROXID_LLAMA_SERVER`（env）→ manual（`runtime.llama_url` 安装物）→ `default_version`（config）→ 锁定链自动下载 |
| GitHub 代理 | `ROXID_GH_PROXY`（env）→ `[proxy].gh`（config）→ 直连 |
| HF 镜像 | `ROXID_HF_PROXY`（env）→ `HF_ENDPOINT`（env）→ `[proxy].hf`（config）→ 官方直连 |
| 服务地址（CLI） | `ROXID_HOST`（env）→ `OLLAMA_HOST`（env）→ 默认 `127.0.0.1:11434`（`serve --addr` 仅影响服务端监听地址，二者独立） |
| 采样参数 | 请求 `options`（逐键）→ Modelfile `PARAMETER`（默认层）→ 后端默认 |
| 运行时启动参数 | 请求 `options.runtime`（临时，不落盘）→ Modelfile `RUNTIME`（持久）→ 无参数（`--fit` 自动 GPU 分载） |

## 多实例 / 命名配置

roxid 未内置多 profile 切换；**多实例隔离经 `ROXID_HOME` 实现**——不同 `ROXID_HOME` 即完全独立的数据目录（config.toml / 模型 / 后端缓存 / 凭据）：

```sh
ROXID_HOME=~/.roxid-dev roxid serve --addr 127.0.0.1:11435   # 独立开发实例
```

## 数据目录布局

```text
~/.roxid/                          # ROXID_HOME 缺省值
├─ config.toml                     # 本文档所述配置
├─ auth.json                       # signin 凭据（0600）
├─ models/                         # 模型仓库（三目录按来源分离）
│  ├─ ollama/                      #   Ollama 主源拉取
│  ├─ HF/                          #   HF 直引（{user}/{repo}/{quant}/）
│  └─ derived/                     #   create/copy 衍生（硬链接基础模型）
└─ llama.cpp/                      # 后端运行时缓存
   ├─ {tag}/{variant}/             #   多版本并存（vulkan / cpu 变体）
   └─ manual/llama-server          #   手动版本（setup --llama-url / runtime install --url）
```

首次启动时 serve 自动执行存量旧布局迁移（幂等，fail-fast）。

## 下一步

- [错误与状态码](errors.md)
- [CLI 命令参考](cli.md)
