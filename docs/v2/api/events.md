# SSE 与初始同步

**已确认：Console 建立 SSE 后先接收完整可见 Config／State，再接收变化。** 本页说明 V2 同步语义，并保留尚未落地的扩展通知提案；当前事件字段以新 Controller 实际路由导出的 OpenAPI 为准。

## 连接结构

```text
Console 登录、取得网络清单并选择网络
  → GET /api/v1/networks/{networkId}/events
  → config.snapshot
  → entities.snapshot
  → state.snapshot
  → sync.ready
  → entities.delta / state.delta / configuration.changed / configuration.progress / observation.*
```

每个标签页的当前网络复用一条主要事件流；发起配置保存、设备批准和观察任务仍用独立 HTTP 请求。SSE 只有服务端到客户端的方向，接收事件不等于已提交操作。

请求携带：

```http
Accept: text/event-stream
Authorization: Bearer <opaque-user-session>
```

浏览器使用 fetch 读取响应流。原生 EventSource 的构造参数不能设置任意 Authorization header；不把 session 塞进 URL，也不为 SSE 更换既有登录机制。

## Full snapshot

“完整”指**所选网络、当前用户、所请求视图的全部当前内容**：

| 内容 | 首次同步 |
| --- | --- |
| Config | 当前权限可见的完整配置视图与 revision |
| Entities | 当前可见 Node 元数据、成员及服务端 actions；不含账户／节点秘密 |
| State | 当前权限可见的节点／资源及最新观测，保留来源、时间、实例、配置引用和过期状态 |
| 配置应用进度 | 当前相关 publication 的目标、最后接受的回执及实时阶段；失联／过期明确标注 |
| 配置原文与秘密材料 | 不默认广播；管理员编辑时通过显式读取取得可写原文／材料视图 |
| History | 不随首屏全量加载，按需查询 |
| Path observation | 不因订阅而创建探测；只接收自己有权订阅的已创建任务 |

Owner／Admin 可以读取本网络管理视图，普通成员只获得相关视图。脱敏视图与可写完整配置使用不同类型，不能把省略字段或脱敏占位作为真实值整份回写。

先全量不要求一个无限大的 SSE frame。初版可以分别发送 Config 和 State snapshot；若超过声明的帧／快照预算，需要类型化分块或明确失败，不能静默截断后宣称完成。客户端暂存完整同步批次，校验完成才切换缓存。

### 同步截点

服务端先建立事件捕获与一致的读取截点，再读取快照，缓冲截点之后的变化。snapshot 发布与追赶事件必须连续；权限在同步过程中变化时重新裁剪／reset，不能继续发送旧权限范围的数据。

- Config 有自己的 revision；State 保留 stateEpoch、每个来源的实例／序号及报告引用。
- 同步截点只保证这个客户端视图与后续事件连续，不宣称所有 Node 同时采样或执行同一版配置。
- `sync.ready` 表示完整初始视图已接收，可以展示；不表示节点在线、隧道已建立或配置已 applied。
- 初始 snapshot 帧不提供可独立恢复的 event ID；到 ready 后才提交这批缓存和 cursor。中途断开则丢弃不完整暂存并重新 full sync。

普通 GET 配置／State 查询仍供按需读取和排错使用。若以后使用 HTTP snapshot 后接 SSE，也必须复用同一截点与 cursor 合同。

## 事件结构

JSON `data` 使用 Rust 定义的封闭判别联合，以 `type` 区分。可以统一使用默认 SSE message 事件，不另维护一份 event 名称到 DTO 的手写映射。

