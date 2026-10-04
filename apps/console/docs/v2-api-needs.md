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

## 协调线程答复：用户确认后的 V2 方向（2026-10-03）

本节按用户最新答复追加，前文保留为原始需求。以下是设计确认与修订，**不表示接口已经交付**；具体契约由 Rust routes／DTO 生成。规范入口：[API 索引](../../../docs/v2/api.md)、[编辑与应用对照](../../../docs/v2/configuration-reference/editing.md)。

### D1–D5

| 项目 | 结论 |
| --- | --- |
| D1 | 同意：一次性、有期限、绑定网络／角色的加入链接，由已登录用户接受；不提供按邮箱直接添加。不恢复邀请注册，也不绕过 registrationEnabled。链接不绑定特定收件人，持有人可以消费，签发／消费均检查角色和当前权限。 |
| D2 | 同意：Controller 主机运维 CLI 生成重置链接，Console 提供 `/reset-password` 兑换页。网络 Owner 不能重置他人的全局账户密码，即使对方只加入本网。 |
| D3 | 同意：当前真实 links 与有明确时间范围的历史淡线分开，历史层可切换。只有真实历史事件才画线，不展开 N² 允许图、不为画线额外探测。 |
| D4 | 用户要求直接删除、不做节点恢复：提交时永久吊销凭据并移除 nodes[id]，同事务处理旧地址退出／执行撤回。没有回收站或“仅移除配置”入口；最终 JSONC／ops 删去已有 Node 也具有相同删除语义。最小吊销／历史引用记录仍保留以拒绝旧身份，重新接入是新 Node／密钥。 |
| D5 | 同意：每个用户编辑上下文、每张网络一份草稿，跨页面保留，一个应用入口。**最终应用前必须展示有效修改数量、前后对照、受影响节点，并单列永久删除／改址／身份和材料变化。** 无效 JSONC 和注释必须保留，不能只用 ops 重建文本。 |

这里“不考虑回滚”指 Node 删除不可恢复；配置版本历史仍保留。历史目标含已删除 Node 时不能复活；若历史恢复的最终差异会移除当前 Node，也按删除语义列在应用对照中。

### 对通用建议的修订

1. **actions 接受**，但按资源类型定义固定 schema，服务端 mutation 继续实时检查权限。
2. **Problem 共享核心接受**，HTTP 保留 requestId，配置诊断保留 severity／pointer，State 保留资源／实例／观测时间；不把三类上下文压成相同含义。错误 body 经生成 schema 校验后交给调用方。
3. **强类型 ops 接受**，`replace(pointer,value)` 改成 replaceAccess／replaceTopology／replaceRouting／replaceDns 等明确分支，禁止任意 JSON。Tag owner 权限限定到具体操作。
4. **严格 CAS 保留，不采用按 pointer 未变就自动提交的 rebase。** access／groups／defaults 等间接读取依赖也可能改变结果。可以辅助重放为新草稿，但要重新验证、展示对照并由用户应用；SSE changedPointers 只是提示。
5. 创建请求带 requestId。接入密钥／加入链接原值只在首次创建响应交付；若响应丢失，重试返回同一资源的元数据和“原值不可重取”，不生成第二个资源。需要新原值时撤销旧项再签发，不能声称仅有摘要也能恢复秘密。
6. 字段统一 camelCase。access、topology、Mesh 路由及出口规则统一用 remark，定位用 revision＋JSON Pointer；命名 profile／ruleSet／listener 的 map key 保留。

### 接口与 State 补齐

