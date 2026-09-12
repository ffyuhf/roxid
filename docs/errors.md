# Errors & Status Codes

English | [中文](zh/errors.md)

---

## Error response structure

Non-streaming endpoint errors use **HTTP status codes + single-layer JSON text**:

```json
{"error": "model 'foo' not found"}
```

Streaming endpoints (`/api/pull`, `/api/create`) always return HTTP 200; failures arrive as **in-stream `error` events**:

```json
{"error": "pull request failed: ..."}
```

Error text is single-layer and human-readable (nested upstream errors are unwrapped and reshaped; internal URLs and dynamic ports are stripped — no topology details leak).

## Status code and error table

| Status | Error text (examples) | Common triggers | Class |
|---|---|---|---|
| `400` | `请求体不是合法 JSON` (body is not valid JSON) | malformed JSON body (parsing details go to the server log; clients receive the generic message) | client |
| `400` | `/embed` overlong-input errors | `truncate: false` with input exceeding the model context | client |
| `400` | llamacpp GET missing `?model=` | `/props`, `/slots`, `/metrics` without a model | client |
| `404` | `model 'foo' not found` | model not installed / wrong name (the run command auto-pulls once) | client |
| `404` | — | `/api/blobs/{digest}` not present locally | client |
| `500` | `serve error: ...` | server process startup/runtime failure (e.g. port already bound) | server |
| `500` | `llama-server process error: ...` | inference subprocess failure (spawn failure, abnormal exit) | server |
| `500` | `io error: ...` | local filesystem failure (disk, permissions) | server |
| `500` | `registry request failed: ...` | upstream registry (ollama / HF) request failure | server/network |
| `500` | `{"error":{...}}` (nested) | upstream llama-server error body passed through as-is | server |
| `502` | `模型实例已停止或卸载，请求中断` (model instance stopped or unloaded; request interrupted) | in-flight requests after `rm` / `stop` unloaded the instance | server |
| `502` | `This server does not support embeddings. Please use an embedding model` | embedding request against a generation model (Pooling none reshaped to the official message) | client |
| `502` | reshaped upstream error text | upstream failures such as non-JSON llama-server bodies | server |

## Client errors vs server errors

| Class | Range | Suggested handling |
|---|---|---|
| Client errors | `4xx` | Fix the request and retry: check the model name with `roxid list`, validate the JSON, check argument values |
| Server errors | `5xx` | Inspect server logs: instance state (`roxid ps`), backend version (`roxid runtime list`), disk and ports |

## Troubleshooting entry points

```sh
curl http://127.0.0.1:11434/api/version   # server reachability and version
roxid ps                                  # running model instances
roxid list                                # installed local models
roxid runtime list                        # backend versions and resolution order
pgrep -x roxid                            # process liveness
```

Common CLI messages:

| Message | Meaning and handling |
|---|---|
| `无法连接 roxid 服务（…）：… 请先运行：roxid serve` | serve is not running, or `ROXID_HOST` points to the wrong address |
| `⚠ 注意：当前连接 … 不是 roxid 实例（可能是官方 Ollama）` | `ROXID_HOST` / `OLLAMA_HOST` points at the official Ollama; commands still operate on that instance, stdout stays ollama-aligned |
| `拉取中断` (pull interrupted) | the pull progress stream ended without a success event (network drop); re-run `roxid pull` to resume |
| `交互创建需要终端` (interactive creation needs a terminal) | `create -i` in a non-TTY environment; use `-f <Modelfile>` instead |
| `版本 … 是当前默认版本，删除前请先 … 切换` | switch away the default before `runtime rm` can delete it |

## Getting logs

- **Server logs**: `roxid serve` runs in the foreground and logs to stdout/stderr; under systemd, use `journalctl -u roxid` (add `--user` for user instances);
- **Log level**: controlled by `RUST_LOG` (default `info`; set `RUST_LOG=debug` to see the pull chain and lenient-JSON parsing details — parse line/column info goes to the server log only, never to clients);
- **CLI logs**: also honor `RUST_LOG` (EnvFilter); usually no adjustment needed.

## Errors related to known boundaries

| Item | Behavior |
|---|---|
| `POST /api/push` | deferred/unimplemented — returns an error response |
| `POST /api/blobs` (upload) | deferred/unimplemented |
| HF direct reference without a quant suffix (e.g. `hf.co/user/repo` without `:quant`) | the server returns a clear guidance message (including installed quant list) |
| No matching HF quant | a human-readable "no match" message (with suggestions of available quants) |

## Next steps

- [API Reference](api.md) — endpoints and fields in full
- [Configuration](configuration.md) — environment variables and precedence chains