| `type` 提案 | 载荷与客户端行为 |
| --- | --- |
| `config.snapshot` | 当前 ConfigurationView、revision、同步批次标识；初始暂存 |
| `entities.snapshot` | 同一批次的可见 Node／成员／actions，供实体与配置／报告组合 |
| `entities.delta` | baseCursor／cursor、按实体类型的 updated／removed；完整替换受影响实体及 actions |
| `state.snapshot` | 当前 NetworkStateView、epoch、读取截点与同批次标识；初始暂存 |
| `sync.ready` | 完整批次结束、cursor、heartbeatIntervalMs／disconnectAfterMs；启用本批视图 |
| `state.delta` | baseCursor、updated、removed；按节点替换完整可见项，显式删除已移除／不可见项；过期按 freshness 表达 |
| `configuration.changed` | 已提交的新 revision、时间、允许显示的修改者／请求关联；使旧配置缓存过时 |
| `configuration.progress` | revision、服务端去重 summary、相关 publication／逐节点变化与新鲜度；统计范围随权限裁剪 |
| `resources.changed` | networkId／revision；Owner／Admin 重读规则集 GET，包含失败检查的新错误／时间；不含下载 URL，不修改配置草稿 |
| `node.registered` / `node.deleted` | 当前有权读取的生命周期通知；实体缓存以 entities.delta 更新 |
| `members.changed` | 可见成员／角色变化通知；新的成员／actions 随 entities.delta 发送 |
| `enrollmentKey.consumed` | 有权查看该密钥的订阅者收到安全的消费 Node 引用 |
| `joinLink.redeemed` | 有权查看该链接／成员关系的订阅者收到安全引用 |
| `activity.created` | 新操作历史 ID／时间，列表按需补读 |
| `observation.status` | 任务 preparing／running／completed／failed／expired 等实际状态 |
| `observation.sample` | observationId、measurementId、实际路径／配置引用与测量结果 |
| `observation.removed` | subscriptionId 对应的需求已失效或超过终态保留期；移除本地任务视图 |
| `sync.reset` | cursor_too_old／view_changed 等原因；清理服务端缓存视图并重新授权、full sync |
| `stream.closed` | session_expired／membership_revoked 等原因，不输出秘密 |

配置通知示例：

```text
id: <opaque-view-cursor>
data: {"type":"configuration.changed","networkId":"<network-id>","previousRevision":42,"revision":43,"cause":"edit","changedPointers":["/access"],"changesComplete":true,"sourceChanged":true,"changedAt":"2026-10-03T12:00:00Z","actor":{"type":"user","userId":"<user-id>"}}

```

自动产生的配置版本可以使用 system actor；不能伪称另一个用户保存。State 的来源标识与测量 ID 不由 SSE cursor 代替。

changedPointers 是允许展示的变更提示，changesComplete 说明列表是否完整；它不覆盖间接依赖，不是自动 rebase 或免除 CAS 的依据。仅注释变化仍推进 revision／sourceChanged。敏感值、链接 token、tokenHash 和完整材料不进入事件。

entities／config／state 的首次读取和后续事件使用同一授权截点；成员加入／离开及 Node 删除不能落在快照与订阅之间而丢失。成员事件不是授权检查的替代品，服务端在失权时直接裁剪／reset／关闭。

### Entities delta

```json
{
  "type": "entities.delta",
  "stateEpoch": "<epoch>",
  "baseCursor": "<previous-stream-cursor>",
  "cursor": "<next-stream-cursor>",
  "configurationRevision": 43,
  "updated": { "nodes": {}, "members": {} },
  "removed": { "nodes": [], "members": [] }
}
```

updated.nodes 使用 `Map<NodeId,NodeEntityView>`，updated.members 使用 `Map<MembershipId,MemberView>`；可选 updated.network 为完整 NetworkEntityView。每个值包含服务端计算的 actions，按项整体替换，不使用任意 JSON patch。同种实体的 updated／removed 不能重叠。

配置（如加 Tag／修改允许方案）、成员角色、账户资格、节点注册／删除变化都触发相关实体和 actions 重算。仍可见的条目发送 updated，失去可见性的条目发送 removed；不能只在 members.changed 时更新权限。无需为了补 actions 混入单独 GET 或另一套 cursor。

GET 的 entity／settings／state 片段分别复用 entities、ConfigurationView 和 NodeStateView 的类型；entities 只承载元数据和 actions，避免覆盖更新的运行报告。实体、配置和运行状态的更新使用同一流顺序及各自版本，不混淆所有权或新鲜度。

baseCursor 对齐上一条已应用的流事件（包含通知事件），不是单独的 entity 序号。缺基准／权限上下文变化走 reset/full sync；客户端原子应用一帧再推进 cursor，removed 同时清理对应可见实体缓存，未保存草稿继续按原身份隔离保留。

