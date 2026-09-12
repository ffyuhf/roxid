# API 参考

[English](../api.md) | 中文

---

## 总览

| 项 | 说明 |
|---|---|
| 服务地址 | 默认 `http://127.0.0.1:11434`（`roxid serve` 启动） |
| 鉴权 | **无鉴权**——本地推理网关，无需 Authorization 头 |
| 请求头 | **不校验 `Content-Type`**——裸 `curl -d` 直接可用（对齐官方生态）；body 一律 JSON |
| CORS | 默认放行 localhost 系来源（含 `app://*`、`tauri://*` 桌面壳协议）；`ROXID_ORIGINS` / `OLLAMA_ORIGINS` 覆盖（逗号分隔，支持 `*` 通配） |
| 流式语义 | `/api/*` 为 NDJSON（每行一个 JSON 事件，`done:true` 终止包含 usage 统计）；`/v1/*` 为 SSE（`data: [DONE]` 终止） |

三套 API 面：

| 面 | 前缀 | 端点数 | 说明 |
|---|---|---|---|
| Ollama 原生 | `/api` | 15 | 与官方 Ollama API 对齐，ollama 客户端可直接使用 |
| OpenAI 兼容 | `/v1` | 6 | 透传 llama-server，OpenAI SDK 指向即可 |
| llama.cpp 原生 | 根级 | 8 | 端点直通（POST 类 body 携带 `model` 路由键；GET 类 `?model=` 指定实例） |

---

## Ollama API（/api/*）

### GET /api/version

返回 roxid 自身版本号。

```sh
curl http://127.0.0.1:11434/api/version
```

```json
{"version": "0.1.0"}
```

> 注：返回 roxid 版本（非模型版本），与 CLI `-v` 同源。

### GET /api/tags

列出本地模型。

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

列出运行中的模型实例（含 `name` / `model` / `size` 等）。

### POST /api/show

查看模型详情。

请求：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `model` | string | 是 | 模型名 |
| `verbose` | bool | 否 | 附带 verbose 字段 |

响应字段：`modelfile`（复原的 Modelfile 文本，含 RUNTIME 行）、`parameters`（**Modelfile 指令文本形态**，非官方 map 形态——已知边界，见文末）、`template`、`system`、`details`、`model_info`（官方键名：`{arch}.context_length`、`general.parameter_count`、`general.file_type`）、`capabilities`（字符串数组：`completion` / `embedding` / `vision` / `tools` / `thinking` / `rerank`…）、`license`、`projector`。

```sh
curl http://127.0.0.1:11434/api/show -d '{"model": "llama3.2:3b"}'
```

### POST /api/generate

文本生成（NDJSON 流式，默认 `stream: true`）。

请求字段：

| 字段 | 类型 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `model` | string | 是 | — | 模型名 |
| `prompt` | string | 是 | — | 提示词 |
| `images` | string[] | 否 | — | base64 图像（vision 模型；`raw:true` 通道下丢弃——已知边界） |
| `format` | any | 否 | — | `"json"` / JSON Schema 对象 / `true` |
| `options` | object | 否 | — | 采样与上下文参数（`num_ctx`、`temperature`、`num_predict`、`stop` 等；含 roxid 扩展键 `runtime`） |
| `system` | string | 否 | — | 请求级 system（覆盖模型 SYSTEM） |
| `template` | string | 否 | — | **当前静默丢弃**（后端无 per-request 模板能力——已知边界） |
| `raw` | bool | 否 | — | `true` 时 prompt 原样直发（不套模板） |
| `stream` | bool | 否 | `true` | 流式开关 |
| `keep_alive` | any | 否 | `5m` | 空闲存活时长（数字秒 / `"5m"` / `"0"`） |
| `think` | any | 否 | — | `false` 抑制思考（双保险）；`"low"`/`"medium"`/`"high"`/`"max"` 档位 |
| `context` | u32[] | 否 | — | 上轮终包 context（续传协议） |
| `suffix` | string | 否 | — | FIM 后缀提示（存在时路由原生 `/infill`） |
| `logprobs` / `top_logprobs` | bool / u32 | 否 | — | token 对数概率 |

流式响应（NDJSON 每行）：

```json
{"model": "llama3.2:3b", "created_at": "...", "response": "天空", "done": false}
```

终止包（单终包契约，含 usage 与续传 context）：

```json
{"model": "llama3.2:3b", "created_at": "...", "response": "", "done": true,
 "context": [123, 456], "total_duration": 5123456789, "load_duration": 1098765432,
 "prompt_eval_count": 12, "prompt_eval_duration": 234567890,
 "eval_count": 88, "eval_duration": 3987654321}
```

```sh
curl http://127.0.0.1:11434/api/generate \
  -d '{"model": "llama3.2:3b", "prompt": "你好", "stream": false}'
```

