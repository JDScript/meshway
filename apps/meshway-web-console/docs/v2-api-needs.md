# V2 API：Console 的需求与消费反馈

本页由 console 工作流维护，记录三件事：console 需要 V2 Controller 提供什么、与后端已经商定的约定，以及对照最新已接入的后端版本还有哪些问题。

- 接口以 Rust 路由导出的 `packages/protocol/openapi.json` 和生成的 SDK 为准；设计文档在 `docs/v2`（入口 [API 索引](../../../docs/v2/api.md)）。
- 以往各轮的问答和复核记录在 git 历史中，本页只保留现行内容。
- 路径省略 `/api/v1` 前缀。

## 已定的产品决定

| # | 决定 |
| --- | --- |
| D1 | 已有账户通过一次性、有期限、绑定网络和角色的加入链接加入网络，由已登录用户接受。不提供按邮箱直接添加，不恢复邀请注册，链接也不绕过注册关闭的开关。签发和兑换时都检查当前权限。 |
| D2 | 账户恢复不靠邮件：运维在 Controller 主机上用 CLI 生成重置链接，console 只提供兑换页 `/reset-password`。网络 Owner 不能重置他人的全局账户密码。 |
| D3 | 概览上，当前真实的 links 与历史淡线分开画。历史层默认关闭，时间窗口由用户选择，数据只来自真实的历史事件；不展开组网规则允许的 N² 连接，也不为画线额外探测。 |
| D4 | 删除节点是永久的：提交时吊销凭据并移除 `nodes[id]`，没有回收站，也没有"仅从配置移除"。JSONC 或 ops 删去已有节点同样是删除。重新接入就是一个新节点。业务上的停用用 `enabled=false`，可以回滚。 |
| D5 | 每个用户、每个网络一份草稿，跨页面保留，只有一个应用入口。应用前必须展示有效修改数量和前后对照，并把永久删除、改址、身份和材料变化单独列出。严格 CAS，不按"pointer 没变"自动合并。草稿里要么是 JSONC 整份文本，要么是强类型 ops，两者不互相转换。 |

## 已商定的接口约定

- **权限**：每类资源带服务端计算的 `actions`，console 不从角色推导；mutation 仍由服务端实时重新检查。
- **错误**：`ErrorResponse { requestId, status, problem: { code, params, message } }`，`code` 是每个操作的封闭枚举。配置诊断带 pointer；State 的问题带资源和观测时间。
- **幂等与秘密**：
  - 创建类请求都带 `requestId`。
  - 接入密钥和加入链接的原值只在首次响应里交付。重试拿到的是同一资源的元数据，并注明原值不可重取。
- **命名与定位**：字段统一 camelCase；规则用 `remark`，不带 id；定位一律用 revision + JSON Pointer。
- **配置编辑**：
  - 提交有两种：整份 JSONC 和强类型 ops（`replaceAccess` 等明确分支，不接受任意 JSON）。
  - 历史版本有提交时间、cause 和 actor。系统写入（接入、邮箱同步、删除、身份转换）也会产生新 revision。草稿基于旧版本时，用同一批 ops 在新版本上重新 validate，展示新的对照，由用户应用。
  - `validate` 返回诊断、修改数量与对照，以及规范化后的脱敏 `preview`；它不生成材料、不预留地址。
  - 表单里的秘密只写：保持、覆盖或轮换。读取秘密只有整份 `configuration/source` 一条路。
- **成员移除和离开网络**：永久删除该成员当前的 User 身份设备，保留 Tag 设备。确认前先读 `removalImpact` 或 `leaveImpact`。
- **接入**：
  - 标准安装命令只含公开信息，接入密钥不进进程参数或 shell 历史。
  - 首次接入时，`initialSettings.hostname` 只是建议名，重名时自动加后缀。之后改名若重名，直接报错。
