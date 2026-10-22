# 运行日志

Server、`meshwayd`、`meshway`、`reset-password` 和 `ip-datasets` 使用共同的 `meshway-logging` 配置，组件通过 `tracing` 发送结构化事件。程序入口安装一次 `tracing-subscriber`；嵌入式宿主负责安装自己的 subscriber。

## 配置与输出

| 参数／环境变量 | 默认值 | 行为 |
| --- | --- | --- |
| `--log-filter`／`RUST_LOG` | `info` | tracing 级别及 `target=level` 过滤；显式参数优先，非法过滤器使启动失败 |
| `--log-format`／`MESHWAY_LOG_FORMAT` | `text` | `text` 或每行一个对象的 `json` |
| `NO_COLOR` | 未设置 | 设置后关闭文本颜色；非终端 stderr 和 JSON 始终无颜色 |

日志统一写入 stderr。CLI `--json` 只控制 stdout 的公开状态结果，与 `--log-format json` 独立。Server／daemon 的现有 stdout 就绪行继续供进程管理器读取；重定向 stderr 可单独保存日志。格式化与写出使用 subscriber 的同步 stderr writer，无后台日志队列。文件持久化和轮转由进程管理器或外部收集器负责。

```sh
RUST_LOG=info,meshway_daemon=debug,meshway_node=debug ./target/debug/meshwayd \
  --data-dir "$PWD/.meshway/node" --server http://127.0.0.1:8430 \
  --log-format json 2>node.log

./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" \
  status --json --log-format json --log-filter debug 2>cli.log
```

JSON 事件包含 `timestamp`、`level`、`target`、`fields`；`fields.message` 是事件说明。span 上下文由 subscriber 的 `span`／`spans` 字段表达。

Apple bridge 在 extension 入口用同一 `Options::init_with_writer` 安装一次 subscriber，输出 JSON 行经 Swift 回调写入 os_log（`dev.meshway.node`／`node`），不在 Node 库安装 subscriber；日志回调不持控制凭据或包状态。默认 Rust 程序入口仍写 stderr。

Apple Debug 构建另开启 Control HTTP、控制调度、State 确认与 Connectivity 的定向 debug 事件；Release 保持 info。HTTP 传输失败只记录阶段、I/O 类别、数字 OS 错误与已有 HTTP 状态，Host 主动中断另记 `host_interrupt`，不记录原始错误链、URL 或报文。读取 VM 日志使用 `sudo /usr/bin/log show --info --debug --predicate 'subsystem == "dev.meshway.node"'`。

## 事件与字段

| 来源 | 常规日志 | debug 日志 |
| --- | --- | --- |
| daemon | 启动／就绪／退出、已提交的意图及 revision、状态转换、失败类别和重试等待；包平面每次撤回与每次重装各一条，记在撤回／安装真正执行处、与触发它的输入无关：必需组件失败撤回为 WARN `Required DNS component failed; suspending packet plane`（DNS）或 `Required packet plane component failed; suspending packet plane`（`problem`：TUN、承载），Control 撤回租约／Host 失败为 INFO `Netmap withdrawn; terminating the packet plane`（`host_failed`），租约到期为 INFO `Netmap lease ended; terminating the packet plane`；撤回之后内容再次安装时 INFO `Required DNS component recovered; netmap reinstalled`（DNS）、`Required packet plane component recovered; netmap reinstalled`（`problem`）或 `Netmap installed again after its withdrawal`（`cause`：`leaseEnded`／`netmapWithdrawn`），带 `trigger`（`offer`／`retry`／`dnsRecovered`／`hostRecovered`／`exitRecovered`／`listenersChanged`）、`withdrawn_ms`、`netmap_serial`、`complete`（缺能力的安装为 false） | 控制轮次是否续租／上报 |
| CLI | 命令失败、停止等待 | 命令名称 |
| Server | 监听就绪、关闭、HTTP 5xx 故障 | HTTP 响应状态和响应头就绪耗时 |
| DNS owner | 健康检查／安装失败（`DNS owner health check failed`、`DNS owner operation failed`、`DNS owner preparation failed`）带 `operation`、`error`、`check`、`expected`、`actual`、`source`，同一失败只记一次；恢复（`DNS owner recovered its system installation`）；resolve1 bus owner 变化 | 出口 served 快照变化 |
| Underlay | 宿主内核归属结束的原因：monitor 冲突 `reason`、egress capture owner 的失败 `operation`（含 `verify`、`verify_unavailable`）／`error`、控制中继监听／accept 失败与恢复（`Protected control relay cannot listen; retrying, kernel ownership kept`、`… listening again`）及容量满（`… at capacity; refusing new connections, keeping current ones`、`… accepts connections again`）；egress 回读暂不可用与重新校验一致（`failures`、`unverified_ms`）；暂态阻断后的归属重证（`Kernel ownership re-proof unavailable`／`… read back a conflict`、成功时 `Kernel ownership proved again`：`check`、`attempts`、`elapsed_ms`） | — |
| Connectivity | 组件错误类别 | channel transition；成功安装另记录 schema 1 的实际 channel／socket、端点、carrier、原 acquisition／attempt、Node／instance／配置引用和公钥绑定 |

