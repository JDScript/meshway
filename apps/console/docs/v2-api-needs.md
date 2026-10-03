# V2 API：Console 需要的接口

本页由 console 工作流维护，内容依据 `docs/v2`（2026-10-03，普通 apply 版本）。做法是把 console 现在调用的 42 个操作逐个和 V2 对照，再补上 V2 新功能需要的接口。

- 路径省略 `/api/v1` 前缀；字段名只是示意。
- 标记含义：**缺失**是 V2 里还没有的接口；**重定义**是 V2 有这个接口，但形状不够 console 用；**决定**是需要用户拍板的产品选择。

## 需要先定的产品问题

| # | 问题 | console 的建议 |
| --- | --- | --- |
| D1 | 不再有邀请注册之后，已有账户怎么加入一个网络 | 用一次性、有期限、绑定角色的加入链接，由已登录用户接受。不提供"按邮箱直接添加"，否则会暴露某个邮箱是否已注册 |
| D2 | 账户恢复（没有邮件） | 由运维在 Controller 主机上用 CLI 生成重置链接；console 只保留兑换页 `/reset-password`。网络 Owner 不能重置其他网络里成员的密码 |
| D3 | 按需建连、空闲回收之后，没有业务的网络几乎没有 links，而概览以连线为视觉中心 | 当前的 links 画实线；最近一段时间出现过的 links 画淡线，数据来自历史事件。不画组网规则允许的全部连接（数量可达 N²） |
| D4 | 删除节点的含义（V2 写着"待确认"） | 只保留两个操作：停用，即 `enabled=false`，可回滚；删除，即永久吊销并在同一个事务里移除配置项，不可恢复。不单独提供"仅从配置移除" |
| D5 | 草稿的粒度 | 整个网络一份草稿，跨页面保留，只有一个"应用"按钮；草稿按有类型的修改操作保存，便于在新 revision 上重放 |

## 通用约定（重定义）

1. **视图里带"能执行的操作"。** 每个资源视图附上由服务端计算的 `actions`，例如：

   ```json
   { "editSettings": true, "assignTags": ["tag:a"], "removeLastTag": false, "disable": true, "revoke": true, "diagnose": true, "selectProfile": ["mesh", "work"] }
   ```

   Tag owner 只能操作自己的设备、最后一个 Tag 不能删、普通成员只能诊断自己的设备：这些规则只由服务端实现一次，console 不再复刻。服务端执行时照常重新检查。

2. **统一的 `Problem`。** HTTP 错误、配置诊断和 State 里的 `problems` 都用同一结构：

   ```json
   { "code": "hostname_taken", "pointer": "/nodes/<id>/hostname", "params": { "hostname": "web" }, "message": "…" }
   ```

   - `code` 是每个操作的封闭枚举，`params` 带类型。
   - `message` 只作后备文本，不回显秘密。
   - 生成的 SDK 要把错误 body 校验后交给调用方。现在的 SDK 会把它丢掉。

3. **创建类请求都带 `requestId`**，用于幂等：建网络、签发接入密钥、生成加入链接都适用。
4. **字段统一用 camelCase**，SSE 载荷也一样。现行 API 是 snake_case。
5. **规则是否带 id 要一致。** access 和 topology 已去掉 id；`routing.mesh` 和出口规则仍然有 id。两种做法要么统一，要么在文档里写明理由。console 定位规则统一用"revision + JSON Pointer"。

## 现行操作对照

| console 现在用的 | 在 V2 里 | 说明 |
| --- | --- | --- |
| `loginAccount`、`logoutAccount`、`getAccountSession` | 保留：`/auth/login`、`/auth/logout`、`/auth/session` | |
| `registerInvitedAccount`、`acceptAccountInvitation`、邀请的增删查 | 邀请注册取消；**加入网络缺失** | D1，§3 |
| `claimFirstOwner`、实例级密码重置、`/setup` 页 | 取消 | D2 |
| `redeemAccountPasswordReset` | 待定 | D2 |
| `changeAccountPassword`、`revokeAccountSessions` | **缺失** | §1 |
| `listNetworks`、`createNetwork` | 保留 | 建网补 `requestId` |
| `getNetwork`、`saveNetwork`（名称和 DNS） | DNS 移入配置；**改名缺失** | §2 |
| `listAccountMembers`、`saveAccountMember`、`leaveAccountNetwork` | **缺失** | §3 |
| `claimNodeOwnership`、`listNodeOwnership` | 取消，改为 `nodes[id].identity`；更换用户须由设备重新登录 | |
| `getNetworkPolicy`、`saveNetworkPolicy` | 合入 `GET/POST /configuration` | §4 |
| `getConnections`、`saveConnections` | 合入 topology；手工候选取消 | |
| `getManagedListeners`、`saveManagedListener` | 合入 `nodes[id].listeners` | §4 |
| `getTags`、`saveTags`、`setNodeTags` | 合入 `tagsOwners` 和 `identity.tags` | §4 |
| `setNodeCountry` | 取消，改用 GeoIP | §7 |
| `listNodes` | 由 Node 实体、配置和 State 拼出；**实体部分读不到** | §5 |
| `revokeNode` | **缺失** | D4，§5 |
| `createEnrollmentToken`、`createManagedEnrollmentToken` | 只有 `EnrollmentKey` 表结构，**没有接口** | §5 |
| `authorizeDevice` | `POST /device-authorizations/{id}/approve`；**读取请求详情缺失** | §5 |
| `getConnectionRuntimes`、`getEndpointDiscoveries` | 改由 SSE 推送 State | §7 |
| `queryLatencyHistory` | **历史查询缺失** | §8 |

