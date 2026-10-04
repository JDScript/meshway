# 运行日志

Server、`meshwayd`、`meshway` 和 `reset-password` 使用共同的 `meshway-logging` 配置，组件通过 `tracing` 发送结构化事件。程序入口安装一次 `tracing-subscriber`；嵌入式宿主负责安装自己的 subscriber。

## 配置与输出

| 参数／环境变量 | 默认值 | 行为 |
| --- | --- | --- |
| `--log-filter`／`RUST_LOG` | `info` | tracing 级别及 `target=level` 过滤；显式参数优先，非法过滤器使启动失败 |
| `--log-format`／`MESHWAY_LOG_FORMAT` | `text` | `text` 或每行一个对象的 `json` |
| `NO_COLOR` | 未设置 | 设置后关闭文本颜色；非终端 stderr 和 JSON 始终无颜色 |

日志统一写入 stderr。CLI `--json` 只控制 stdout 的公开状态结果，与 `--log-format json` 独立。Server／daemon 的现有 stdout 就绪行继续供进程管理器读取；重定向 stderr 可单独保存日志。格式化与写出使用 subscriber 的同步 stderr writer，无后台日志队列。文件持久化和轮转由进程管理器或外部收集器负责。

```sh
RUST_LOG=info,meshway_daemon=debug ./target/debug/meshwayd \
  --data-dir "$PWD/.meshway/node" --server http://127.0.0.1:8430 \
  --log-format json 2>node.log

./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" \
  status --json --log-format json --log-filter debug 2>cli.log
```

JSON 事件包含 `timestamp`、`level`、`target`、`fields`；`fields.message` 是事件说明。span 上下文由 subscriber 的 `span`／`spans` 字段表达。

## 事件与字段

| 来源 | 常规日志 | debug 日志 |
| --- | --- | --- |
| daemon | 启动／就绪／退出、已提交的意图及 revision、状态转换、失败类别和重试等待 | 控制轮次是否续租／上报 |
| CLI | 命令失败、停止等待 | 命令名称 |
| Server | 监听就绪、关闭、HTTP 5xx 故障 | HTTP 响应状态和响应头就绪耗时 |

Server HTTP 使用 `tower-http` 的 `TraceLayer`，span 只包含 method、声明的 route 模板及 Server 生成的 request ID。原始 URI、查询参数、headers、请求／响应 body 均不进入该层日志。SSE 的响应耗时表示开始响应，不表示整个订阅持续时间。

日志调用显式选取字段，不格式化完整设置、协议请求、控制快照或配置。Node 私钥、兑换码、会话 token、密码及配置原文保留在各自私有边界；批准链接或密码重置链接仅由相应命令的明确结果通道交付。日志不作为 saved／applied、执行授权或测量历史的权威来源。
