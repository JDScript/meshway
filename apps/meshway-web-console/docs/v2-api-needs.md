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

## 接入状态（后端 `4b6cdd6a`）

console 已接入 `/api/v1` 下全部 66 个 Console 操作：

- 账户、会话和账户活动；
- 网络、成员、加入链接和网络活动；
- 配置：JSONC 和全部区段表单、validate（含规则集准备与监听器材料计划）、apply、历史、对照和恢复；
- 节点、节点运行 State、节点设置（成员通过 `getNodeSettings` 读取）和节点密钥；
- 接入密钥和设备授权；
- 按 revision 的部署进度；
- 网络 SSE，含观察事件、`stream.closed` 和增量里的 State 元数据；续传不重发初始批次，console 以 SDK 的 `onOpen`（连接被接受）确认服务端接受了 cursor；
- 规则集；
- 观测历史；
- 主动路径观察。

撤销已失效的加入链接、接入密钥，以及重复吊销节点密钥时的 409 `state_conflict`，console 提示"这次没有撤销"并重读列表。

## 尚未提供的接口

- 删除网络（含删除时节点凭据的处理和保留期）。
- 策略验证 `configuration/evaluate`：给定源、目的、协议和端口，返回命中的 access、topology、routing 规则，以及规划的完整路径或出口；结果标明是规划，不是实测。
- 成员切换路由方案：`requestedProfile`、`appliedProfile`、允许的方案、`locked` 和自动方案的判断依据。
- 节点的版本兼容状态 `versionStatus`（current、outdated、unsupported），由服务端按兼容门槛判定。
- `reassignAddresses`、`rotateMaterial`（editing.md 已不再列出，需要时重新定义）。
- 安装脚本和 Agent 发布。
- 可信 DNS 与 GeoIP 的数据输入，以及基于它们的自动方案。

## 待处理的问题

以下按 main 上的 `4b6cdd6a` 核对：契约按 `packages/protocol/openapi.json`；行为用隔离 Server（`0c3ec192` 构建，之后的服务端提交只改 link 证据和规则集校验性能）和脚本节点验证（脚本节点只接入、持有实例和上报 State，不应用配置、不执行任务）。

**已修复并已验证**（不再跟踪）：

- OpenAPI 组件重名：IP 查询改用 `LookupUnavailableReason`。
- 成员的设置变化后，服务端发 `sync.reset` 并重发完整批次；console 不再自行断开重连。
- 会话被撤销时先发 `stream.closed`（`session_expired`）再结束。
- 断开的流立即释放名额：同一会话连续打开并断开 7 次均为 200。
- 续传时响应头立即送达，之后只补发缺失的事件。
- `state.delta` 带 `asOf`、`sequence`、`configurationRevision`；概览的"截至"随增量保持最新。同一 sequence 下，流写入的 State 与重新 GET 的只差 `asOf`（各自的投影读取时间）。
- 配置变更、节点删除和成员移除的事件都在提交后立即送达，被拒的提交不产生事件。
- 新增 VLESS listener 可以预览：`willGenerate` 列出将生成的 UUID、证书和私钥，`requirements` 列出待完成的证书校验。
- 成员的 SSE 库存与 `GET /nodes` 一致，只有自己的 User 身份设备。
- 观察鉴权失败不再返回 401：源节点重新 acquire 后，订阅被移除（读取为 404），console 不会登出。
- SDK 不再使用 DOM 类型 `ReadableStreamReadResult`。
- 所有分页列表（网络、账户会话和活动、成员、加入链接、网络活动、接入密钥）的游标失效时都返回 `invalid_cursor`。
- `removeMember` 的 revision 不匹配时返回 `revision_conflict`。
- 删除节点的预览不再把该节点列进 `addressChangedNodeIds`。
- 撤销已使用、过期或撤销的加入链接和接入密钥返回 409 `state_conflict`；这些撤销操作和 `getNodeSettings` 的 OpenAPI 声明了实际的错误状态。
- `docs/v2/api/events.md` 不再给 `entities.delta` 写 `configurationRevision`。

**按设计，不再跟踪**：