## 1. 账户

- **重定义** `GET /auth/options`：在 `registrationEnabled` 之外，再给出密码规则（如最短长度），console 提交前就能校验。
- **重定义** `POST /auth/register` 的错误码：`registration_disabled`、`password_weak`、`email_taken`。是否返回 `email_taken` 需要权衡账户枚举风险，但必须明确选一种。
- **缺失**：
  - `POST /account/password`：提交当前密码和新密码，并说明其他会话是否随之失效。
  - `POST /account/email`：需要验证密码。改完后由系统产生新的配置版本，同步可读的邮箱引用，并发送 `configuration.changed`（actor 为 system）。
  - `GET /account/sessions`：列出会话，字段为 `sessionId`（不是令牌）、`createdAt`、`lastUsedAt`、`expiresAt`、`current`、客户端摘要。
  - `POST /account/sessions/{id}/revoke`，以及"退出其他会话"。

## 2. 网络

- **重定义** `GET /networks`，每一项包含 `networkId`、`name`、`role`、`createdAt`。
- **重定义** `POST /networks`，请求为 `{name, requestId}`，返回网络和初始 revision。
- **缺失**：
  - `POST /networks/{id}/rename`：网络名称是实体元数据，不在配置里。
  - `POST /networks/{id}/leave`：服务端保护"最后一个 Owner"。
  - `POST /networks/{id}/delete`：数据库已有 `deletedAt` 字段，但没有操作。需要说明删除网络时节点凭据怎么处理，以及保留期多长。

## 3. 成员

- **缺失** `GET /networks/{id}/members`：字段为 `membershipId`、`userId`、`email`、`role`、`joinedAt`、`actions`。所属 groups 由 console 根据配置推出。
- **缺失**：
  - `POST /networks/{id}/members/{membershipId}/role`；
  - `POST /networks/{id}/members/{membershipId}/remove`。
- **缺失**，按 D1 方案：
  - `POST /networks/{id}/join-links`：请求为 `{role, expiresIn, requestId}`，原值只返回一次。
  - `GET /networks/{id}/join-links`：只给元数据，状态为待使用、已使用、已过期、已撤销。
  - `POST /networks/{id}/join-links/{linkId}/revoke`。
  - `POST /join-links/redeem {token}`：由已登录用户调用。链接不能绕过"注册已关闭"的开关。

## 4. 配置读写

**三种读取，类型分开（重定义）。**

| 视图 | 用途 | 取得方式 |
| --- | --- | --- |
| `ConfigurationView` | 展示；所有能读的人 | SSE 的 `config.snapshot`；`GET /configuration?revision=` |
| `EditableConfiguration` | 管理员进入编辑 | `GET /configuration/source?revision=`，返回 `{revision, sourceJsonc, configuration}`，含秘密；console 不缓存、不写入 LocalStorage |
| `JSON Schema` 与承载目录 | 表单约束、JSONC 补全 | `GET /configuration-schema`、`GET /transport-catalog`，由 Rust 生成；目录只列已托管的组合 |

脱敏字段用单独的类型表示，例如 `{ "present": true, "fingerprint": "…" }`，不能拿占位值当真实值回写。

**两种提交（重定义）。**

- **整份提交**：`POST /configuration {expectedRevision, sourceJsonc}`，供 JSONC 编辑器使用。适合中小网络。
- **有类型的操作**：`POST /configuration {expectedRevision, ops}`，供表单使用。服务端把操作作用到 JSONC 上，并保留注释。
  - 原因：一万个节点时整份 JSONC 约 10 MB，不能为了改一个节点就下载整份。
  - Tag owner 的局部权限也落在具体操作上（`setIdentityTags`）。