Server HTTP 使用 `tower-http` 的 `TraceLayer`，span 只包含 method、声明的 route 模板及 Server 生成的 request ID。原始 URI、查询参数、headers、请求／响应 body 均不进入该层日志。SSE 的响应耗时表示开始响应，不表示整个订阅持续时间。

日志调用显式选取字段，不格式化完整设置、协议请求、控制快照或配置。Node 私钥、兑换码、会话 token、密码及配置原文保留在各自私有边界；批准链接或密码重置链接仅由相应命令的明确结果通道交付。日志不作为 saved／applied、执行授权或测量历史的权威来源。

成功 channel 安装记录的 `install_*` 字段来自正式 Manager 的当前 Scope 和实际 LinkFact。`install_attempt_id` 只表示该通道原始的协调获取来源，普通任务日志的 `attempt` 不能替代它；合法续接不会将新 publication 写回原安装记录。读取日志不延长任何权限或期限。

## 显式分层性能采样

`MESHWAY_PROFILE=0`（默认）关闭采样；`1..65536` 指定根调用的抽样分母，例如 `64` 约抽取 1/64。同一同步调用树继承根抽样决定，采用非密码用途的随机选择避免与周期性收发发生混叠。`MESHWAY_PROFILE_DETAIL=0` 默认只采粗层，`1` 增加执行 guard，`2` 再增加高频时钟子层。高频子层会显著影响被抽中调用的成本，应逐层开启并比较插桩关闭的同一程序。

daemon 由工作循环每隔至少 5 秒通过 `meshway_profile` target 输出累积计数；没有后台采样任务或逐包日志。使用 `info` 级别和 JSON stderr 即可收集。注册项、层级和标签均有上限，仅使用静态名称，不包含业务包、凭据或节点标签。

`metrics_json` 包含各层 `calls_started`、已完成的 `samples`、累积 wall／thread-CPU 纳秒及其 exclusive（self）值。父层 inclusive 已包含子层，不可再次相加；异步包装统计每次 poll 的运行时间，不包含 poll 之间的等待。线程 CPU 不可测时保持 null，并独立记录 CPU 样本数量；wall 时间可能包含被调度器暂停的时间。`queue.*` 记录已观测的排队年龄，不推断 CPU，`tun_attempt_age` 包括尚未取得路由的调度尝试。`counters_json` 为准确累积的丢弃原因计数，`gauges_json` 为最近一次 owner 采集的保留流／circuit 数量，缺少 owner 时为 null。它们均不是 State／历史业务测量，也不参与权限判断。

报告口径与各层测量入口见[分层性能测量](/v2/node/performance#共同报告口径)。