- **部署进度**：按 revision 由服务端去重计数，标明 full 还是 partial 范围；GET 和 SSE 用同一类型。
- **SSE**：
  - 首次同步发送完整可见的 Config、实体和 State 快照，之后是增量。实体和 `actions` 有 `entities.delta`。
  - `configuration.changed` 带 changedPointers 和 cause。
  - 服务端给出保活间隔，reset 和关闭都带原因。
  - SSE 与 GET 复用同一组片段类型。
- **State 与历史**：
  - 位置由 Controller 根据 endpoint IP 用 GeoIP 推导，带来源和时间；拿不到时给出缺失原因。
  - links 和 paths 的聚合保留每个报告者的证据。
  - 历史层通过 `history/query` 的 `recentLinks` 模式读取，不进入默认的实时流。
- **加入链接与重置链接**：兑换前可以用 `inspect` 预览，不会消费；最终兑换时重新校验。
- **实时观察**：`auto` 模式由 Controller 计算候选、执行端选择路径；`explicit` 模式固定路径。console 只在页面可见时续租。

## 接入状态（后端 `5cfa1b9c`）

console 已接入 `5cfa1b9c` 提供的全部 Console 操作（`/api/v1` 下共 66 个）：

- 账户、会话和账户活动；
- 网络、成员、加入链接和网络活动；
- 配置：JSONC 和全部区段表单、validate（含规则集准备）、apply、历史、对照和恢复；
- 节点、节点运行 State、节点设置（成员通过 `getNodeSettings` 读取）和节点密钥；
- 接入密钥和设备授权；
- 按 revision 的部署进度；
- 网络 SSE，含观察事件；
- 规则集；
- 观测历史；
- 主动路径观察。

## 尚未提供的接口

- 删除网络（含删除时节点凭据的处理和保留期）。
- 策略验证 `configuration/evaluate`：给定源、目的、协议和端口，返回命中的 access、topology、routing 规则，以及规划的完整路径或出口；结果标明是规划，不是实测。
- 成员切换路由方案：`requestedProfile`、`appliedProfile`、允许的方案、`locked` 和自动方案的判断依据。
- 节点的版本兼容状态 `versionStatus`（current、outdated、unsupported），由服务端按兼容门槛判定。
- `reassignAddresses`、`rotateMaterial`。
- 安装脚本和 Agent 发布。
- 可信 DNS 与 GeoIP 的数据输入，以及基于它们的自动方案。

## 待处理的问题

以下按 `5cfa1b9c` 核对。

**`5cfa1b9c` 已修复的问题**（不再跟踪）：

- `DefaultsUpdate.dialers` 改为可省略。
- 没有目标节点时，apply 返回 `publicationIds: []`。
- `request_id_conflict`、`version_conflict`、`state_conflict` 各自返回。
- 加入链接 `inspect` 区分各种失效状态。
- 补齐了加入链接、接入密钥和节点密钥的 actions。
- 补齐了字段：
  - 设备请求的 `existing` 和 `returnMethod`；
  - 批准信息；
  - 接入密钥的创建人、使用时间、接入后的节点和预设摘要；
  - 密钥的创建时间；
  - 活动 actor 的邮箱快照。
- 加入链接和接入密钥的列表带 `total`，并按倒序分页。
- 成员可读自己设备的设置（`getNodeSettings`）。
- 承载目录的 schema 名称说明。

### 影响功能或安全

1. **OpenAPI 组件重名已在主线整合中修复。** 执行保留 `UnavailableReason`，IP 查询使用独立的 `LookupUnavailableReason`；路由、子 API 与 SSE schema 注册检查同名冲突并失败，生成 SDK 已区分两组词汇。其余反馈仍按下列原范围逐项核对。
2. **成员的配置视图不会更新。** `configuration.changed` 之后不会重发成员的节点设置，console 只能断开重连做完整同步，或者重新调用 `getNodeSettings`。
3. **SSE**
   - 从不发送 `stream.closed`：会话过期、被移除、角色变化时，连接都只是 EOF 结束。console 要靠重连拿到 401 或 404 才知道原因。
   - 每个会话每个网络最多 4 条流，已断开的订阅还要占名额 1 分钟。刷新几次或开多个标签页，就会连续收到约 1 分钟的 429。
   - `state.delta` 不带 `asOf`、`sequence` 和 `configurationRevision`，应用增量后这三项一直是快照时的值。
   - 快照不带 requestId，无法直接当 GET 响应写入缓存，管理员的配置只能重新读取。