### Config 更新与草稿

`configuration.changed` 在持久事务提交后发送，包括本标签页自己的保存。事件带版本即可，初版不在每次修改时广播整份配置及材料。

- 无本地编辑：按新版本重新读取，合并短时间内的重复失效通知。
- 有未保存编辑：保留 JSONC、图形草稿和原 CAS 基准，提示服务器已有新版；由用户决定比较／合并／重新加载。
- 最终应用前展示有效修改数量与对照；辅助重放必须重新验证并由用户应用，不能因 changedPointers 没碰到编辑路径就自动提交。
- 不把旧草稿的 expectedRevision 偷偷推进到新值。保存时仍做 CAS，不能借 SSE 到达绕过冲突。
- GET 结果与 SSE 并发时按 revision 防止旧响应覆盖新版缓存。reset 丢弃的是服务端缓存视图，不静默抹掉未保存草稿；身份变化按原身份隔离并锁定草稿。

### 配置应用进度

`configuration.changed` 表示期望配置已受理，不能据此显示“全部节点应用成功”。`configuration.progress` 跟踪[普通应用流程](/v2/configuration-reference/deployment)：节点独立拉取、校验、保存、应用并回报，offline／未确认状态如实展示。

按 publication 的固定目标集合与各 Node 最新有效回报计算人数，重复读取／重放不能加一。新目标／重规划使用新 publication 并说明替代关系；后来 demand／refresh 的发布不自动加入原配置应用的分母。

配置应用事件同时携带 revision 和服务端按该次应用去重的 summary，与 `GET /configuration/revisions/{revision}/progress` 使用同一汇总类型／计算来源；包括 targetCount、各阶段计数和 full／partial 范围。逐节点只推变化的同类型片段，Console 替换 summary 并更新节点，不合并多个 publication 自算人数，也不为每条进度事件再发 GET。后续 demand／refresh 的独立事件不改写这个配置应用汇总。

首次 sync／重连读取当前进度，不要求重放全部中间步骤才能恢复页面。关闭页面停止订阅，节点继续同步已受理目标；SSE 只通知真实结果，不承担应用许可或提交确认。

## Path observation