- 补齐账户／会话、网络／成员、加入链接、Node 读取／删除、接入密钥、设备批准详情／拒绝、完整管理读取、validate／evaluate、版本列表／diff／恢复、publication 列表、历史查询、路由方案选择和下载目录。以 [API 索引](../../../docs/v2/api.md) 为路径收敛处，不并行维护别名。
- validate 既返回诊断，也返回修改数量／对照摘要和影响范围；不生成材料、不预留地址，最终提交仍重验。操作只是普通 apply 的应用前预览，不恢复两阶段。
- GeoIP／ASN 由 Controller 对合格 endpoint IP 派生，节点位置保留来源、时间、精度／缺失原因；声明入口的地理信息不自动成为设备位置。见 [IP 信息](../../../docs/v2/state-reference/ip-info.md)。
- 联系状态、versionStatus、links／paths 聚合由服务端生成。聚合保留 linkId／实例／承载／发起方／完整路径／每个报告者证据，不能按节点对压掉主备或并存路径；未知 since／RTT 不补值。
- recentLinks 只来自真实有界历史，标为历史而非 active。HistoryCoverage 只说明保留／采集缺口；空闲须真实事件支持，不能由“无样本”推出。
- 实时观察使用 **auto／explicit**：auto 按目标计算获准候选并由执行端选择／建立，explicit 固定路径。auto 首次选定后按该路径采样，重选须新分段；跟随某条已有业务流不是 current 的模糊默认。
- publication 的计数带 full／partial 范围，superseded 明确显示。无部署目标用 targetCount=0／空 publication 引用表示，不把缺少即时计划或离线节点误标为“无需部署”。
- snapshot 补齐当前可见节点实体、成员、actions 和相关 publication；加入链接／密钥事件仅发给当前有权订阅者。SSE 给出保活／reset／关闭原因，历史列表仍通过有界 cursor 查询。

仍需工程定版的是 Node 持钥协议、预算／保留周期和精确字段编码；网络删除／账户注销另有产品生命周期，不能从 D4 的节点删除自动扩展。Console 源码／现行 SDK 本次不改。

## Console 第二轮复核（2026-10-03）

对照更新后的 `docs/v2/api.md`、`api/authentication.md`、`api/history.md`、`api/problems.md`、`configuration-reference/editing.md` 和 `state-reference/ip-info.md`，第一轮的 D1–D5 以及 §1–§10 基本都已覆盖。剩下的问题如下，按影响从大到小排列。

**状态（按 docs/v2 后续修改更新）**

后续修改涉及 `state-reference/network.md` 的"Console 组合视图"、`storage/database.md`（会话、成员 version、JoinLink、接入密钥的 initialSettings、ActivityEvent）、`changedSections`、rule remark 和观察任务的 mode 字段。逐项状态：

| # | 状态 |
| --- | --- |
| 1 SSE 文档 | 已处理：新增 `entities.snapshot`、节点／成员／密钥／链接／活动事件、保活间隔、reset 与 close 原因，`configuration.changed` 带 changedPointers、changesComplete、sourceChanged |
| 2 系统 revision 冲突 | 部分处理：版本摘要有了 `changedSections`，ActorRef 能区分出接入密钥。仍缺 `cause`，因为 system actor 分不清改邮箱、移除成员和身份转换；冲突频率的问题也还没有决定 |
| 3 多个 publicationIds | 未处理 |
| 4 validate 返回规范化视图 | 未处理：`editing.md` 仍写"前后值来自类型化读取与草稿" |
| 5 加入链接预览 | 已处理：`POST /join-links/inspect` |
| 6 GET 与 SSE 同一类型 | 已处理：`network.md` 的组合视图写明 SSE 和 GET 共用 |
| 7 秘密只写不读 | 未回复：`editing.md` 仍保留"轻量 ops 编辑可按需读取对应 Node 或区段" |
| 8 JSONC 只做整份文档 | 已处理：`synchronization.md` 写明"JSONC 使用整份文本" |
| 9 移除成员的影响 | 部分处理：Membership 有了 `version`；`members/{id}/remove` 仍只写"联动 User 身份节点资格" |
| 10 安装命令、hostname 冲突 | 未处理 |
| 错字（见下文"新发现"） | 已修正 |

本节末尾"新发现"一共三点。

### 1. SSE 文档还没同步（`api/events.md`）

答复里已经同意的内容，事件文档还没有反映：

- Full snapshot 表缺少四项：节点实体、成员、actions、相关 publication。
- 事件表缺少：
  - `node.registered`、`node.deleted`；
  - `members.changed`；
  - `enrollmentKey.consumed`、`joinLink.redeemed`；
  - `activity.created`（目前只写在 `history.md` 里）。
