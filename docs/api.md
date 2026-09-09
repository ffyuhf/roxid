# API Reference

English | [中文](zh/api.md)

---

## Overview

| Item | Description |
|---|---|
| Server address | default `http://127.0.0.1:11434` (started by `roxid serve`) |
| Authentication | **None** — a local inference gateway; no Authorization header required |
| Request headers | **`Content-Type` is not checked** — bare `curl -d` works (matching the official ecosystem); bodies are always JSON |
| CORS | localhost-family origins allowed by default (including `app://*` and `tauri://*` desktop shells); override via `ROXID_ORIGINS` / `OLLAMA_ORIGINS` (comma-separated, `*` wildcards) |
| Streaming | `/api/*` uses NDJSON (one JSON event per line, a single terminal event with `done:true` carries usage); `/v1/*` uses SSE (terminated by `data: [DONE]`) |

Three API surfaces:

| Surface | Prefix | Endpoints | Notes |
|---|---|---|---|
| Native Ollama | `/api` | 15 | aligned with the official Ollama API; ollama clients work as-is |
| OpenAI-compatible | `/v1` | 6 | passed through to llama-server; point OpenAI SDKs here |
| Native llama.cpp | root level | 8 | direct passthrough (POST bodies carry a `model` routing key; GET takes `?model=`) |

---

## Ollama API (/api/*)

### GET /api/version

Returns the roxid version.

```sh
curl http://127.0.0.1:11434/api/version
```

```json
{"version": "0.1.0"}
```

> Note: this is the roxid version (not a model version), same source as CLI `-v`.

### GET /api/tags

List local models.

```json
{
  "models": [
    {
      "name": "llama3.2:3b",
      "model": "llama3.2:3b",
      "modified_at": "2026-09-10T02:00:00.000000000Z",
      "size": 6432345667,
      "digest": "a80c4f17acd5...",
      "details": {
        "parent_model": "",
        "format": "gguf",
        "family": "llama",
        "families": ["llama"],
        "parameter_size": "3.2B",
        "quantization_level": "Q4_K_M"
      }
    }
  ]
}
```

### GET /api/ps

List running model instances (includes `name` / `model` / `size`, etc.).

### POST /api/show

Show model details.

Request:

| Field | Type | Required | Description |
|---|---|---|---|
| `model` | string | yes | Model name |
| `verbose` | bool | no | Include verbose fields |

Response fields: `modelfile` (reconstructed Modelfile text, including RUNTIME lines), `parameters` (**Modelfile instruction text form**, not the official map form — known boundary, see the end), `template`, `system`, `details`, `model_info` (official key names: `{arch}.context_length`, `general.parameter_count`, `general.file_type`), `capabilities` (string array: `completion` / `embedding` / `vision` / `tools` / `thinking` / `rerank`...), `license`, `projector`.

```sh
curl http://127.0.0.1:11434/api/show -d '{"model": "llama3.2:3b"}'
```

### POST /api/generate

Text generation (NDJSON streaming; `stream: true` by default).

Request fields:

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `model` | string | yes | — | Model name |
| `prompt` | string | yes | — | Prompt |
| `images` | string[] | no | — | base64 images (vision models; dropped in the `raw:true` channel — known boundary) |
| `format` | any | no | — | `"json"` / JSON Schema object / `true` |
| `options` | object | no | — | sampling and context options (`num_ctx`, `temperature`, `num_predict`, `stop`, etc.; includes the roxid extension key `runtime`) |
| `system` | string | no | — | request-level system (overrides the model SYSTEM) |
| `template` | string | no | — | **currently silently dropped** (no per-request template support in the backend — known boundary) |
| `raw` | bool | no | — | with `true`, the prompt is sent verbatim (no template) |
| `stream` | bool | no | `true` | streaming switch |
| `keep_alive` | any | no | `5m` | idle lifetime (seconds as number / `"5m"` / `"0"`) |
| `think` | any | no | — | `false` suppresses thinking (double insurance); tiers `"low"`/`"medium"`/`"high"`/`"max"` |
| `context` | u32[] | no | — | context token ids from the previous terminal event (continuation protocol) |
| `suffix` | string | no | — | FIM suffix hint (routes to the native `/infill` when present) |
| `logprobs` / `top_logprobs` | bool / u32 | no | — | token log-probabilities |