4. **部署与执行**
   - 节点应用配置后如果重新 acquire instance，在原版本中它会被标为 `superseded`，并一直显示"未回报"，实际上它已经应用。
   - 任何节点发起 demand 都会重新发布整网配置。其他节点立刻变成 `config_mismatch`，links 变成 `expired`，直到它们重新上报。报告有效期只有 60 秒，概览上的状态因此反复跳动。
5. **需要生成材料的配置无法预览。** 新增 VLESS listener，或者删除后重建（轮换）时，validate 仍直接返回 422 requirements（uuid、certificate、privateKey、certificateValidation），拿不到 preview。这与 editing.md 写的"预览中显示将生成"不一致，界面因此无法完成新增 VLESS。
6. **GeoIP/ASN 数据集没有部署入口。** `IpDatasets::install` 没有调用方，所以所有节点都没有位置。
7. **设备授权的浏览器链接格式没有定义。** console 暂用 `/authorize-device/<enrollmentRequestId>`，需要和 Agent、App 一起定下来。
8. **SSE 把 Tag 节点的库存推给了普通成员。** 同一个成员，`GET /nodes` 只返回自己的 User 身份设备，而 `entities.snapshot` 返回了全网节点（含 identity、地址和密钥）。两者的可见范围不一致；按文档，GET 的范围是对的。
9. **主动观察**
   - 创建、续租、取消都会重新生成整网的执行发布。正在运行的观察每 10 秒就被打回"准备中"，path 变为 null、分段加一，无法连续测量；每个 console 的事件流也随之收到 `sync.reset` 并全量重新同步；节点频繁显示"报告与配置不符"。
   - 续租和读取在内部校验失败时（例如节点租约已过）返回 401 `invalid_session`，console 无法和会话失效区分，会把用户登出。应改为 403、404 或 409。
   - 有观察在运行时，controller 处理 Agent 请求（tasks、renew、receipts）要 0.4–1 秒。

### 契约和字段

10. **配置诊断**
   - `ConfigurationIssue` 只有英文 message，没有 code 和 params，无法本地化。
   - 路径只到集合一级，解析错误时 path 为空。节点级的出口授权问题挂在 `/nodes/<id>` 上，指不到具体的规则或方案。
   - 没有 warning 通道，无法提示被前面规则完全遮挡的规则。
   - 数组变化只按整体报告。
   - 预览缺 `affectedNodeIds`、`identityChangedNodeIds` 和 `changedListeners`；删除节点时，该节点也会出现在 `addressChangedNodeIds` 里。
11. **配置操作**：`unmanaged` 在普通规则里会被拒绝，但 schema 允许它出现。
12. **Listener**
    - `ListenerView` 只给出 WebSocket 响应头的名字，而 `ListenerUpdate` 整体替换、不继承旧值，所以表单没法在保留这些响应头的前提下修改监听。editing.md 现在写明 dialer 更新时省略 headers 表示保持，listener 是否同理需要确认。
    - 托管证书模式下修改 `serverName` 时，`inherit_materials` 可能沿用旧证书。这是读代码推断的，没有验证。