- 响应里的 `requestId` 是服务端为这次 HTTP 请求生成的关联 ID（与 `x-request-id` 一致），不回显请求体里的 `requestId`。
- `account.email` 活动的 actor 是提交时刻的身份，所以是新邮箱。要显示"从 A 改为 B"需要给活动加字段，列在下面的契约增补里。
- 设备请求不带系统和 Agent 版本（不在签名的 intent 里），console 不显示。

**后端已说明、console 未实测**：

- `revokeNodeKey` 重复吊销返回 409 `state_conflict`，已退休代际仍可吊销（204，不撤当前执行），generation 不匹配是 409 `version_conflict`。
- 绑定大规则集（8483 条）后 validate 很慢的问题，在 `70f5e95f` 改为线性覆盖计算。

### 需要修正

- 节点停止上报、租约记录清掉以后，它的 `freshness` 从 `expired` 变成 `superseded`：`CurrentNode.instance_id` 为 `None` 时，`None != Some(上报的实例)` 先命中 superseded。结果一个已经停掉的节点在 console 里一直显示"刚重启"。应该只在有另一个当前实例时判 superseded，否则继续往下按租约判 expired（`apps/meshway-server/src/state/mod.rs` 的 freshness 判定）。用脚本节点复现：报一次后不再续租，约 1 分钟内是 `expired`，之后变成 `superseded`。

### 需要真实 Node 复核

Node 已有 `meshwayd` 的控制流程（接入、认证、续租、保存配置、上报状态），下面几项可以用它复核，这一轮没有做：

1. 节点应用配置后重新 acquire instance，原版本中是否仍被标为 `superseded`。
2. 节点发起 demand 是否重新发布整网配置，使其他节点变成 `config_mismatch`、links 变成 `expired`，直到重新上报。
3. 主动观察的创建、续租、取消是否重新生成整网执行发布；有观察时 Agent 请求（tasks、renew、receipts）的耗时。

### 契约增补（console 建议的优先级）

后端希望一次性加入并重生成 SDK。console 按影响排序：

1. `NetworkActions` 加"读取完整配置"的动作。现在 console 只能先请求 `GET /configuration`，用 403 判断自己是成员，再改读成员视图，成员每次打开节点页都多一次被拒绝的请求。
2. 主动观察：响应带服务器时间（续租时机现在按浏览器时钟计算）、观察权限的 `actions`（console 现在按"角色 + 身份"重述规则）、允许的目标和路径候选清单。
3. 成员视角的 StateView 给对端节点的主机名。
4. `JoinLinkView` 带签发人。使用者的邮箱现在只能从当前成员列表里查，使用者离开网络后就无法显示。
5. SSE 的配置快照带 `requestId`，管理员同步后就不必重读配置。
6. `unmanaged` 在普通规则里的 schema 收紧，与校验一致（现在 schema 允许、校验拒绝）。
7. `account.email` 活动保留修改前的邮箱。

### 后端回复（契约增补，`server/configuration-pipeline`）

已加入并重生成 OpenAPI／SDK；console 的一处消费（`-member-list.tsx` 的 `consumedByUserId`）和受类型影响的测试 fixture 已随同一提交最小改动，保证 `mise run check` 通过，页面展示请按需调整。

| # | 增补 | 契约 |
| --- | --- | --- |
| 1 | `NetworkActions.readConfiguration` | owner／admin 为 true；为 false 直接读成员视图，不必用 403 探测 |
| 3 | 成员视角对端主机名 | `NodeStateView.hostname`：配置里的规范 hostname（显示名），所有可见节点都有，manager 视图也带；不是报告里的 OS hostname，PeerState 其他裁剪不变 |
| 4 | `JoinLinkView` 签发人 | `issuedBy: {userId, email}`；`consumedByUserId` 改为 `consumedBy: {userId, email} \| null`。email 是账户当前地址，读取时联接 users，领取者离网后仍可读 |
| 5 | SSE 配置快照 `requestId` | `config.snapshot.requestId`：承载本连接的 HTTP 请求关联 ID（同 `x-request-id`）；重连续传后重发的快照用新连接的 ID |
| 7 | `account.email` 旧邮箱 | `ActivityTarget.account.previousEmail`：只在 `account.email` 上非空，其他账户活动为 null |
| 其他 | 降级 admin 后 inspect 与列表不一致 | 链接自身状态先于签发者状态报告：已撤销的链接 inspect 返回 `join_link_revoked`，`join_link_issuer_unavailable` 只剩"仍有效却找不到签发时权限关系"这一种情况 |

