# SSE 与初始同步

SSE 使用 `text/event-stream`。每个 `data` 是 [OpenAPI](/v2/contracts/openapi.json) 中的 `UserEvent` 或 `NodeEvent`，由生成 SDK 校验和读取。

## 连接

| 操作 | 路径 | 身份 |
| --- | --- | --- |
| watchNetwork | `GET /api/v1/networks/{networkId}/events` | 用户会话，当前网络权限 |
| watchNode | `GET /agent-api/v1/events` | Node 会话与有效实例 |

```http
Accept: text/event-stream
Authorization: Bearer <session>
```

浏览器通过 fetch 携带 Authorization，凭据不放 URL。每个标签页的所选网络使用一条主要流；提交配置、批准和创建任务使用独立 HTTP 操作。

## 初始批次

```text
config.snapshot → entities.snapshot → state.snapshot → sync.ready
```

前三帧使用相同 batch，在收到 ready 前暂存。它们没有可独立恢复的 event ID；批次中途断开时丢弃暂存并重新同步。ready 表示客户端可提交这批缓存，不表示 Node 在线或数据面已应用。

| 帧 | 内容 |
| --- | --- |
| config.snapshot | 当前可见配置与配置应用进度；用户和 Node 使用不同配置视图 |
| entities.snapshot | 与 GET 共用的库存、成员、网络和 actions 片段 |
| state.snapshot | 当前可见 StateView，保留报告来源、时间与期限 |
| sync.ready | batch、cursor、heartbeatIntervalMs、disconnectAfterMs |

Owner/Admin 获得本网络非秘密管理视图，成员获得其本人节点设置及获准 State，Node 获得自身设置和必要执行依赖。完整 source／私钥／协议凭据不广播，历史按需查询，订阅不会创建探测任务。

用户实体库存与 `GET /nodes` 使用相同的本人／管理员范围；获准 peer 的窄 State 视图不授予库存或设置读取权。成员的已授权设置变化时，服务端通过 `sync.reset` 与新的完整批次替换设置，客户端无需自行断线刷新。

服务端在一致读取截点建立事件捕获，缓冲截点之后的变化。超出帧／批次预算明确失败，不截断内容后发送 ready。

## 事件结构

| type | 用途 |
| --- | --- |
| entities.delta | baseCursor／cursor、configurationRevision、类型化 updated 和 removed |
| state.delta | stateEpoch、baseCursor／cursor、完整节点更新、删除项、links／paths，以及投影的 asOf／sequence／configurationRevision |
| configuration.changed | 已提交 revision、previousRevision、cause、允许披露的 actor／requestId、changedPointers、changesComplete、sourceChanged |
| configuration.progress | 与按 revision GET 相同的 ConfigurationApplyProgress |
| resources.changed | networkId／revision，通知有权管理者重读资源视图 |
| observation.status／observation.sample | 相同 ObservationTask 类型，带独立 subscriptionId 和当前执行上下文 |
| observation.removed | 订阅终态保留结束，清理对应视图 |
| sync.reset | cursor_too_old、view_changed、backpressure 或 epoch_changed |
| stream.closed | session_expired、membership_revoked、node_lease_expired 或 server_shutdown |

事件使用默认 SSE message；不要根据节点注册、链接消费或活动写入自行假定存在另一个事件名称。完整联合以下载契约为准。

### Entities delta

updated 复用 UserEntitiesView／NodeEntitiesView，removed 按 nodes／members 列出 ID。项更新整体替换，删除明确执行；字段结构和必填项由各自 schema 决定。角色、身份或配置变化会重新计算 actions，即使没有 Node 新报告。

实体元数据、配置与运行报告分别维护，实体更新不覆盖更新的 NodeState。baseCursor 对齐上一条已应用的流事件，包括配置／资源通知，不是单独的实体计数。

当前 Server 的 `state.delta` 带与该次 StateView 相同的 `asOf`、`sequence` 和 `configurationRevision`。生成类型允许保留的旧增量缺少这些元数据；客户端在字段存在时替换缓存对应值。`asOf` 是投影读取时间，`sequence` 是网络已提交变化的序号（含配置、回执、资源和观察事件），`configurationRevision` 是投影使用的当前配置版本，均不是测量的新时间或执行 applied 证明。仅时钟推进不会发送增量；报告／到期／配置变化可触发投影更新，原报告时间与期限保持原值。

### 配置与草稿

配置通知使服务端视图缓存过时，但保留本地草稿及原 CAS 基准。changedPointers 只是提示，不证明间接依赖未变；重新验证和用户应用后才提交。并发 GET 用 revision 防止旧响应覆盖新缓存。

配置应用进度来自固定提交目标；后续 demand／refresh 不扩大原分母。phase 与 freshness 分开，历史 applied 不表示当前健康，见[配置应用](/v2/configuration-reference/deployment)。

### 观察任务 {#path-observation}

任务通过[观察 API](/v2/state-reference/realtime)创建、续租和取消。事件可先于创建响应抵达，按 subscriptionId／任务 ID 关联；缺失样本不补造。取消、到期或失权会清除旧范围排队／重放结果，不取消其他有效订阅。

## 重连与权限

- 只有完整批次或事件已校验并应用后，调用者才保存 cursor；收到或解码成功不等于应用成功。
- 同一身份、网络和缓存重连时携带 `Last-Event-ID`；只保留 cursor 而丢失缓存时重新 full sync。
- 重放有界。断档、范围改变或背压触发 reset，不能跳过缺口继续更新。
- 每次输出重检权限；缩小范围先丢弃旧队列再重建视图，完全失权关闭。
- 保活注释不刷新 Node 报告、测量或执行 lease。代理须支持流式 flush 并禁用响应缓冲／缓存。
- 断线立即释放活动流名额，每个身份保留至多四份、至多一分钟的断线重放缓存。缓存淘汰后携带旧 cursor 会重新同步。
- 会话失效、成员被移除、Node lease 失效和正常服务关闭发送相应 `stream.closed` 后 EOF；角色变化及流的最长生命周期以 EOF 要求重新鉴权连接，不把仍有效的用户会话标成失效。连接已断开或进程异常终止时可能只能观察到 EOF。

## 生成声明 {#openapi}

成功响应是字符串字节流。OpenAPI 3.1 使用 `x-meshway-sse` 的 `eventSchema` 指向逐帧 JSON 类型；普通错误响应仍是 ErrorResponse。

Rust 客户端提供有界 EventStream，由调用方管理重连；TypeScript 客户端提供 async iterator、逐帧 Zod 校验、空闲超时和有界重连，重连 cursor 由调用者提供。UTF-8、分帧、多行 data、取消和大小限制在公共运行时处理。