13. **错误码与幂等**
    - `removeMember` 的 version 或 revision 不匹配时一律返回 `version_conflict`；revision 不匹配应返回 `revision_conflict`。
    - 账户会话、活动的游标失效时返回 `invalid_request`，应与网络列表一致，返回 `invalid_cursor`。
    - `refreshRuleSet` 没有请求体，也就没有 requestId，重试会重新下载。
    - 对已失效的对象执行撤销时也返回 204，客户端分不清"这次撤销成功"和"本来就已失效"，应返回 `state_conflict`。涉及两处：
      - `revokeEnrollmentKey` 撤销已使用或已过期的密钥；
      - `revokeJoinLink` 撤销已使用、已撤销或已过期的链接，这时也不记活动。
    - `revokeNodeKey` 只核对 keyId + generation 是否存在。轮换后旧代际仍在，用旧 generation 也能成功，不会返回 `version_conflict`。
    - 降级 admin 时，它签发的未用链接会被写上 `revokedAt`，但 inspect 对这些链接返回 `join_link_issuer_unavailable`，与列表里的"已撤销"说法不一致。角色变更也没有影响预览，不会告诉操作者哪些链接会被撤销。
    - `getNodeSettings` 的 OpenAPI 只声明了 401 和 404。
    - 规则集准备失败只返回一条英文 issue，消息里是 Debug 枚举名（`remote rule set preparation failed: DownloadFailed`），没有 `ResourceProblem` code。
    - 规则集超出上限的处理不一致：超过 32 个、超过 8 MiB 或准备并发已满时返回 503，30 秒超时却作为 `/ruleSets` 上的 issue 返回。
    - 主动观察的 429 `observation_limit` 同时表示续租过早、预算用满、已到终态和 4096 窗口满，无法据此给出准确提示。
    - validate 响应的 `requestId` 与请求里的不同。如果本意是回显请求的 ID，需要修正。
14. **缺少的字段**
    - 设备请求没有系统和 Agent 版本。后端说明这是有意不在 intent 里提供，console 不显示。如果以后需要，要加进签名的 intent。
    - 成员视角的 StateView 不给对端节点的主机名。
    - `JoinLinkView` 没有签发人；使用者的邮箱只能从当前成员列表里查，使用者离开网络后就无法显示。
    - 主动观察：
      - 没有观察权限的 `actions`，console 只能按"角色 + 身份"自行重述规则；
      - 没有允许的目标和路径候选清单，空闲网络里普通成员根本没有可选的目标；
      - 响应里没有服务器时间，续租时机只能按浏览器时钟计算；
      - 样本只给 `latest`，重载或接管后看不到之前的序列。
    - `account.email` 活动的 actor 快照是修改后的新邮箱，需要确认是否有意如此。
15. **历史**
    - `HistoryBucket` 没有 `peerNodeId`，而且按 instance 和 publication 拆开。每次 demand 都会重新发布，同一条曲线因此被切碎，console 端做了合并。
    - `RecentLink` 没有 RTT 和关闭原因。
    - 一次查询最多 10000 行，计数在按 kind 或节点过滤之前做，繁忙的网络即使只查一个节点也会收到 429。
    - 窗口外的清理记录也会返回。bucket 把 `pending` 计入 `failed`，语义不明确。
16. **规则集**
    - 检查失败的记录不跨版本保留，换到新版本后变成 `not_ready`、`checkedAt: null`；成功的绑定则会延续。
    - 绑定了 8483 条的集合后，即使只改 defaults 或 groups，validate 也要约 40 秒；只有 12 条时约 10 ms。测试用的是 debug 构建，但值得查一下。
17. **双向同时拨号**：两端都报告自己是发起方时，link 被判为 `conflicting`，`inUse` 变成 null。这种情况需要定义。
18. **给 Agent 实现方的提醒**：每个节点最多 8 个会话、8 个 challenge。
19. **SDK**：`transport.ts` 用了 DOM 类型 `ReadableStreamReadResult`，只带 Node 类型的 TS 项目编译不过。
20. **部署**：`--listen` 的默认值仍是 `127.0.0.1:8420`，与现在运行的旧服务冲突。
