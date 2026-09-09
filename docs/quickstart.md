# Quickstart

English | [中文](zh/quickstart.md)

---

This guide takes you from zero to your first successful model call. Prerequisite: roxid is installed per the [installation guide](installation.md) (`roxid -v` prints a version).

## Step 1: Start the server and complete the first-run wizard

```sh
roxid serve
```

On the very first run (when `~/.roxid/config.toml` does not exist yet), serve enters the **first-run wizard**:

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

- The proxy questions are skipped automatically outside mainland-China network environments; re-open the wizard anytime with `roxid setup`;
- After the wizard, serve auto-downloads the llama.cpp backend (locked tag by default; vulkan variant on GPU, cpu otherwise) and starts listening:

```text
roxid API 服务已启动: http://127.0.0.1:11434
```

> In non-TTY environments (pipes/CI) the wizard is skipped; set `ROXID_GH_PROXY` / `ROXID_HF_PROXY`, or install a custom backend non-interactively via `roxid setup --llama-url <url>`.

## About credentials

**Local inference needs no credentials at all** — pulling public models, chatting, and API calls all work anonymously. Logging in (`roxid signin`, which stores a username and access token in `~/.roxid/auth.json` with 0600 permissions) is only relevant for pushing models to ollama.com (the push pipeline is currently deferred; see the [API reference](api.md)).

## Step 2: Pull a model

Open another terminal:

```sh
roxid pull llama3.2:3b
```

Expected output (a live single-line spinner under TTY; the process looks like):

```text
pulling manifest
pulling a80c4f17acd5... 100% 614 MB (100%)
success
已拉取：llama3.2:3b
```

You can also pull quantized models straight from HuggingFace:

```sh
roxid pull -hf Qwen/Qwen2.5-0.5B:Q4_K_M   # equivalent to hf.co/Qwen/Qwen2.5-0.5B:Q4_K_M
```

## Step 3: Chat (CLI)

```sh
roxid run llama3.2:3b
```

This opens an interactive REPL (multi-turn, context kept automatically):

```text
>>> 提示词送出，/bye 退出，/clear 清空对话 <<<
llama3.2:3b> 用一句话介绍你自己
我是一个本地运行的大语言模型……
llama3.2:3b> /bye
```

- One-shot without the REPL: `roxid run llama3.2:3b "introduce yourself in one sentence"`;
- Add `--verbose` for token timing;
- If the model is not installed, run **auto-pulls it once** upon a 404 and retries.

## Step 4: Make one API call

```sh
curl http://127.0.0.1:11434/api/generate \
  -d '{"model": "llama3.2:3b", "prompt": "Why is the sky blue?", "stream": false}'
```

Expected output (a single JSON line, pretty-printed here):

```json
{
  "model": "llama3.2:3b",
  "created_at": "2026-09-10T03:00:00.000000Z",
  "response": "The sky is blue because of Rayleigh scattering...",
  "done": true,
  "total_duration": 5123456789,
  "load_duration": 1098765432,
  "prompt_eval_count": 12,
  "eval_count": 88,
  "eval_duration": 3987654321
}
```

> The API does not check `Content-Type` — bare `curl -d` just works (matching the official Ollama ecosystem). Point existing ollama clients and OpenAI SDKs (`http://127.0.0.1:11434/v1`) at this server.

## Next steps

| Want to | Read |
|---|---|
| See all CLI commands and arguments | [CLI Reference](cli.md) |
| Integrate clients / use all 29 endpoints | [API Reference](api.md) |
| Proxies, backend versions, env vars | [Configuration](configuration.md) |
| Troubleshoot errors | [Errors & Status Codes](errors.md) |