- `configuration.changed` 还没有 `changedPointers`。
- `sync.ready` 还没有给出保活间隔。
- `stream.closed` 和 `sync.reset` 的原因码没有对应到 `problems.md`。

### 2. 系统产生的 revision 会让草稿频繁冲突（需要决定）

严格 CAS 下，任何新 revision 都会让所有打开的草稿冲突。按目前的设计，下面这些动作也会推进配置 revision：

- 新节点接入（写入 `nodes[id]`）；
- 改邮箱（同步可读引用）；
- 删除节点；
- 移除成员，如果它会联动修改 User 身份节点。

也就是说，管理员正在改策略时，只要有一台设备接入，草稿就作废。

建议：

- `ConfigurationRevisionSummary` 和 `configuration.changed` 都带上 `cause`，取值为 `edit | restore | enrollment | nodeDeleted | emailSync | memberRemoved | identityTransition`。console 据此说明冲突原因，例如"期间接入了节点 X"。
- console 的处理：草稿基于旧 revision 时，用同一批 ops 在新 revision 上重新调用 validate，展示新的对照，再由用户点"应用"。这仍然是严格 CAS，只是在冲突来自系统动作时让用户少做几步。
- 需要确认的是：接入、改邮箱这类系统写入是否一定要占用一个配置 revision。如果是，就接受上面的做法。

### 3. 一次提交返回多个 `publicationIds`

console 的"应用进度"是按一次提交来展示的。需要后端二选一：

- 保证 `cause=configuration` 的提交只产生一个 publication；或者
- 提供按 revision 聚合的进度：`GET /configuration/revisions/{revision}/progress`，或者 publication 列表支持 `revision=` 过滤并返回去重后的节点集合与计数。

否则 console 要自己合并多个 publication 并对节点去重，计数容易出错。

### 4. validate 要返回规范化之后的结果

JSONC 草稿提交后，默认值补全、继承规则、hostname 加后缀、地址"将分配"这些规范化都在 Rust 配置库里完成。console 不应该再实现一遍。

建议 validate 额外返回 `preview: ConfigurationView`：这是草稿规范化后的**脱敏视图**，类型和普通视图相同，不是任意 JSON。console 用"基准视图 + preview"渲染前后对照。秘密字段仍只显示"保持、修改、将生成"。

### 5. 加入链接缺少兑换前的预览

`authentication.md` 要求"前端明确显示将加入的网络和角色"，但目前只有会直接消费链接的 `POST /join-links/redeem`。

需要一个不消费的预览接口：`POST /join-links/inspect {token}`。token 放在请求体里，返回网络名称、角色、`expiresAt` 和状态（有效、已过期、已使用、已撤销、已是成员）。重置链接可以照样提供 `POST /auth/password-reset/inspect`，让用户在输入新密码之前就知道链接已经失效；这一项优先级较低。

### 6. GET 视图和 SSE snapshot 要用同一组类型

`GET /nodes` 被描述为"组合实体、配置摘要与 State"，而 SSE 分别推送 config 和 state。两种形状意味着 console 的缓存要写两套代码。

建议：GET 返回的就是 snapshot 的同类片段，即 entity、settings 视图、state 三元组，以及相同的 actions，并带 `asOf`、`revision` 和 `stateEpoch`。console 以 SSE 为主，GET 只在打开流之前或单次读取时使用，比如设备批准页。

### 7. 表单里的秘密字段只写不读（提议）

`inheritance.md` 规定：对已有资源再次保存时，省略自动材料就表示沿用原值。所以表单可以做到秘密字段只写不读：

- 默认显示"保持"；
- 管理员可以填入新值；
- 或者用 `rotateMaterial` 重新生成。

这样编辑 listener 不需要读取任何秘密，也就不需要"按节点读取可编辑区段"的接口。读取秘密只通过整份 `configuration/source`，也就是 JSONC 编辑。需要确认这条规则。

### 8. JSONC 编辑只做整份文档（建议）

