# CLI Reference

English | [中文](zh/cli.md)

---

The roxid CLI ships **19 command forms**: 15 ollama-aligned subcommands + the `setup --llama-url` parameter mode + the `runtime` multi-version family + the `completion` family + the hidden `__complete` internal command. Command names and argument structures align with the original ollama, so existing scripts migrate seamlessly.

- The CLI talks to a local serve instance over HTTP (default `127.0.0.1:11434`) and prints a clear hint when serve is not running;
- `--help` text is bilingual based on locale (`LC_ALL` → `LC_MESSAGES` → `LANG`; `zh*` prefix → Chinese, otherwise English), in GNU-style complete form.

## Syntax overview

```text
roxid [global flags] <command> [args...]

Global flags:
  --nowordwrap   disable automatic word wrapping
  --verbose      show response token timings (run)
  -v, --version  show version information
```

The global flags `--nowordwrap` / `--verbose` are clap `global` flags — they may appear before or after the subcommand (`roxid --verbose run m` equals `roxid run --verbose m`).

**Colors & width adaptation**: on a TTY, error lines render red, success echoes green, table headers bold, and the REPL banner/prompt colored. All ANSI output is disabled when `NO_COLOR` is set to a non-empty value, when `TERM=dumb`, or when the stream is redirected (stdout and stderr are judged independently, keeping pipe parsers clean). Progress frames and the `list` / `ps` tables converge to the real terminal width (the `COLUMNS` env var takes precedence), with CJK-aware truncation of over-wide content.

## Command tree

