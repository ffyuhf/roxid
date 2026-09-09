# CLI 命令参考

[English](../cli.md) | 中文

---

roxid CLI 为 **19 命令形态**：15 个官方 Ollama 对齐子命令 + `setup --llama-url` 参数模式 + `runtime` 多版本管理族 + `completion` 补全族 + `__complete` 隐藏内部命令。命令名、参数结构与原版 ollama 对齐，现有脚本可无缝迁移。

- CLI 经 HTTP 与本机 serve（默认 `127.0.0.1:11434`）通信，serve 未运行时给出明确提示；
- `--help` 文本跟随 locale 双语显示（`LC_ALL` → `LC_MESSAGES` → `LANG`，`zh*` 前缀 → 中文，其余 → 英文），GNU 风格完整帮助。

## 语法总览

```text
roxid [全局标志] <命令> [参数...]

全局标志：
  --nowordwrap   输出不自动换行
  --verbose      显示响应 token 计时（run 通道）
  -v, --version  显示版本信息
```

## 命令树

| 命令 | 用途 | 来源 |
|---|---|---|
| [`serve`](#serve) | 启动 API 服务 | 官方对齐 |
| [`create`](#create) | 从 Modelfile 创建模型 | 官方对齐 |
| [`show`](#show) | 查看模型信息 | 官方对齐 |
| [`run`](#run) | 运行模型（单次 / REPL） | 官方对齐 |
| [`stop`](#stop) | 停止运行中的模型 | 官方对齐 |
| [`pull`](#pull) | 从 registry 拉取模型 | 官方对齐 |
| [`push`](#push) | 推送模型（挂账边界） | 官方对齐 |
| [`signin`](#signin--signout) | 登录 ollama.com | 官方对齐 |
| [`signout`](#signin--signout) | 退出登录 | 官方对齐 |
| [`list`](#list-ls) | 列出本地模型 | 官方对齐 |
| [`ps`](#ps) | 列出运行中的模型 | 官方对齐 |
| [`cp`](#cp) | 复制模型 | 官方对齐 |
| [`rm`](#rm) | 删除模型 | 官方对齐 |
| [`launch`](#launch) | 菜单集成（占位） | 官方对齐 |
| [`setup`](#setup) | 首次运行配置 / 手动后端安装 | 官方对齐 + 参数扩展 |
| [`runtime`](#runtime-族) | llama.cpp 后端多版本管理 | roxid 扩展 |
| [`completion`](#completion-族) | shell 补全管理 | roxid 扩展 |
| `__complete` | 补全候选内部命令（隐藏） | roxid 扩展 |

---

## serve

启动 API 服务（前台阻塞运行；首次启动先执行存量布局迁移，再按需下载 llama.cpp 后端）。

```text
roxid serve [--addr <addr>]
```

| 参数 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `--addr` | string | `127.0.0.1:11434` | 监听地址（`IP:PORT` 形态） |

```sh
roxid serve
roxid serve --addr 0.0.0.0:11434
```

## create

从 Modelfile 创建模型；或进入交互式创建向导。

```text
roxid create <model> [-f <Modelfile>] [-i]
```

| 参数 | 类型 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `model` | string | 是 | — | 新模型名 |
| `-f, --file` | string | 与 `-i` 二选一 | — | Modelfile 路径 |
| `-i, --interactive` | flag | 与 `-f` 二选一 | — | 交互向导：FROM → SYSTEM（多行，`.` 结束）→ RUNTIME → 摘要确认 |

两者皆缺省时报错退出（对齐官方必须给 Modelfile 的行为）。结果为 NDJSON 流式事件，失败经流内 `error` 事件传递（退出码 1）。

```sh
roxid create my-model -f Modelfile
roxid create my-model -i
```

Modelfile 支持指令：`FROM` / `SYSTEM` / `TEMPLATE` / `PARAMETER` / `MESSAGE` / `ADAPTER` / `RUNTIME`。

## show

查看模型信息（三段渲染：参数键值区 / Capabilities / RUNTIME 行；空段不渲染）。

```text
roxid show <model>
```

```sh
roxid show llama3.2:3b
```

## run

运行模型：带 prompt 参数为单次生成，缺省进入多轮 REPL。

```text
roxid run <model> [prompt...] [--hf] [--runtime <flags>]
```

| 参数 | 类型 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `model` | string | 是 | — | 模型名（三形态：`name:tag` / `hf.co/{user}/{repo}:{quant}` / `{user}/{repo}:{quant}` 配 `--hf`） |
| `prompt...` | string[] | 否 | — | 首条提示（缺省进入 REPL） |
| `--hf`（或 model 位前 `-hf`） | flag | 否 | — | 模型名按 HuggingFace 直引解析 |
| `--runtime` | string | 否 | — | 本次运行的 llama.cpp 启动参数（临时覆盖，不落盘；变化触发实例重建） |

REPL 内建命令：`/bye`（或 `/exit` / `/quit`）退出、`/clear` 清空对话；thinking 增量以暗色渲染不回填上下文。模型未安装时收到 404 自动拉取一次后重发。

```sh
roxid run llama3.2:3b
roxid run llama3.2:3b "一句话介绍量子计算"
roxid run -hf Qwen/Qwen2.5-0.5B:IQ2_S
roxid run my-model --runtime "--threads 3 -ngl 30"
```

## stop

停止（卸载）运行中的模型实例。

```text
roxid stop <model>
```

## pull

从 registry 拉取模型。支持 Ollama 主源与 HuggingFace 直引双源；断点续传、NDJSON 进度（TTY spinner / 非 TTY 周期文本行）、同层已存在且摘要一致时跳过下载。

```text
roxid pull <model> [--hf]
```

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `model` | string | 是 | 模型名（HF 直引形态配 `--hf`，等价 `hf.co/` 前缀写法） |
| `--hf`（或 `-hf`） | flag | 否 | 按 HuggingFace 直引解析 |

```sh
roxid pull llama3.2:3b
roxid pull -hf Qwen/Qwen2.5-0.5B:Q4_K_M
```

## push

推送模型至 registry。**当前为挂账边界**：推送管线未实现，调用返回服务端错误；`signin` 凭据已就位待后续启用。

```text
roxid push <model>
```

## signin / signout

登录 / 退出 ollama.com。`signin` 交互输入用户名与访问令牌（令牌隐蔽输入不回显），凭据保存于 `~/.roxid/auth.json`（权限 0600）。`signout` 删除凭据文件。

```text
roxid signin
roxid signout
```

## list (ls)

表格列出本地模型：`NAME` / `SIZE` / `MODIFIED` 三列。`MODIFIED` 为人类可读混合格式——绝对时间 + 中文相对短语，如 `2026-06-04 03:04 (3 个月前)`（分段：秒 / 分钟 / 小时 / 天 / 周 / 个月 / 年 前；差值 ≤0 显示 `刚刚`；无法解析的时间串原样输出）。

```text
roxid list    # 别名：roxid ls
```

## ps

列出运行中的模型：`NAME` / `SIZE` 两列。

```text
roxid ps
```

## cp

复制模型（目标为衍生模型，大文件硬链接基础模型，不占双份磁盘）。

```text
roxid cp <source> <destination>
```

```sh
roxid cp llama3.2:3b llama3.2:3b-copy
```

## rm

删除本地模型。

```text
roxid rm <model>
```

## launch

菜单集成入口。**当前为占位命令**：打印提示后退出（退出码 0），菜单在后续版本提供。

```text
roxid launch
```

## setup

首次运行配置向导（可随时重开）；或参数模式非交互安装手动后端。

```text
roxid setup [--llama-url <url>]
```

| 参数 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `--llama-url` | string | — | 手动指定 llama.cpp 包下载链接（tar.gz 或裸 llama-server 二进制）并立即安装；安装成功记录至 config.toml `[runtime].llama_url`。`https://github.com/` 开头的链接自动经已配置的 GitHub 代理前缀下载，其余链接原样直用 |

交互向导流程：中国网络环境检测（时区 / locale）→ 代理配置（推荐值预填，可改可清除）→ 自定义 llama.cpp 链接（可选，默认跳过）→ shell 补全安装（默认 Y）。联网命令（serve / pull / create / runtime install）首次运行时自动触发引导（config.toml 存在即不再触发）。

## runtime 族

llama.cpp 后端多版本管理（本地操作，不经 serve）。运行时解析优先级：`ROXID_LLAMA_SERVER` 环境变量 → manual → 默认版本（`default_version`）→ 锁定链兜底。

```text
roxid runtime list
roxid runtime install <tag> | --url <url>
roxid runtime use <tag | manual>
roxid runtime rm <tag>
```

| 子命令 | 参数 | 说明 |
|---|---|---|
| `list` | — | 列出已装版本（标注 `[默认]`）与 manual；含解析优先级说明 |
| `install` | `<tag>`（`b\d+` 形态，如 `b10700`）或 `--url <url>`（二选一） | 按官方 tag 下载（自动探测变体：GPU → vulkan / 无 GPU → cpu）；`--url` 安装为 manual 版本 |
| `use` | `<tag>` 或 `manual` | 设默认版本并持久化；切换后首个请求即卸旧实例、以新版本拉起 |
| `rm` | `<tag>` | 删除已装版本；默认版本需先切换后才能删除 |

```sh
roxid runtime list
roxid runtime install b10700
roxid runtime install --url https://example.com/llama-server.tar.gz
roxid runtime use manual
```

## completion 族

shell 补全管理（bash / zsh / fish；静态候选 + 动态值候选——模型名与 runtime tag 直读本地 `~/.roxid`，零网络零延迟）。

```text
roxid completion bash | zsh | fish    # 输出 shim 脚本（供 eval / 管道）
roxid completion install              # 探测 $SHELL 自动落位标准补全目录
```

| 子命令 | 落位 | 说明 |
|---|---|---|
| `bash` | `~/.local/share/bash-completion/completions/roxid` | 新终端自动加载 |
| `zsh` | `~/.zfunc/_roxid` + `.zshrc` 激活块（幂等追加，带标记） | 缺 fpath/compinit 时自动补 |
| `fish` | `~/.config/fish/completions/roxid.fish` | — |

`install` 幂等覆盖，重开终端生效；setup 向导末尾默认顺带执行。

## __complete（隐藏）

`shell shim` 内部调用的候选输出命令（`hide = true`，不进 help 体系）。手写前缀分析输出子命令 / 选项 / 动态值三类候选，无需手工调用。

---

## 环境变量

| 变量 | 作用 |
|---|---|
| `ROXID_HOST` | CLI 连接的服务地址（默认 `http://127.0.0.1:11434`） |
| `OLLAMA_HOST` | 同义回退变量（`ROXID_HOST` 优先），便于现有脚本无缝切换 |

服务端变量（代理 / 后端 / 并行数等）见 [配置](configuration.md)。

## 退出码

| 码 | 含义 |
|---|---|
| `0` | 成功 |
| `1` | 失败：参数错误 / 服务端错误响应 / 文件读写失败 / 交互向导取消等 |

服务不可达时（连接失败）打印统一提示并退出 1：

```text
无法连接 roxid 服务（http://127.0.0.1:11434）：...
请先运行：roxid serve
```

> 若 `ROXID_HOST` / `OLLAMA_HOST` 指向官方 Ollama 实例，CLI 会探测并在 stderr 提示「当前连接的不是 roxid 实例」，命令仍作用于该实例（stdout 保持官方对齐，不污染管道解析）。