Streaming response (each NDJSON line):

```json
{"model": "llama3.2:3b", "created_at": "...", "response": "The sky", "done": false}
```

Terminal event (single-terminal contract, carries usage and the continuation context):

```json
{"model": "llama3.2:3b", "created_at": "...", "response": "", "done": true,
 "context": [123, 456], "total_duration": 5123456789, "load_duration": 1098765432,
 "prompt_eval_count": 12, "prompt_eval_duration": 234567890,
 "eval_count": 88, "eval_duration": 3987654321}
```

```sh
curl http://127.0.0.1:11434/api/generate \
  -d '{"model": "llama3.2:3b", "prompt": "Hello", "stream": false}'
```

### POST /api/chat

Chat endpoint (message array; NDJSON streaming; `stream: true` by default).

Request fields:

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `model` | string | yes | — | Model name |
| `messages` | array | yes | — | messages: `{role, content, images?, tool_calls?, tool_name?, thinking?}` |
| `format` / `options` / `tools` / `stream` / `keep_alive` / `think` / `logprobs` / `top_logprobs` | — | no | as generate | `tools` matches the OpenAI structure |

Message roles: `system` / `user` / `assistant` / `tool`; assistant thinking is passed back via `thinking` to preserve multi-turn context.

Streaming response (each NDJSON line):

```json
{"model": "llama3.2:3b", "created_at": "...",
 "message": {"role": "assistant", "content": "The sky", "thinking": "..."}, "done": false}
```

The terminal event carries `done_reason` (`stop` / `length` / `tools`) and usage fields (same as generate).

```sh
curl http://127.0.0.1:11434/api/chat -d '{
  "model": "llama3.2:3b",
  "messages": [{"role": "user", "content": "Why is the sky blue?"}]
}'
```

### POST /api/embed

Embeddings (current endpoint, batch-capable input).

| Field | Type | Required | Description |
|---|---|---|---|
| `model` | string | yes | embedding model |
| `input` | string or string[] | yes | single or batch text |
| `truncate` | bool | no | truncation switch for overlong input; `false` plus overlong input yields an explicit 400 |
| `options` / `keep_alive` | — | no | as above |

Response: `{model, embeddings: [[...]], total_duration, load_duration, prompt_eval_count}`.

> Asking a generation model for embeddings returns the official-style message: `This server does not support embeddings. Please use an embedding model`.

### POST /api/embeddings

Legacy embeddings endpoint (single `prompt` → `{embedding: [...]}`), kept for old clients.

### POST /api/create

Create a model (NDJSON status stream; HTTP is always 200 — failures surface as in-stream `error` events).

| Field | Type | Required | Description |
|---|---|---|---|
| `model` | string | yes | New model name |
| `from` | string | one of from/modelfile | base model name (official 0.5+ structured form) or full Modelfile text (compat form) |
| `modelfile` | string | one of from/modelfile | Modelfile text |
| `system` / `parameters` / `messages` / `template` | — | no | structured fields (combined with a `from` base model name) |

```sh
curl http://127.0.0.1:11434/api/create -d '{"model": "m2", "from": "FROM llama3.2:3b\nSYSTEM \"\"\"You are an assistant\"\"\"\n"}'
```

### POST /api/pull