`replaceAccess` 这类操作会丢掉该区段里的注释，所以按区段编辑 JSONC 不成立。建议首版的 JSONC 编辑只针对整份文档：

- 只有一个编辑器，从某一层进入时定位到对应的 pointer，例如 `/access`，并折叠其他区段。
- 大网络主要靠表单（ops）编辑。

这与 AGENTS"万级节点先保持简单"的方向一致。如果以后需要按区段编辑 JSONC，再加一个按封闭区段枚举的 `replaceSectionSource`，不开放任意 pointer。

### 9. 移除成员的影响

`members/{id}/remove` 写着"联动 User 身份节点资格"，但没有说明会发生什么：这些节点是被停用、删除，还是保留但不可用？会不会生成配置 revision？

console 的确认弹窗需要显示"该成员的 N 台设备将……"，因此需要：

- 成员视图带 `nodeCount`，或者 actions 附带影响说明；
- 修改角色需要的 `version` 字段出现在成员列表里。

### 10. 接入相关的细节

- **安装命令的格式。** `sudo MESHWAY_AUTH_KEY=… sh` 会让密钥出现在 sudo 的进程参数和 shell 历史里，和"不放进进程参数"的要求相冲突。需要发布流水线定一个标准命令，console 照着展示，不自己拼写。例如一行命令只负责安装，密钥在提示符下输入，或者从标准输入读入。
- **`initialSettings.hostname` 冲突。** 消费接入密钥时如果 hostname 已被占用，是自动加后缀还是接入失败，需要明确。

### 已确认、console 侧采纳

- 严格 CAS。console 的辅助重放就是"同一批 ops + 新的 validate + 用户再次应用"。
- 删除节点：如果当前没有草稿，直接调用删除接口；如果已有草稿，把 `deleteNode` 加进草稿，避免自己的草稿和自己的删除操作冲突。
- 地球：当前的 links 画实线；历史层可以切换，并标明时间范围。没有坐标时，按地区聚合显示或标为"待定位"，不生成假的坐标。

### 新发现

- **recentLinks 的窗口。** `network.md` 写的是"有界窗口并可切换"，但没说明窗口由谁选、怎么返回。需要明确两点：
  - 视图里带上 `window: {from, to}`；
  - 打开历史图层时，是在 SSE 订阅的 `include` 里加 recentLinks，还是单独查询。

  console 的做法是：默认关闭历史层；打开时按所选时间范围（例如 24 小时）请求，并在图例里标出这个范围。
- **错字。** `api/history.md` 第 21 行"最终差异移除当前 Node 則按永久删除执行"里的"則"应为"则"。（已修正）
- **entities 只有 snapshot，没有增量。** 节点和成员上的 actions 也会因为配置变化而改变，例如节点加上 Tag 之后，能执行的操作随之变化，但目前只有 `members.changed` 写了"重新读取 actions"。建议增加 `entities.delta`，形状和 `state.delta` 一样，按项整体替换，并显式列出删除项。配置变更、角色变化、节点注册或删除之后，由服务端推送受影响的实体和 actions，不要让 console 混用 GET 和 SSE 的游标去补读。

## 协调线程收敛答复：第二轮与实体增量（2026-10-03）

以下以包含本节的已提交版本为复核基准；前面的状态表保留为当时的复核记录，不覆盖 console 原文。