> **超窗自动扩窗**（迭代43）：请求超过实例上下文窗口（默认 4096）时，roxid 不再透出 llama.cpp 的 400 `exceed_context_size_error`，而是按实际 token 数扩窗重建实例后重放（目标 = 实际 token 数 + 1024 余量，向上对齐 512 倍数，GGUF 训练长度封顶；重试 1 次，仍失败则透传原始错误）。显式传入 `options.num_ctx` 时仍按 D4c 语义以请求值为准。`/api/chat` 与 `/api/generate`（含 raw、`context` 续传、FIM 通道）一致生效。

### POST /api/chat

对话端点（多轮消息数组；NDJSON 流式，默认 `stream: true`）。

请求字段：

| 字段 | 类型 | 必填 | 默认 | 说明 |
|---|---|---|---|---|
| `model` | string | 是 | — | 模型名 |
| `messages` | array | 是 | — | 消息数组：`{role, content, images?, tool_calls?, tool_name?, thinking?}` |
| `format` / `options` / `tools` / `stream` / `keep_alive` / `think` / `logprobs` / `top_logprobs` | — | 否 | 同 generate | `tools` 结构与 OpenAI 一致 |

消息 role：`system` / `user` / `assistant` / `tool`；assistant 思考内容经 `thinking` 字段回传保持多轮上下文。

工具调用：后端以 `--jinja` 启动（使用各模型内嵌对话模板）。非流式响应与流式事件的 `tool_calls[].function.arguments` 恒为对象形态——流式分片由服务端跨片重组，JSON 闭合时以单事件一次性下发完整工具调用（name + 完整 arguments 对象，对齐官方 Ollama 流式形态——官方 docs/api.md 单 chunk 完整 tool_calls）；中间分片事件不携带 tool_calls。

流式中断：后端中途死亡时以可读的 `{"error":"上游中断：…"}` 行收尾（可得时附 `stderr_tail` 实例 stderr 尾部行）——传输层始终正常终止。

模型加载行为：提供相同服务的模型（文本生成 / 向量 / TTS，按 GGUF 架构动态归组）同类互斥——加载新模型时立即卸载同类空闲实例（在途请求绝不中断；异类实例不受影响）。生成类模型自动附带投机解码启动，并补齐使 llama-server 真正构建投机上下文的配套参数：内嵌 MTP 头的模型为 `--spec-type draft-mtp,ngram-mod --spec-draft-n-max 3`，其余为 `--spec-type ngram-mod`；两者均附带按架构分档的 `--spec-ngram-mod-n-match/-n-min/-n-max`（MoE 24/48/64 官方推荐值；dense 16/24/32）。每次拉起后 roxid 会探测 `/props` 并在日志中报告投机解码实际激活状态（`speculative: true/false`）；在 RUNTIME / 请求 `options.runtime` 中设置含 `--spec-type` 或 `-md` 的参数即可手动接管。

流式响应（NDJSON 每行）：

```json
{"model": "llama3.2:3b", "created_at": "...",
 "message": {"role": "assistant", "content": "天空", "thinking": "…（思考增量）"}, "done": false}
```

终止包含 `done_reason`（`stop` / `length` / `tools`）与 usage 统计字段（同 generate）。

```sh
curl http://127.0.0.1:11434/api/chat -d '{
  "model": "llama3.2:3b",
  "messages": [{"role": "user", "content": "为什么天空是蓝色的？"}]
}'
```

### POST /api/embed

向量化（新端点，批量输入）。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `model` | string | 是 | embedding 模型 |
| `input` | string 或 string[] | 是 | 单条或批量文本 |
| `truncate` | bool | 否 | 超长输入截断开关；`false` 且超长时显式 400 |
| `options` / `keep_alive` | — | 否 | 同上 |

响应：`{model, embeddings: [[…]], total_duration, load_duration, prompt_eval_count}`。

> 对生成模型请求向量化时返回：`此模型不支持向量化，请使用 embedding 模型`（上游英文 message 原样透传）。

### POST /api/embeddings

旧版向量化端点（单 `prompt` → `{embedding: […]}`）。保留用于兼容旧客户端。

### POST /api/create

创建模型（NDJSON 状态流；HTTP 恒 200，失败经流内 `error` 事件传递）。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `model` | string | 是 | 新模型名 |
| `from` | string | 与 modelfile 二选一 | 基础模型名（官方 0.5+ 结构化形态）或 Modelfile 全文（兼容形态） |
| `modelfile` | string | 与 from 二选一 | Modelfile 文本 |
| `system` / `parameters` / `messages` / `template` | — | 否 | 结构化字段（与 from 基础模型名组合时生效） |

```sh
curl http://127.0.0.1:11434/api/create -d '{"model": "m2", "from": "FROM llama3.2:3b\nSYSTEM \"\"\"你是助手\"\"\"\n"}'
```

### POST /api/pull

拉取模型（NDJSON 进度流；支持并发去重——同模型并发请求等待首请求终态转发）。

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `model` | string | 是 | 模型名（`hf.co/{user}/{repo}:{quant}` 形态走 HF 直引源） |
| `insecure` | bool | 否 | 显式请求时跳过 TLS 证书校验 |
| `mmproj` | string | 否 | roxid 扩展：CLI 交互选定的多模态投影器（mmproj）文件名——多变体且未传时拉取报错并引导在终端运行 `roxid pull` 选择；单一变体自动下载 |