- 建议的操作：

  | 操作 | 作用 |
  | --- | --- |
  | `replace(pointer, value)` | 整段替换 access、topology、routing、dns、defaults、groups、tagsOwners、ruleSets |
  | `setNode(nodeId, fields)` | 修改 hostname、remark、enabled、exitEnabled、dialers |
  | `setListener(nodeId, key, listener \| null)` | 增、改、删 listener |
  | `setIdentityTags(nodeId, tags)` | 设置节点的 Tag |
  | `setAddressPools(pools)` | 修改地址池 |
  | `setAddresses(nodeId, {ipv4?, ipv6?})` | 指定节点地址 |
  | `reassignAddresses(nodeIds, families)` | 重新分配节点地址 |
  | `rotateMaterial(nodeId, key)` | 轮换 listener 材料 |

- **冲突处理（提议）。** 现在的规定是 CAS 冲突就返回 409，那么任何人改了任何一个节点，其他管理员的草稿都会冲突。建议：
  - 如果从 `expectedRevision` 到当前版本之间，这些操作碰到的 JSON Pointer 都没有变化，服务端在当前版本上重放，返回 `rebasedFrom`。
  - 否则返回 409，附 `currentRevision` 和发生变化的 pointer 列表。

**提交前检查（缺失）。** `POST /configuration/validate`，请求与提交相同，返回：

- 诊断：`Problem[]`，带 pointer。
- 影响范围：
  - 节点配置会变的节点；
  - 因改址而中断会话的节点；
  - 会重建、材料会变的 listener；
  - 身份切换；
  - 停用或删除的节点。
- 不生成材料；只读查询本来就不应生成材料。

普通 apply 没有准备阶段，一提交就直接安装，所以这一步要放在"应用"之前。

**策略验证（缺失）。** `POST /configuration/evaluate`，输入源、目的、协议和端口，可以针对草稿或已保存版本。返回：

- 命中的 access 规则；
- topology 的判定结果；
- 命中的 routing 规则；
- 规划的完整路径，或出口及中转。

结果标明是"规划"，不是实测。

**提交结果（重定义）。** 返回 `{revision, publicationId | null, targetCount, allocations}`：

- `publicationId` 为 null 表示没有目标节点，console 显示"无需节点部署"。
- `allocations` 是本次实际分配的地址。

**历史与回滚（缺失）。**

- `GET /configuration/revisions?cursor=`：每一项包含 revision、时间、actor、`rollbackFrom`、改动的顶层区段。
- `GET /configuration/diff?from=&to=`：由服务端计算，秘密只显示是否变化。
- `POST /configuration/rollback {fromRevision, expectedRevision}`：生成新 revision；失败时返回 422，列出缺失的依赖（已吊销的材料、失效的用户、缺失的规则集）。

回滚不能简单做成"把旧 JSONC 载入草稿再保存"，因为用户、listener 代际和规则集的绑定要明确选定。

## 5. 节点生命周期

- **缺失：节点实体的读取。** 配置的 key 和 State 都不包含实体字段：`registeredAt`、`createdBy`、`credentialsRevokedAt`。可以放进 SSE snapshot 的 `nodes` 部分，或者提供 `GET /nodes`。还要定义三种不一致的状态怎么展示：
  - 有实体，没有配置项；
  - 有配置项，凭据已吊销；
  - 有实体，还没有任何报告。
- **缺失** `POST /nodes/{id}/revoke {expectedRevision}`：按 D4，在同一事务里吊销凭据并移除配置项，生成新 revision。
- **缺失：接入密钥。**
  - `POST /enrollment-keys {tags, expiresIn, initialSettings?, requestId}`：原值只返回一次，同时附带安装命令。
    - `initialSettings` 包括 `hostname`，以及可选的 listener。
    - 用户已确认"添加节点时提供可选 listener 配置"；V2 的接入密钥只有 `tags`，无法表达这一点。
  - `GET /enrollment-keys`：只给元数据。状态为 pending、consumed、expired、revoked，并给出 `consumedByNodeId`。
  - `POST /enrollment-keys/{id}/revoke`。
- **缺失：一行安装。** `GET /install.sh` 加 Agent 二进制（附 SHA-256）。接入密钥通过环境变量或 stdin 传给 Agent，不出现在进程参数里。
- **重定义：设备授权页。**
  - **缺失** `GET /device-authorizations/{requestId}`：给出设备自报的名称、系统和 Agent 版本，以及 `expiresAt`、返回方式（callback 或 poll），还有可选择的网络列表（含各自的 `actions`）。需要登录。
  - `approve` 的请求为 `{networkId}`。
  - **缺失** `deny`。
  - poll 方式批准后，页面提示用户回到终端。