| 复核项 | 收敛结果与位置 |
| --- | --- |
| 系统 revision／cause | 实际修改配置的接入、邮箱同步、删除、身份转换等仍产生版本。ConfigurationVersion／RevisionSummary／configuration.changed 增加 cause。严格 CAS，旧基准不能直接提交；草稿保留，可用原 ops 在新基准 validate，展示新对照后由用户应用。见 `editing.md`、`api/history.md`。 |
| 多 publication 汇总 | 增加 `GET /configuration/revisions/{revision}/progress`，服务端按 NodeId 去重，返回精确目标／实际引用、替代关系与 full／partial 计数；不由 console 拼计数，不把后续 demand／refresh 混进原分母。 |
| 规范化预览 | validate 返回 `preview: ConfigurationView`，由共享 Rust 库规范化并脱敏；待生成值明确为计划，不持久化或预留。见 `editing.md`。 |
| 秘密与 JSONC | 表单只保持／覆盖／轮换，不读取旧秘密。节点配置 GET 为非秘密视图；完整秘密读取仅整份 configuration/source。JSONC 只有一个整份编辑器，进入具体项时定位 pointer。 |
| 成员移除 | 永久删除当前属于该 membership 的 User 身份设备，Tag 身份设备保留；同事务处理 membership、配置和退出。成员列表返回 version，有权执行者取得 removalImpact／nodeCount；只有实际配置变化才产生 revision。 |
| 安装与 hostname | 标准模板 `curl -fsSL https://controller.example/install.sh \| sudo sh` 仅含公开安装信息；接入另用浏览器或 `/dev/tty` 隐藏输入，不能混读脚本 stdin、放 argv／history。首次 initialSettings.hostname 作为建议，冲突加唯一后缀；已有节点改名冲突仍报错。 |
| recentLinks 窗口 | 历史层默认关闭；用户选 from／to，经 history/query 的 recentLinks 模式取得 window／coverage／cursor／摘要，不放默认实时 SSE。 |
| 加入／重置预览 | 加入链接有 POST /join-links/inspect；重置链接有 POST /auth/password-reset/inspect。两者不消费，最终兑换重验。 |
| SSE 实体增量 | 新增 entities.delta：同一 stream cursor，按类型 updated／removed 整项替换。配置、角色、注册／删除及账户资格变化均重算 actions；失去可见性发送 removed／reset。见 `api/events.md`。 |
| GET 与 SSE 类型 | 复用 entity／settings／state 片段；entity 元数据及 actions 与运行报告分开更新，避免权限更新覆盖更新的 NodeState。 |

`api/events.md` 同步了实体首屏、生命周期／activity 通知、changedPointers／cause、保活间隔和 reset／关闭原因；拼写已修正。实现仍以之后的 Rust 契约／生成 SDK 为准，本次提交是完整设计与跨线程答复，不是已上线 API。

## Console 第三轮复核（2026-10-03）

协调线程的收敛答复逐项核对过，都已写进 `docs/v2`：`cause`、按 revision 汇总的进度、`validate.preview`、表单秘密只写、JSONC 只做整份文档、成员移除的影响、安装命令、hostname 加后缀、recentLinks 走 history/query、两个 inspect 接口、`entities.delta`。剩下两处小的不一致：

- **进度的实时推送。** `configuration.progress` 仍按 publication 推送，按 revision 去重的汇总只能 GET。需要选一种写法：
  - 事件里带上 `revision` 和同一份去重汇总；或者
  - 写明 console 收到进度事件后（合并短时间内的多次事件）重新读取 `GET /configuration/revisions/{revision}/progress`。
- **`api.md` 成员表的两行。**
  - `members/{membershipId}/remove` 一行仍写"联动 User 身份节点资格"，应改为与第 60 行一致的"永久删除其当前 User 身份设备，Tag 设备保留"。
  - `leave` 一行也要写明会删除本人的 User 身份设备，console 在确认弹窗里显示本人的 removalImpact。

## 协调线程最终收尾（2026-10-03）

主批答复与 V2 文档已提交 `82a64e7`。第三轮两项已补：configuration.progress 带 revision 与服务端去重 summary，复用按 revision GET 的类型／计算，节点变化仍按片段推送，Console 不额外 GET 或自行汇总；remove／leave 表格已明确永久删除当前 User 身份设备、保留 Tag 设备及 removalImpact。以下提交包含本次收尾，后续按提交内容核对。

## Console 消费反馈：后端 `ece13cfc`（2026-10-03）

console 已接入到 `ece13cfc` 为止交付的 Console 操作：账户、网络、成员、加入链接、配置（JSONC 与全部区段表单、预览、应用、历史）、节点及其运行 State、接入密钥、设备授权、按 revision 的部署进度、网络 SSE、规则集和观测历史。`ece13cfc` 新增的主动观察（`createObservation` 等）尚未接入。本节取代此前按 `ce785928` 写的清单，只列后端需要处理的问题。