进度事件：`{"status": "pulling manifest"}` → `{"status": "pulling xxx…", "digest": "...", "total": 6432345667, "completed": 123456789}` → `{"status": "success"}`；失败：`{"error": "..."}`。

> 主源重复拉取：本地同层摘要一致且文件在位时跳过下载（digest 级判定）。

### POST /api/push

推送模型。**挂账边界**：推送管线未实现，返回错误响应。

### POST /api/copy

复制模型（衍生模型硬链接基础模型）。

```json
{"source": "llama3.2:3b", "destination": "llama3.2:3b-copy"}
```

### DELETE /api/delete

删除模型（body 仍为 JSON：`{"model": "..."}`）。

### POST /api/stop

卸载运行中的模型实例（roxid 扩展端点；原版 CLI 经内部通道）。

```json
{"model": "llama3.2:3b"}
```

### GET/HEAD /api/blobs/{digest}

按摘要取本地 blob：GET 流式直通文件内容；HEAD 存在性探测（200 + Content-Length / 404 无 body）。

> 仅支持读取；`POST /api/blobs`（上传）为挂账边界。

---

## OpenAI 兼容 API（/v1/*）

全部透传 llama-server 实例（请求体需携带 `model` 字段作为路由键）。

| 端点 | 方法 | 说明 |
|---|---|---|
| `/v1/models` | GET | 本地仓库模型列表（OpenAI 形态；`created` 为模型创建时间 Unix 秒） |
| `/v1/chat/completions` | POST | Chat Completions（`stream` 可选；透传含 `reasoning_content` 等原生字段） |
| `/v1/completions` | POST | 文本补全 |
| `/v1/embeddings` | POST | 向量化 |
| `/v1/rerank` | POST | 重排序 |
| `/v1/responses` | POST | Responses API（SSE 事件流直通） |

```sh
curl http://127.0.0.1:11434/v1/chat/completions -d '{
  "model": "llama3.2:3b",
  "messages": [{"role": "user", "content": "Hello"}]
}'
```

OpenAI SDK 指向 `base_url="http://127.0.0.1:11434/v1"` 即可（api_key 任意非空值）。

---

## llama.cpp 原生端点直通（根级）

llama.cpp 生态客户端零改造可用。POST 类 body 携带 `model` 路由键；GET 类 `?model=` 指定实例（缺省时 400）。

| 端点 | 方法 | 说明 |
|---|---|---|
| `/tokenize` | POST | 文本 → token id 序列（`{tokens: [...]}`） |
| `/detokenize` | POST | token id 序列 → 文本 |
| `/infill` | POST | FIM 代码补全（前缀/后缀填充） |
| `/completion` | POST | 原生补全（非 OpenAI 格式） |
| `/embedding` | POST | 原生向量化（非 OpenAI 格式） |
| `/props` | GET | 实例配置快照（`total_slots` / `chat_template` / `modalities` 等） |
| `/slots` | GET | slot 状态数组（每 slot 参数与处理进度） |
| `/metrics` | GET | Prometheus 指标（文本格式直通） |

```sh
curl http://127.0.0.1:11434/props?model=llama3.2:3b
```

---

## 状态码清单

| 码 | 含义 | 常见触发 |
|---|---|---|
| `200` | 成功（流式端点含 NDJSON/SSE 流；`/api/create` 恒 200，失败看流内 error 事件） | — |
| `400` | 请求不合法 | body 非合法 JSON（`{"error": "请求体不是合法 JSON"}`）；`/embed` 输入超长且 `truncate:false`；llamacpp GET 类缺 `?model=` |
| `404` | 资源不存在 | 模型未找到（`model '...' not found`）；`/api/blobs/{digest}` 本地无此 blob |
| `500` | 服务端内部错误 | 本地文件系统错误、实例异常等 |
| `502` | 上游（llama-server）失败 | 上游非 JSON 响应体；实例停止/卸载后连接中断（`模型实例已停止或卸载，请求中断`）；生成模型请求 embedding 的上游错误整形 |

错误响应统一结构（单层人类可读文案）：

```json
{"error": "model 'foo' not found"}
```

完整错误语义见 [错误与状态码](errors.md)。

---

## 已知边界（挂账项如实标注）

| 项 | 状态 |
|---|---|
| `POST /api/push` | 未实现（挂账），返回错误 |
| `POST /api/blobs`（上传） | 未实现（挂账）；读取（GET/HEAD）可用 |
| `/api/generate` 的 `template` 字段 | 静默丢弃（后端无 per-request 模板能力） |
| `/api/generate` `raw:true` 通道的 `images` | 丢弃（纯文本通道边界） |
| `/api/show` 的 `parameters` 字段 | Modelfile 指令文本形态（官方为 map 形态；协议层 map 化待后续裁决） |
| `/api/version` | 返回 roxid 自身版本号（既有保留语义） |
