# 快速开始

[English](../quickstart.md) | 中文

---

本篇带你从零走到第一次成功的模型调用。前置条件：已按 [安装文档](installation.md) 完成 roxid 安装（`roxid -v` 可输出版本号）。

## 第 1 步：启动服务并完成首次引导

```sh
roxid serve
```

首次运行时（`~/.roxid/config.toml` 尚不存在），serve 会先进入**初次运行引导**：

```text
=== roxid 初次运行引导 ===
检测到中国网络环境（时区/locale 命中）。
是否配置下载代理以加速模型与运行时获取？[Y/n]
GitHub 代理前缀（拼接于 GitHub URL 之前）[回车=https://gh.jasonzeng.dev/ | ...]:
HuggingFace 镜像基址（整体替换官方域名）[回车=https://hf-mirror.com/ | ...]:
是否配置自定义 llama.cpp 下载链接（手动更新后端）？[Y/n]
是否安装 shell 命令补全（bash/zsh/fish，装完重开终端即可 TAB 补全）？[Y/n]
配置已保存：/home/you/.roxid/config.toml
```

- 未检测到中国网络环境时自动跳过代理询问；引导可随时用 `roxid setup` 重开；
- 引导完成后，serve 自动下载 llama.cpp 后端（默认锁定链，变体按宿主 CPU 架构匹配——GPU 环境取 vulkan 变体、无 GPU 取 cpu 变体），随后开始监听：

```text
roxid API 服务已启动: http://127.0.0.1:11434
```

> 非 TTY 环境（管道/CI）不进引导，可设 `ROXID_GH_PROXY` / `ROXID_HF_PROXY` 环境变量，或 `roxid setup --llama-url <url>` 非交互安装自定义后端。

## 关于凭据

**本地推理不需要任何凭据**——拉取公共模型、对话、API 调用均匿名可用。仅当需要向 ollama.com 推送自建模型时才需登录（`roxid signin` 保存用户名与访问令牌至 `~/.roxid/auth.json`，权限 0600；推送管线当前为挂账状态，详见 [API 参考](api.md)）。

## 第 2 步：拉取一个模型

另开一个终端：

```sh
roxid pull llama3.2:3b
```

预期输出（TTY 下为单行 spinner 实时刷新，此处为过程示意）：

```text
pulling manifest
pulling a80c4f17acd5... 100% 614 MB (100%)
success
已拉取：llama3.2:3b
```

也可以直接从 HuggingFace 拉取量化模型：

```sh
roxid pull -hf Qwen/Qwen2.5-0.5B:Q4_K_M   # 等价 hf.co/Qwen/Qwen2.5-0.5B:Q4_K_M
```

## 第 3 步：对话（CLI）

```sh
roxid run llama3.2:3b
```

进入交互 REPL（多轮对话，上下文自动保持）：

```text
>>> 提示词送出，/bye 退出，/clear 清空对话 <<<
llama3.2:3b> 用一句话介绍你自己
我是一个本地运行的大语言模型……
llama3.2:3b> /bye
```

- 单次提问可以不带 REPL：`roxid run llama3.2:3b "用一句话介绍你自己"`；
- 加 `--verbose` 显示 token 计时；
- 模型未安装时 run 会在收到 404 后**自动拉取一次**再重发。

## 第 4 步：调用一次 API

```sh
curl http://127.0.0.1:11434/api/generate \
  -d '{"model": "llama3.2:3b", "prompt": "为什么天空是蓝色的？", "stream": false}'
```

预期输出（单行 JSON，此处展开示意）：

```json
{
  "model": "llama3.2:3b",
  "created_at": "2026-09-10T03:00:00.000000Z",
  "response": "天空呈蓝色是因为瑞利散射……",
  "done": true,
  "total_duration": 5123456789,
  "load_duration": 1098765432,
  "prompt_eval_count": 12,
  "eval_count": 88,
  "eval_duration": 3987654321
}
```

> API 不校验 `Content-Type`——裸 `curl -d` 即可（对齐官方 Ollama 生态）。Ollama 客户端、OpenAI SDK（`http://127.0.0.1:11434/v1`）可直接指向本服务。

## 下一步阅读

| 想做什么 | 去看 |
|---|---|
| 了解全部 CLI 命令与参数 | [CLI 命令参考](cli.md) |
| 对接客户端 / 调用全部 29 个端点 | [API 参考](api.md) |
| 代理、多版本后端、环境变量 | [配置](configuration.md) |
| 报错排查 | [错误与状态码](errors.md) |