**已修复（不再跟踪）**：
- 活动 action 改用 wire 名；新增 `invalid_current_password`，返回 400。
- 重置链接区分已使用、已过期、已被替换。
- 预览增加 `unavailableDependencies`。
- 移除成员时联动删除其 User 身份设备。
- 会话写入 `clientLabel`。
- 部署进度改为按 revision 计算并经 SSE 推送，不再需要拼凑 publication。
- 规则集支持单次 apply 自动准备，并按周期更新。
- 签发接入密钥时校验 `initialSettings`；拒绝设备只允许批准者撤回自己的批准。这两项依据 `3251179b` 的提交说明和 `api.md`，console 尚未实测。

### A. 影响功能或安全

1. **OpenAPI 组件重名，生成物已经错了。** `crates/configuration` 和 `crates/model` 各有一个 `UnavailableReason`，导出时后者覆盖前者。结果 Agent API 中 `DemandDecision` 的 `unavailable.reason` 只剩 `datasetUnavailable`、`lookupFailed`，原来的 `unauthorized`、`noPath` 等都丢了，生成的客户端随之出错。导出时应检测重名并直接失败。
2. **成员读不到自己设备的配置。**
   - `getConfiguration` 对成员仍返回 403。
   - 成员自身节点的设置只在 SSE 的 `config.snapshot` 里出现，没有对应的 GET。
   - `configuration.changed` 之后不会重发成员视图，console 只能断开重连，做一次完整同步。
3. **SSE**
   - 从不发送 `stream.closed`：会话过期、被移除、角色变化时都只是 EOF。console 要靠重连拿到的 401/404 才知道原因。
   - 每个会话每个网络最多 4 条流，已断开的订阅还占名额 1 分钟。刷新几次或开多个标签页，就会连续收到约 1 分钟的 429。
   - `state.delta` 不带 `asOf`、`sequence`、`configurationRevision`，增量应用后这三项停在快照时的值。
   - 快照不带 requestId，无法作为 GET 响应写入缓存，管理员的配置只能重新读取。
4. **部署与执行**
   - 节点应用配置后若重新 acquire instance，原版本中它会被标为 `superseded`，并一直停在"未回报"，实际上它已经应用。
   - 任一节点发起 demand 都会重新发布整网配置。其他节点立刻变成 `config_mismatch`，links 变成 `expired`，直到它们重新上报；报告有效期只有 60 秒，概览上的状态因此反复跳动。
   - 网络里没有节点时，apply 仍返回 1 个空 publication。
5. **需要生成材料的配置无法预览。** 新增 VLESS listener，或删除后重建（轮换）时，validate 直接返回 422 requirements（uuid、certificate、privateKey、certificateValidation），拿不到 preview。这与 editing.md 中"预览中显示将生成"不一致，界面因此无法完成新增 VLESS。
6. **`DefaultsUpdate.dialers` 是必填。** 只改 `defaults.topology` 也必须原样带回 dialers，可能覆盖别人同时做的修改。建议改为可省略，省略即保持。
7. **GeoIP/ASN 数据集没有部署入口。** `IpDatasets::install` 没有任何调用方，所以所有节点都没有位置。
8. **设备授权的浏览器链接格式未定义。** console 暂用 `/authorize-device/<enrollmentRequestId>`，需要和 Agent/App 一起定版。

### B. 契约和字段

9. **配置诊断**
   - `ConfigurationIssue` 只有英文 message，没有 code 和 params，无法本地化。
   - 路径只到集合一级，解析错误时 path 为空。节点级的出口授权问题挂在 `/nodes/<id>` 上，指不到具体的规则或方案。
   - 没有 warning 通道，无法提示被前面规则完全遮挡的规则。
   - 数组只报整体变化。
   - 预览缺 `affectedNodeIds`、`identityChangedNodeIds`、`changedListeners`；删除节点时，该节点也会出现在 `addressChangedNodeIds` 里。
10. **配置操作**
    - 缺 `reassignAddresses` 和 `rotateMaterial`。
    - `unmanaged` 在普通规则里会被拒绝，但 schema 允许它出现。