- **2 · 主动观察**（服务器时间、observation `actions`、候选清单）：observations 冻结，等 Node 后重做时一并加。
- **6 · `unmanaged` schema 收紧**：要把 egress 类型拆成"规则可用"和"fallback 可用"两个，Node 也消费该 crate；并入配置提交流水线那次重生成。
- 角色变更的影响预览（哪些链接会被撤销）没有做：目前没有角色变更预览接口，是否要加属于产品决定。

### 后端排期中

- **配置诊断与 Listener**：随提交流水线合并一并定义。
  - `ConfigurationIssue` 只有英文 message，没有 code 和 params，无法本地化。
  - 路径只到集合一级，解析错误时 path 为空；节点级的出口授权问题挂在 `/nodes/<id>` 上，指不到具体的规则或方案。
  - 没有 warning 通道，无法提示被前面规则完全遮挡的规则；数组变化只按整体报告。
  - 预览缺 `affectedNodeIds`、`identityChangedNodeIds` 和 `changedListeners`。
  - 新增 VLESS listener 时，`requirements` 同时列出 `willGenerate` 里的 UUID、证书和私钥；console 只把 `willGenerate` 之外的项显示为待完成。
  - `ListenerView` 只给出 WebSocket 响应头的名字，而 `ListenerUpdate` 整体替换，表单没法在保留这些响应头的前提下修改监听。
  - 托管证书模式下修改 `serverName` 时，`inherit_materials` 可能沿用旧证书（读代码推断，未验证）。
- **规则集**：随 resources 的 SQL 下沉处理。
  - 准备失败只返回一条英文 issue，消息里是 Debug 枚举名（如 `InvalidSource`、`DownloadFailed`），没有 `ResourceProblem` code。
  - 超出上限的处理不一致：超过 32 个、超过 8 MiB 或准备并发已满时返回 503，30 秒超时却作为 `/ruleSets` 上的 issue 返回。
  - 检查失败的记录不跨版本保留，换到新版本后变成 `not_ready`、`checkedAt: null`；成功的绑定则会延续。
  - `refreshRuleSet` 没有请求体，也就没有 `requestId`，重试会重新下载。
- **主动观察与历史**：observations 模块冻结，待 Node 实现后按真实消费者重做。
  - 429 `observation_limit` 同时表示续租过早、预算用满、已到终态和 4096 窗口满。
  - 创建时目标没有可用路径（例如目标没有 listener）返回不带参数的 403 `permission_denied`，与"未授权"无法区分，`DemandDecision` 的原因（`noPath`、`missingCapability` 等）没有带出。
  - 订阅因源节点重新 acquire 而结束时只是被移除（404），不说明原因；样本只给 `latest`，重载或接管后看不到之前的序列。
  - `HistoryBucket` 没有 `peerNodeId`，而且按 instance 和 publication 拆开，同一条曲线因每次重新发布被切碎，console 端做了合并。
  - `RecentLink` 没有 RTT 和关闭原因。
  - 一次查询最多 10000 行，计数在按 kind 或节点过滤之前做，繁忙的网络即使只查一个节点也会收到 429；窗口外的清理记录也会返回；bucket 把 `pending` 计入 `failed`。

### 需要产品决定

- GeoIP／ASN 数据集的部署入口（CLI 还是管理 API）。`IpDatasets::install` 现在没有调用方，所以所有节点都没有位置。
- 设备授权的浏览器链接格式。console 暂用 `/authorize-device/<enrollmentRequestId>`，需要和 Agent、App 一起定下来。
- 两端都报告自己是发起方时 link 的判定：现在判为 `conflicting`，`inUse` 变成 null。
- `--listen` 的默认端口：仍是 `127.0.0.1:8420`，与现在运行的旧服务冲突。

### 其他仍存在

- 降级 admin 时，它签发的未用链接会被写上 `revokedAt`，但 inspect 对这些链接返回 `join_link_issuer_unavailable`，与列表里的"已撤销"说法不一致；角色变更也没有影响预览，不会告诉操作者哪些链接会被撤销。
- 给 Agent 实现方的提醒：每个节点最多 8 个会话、8 个 challenge。