| Command | Purpose | Origin |
|---|---|---|
| [`serve`](#serve) | Start the API server | ollama-aligned |
| [`create`](#create) | Create a model from a Modelfile | ollama-aligned |
| [`show`](#show) | Show model information | ollama-aligned |
| [`run`](#run) | Run a model (one-shot / REPL) | ollama-aligned |
| [`stop`](#stop) | Stop a running model | ollama-aligned |
| [`pull`](#pull) | Pull a model from a registry | ollama-aligned |
| [`push`](#push) | Push a model (deferred boundary) | ollama-aligned |
| [`signin`](#signin--signout) | Sign in to ollama.com | ollama-aligned |
| [`signout`](#signin--signout) | Sign out | ollama-aligned |
| [`list`](#list-ls) | List local models | ollama-aligned |
| [`ps`](#ps) | List running models | ollama-aligned |
| [`cp`](#cp) | Copy a model | ollama-aligned |
| [`rm`](#rm) | Remove a model | ollama-aligned |
| [`launch`](#launch) | Menu integration (placeholder) | ollama-aligned |
| [`setup`](#setup) | First-run config / manual backend install | aligned + parameter extension |
| [`runtime`](#runtime-family) | llama.cpp backend multi-version management | roxid extension |
| [`completion`](#completion-family) | Shell completion management | roxid extension |
| `__complete` | hidden completion-candidate command | roxid extension |

---

## serve

Start the API server (runs in the foreground; performs a one-time legacy layout migration first, then downloads the llama.cpp backend on demand).

```text
roxid serve [--addr <addr>]
```

| Argument | Type | Default | Description |
|---|---|---|---|
| `--addr` | string | `127.0.0.1:11434` | Listen address (`IP:PORT`) |

```sh
roxid serve
roxid serve --addr 0.0.0.0:11434
```

## create

Create a model from a Modelfile, or launch the interactive creation wizard.

```text
roxid create <model> [-f <Modelfile>] [-i]
```

| Argument | Type | Required | Default | Description |
|---|---|---|---|---|
| `model` | string | yes | — | New model name |
| `-f, --file` | string | one of `-f`/`-i` | — | Modelfile path |
| `-i, --interactive` | flag | one of `-f`/`-i` | — | Wizard: FROM → SYSTEM (multi-line, ends with `.`) → RUNTIME → summary confirmation |

Missing both reports an error and exits (matching the official requirement of a Modelfile). Results stream as NDJSON events; failures surface as in-stream `error` events (exit code 1).

```sh
roxid create my-model -f Modelfile
roxid create my-model -i
```

Supported Modelfile instructions: `FROM` / `SYSTEM` / `TEMPLATE` / `PARAMETER` / `MESSAGE` / `ADAPTER` / `RUNTIME`.

## show

Show model information (four sections: Model key-values (architecture / parameters / quantization / system) / Parameters entries / Capabilities / RUNTIME lines; empty sections are omitted; the parameter count is folded into the Model section keys instead of colliding with the Parameters entries title, aligned with upstream).

```text
roxid show <model>
```

```sh
roxid show llama3.2:3b
```

## run

Run a model: with a prompt argument it is a one-shot generation; without, it opens a multi-turn REPL.

```text
roxid run <model> [prompt...] [--hf] [--runtime <flags>]
```

| Argument | Type | Required | Default | Description |
|---|---|---|---|---|
| `model` | string | yes | — | Model name (three forms: `name:tag` / `hf.co/{user}/{repo}:{quant}` / `{user}/{repo}:{quant}` with `--hf`) |
| `prompt...` | string[] | no | — | First prompt (omitting opens the REPL) |
| `--hf` (or `-hf` before the model position) | flag | no | — | Resolve the model name as a HuggingFace direct reference |
| `--runtime` | string | no | — | llama.cpp launch flags for this run (temporary override, not persisted; a change rebuilds the instance) |

Built-in REPL commands: `/bye` (or `/exit` / `/quit`) to exit, `/clear` to reset the conversation; thinking increments render dimmed and are not fed back into context. If the model is missing, a 404 triggers one automatic pull and the request is retried.

REPL presentation: the banner prints once in color on session start; the prompt is a cyan `{model}> `; each reply is followed by a blank separator line; the `--verbose` timing line reads `(input N tok / output N tok / total N.NN s / output N.N tok/s)` in the Chinese locale build (missing fields are omitted).

```sh
roxid run llama3.2:3b
roxid run llama3.2:3b "explain quantum computing in one sentence"
roxid run -hf Qwen/Qwen2.5-0.5B:IQ2_S
roxid run my-model --runtime "--threads 3 -ngl 30"
```

## stop

Stop (unload) a running model instance. Prints `已停止 {model}` on success (green).

```text
roxid stop <model>
```

Tab completion offers **running models only** (live query via `/api/ps`, 1s timeout; zero candidates when serve is down) — unlike `run` / `show` which complete all local models.

## pull

Pull a model from a registry. Supports both the Ollama registry and direct HuggingFace references; resumable downloads, NDJSON progress, and digest-level skipping of layers already present locally.

Progress presentation:

- **TTY**: single-line spinner overlay; messages are CJK-aware truncated to the terminal width (no more wrapped-line residue on narrow terminals); phase distinction — the verifying / writing-manifest / retrying phases switch the spinner to yellow, a completed layer shows a green ✓, and status texts render in Chinese (protocol fields keep the official English originals);
- **non-TTY** (pipe / redirect / CI): one line per phase, aligned with upstream — `拉取清单` / a per-layer summary line (`拉取 {digest}: 100% 2.00 GB（平均 5.3 MB/s）`) / `校验 sha256 摘要` / `写入清单`; automatic retries (up to 3) print `下载中断，正在重试（第 N/3 次）`.

```text
roxid pull <model> [--hf]
```

| Argument | Type | Required | Description |
|---|---|---|---|
| `model` | string | yes | Model name (HF reference form with `--hf`, equivalent to the `hf.co/` prefix form) |
| `--hf` (or `-hf`) | flag | no | Resolve as a HuggingFace direct reference |

```sh
roxid pull llama3.2:3b
roxid pull -hf Qwen/Qwen2.5-0.5B:Q4_K_M
```

## push

Push a model to a registry. **Currently a deferred boundary**: the push pipeline is not implemented; calls return a server error. `signin` credentials are stored and reserved for future enablement.

```text
roxid push <model>
```

## signin / signout

Sign in to / out of ollama.com. `signin` interactively asks for a username and access token (token input is hidden), saving credentials to `~/.roxid/auth.json` (permission 0600). `signout` removes the file.

```text
roxid signin
roxid signout
```

## list (ls)

List local models in a table: columns `NAME` / `SIZE` / `MODIFIED`. `MODIFIED` is rendered in a human-readable hybrid form — absolute time plus a Chinese relative phrase, e.g. `2026-06-04 03:04 (3 个月前)` (tiers: 秒 / 分钟 / 小时 / 天 / 周 / 个月 / 年 前; ≤0 delta shows `刚刚`; unparseable timestamps are printed verbatim).

Column alignment & width adaptation: `NAME` pads to the longest name of the batch (dynamic column width, the same aligned form as the official `ollama` table); names wider than the terminal budget truncate with a trailing `…` (CJK-aware); `SIZE` adapts its unit (KB/MB/GB, two decimals, right-aligned); on narrow terminals (<60 columns) `MODIFIED` keeps only the relative phrase so all three columns converge within the terminal width (no overflow at 40 columns).

```text
roxid list    # alias: roxid ls
```

## ps

List running models in six columns:

| Column | Meaning |
|---|---|
| `NAME` | Model name (padded to the longest running name for alignment; truncated on narrow terminals; all six columns kept) |
| `ID` | First 12 chars of content digest |
| `SIZE` | Model size in bytes (adaptive unit) |
| `PROCESSOR` | GPU/CPU layer split (e.g. `20%/80% CPU/GPU`, parsed from backend load log; failure reason shown verbatim) |
| `CONTEXT` | Context window total (e.g. `4096 token`; no usage ratio) |
| `UNTIL` | Time left before keep_alive unload (Chinese phrase, e.g. `5 分钟后`; `即将卸载` when expired) |

```text
roxid ps
```

## cp

Copy a model (the destination is a derived model; large files are hard-linked to the base model, costing no extra disk space). Prints `已复制 {source} → {destination}` on success (green; diverges from the silent upstream ollama behavior).

```text
roxid cp <source> <destination>
```

```sh
roxid cp llama3.2:3b llama3.2:3b-copy
```

## rm

Remove a local model. Prints `已删除 {model}` on success (green).

```text
roxid rm <model>
```

## launch

Menu integration entry point. **Currently a placeholder**: prints a hint and exits (code 0); the menu arrives in a future version.

```text
roxid launch
```

## setup

First-run configuration wizard (re-openable anytime), or non-interactive manual backend installation via parameter mode.

```text
roxid setup [--llama-url <url>]
```

| Argument | Type | Default | Description |
|---|---|---|---|
| `--llama-url` | string | — | Manually specified llama.cpp package download link (tar.gz or a bare llama-server binary), installed immediately; recorded in config.toml `[runtime].llama_url` on success. Links starting with `https://github.com/` are automatically downloaded through the configured GitHub proxy prefix; other links are used verbatim |

Wizard flow: mainland-China network detection (timezone / locale) → proxy configuration (recommended prefills, editable/clearable) → optional custom llama.cpp link (skipped by default) → shell completion install (default Y). Networked commands (serve / pull / create / runtime install) trigger the wizard automatically on first run (suppressed once config.toml exists).

## runtime family

llama.cpp backend multi-version management (local operations, no serve required). Resolution order: `ROXID_LLAMA_SERVER` env → manual → default version (`default_version`) → locked-tag fallback.

```text
roxid runtime list
roxid runtime install <tag> | --url <url>
roxid runtime use <tag | manual>
roxid runtime rm <tag>
```

| Subcommand | Arguments | Description |
|---|---|---|
| `list` | — | List installed versions (default marked `[默认]`) plus manual; includes the resolution order |
| `install` | `<tag>` (form `b\d+`, e.g. `b10700`) or `--url <url>` (mutually exclusive) | Download by official tag (variant auto-detected per host CPU arch `x64`/`arm64` + GPU → vulkan / otherwise cpu); `--url` installs as the manual version |
| `use` | `<tag>` or `manual` | Set and persist the default version; the first request after switching tears down the old instance and starts the new version |
| `rm` | `<tag>` | Remove an installed version; the current default must be switched away first |

```sh
roxid runtime list
roxid runtime install b10700
roxid runtime install --url https://example.com/llama-server.tar.gz
roxid runtime use manual
```

## completion family

Shell completion management (bash / zsh / fish; static candidates plus dynamic value candidates — model names and runtime tags are read directly from local `~/.roxid`, zero network, zero latency).

```text
roxid completion bash | zsh | fish    # print the shim script (for eval / piping)
roxid completion install              # detect $SHELL and install to standard dirs
```

| Subcommand | Placement | Notes |
|---|---|---|
| `bash` | `~/.local/share/bash-completion/completions/roxid` | auto-loaded in new terminals |
| `zsh` | `~/.zfunc/_roxid` + `.zshrc` activation block (idempotent, marked) | adds fpath/compinit if missing |
| `fish` | `~/.config/fish/completions/roxid.fish` | — |

`install` overwrites idempotently; reopen the terminal to take effect. The setup wizard runs it by default at the end.

## __complete (hidden)

The candidate-output command invoked internally by shell shims (`hide = true`, absent from help). Hand-written prefix analysis yields subcommand / option / dynamic-value candidates; no need to call it manually.

---

## Environment variables

| Variable | Purpose |
|---|---|
| `ROXID_HOST` | Server address the CLI connects to (default `http://127.0.0.1:11434`) |
| `OLLAMA_HOST` | Compatible fallback (`ROXID_HOST` wins) for seamless migration of existing scripts |

Server-side variables (proxies / backend / parallelism) are covered in [Configuration](configuration.md).

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Failure: bad arguments / server error responses / file I/O errors / wizard cancellation, etc. |

When the server is unreachable (connection failure), a unified hint is printed and the process exits 1:

```text
错误： 无法连接 roxid 服务（http://127.0.0.1:11434）：...
请先运行：roxid serve
```

Unified error output: server error bodies `{"error":"..."}` are parsed by the CLI and shown as plain text (the raw JSON envelope no longer leaks), with a uniform red `错误：` prefix on stderr.

> If `ROXID_HOST` / `OLLAMA_HOST` points at an official Ollama instance, the CLI detects it and warns on stderr that the connected instance is not roxid; commands still operate on that instance (stdout stays ollama-aligned, keeping pipe parsing clean).