11. **Listener**
    - `ListenerView` 只给出 WebSocket 响应头的名字，而 `ListenerUpdate` 整体替换、不继承旧值，所以表单无法在保留这些头的前提下修改监听。
    - 托管证书模式下改 `serverName`，`inherit_materials` 可能沿用旧证书。这是读代码推断的，未验证。
    - 承载目录的 `listenerSchema` 只是一个类型名；目录列出了 trojan，但文档写的是尚未托管。
12. **权限 actions 缺失**
    - `NetworkActions` 没有"可签发哪些角色的加入链接"。
    - `JoinLinkView` 没有撤销权限。
    - 节点只有 `canDelete`，吊销密钥没有单独的权限。
    - 接入密钥没有 actions。console 暂按 `editConfiguration` 判断，controller 实际按 owner/admin 检查。
13. **错误码与幂等**

    | 场景 | 现在返回 | 应返回 |
    | --- | --- | --- |
    | 同一 requestId 带不同请求体提交配置 | `revision_conflict` | `request_id_conflict` |
    | `revokeNodeKey` 代数不匹配 | `request_id_conflict` | 版本冲突类 |
    | 设备批准时请求已不是待批准状态 | `request_id_conflict` | 状态冲突类 |
    | `removeMember` 的 version 或 revision 不匹配 | 一律 `version_conflict` | 分别返回，revision 不匹配时用 `revision_conflict` |
    | 账户会话、活动的游标失效 | `invalid_request` | `invalid_cursor`（与网络列表一致） |

    另外：
    - `inspectJoinLink` 不区分已使用、已撤销、已过期和签发者已失权；"已是成员"要到 redeem 时才返回。
    - `refreshRuleSet` 没有请求体，也就没有 requestId，重试会重新下载。
14. **字段缺失**
    - `EnrollmentRequestView`：缺 `existing` 绑定、批准者和批准的网络、系统、Agent 版本、返回方式；设备取消和用户拒绝都写 `denied_at`，无法区分。
    - `EnrollmentKeyView`：缺使用时间、创建人、接入后的节点 ID，以及 hostname 和 listener 摘要。
    - `BoundNodeKey`：缺创建时间。
    - 成员视角的 StateView 不给对端节点的主机名。
    - 网络活动的 actor 只有 userId，已离开的成员无法显示成邮箱。
    - 列表：加入链接列表没有 total，且按创建时间正序；接入密钥列表没有分页，超过 1000 条返回 503 `busy`。
15. **历史**
    - `HistoryBucket` 没有 `peerNodeId`，而且按 instance 和 publication 拆开。每次 demand 都会重新发布，同一条曲线因此被切碎，console 端做了合并。
    - `RecentLink` 没有 RTT 和关闭原因。
    - 一次查询最多 10000 行，计数在按 kind 或节点过滤之前做，繁忙网络即使只查一个节点也会收到 429。
    - 窗口外的清理记录也会返回；bucket 把 `pending` 计入 `failed`，语义不明确。
16. **规则集**：检查失败的记录不跨版本保留，换到新版本后变成 `not_ready`、`checkedAt: null`；成功的绑定则会延续。
17. **双向同时拨号**：两端都报告自己是发起方时，link 被判为 `conflicting`，`inUse` 变成 null。这种情况需要定义。
18. **给 Agent 实现方的提醒**
    - 接入签名的 intent digest 必须包含 `existing: null`，而生成的 zod 把这个字段标为可省略。
    - 每个节点最多 8 个会话、8 个 challenge。
19. **SDK**：`transport.ts` 使用了 DOM 类型 `ReadableStreamReadResult`，只带 Node 类型的 TS 项目编译不过。
20. **部署**：`--listen` 的默认值仍是 `127.0.0.1:8420`，与现在运行的旧服务冲突。

### C. 尚未交付（console 已按"未提供"如实显示）

可信 DNS 与 GeoIP 数据输入及自动方案、安装脚本与 Agent 发布、`reassignAddresses` / `rotateMaterial`。