创建／续租／取消使用 [Console API](/v2/api#console-api)。创建请求携带独立 subscriptionId，返回可复用的任务 ID 与实际租约；续租／取消按 subscriptionId 独立操作。结果复用网络事件流，同时包含任务及订阅引用。只有创建者或另行获准的订阅者接收，不广播本网络所有管理员诊断。发送 pending／replay 前重新检查当前订阅，取消／到期会清除旧范围队列并按需 reset，不撤销仍合法的其他需求。

创建任务前 Console 已建立网络订阅；任务创建、建立有权订阅关系与记录初始状态有一致的发布顺序。事件可能先于 POST 响应抵达，因此用任务 ID／请求关联去重并有界暂存；重连时同步当前任务状态，遗漏的历史样本不能补造。

订阅只消费事实，不自动发探测或续租。观察页在前台明确续租；关闭／切换网络时取消自己的需求，丢失取消也由短租约结束。连接仍打开不等于观察仍获授权。

RTT、业务流量、探测未回应率沿用 [Realtime Observation](/v2/state-reference/realtime) 的口径；重复事件不增加样本数，缺测不填零，SSE 往返耗时不作为 Mesh RTT。

## 重连与权限

- 客户端仅在事件解码、校验并应用成功后记录 `id`；它是当前视图的不透明 cursor，不是授权凭证或全网序号。
- 同一身份、网络、筛选范围和完整缓存仍存在时，重连使用 `Last-Event-ID`。页面只保存了 cursor 而没有对应缓存时，必须重新 full sync。
- 后续每个事件推进同一视图 cursor；State delta 的 baseCursor 与上一已应用事件对齐，包括期间的配置／观察事件。重复投递可去重，断档不能跳过后继续假装一致。
- 服务端只保留有界重放缓冲。cursor 过旧、epoch／可见范围改变或积压过多时 reset；不承诺无限回放或恰好一次投递。
- 账户／会话到期、退出、成员移除和角色变化适用于已打开的流。权限缩小先停止旧范围输出、丢弃旧队列再裁剪；完全失权时关闭。不能只在握手时鉴权。
- 服务端定时发送传输保活注释，客户端使用断流超时与有界退避。保活不是 Node 心跳，不刷新 State 或执行 lease。
- 代理需允许流式 flush，禁用响应缓冲与缓存；HTTP/2 优先，但不靠“一节点一连接”维持 Console 视图。

## OpenAPI 与生成客户端 {#openapi}

**可以生成，但声明 SSE 响应不等于现有生成器已经能处理流。**

| 层 | 支持情况 |
| --- | --- |
| OpenAPI 3.1 | 可以声明 `text/event-stream` 响应和组件 DTO；缺少标准的逐事件 itemSchema 字段，需要明确生成器支持的流式扩展约定 |
| OpenAPI 3.2 | 引入 `itemSchema` 描述流中每项，并规定 SSE 帧与字符串 data 的 schema 表达 |
| 当前 TypeScript 生成链 | 自有 emitter 从实际路由生成 JSON GET／POST 和显式声明的 SSE 操作；逐帧严格 JSON／Zod 校验、调用者已应用 cursor 恢复、读空闲超时和有界退避 |
| 当前 Rust 生成链 | 自有 emitter 复用公开 protocol 类型生成 JSON／SSE 方法；有界增量解码，调用者显式提供已应用 cursor 并管理重连／超时 |

当前使用 OpenAPI 3.1 的 operation 扩展 `x-meshway-sse: { format: "json-data", eventSchema: { $ref: "#/components/schemas/..." } }`。成功响应本身声明 `text/event-stream` 字符串字节，扩展声明每帧 JSON 数据的真实根类型；可省略的 `Last-Event-ID` 是字符串，不是 nullable JSON 字段。

新 Controller（`apps/meshway-controller`）的正常 Router 已绑定具体 `UserEvent`／`NodeEvent` 与分别重鉴权的生产 provider，共用 App 的 State core。初始 Config 使用 `UserConfigurationView`（管理员完整非秘密视图／成员自身节点视图）与 `NodeConfigurationView`（自身非秘密设置及真实 target）；Node 的可安装材料仍从 `configuration/query` 获取。Entities 复用 GET 的库存、成员、网络片段及服务端 actions。

`ConfigurationApplyProgress` 同时用于按 revision 的 GET 与 SSE：分母来自版本内固定的 `execution_targets`，按原 target 的真实 receipt／attempt／sequence 投影。`phase` 与 summary 保留最后接纳结果，`freshness` 独立说明当前、过期、待新实例或已被替代；历史 applied 不等于当前健康，后续 demand／refresh 不扩大原分母。普通成员只统计当前相关范围，历史完整进度限 Owner／Admin。

事件的公开字段以同一 Router 导出的 OpenAPI 为准；生成 SDK 与消费者仍须在联合提交中完成 drift 验证。完整交付检查包括：

1. Rust 定义 UserEvent／NodeEvent 及各自权限视图，真实路由显式声明 stream、认证与 payload schema。
2. 同一 Rust 路由声明导出上述 SSE 扩展元数据。不能将逐事件对象误写成整个响应就是一个 JSON 对象。
3. 生成操作 URL、参数、认证要求、事件联合类型和 Zod／Rust 解码器。SDK 提供 async iterator／stream，应用不复制 endpoint 或 DTO。
4. 共用传输层处理 UTF-8 增量、SSE 分帧／多行 data、取消、大小上限、重连及 cursor；每帧 JSON 经过生成 schema 校验才交付。OAS 3.2 的 contentSchema 注解本身不保证运行时执行校验。
5. drift check 覆盖事件产物；非浏览器行为测试覆盖分帧、错误媒体类型、过期／撤销、full→delta 连续性、断线恢复与背压。不能把仅生成了 TypeScript 类型当成流式客户端已验收。

参考：[OpenAPI 3.2 Media Type / SSE](https://spec.openapis.org/oas/v3.2.0.html#media-type-object)、[EventSource 构造参数](https://developer.mozilla.org/en-US/docs/Web/API/EventSource/EventSource)。