- **hostname**：首次接入时由服务端加后缀保证唯一；改名走 `setNode`；重名返回 `hostname_taken`。

## 6. 应用进度

- **缺失** `GET /publications?cause=configuration&cursor=`，作为"应用记录"。每一项包含 revision、时间、actor、`targetCount`、各阶段计数、`supersededBy`。V2 里只能按 ID 读取单个。
- **重定义** publication 视图中每个节点的进度：
  - `phase`、`step`、`problem`、`observedAt`；
  - 实际 `applied` 引用；
  - 联系状态，由服务端根据 `lastAuthenticatedAt` 判定，console 不自己猜阈值。
- **重定义** 汇总计数带 `scope: full | partial`。普通成员只看到部分节点，界面要标明范围。
- **重定义** 被新发布替代的旧发布明确标 `superseded`，避免它一直显示"未确认"。

## 7. State 视图

- **重定义** `location`，即 GeoObservation：现在只有 `country`，地球落点需要 `latitude`、`longitude` 和精度（或 `city`/`region`）；拿不到时给出 `absentReason`：`no_observation`、`stale`、`not_in_dataset`。
- **重定义** 网络状态视图要包含 `lastAuthenticatedAt`。内存字段里有，但 `state-reference/network.md` 的视图里没有列出。
- **缺失** `versionStatus`：current、outdated、unsupported，由服务端按兼容门槛判定。
- **重定义 links**：提供按节点对聚合的视图，字段为 `{a, b, transport, initiator, phase, evidence: oneSided|matched|conflicting|expired, inUse, since}`，console 不再自己拼两端报告。
- **缺失** `recentLinks`（按 D3）：按节点对给出有界的最近记录：`lastSeenAt`、`lastTransport`、`lastRttUs`、关闭原因。数据来自历史事件。
- **重定义 paths**：按（源，目的）聚合，保留每个报告者的角色和证据阶段。
- **缺失** `ProblemCode` 目录，也就是首批 code 和对应的 `params`。

## 8. 历史

**缺失** `POST /history/query`：

- 请求：`{kinds, from, to, nodeId?, peerId?, linkId?, bucketSeconds?, cursor, limit}`。
- 返回：
  - 原始事件，或分桶数据（含 `sampleCount`）；
  - 覆盖缺口（`HistoryCoverage`），让图表区分"未采集"和"空闲"；
  - 连接事件，如 `link_opened`、`link_closed`、`dial_failed`，用于时间线。

## 9. 实时观察

- **重定义创建请求**：允许 `{sourceNodeId, target, mode: "current" | "explicit", path?}`。网络空闲时还没有路径，用户无法先选一条；由 Controller 计算候选并返回实际选定的路径。
- **重定义返回**：`{observationId, intervalMs, expiresAt, renewAfter}`，由服务端给出续租节奏。console 只在页面可见时续租。
- **缺失**：每个用户的并发上限，以及超出上限时的错误码。

## 10. 路由方案选择

**缺失** `POST /nodes/{id}/routing-selection {profile, expectedVersion}`：普通成员只能用于自己的设备，管理员按配置约束操作。

节点视图里要给出：

- `requestedProfile`、`appliedProfile`；
- 允许的方案；
- `locked`；
- 自动方案的判断依据。

## 11. SSE 补充

- **重定义 snapshot 的内容**，在 Config 和 State 之外再加入：
  - 节点实体（§5）；
  - 成员（§3），这样角色变化能实时生效；
  - 当前 publication；
  - 本用户在本网络的 `actions`。
- **缺失的事件**：
  - `node.registered`、`node.revoked`；
  - `members.changed`；
  - `enrollmentKey.consumed`：添加节点页据此把状态从"待接入"切换为"已接入 → 节点"；
  - `joinLink.redeemed`。
- **重定义** `configuration.changed`：带上改动的 pointer 列表，例如 `["/access", "/nodes/<id>"]`。console 据此判断草稿是否冲突，不用先下载新版本。
- **重定义** `sync.ready` 给出服务端保活间隔，客户端据此设置断流超时。
- **重定义** `stream.closed` 和 `sync.reset` 带原因码，例如 `session_expired`、`membership_revoked`、`cursor_too_old`；console 分别跳转登录页、离开网络或重新同步。
- **生成要求**：TS 客户端要能生成事件联合的 Zod schema 和流式读取。分帧、重连和游标这一层放在 SDK 运行时，console 不手写 DTO。
