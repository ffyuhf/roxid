# 错误与状态码

[English](../errors.md) | 中文

---

## 错误响应结构

非流式端点错误统一为 **HTTP 状态码 + 单层 JSON 文案**：

```json
{"error": "model 'foo' not found"}
```

流式端点（`/api/pull`、`/api/create`）HTTP 恒 200，失败经**流内 `error` 事件**传递：

```json
{"error": "pull request failed: ..."}
```

错误文案为单层人类可读文本（上游嵌套错误已解包整形，内部 URL / 动态端口已剥离，不暴露拓扑细节）。

## 状态码与错误总表

| 状态码 | 错误文案（示例） | 常见触发原因 | 类别 |
|---|---|---|---|
| `400` | `请求体不是合法 JSON` | body 非法 JSON（解析细节入服务端日志，客户端仅收通用文案） | 客户端 |
| `400` | `/embed` 超长输入相关 | `truncate: false` 且输入超出模型上下文 | 客户端 |
| `400` | llamacpp GET 类缺 `?model=` | `/props`、`/slots`、`/metrics` 未指定模型 | 客户端 |
| `404` | `model 'foo' not found` | 模型未安装 / 名称错误（run 场景 CLI 会自动拉取一次） | 客户端 |
| `404` | — | `/api/blobs/{digest}` 本地无此摘要 | 客户端 |
| `500` | `serve error: ...` | 服务进程自身启动/运行失败（如端口被占用） | 服务端 |
| `500` | `llama-server process error: ...` | 推理子进程异常（拉起失败、异常退出） | 服务端 |
| `500` | `io error: ...` | 本地文件系统操作失败（磁盘、权限） | 服务端 |
| `500` | `registry request failed: ...` | 上游 registry（ollama / HF）请求失败 | 服务端/网络 |
| `500` | `{"error":{...}}`（嵌套） | 上游 llama-server 错误体原文透传 | 服务端 |
| `502` | `模型实例已停止或卸载，请求中断` | `rm` / `stop` 卸载实例后在途请求连接中断 | 服务端 |
| `502` | `This server does not support embeddings. Please use an embedding model` | 对生成模型请求向量化（Pooling none 整形为官方口径） | 客户端 |
| `502` | 上游错误整形文案 | llama-server 返回非 JSON 响应体等上游失败 | 服务端 |

## 客户端错误 vs 服务端错误

| 类别 | 码段 | 处置建议 |
|---|---|---|
| 客户端错误 | `4xx` | 修正请求后重试：检查模型名是否已 `roxid list` 在列、JSON 是否合法、参数取值 |
| 服务端错误 | `5xx` | 查看服务端日志定位：实例状态（`roxid ps`）、后端版本（`roxid runtime list`）、磁盘与端口 |

## 排查入口命令

```sh
curl http://127.0.0.1:11434/api/version   # 服务可达性与版本
roxid ps                                  # 运行中的模型实例
roxid list                                # 本地已安装模型
roxid runtime list                        # 后端版本与解析优先级
pgrep -x roxid                            # 进程存活
```

CLI 侧常见报错与提示：

| 现象 | 含义与处置 |
|---|---|
| `无法连接 roxid 服务（…）：… 请先运行：roxid serve` | serve 未启动或 `ROXID_HOST` 指错地址 |
| `⚠ 注意：当前连接 … 不是 roxid 实例（可能是官方 Ollama）` | `ROXID_HOST` / `OLLAMA_HOST` 指向了官方 Ollama；命令仍作用于该实例，stdout 保持官方对齐 |
| `拉取中断` | pull 进度流耗尽未见 success 事件（网络中断）；重跑 `roxid pull` 断点续传 |
| `交互创建需要终端` | `create -i` 在非 TTY 环境运行；改用 `-f <Modelfile>` |
| `版本 … 是当前默认版本，删除前请先 … 切换` | `runtime rm` 删除默认版本前需先 `runtime use` 其他版本 |

## 日志获取方式

- **服务端日志**：`roxid serve` 前台运行，日志直接输出到终端 stdout/stderr；systemd 托管时经 `journalctl -u roxid`（用户实例加 `--user`）查看；
- **日志级别**：环境变量 `RUST_LOG` 控制（缺省 `info`；排查设 `RUST_LOG=debug` 可见拉取链与宽松 JSON 解析细节——解析行列号仅入服务端日志，不外发客户端）；
- **CLI 日志**：同为 `RUST_LOG`（EnvFilter），一般无需调整。

## 已知边界相关错误

| 项 | 行为 |
|---|---|
| `POST /api/push` | 挂账未实现——返回错误响应 |
| `POST /api/blobs`（上传） | 挂账未实现 |
| HF 直引缺量化名（如 `hf.co/user/repo` 无 `:quant`） | 服务端返回明确指引文案（含已装量化列表） |
| 无匹配 HF 量化 | 人类可读的「无匹配」文案（含可用量化建议） |

## 下一步

- [API 参考](api.md) —— 端点与字段全量
- [配置](configuration.md) —— 环境变量与优先级链
