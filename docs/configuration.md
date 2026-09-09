# Configuration

English | [中文](zh/configuration.md)

---

## Configuration file

| Item | Value |
|---|---|
| Path | `{ROXID_HOME}/config.toml`, default `~/.roxid/config.toml` |
| Format | TOML |
| Lookup order | `ROXID_HOME` environment variable first → default `~/.roxid` |
| Existence semantics | a missing file means the first-run wizard has not completed (networked commands `serve` / `pull` / `create` / `runtime install` trigger it automatically on first run) |
| Validation & fault tolerance | a corrupted file (invalid TOML) **does not block startup** — it falls back to all defaults (un-setup, no proxy) and downloads go direct; saves are full overwrites of all fields |

## All fields

```toml
# ~/.roxid/config.toml — generated and maintained by roxid (setup wizard / runtime use, etc.)

setup_done = true            # wizard-completed marker; networked commands stop auto-triggering the wizard

[proxy]                      # download proxies (fallback when env vars are unset)
gh = "https://gh.jasonzeng.dev/"   # GitHub proxy prefix (prepended to GitHub URLs); optional
hf = "https://hf-mirror.com/"      # HuggingFace mirror base (replaces the official domain); optional

[runtime]                    # llama.cpp backend
llama_url = "https://example.com/llama.tar.gz"  # manual backend source link (recorded by setup --llama-url); optional
default_version = "b10700"   # default backend version (written by runtime use; "manual" selects the manual copy); optional
```

| Field | Type | Default | Constraints | Written by |
|---|---|---|---|---|
| `setup_done` | bool | `false` | — | setup wizard (always set true) |
| `[proxy].gh` | string? | unset | URL prefix, same semantics as `ROXID_GH_PROXY` | setup wizard |
| `[proxy].hf` | string? | unset | mirror base, same semantics as `ROXID_HF_PROXY` | setup wizard |
| `[runtime].llama_url` | string? | unset | tar.gz or bare-binary link | `setup --llama-url` |
| `[runtime].default_version` | string? | unset | `b\d+` tag or `"manual"` | `runtime use` |

## Environment variables

### Server side

| Variable | Precedence | Description |
|---|---|---|
| `ROXID_GH_PROXY` | env > `[proxy].gh` > direct | GitHub proxy prefix (backend downloads and other GitHub assets) |
| `ROXID_HF_PROXY` | env > `HF_ENDPOINT` > `[proxy].hf` > official | HuggingFace mirror base (replaces the official domain) |
| `ROXID_LLAMA_SERVER` | **top of the backend chain**: env → manual → `default_version` → locked tag `b10605` | direct path to a llama-server binary (escape hatch for self-built CUDA versions; cannot be overridden by config) |
| `ROXID_HOME` | env > `~/.roxid` | roxid data root (config.toml / models / llama.cpp / auth.json all follow) |
| `ROXID_ORIGINS` | env > `OLLAMA_ORIGINS` > default localhost set | allowed CORS origins (comma-separated, `*` wildcards) |
| `OLLAMA_ORIGINS` | fallback slot above | Ollama-compatible alias |
| `ROXID_NUM_PARALLEL` | model metadata `num_parallel` > env > `OLLAMA_NUM_PARALLEL` > `4` | parallel requests per slot |
| `OLLAMA_NUM_PARALLEL` | fallback slot above | Ollama-compatible alias |
| `RUST_LOG` | — | log level (tracing EnvFilter; default `info`) |

### CLI side

| Variable | Precedence | Description |
|---|---|---|
| `ROXID_HOST` | env > `OLLAMA_HOST` > `http://127.0.0.1:11434` | server address the CLI connects to |

### Installer side (read only during installation; see the [installation guide](installation.md))

`ROXID_INSTALL_SCOPE` / `ROXID_INSTALL_SOURCE` / `ROXID_LOCAL_BIN` / `ROXID_DOWNLOAD_URL` / `ROXID_VERSION`

## Overriding: config file vs environment vs CLI arguments

| Domain | Precedence (high → low) |
|---|---|
| Backend version | `ROXID_LLAMA_SERVER` (env) → manual (artifact installed via `runtime.llama_url`) → `default_version` (config) → locked-tag auto-download |
| GitHub proxy | `ROXID_GH_PROXY` (env) → `[proxy].gh` (config) → direct |
| HF mirror | `ROXID_HF_PROXY` (env) → `HF_ENDPOINT` (env) → `[proxy].hf` (config) → official |
| Server address (CLI) | `ROXID_HOST` (env) → `OLLAMA_HOST` (env) → default `127.0.0.1:11434` (`serve --addr` affects only the server's listen address; the two are independent) |
| Sampling options | request `options` (per key) → Modelfile `PARAMETER` (default layer) → backend defaults |
| Runtime launch flags | request `options.runtime` (temporary, not persisted) → Modelfile `RUNTIME` (persistent) → none (`--fit` auto GPU dispatch) |

## Multiple instances / named configurations

roxid has no built-in profile switching; **multi-instance isolation is achieved via `ROXID_HOME`** — a different `ROXID_HOME` is a fully independent data directory (config.toml / models / backend cache / credentials):

```sh
ROXID_HOME=~/.roxid-dev roxid serve --addr 127.0.0.1:11435   # independent dev instance
```

## Data directory layout

```text
~/.roxid/                          # default ROXID_HOME
├─ config.toml                     # the configuration described here
├─ auth.json                       # signin credentials (0600)
├─ models/                         # model repository (three source-separated dirs)
│  ├─ ollama/                      #   pulled from the Ollama registry
│  ├─ HF/                          #   HF direct pulls ({user}/{repo}/{quant}/)
│  └─ derived/                     #   create/copy derivatives (hard-linked base models)
└─ llama.cpp/                      # backend runtime cache
   ├─ {tag}/{variant}/             #   multi-version coexistence (vulkan / cpu variants)
   └─ manual/llama-server          #   manual copy (setup --llama-url / runtime install --url)
```

On first start, serve performs a one-time legacy-layout migration (idempotent, fail-fast).

## Next steps

- [Errors & Status Codes](errors.md)
- [CLI Reference](cli.md)