Pull a model (NDJSON progress stream; concurrent duplicate pulls are deduplicated — later requests wait for the first request's terminal state and get it forwarded).

| Field | Type | Required | Description |
|---|---|---|---|
| `model` | string | yes | model name (`hf.co/{user}/{repo}:{quant}` routes to the HF direct source) |
| `insecure` | bool | no | skip TLS certificate verification when explicitly requested |

Progress events: `{"status": "pulling manifest"}` → `{"status": "pulling xxx...", "digest": "...", "total": 6432345667, "completed": 123456789}` → `{"status": "success"}`; failure: `{"error": "..."}`.

> Repeated pulls from the primary source skip layers whose digests already match files present locally (digest-level check).

### POST /api/push

Push a model. **Deferred boundary**: the push pipeline is not implemented; an error response is returned.

### POST /api/copy

Copy a model (derived models hard-link the base model's files).

```json
{"source": "llama3.2:3b", "destination": "llama3.2:3b-copy"}
```

### DELETE /api/delete

Delete a model (body is still JSON: `{"model": "..."}`).

### POST /api/stop

Unload a running model instance (roxid extension; the original CLI uses an internal channel).

```json
{"model": "llama3.2:3b"}
```

### GET/HEAD /api/blobs/{digest}

Fetch a local blob by digest: GET streams the file; HEAD probes existence (200 + Content-Length / 404 with no body).

> Read-only; `POST /api/blobs` (upload) is a deferred boundary.

---

## OpenAI-compatible API (/v1/*)

All passed through to the llama-server instance (request bodies must carry a `model` routing key).

| Endpoint | Method | Description |
|---|---|---|
| `/v1/models` | GET | local repository model list (OpenAI shape) |
| `/v1/chat/completions` | POST | Chat Completions (`stream` optional; native fields such as `reasoning_content` pass through) |
| `/v1/completions` | POST | text completions |
| `/v1/embeddings` | POST | embeddings |
| `/v1/rerank` | POST | reranking |
| `/v1/responses` | POST | Responses API (SSE event stream passthrough) |

```sh
curl http://127.0.0.1:11434/v1/chat/completions -d '{
  "model": "llama3.2:3b",
  "messages": [{"role": "user", "content": "Hello"}]
}'
```

Point OpenAI SDKs at `base_url="http://127.0.0.1:11434/v1"` (any non-empty api_key).

---

## Native llama.cpp passthrough (root level)

llama.cpp-ecosystem clients work unmodified. POST bodies carry a `model` routing key; GET takes `?model=` (missing yields 400).

| Endpoint | Method | Description |
|---|---|---|
| `/tokenize` | POST | text → token id sequence (`{tokens: [...]}`) |
| `/detokenize` | POST | token ids → text |
| `/infill` | POST | FIM code completion (prefix/suffix fill) |
| `/completion` | POST | native completion (non-OpenAI format) |
| `/embedding` | POST | native embedding (non-OpenAI format) |
| `/props` | GET | instance config snapshot (`total_slots` / `chat_template` / `modalities`, etc.) |
| `/slots` | GET | slot status array (per-slot params and progress) |
| `/metrics` | GET | Prometheus metrics (text passthrough) |

```sh
curl http://127.0.0.1:11434/props?model=llama3.2:3b
```

---

## Status code list

| Code | Meaning | Common triggers |
|---|---|---|
| `200` | Success (streaming endpoints carry NDJSON/SSE; `/api/create` is always 200 — check in-stream error events) | — |
| `400` | Invalid request | body is not valid JSON (`{"error": "请求体不是合法 JSON"}`); `/embed` overlong input with `truncate:false`; llamacpp GET missing `?model=` |
| `404` | Resource not found | model not found (`model '...' not found`); `/api/blobs/{digest}` not present locally |
| `500` | Internal server error | local filesystem errors, instance failures, etc. |
| `502` | Upstream (llama-server) failure | non-JSON upstream body; connection interrupted after the instance stopped/unloaded (`模型实例已停止或卸载，请求中断`); embedding-request errors from generation models reshaped to the official message |

Error responses use a unified single-layer, human-readable structure:

```json
{"error": "model 'foo' not found"}
```

Full error semantics: [Errors & Status Codes](errors.md).

---

## Known boundaries (deferred items, honestly annotated)

| Item | Status |
|---|---|
| `POST /api/push` | not implemented (deferred); returns an error |
| `POST /api/blobs` (upload) | not implemented (deferred); read (GET/HEAD) works |
| `template` field of `/api/generate` | silently dropped (no per-request template support in the backend) |
| `images` in the `raw:true` channel of `/api/generate` | dropped (plain-text channel boundary) |
| `parameters` field of `/api/show` | Modelfile instruction text form (official uses a map form; protocol-level mapping awaits a future decision) |
| `/api/version` | returns the roxid version (established semantics) |
