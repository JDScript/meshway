# 配置版本与操作历史

配置历史记录完整目标，操作历史记录已完成的管理动作，观测历史记录真实节点事件。三者使用关联 ID，权限和语义分别处理；具体字段以生成合同为准。

## 配置版本接口

下面路径相对 `/api/v1/networks/{networkId}`，需要 Owner／Admin 管理读取权限。

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| `GET /configuration/revisions` | cursor、limit | 固定上界的版本摘要列表 |
| `GET /configuration/revisions/{revision}` | revision | ConfigurationResponse：networkId、revision 与非秘密 ConfigurationView |
| `GET /configuration/source` | 可选 revision，省略为当前 | 获准管理员读取 JSONC 原文与完整规范化值的可编辑形式，listener 秘密只以[保留标记](/v2/configuration-reference/editing#secret-retained-marker)出现，no-store |
| `POST /configuration/compare` | fromRevision、toRevision | 字段路径级变化摘要，区分语义／原文／绑定变化 |
| `POST /configuration` | expectedRevision＋新草稿，或 restoreFrom 历史引用 | 按普通 apply 保存新版本，返回 revision／publication 引用 |

同一配置提交入口用 `type: "restore"` 和 `restoreFrom: Revision` 恢复历史。可选 resourcePublication 必须同网且对应该源版本，指定某次历史数据集绑定；省略／null 使用源版本初始绑定。恢复按当前身份、材料、地址与授权重验，不默认换成最新规则集。

回滚输入由生成契约区分普通编辑和历史恢复。稳定 UserId／MembershipId 和材料／数据集依据在服务端解析；不能仅复制旧邮箱字符串后按当前账户重新匹配。失效依赖或需要设备证明的身份转换按正常规则拒绝。

Node 删除不可恢复；恢复目标含已删除 Node 时拒绝。最终差异移除当前 Node 则按永久删除执行，应用对照明确展示，不能把历史恢复当作不经过预览的特殊通道。见[整网草稿](/v2/configuration-reference/editing)。

### ConfigurationRevisionSummary

| 字段 | 语义约束（类型以[生成契约](/v2/contracts/openapi.json)为准） |
| --- | --- |
| `revision` | 历史版本 |
| `createdAt` | 持久提交时间 |
| `createdBy` | 真实提交主体 |
| `cause` | networkCreated／edit／restore／enrollment／nodeDeleted／emailSync／memberRemoved／identityTransition |
| `rollbackFrom` | 必填可空；回滚来源，非回滚为 null；不是倒退后的版本号 |
| `current` | 是否等于本次读取截点的期望版本 |

列表不携带整份 JSONC 或秘密。节点是否应用从 publication／NodeState 查询，不能在 revision 列表写一个缺少范围与来源的全网 applied=true。

cause 表示主触发来源，具体影响由 compare 的 changes 表达。系统动作实际改变配置内容或固定绑定就形成版本；纯观测不推进版本。

### Compare

比较默认返回路径／变化类型摘要，不返回任意 JSON before／after 值：

| 字段 | 语义约束（类型以[生成契约](/v2/contracts/openapi.json)为准） |
| --- | --- |
| `requestId` | 此次比较请求关联 |
| `fromRevision` / `toRevision` | 被比较的确切版本 |
| `sourceChanged` | 原始 JSONC 文本或注释是否改变 |
| `configurationChanged` | 规范化配置是否改变 |
| `bindingsChanged` | 固定用户／资源／数据集绑定是否改变 |
| `changes` | 合法字段路径的变化摘要 |

ConfigurationChange 包含 `path`（JSON Pointer）与 `kind`（added／removed／changed）。有序数组重排按数组变化说明，不凭 remark 当主键；秘密字段只报告“已变更”，不把密码或私钥写进差异摘要。完整文本 diff 由获准管理员显式读取两个 source 后查看；source 中的保留标记回写时保持当前已保存值，ConfigurationView 不能整份回写。

## 操作历史接口

| 路径 | 范围与权限 |
| --- | --- |
| `GET /api/v1/networks/{networkId}/activity` | 本网络管理操作；Owner／Admin |
| `GET /api/v1/networks/{networkId}/activity/{activityId}` | 同网单条记录；仍按当前权限检查 |
| `GET /api/v1/account/activity` | 当前用户的全局账户操作，不列出别人的账户历史 |

列表接受 limit 和 cursor。历史 ID、游标或当时拥有的权限，都不能绕过当前网络授权。

### ActivityEvent

| 字段 | 语义约束（类型以[生成契约](/v2/contracts/openapi.json)为准） |
| --- | --- |
| `id` | 不透明记录 ID |
| `at` | Server 完成该动作的时间 |
| `actor` | 用户、节点（Node 为自己执行的动作，目前只有 `routing.select`）或系统 |
| `action` | 已知管理动作枚举 |
| `target` | 类型化目标引用，不是任意 metadata |
| `beforeRevision` | 配置变更前版本；不涉及配置则 null |
| `afterRevision` | 配置变更后版本；不涉及配置则 null |
| `requestId` | 同一次逻辑操作的关联，不是授权凭证 |

scope 由所属记录及路径确定。返回时可以按权限投影主体／目标的可读名称，稳定 ID 始终保留；删除或改名不把记录改指向另一个对象。

用户 actor 返回稳定 `userId` 与发生时的 `email` 快照，成功活动及配置版本在同一事务记录。后续改邮箱或离网不改写历史，也不向原网络披露离网后的新邮箱。没有快照时 `email` 明确为 `null`，展示 User ID，不用当前邮箱猜补，也不推断为已删除用户。

ActivityAction 覆盖账户注册／登录／密码／邮箱／会话撤销、网络创建／改名、成员加入／角色／移除、加入链接创建／撤销、configuration.apply 以及 routing.select（节点或用户切换路由方案，见 [Routing · 选择的入口](/v2/configuration-reference/routing#选择的入口)）。配置系统变更的具体来源使用 ConfigurationChangeCause，不能把它当成另一组活动枚举。

ActivityTarget 分支为 account(userId, previousEmail)、session(sessionId)、network(networkId)、membership(membershipId)、joinLink(linkId)、configuration(revision)、routingSelection(nodeId, profile)。`previousEmail` 只在 account.email 上携带修改前的地址，其他账户活动为 null；actor 的 email 快照是提交时刻的新地址。完整枚举见[生成契约](/v2/contracts/openapi.json)。

### 记录语义

- 活动记录表示对应的**已完成管理动作**，与其状态变化同事务提交；失败的验证／未提交请求不写成成功动作。
- 身份、actor、时间和 target 来自已验证的服务端上下文，不接受客户端自行声明“管理员已操作”。幂等恢复不重复生成动作记录。
- 配置提交记录只表示期望已保存。稍后某个 Node apply 失败在 publication 回执／进度中展示，不改写这条动作成“配置没有提交”。
- Node 心跳、每次 State query、SSE 重放和连续探测样本不写成管理操作历史。失败请求的运维诊断另受日志预算约束，不宣称操作历史覆盖所有失败尝试。
- ActivityEvent 不包含 token／密码／证书私钥、token 摘要、原始请求响应 body 或完整配置 diff；通过 revision／target 关联授权读取。
- 事件为历史事实，无普通编辑／删除 API；保留策略由部署管理。它不承担恢复当前配置或重放权限的职责。

## 分页与保留

| 数据 | 固定读取上界 |
| --- | --- |
| Revision 列表 | 首次查询时的最大 revision，倒序 keyset 分页 |
| Activity 列表 | 首次查询时的内部提交序列上界，倒序 keyset 分页 |
| ObservationHistory | 见[观测与历史](/v2/state-reference/observations) |

ActivityListResponse 包含 requestId、items、nextCursor、asOf；ConfigurationRevisionList 另有 currentRevision。cursor 不透明，绑定身份／网络／读取上界；每页重新鉴权，变更身份不能沿用旧 cursor。并发新增记录通过新一轮查询获取。

完整配置版本保留用于恢复。ActivityListResponse 没有 coverage 字段，不从观测历史的 HistoryCoverage 推导活动保留合同。

## SSE 通知

`configuration.changed` 附新 revision，使版本列表失效；`configuration.progress` 表示 Node 应用结果，不是管理动作提交确认。操作历史没有对应 SSE 事件，按需通过 GET 重读；断流或重放不丢失也不制造已提交的版本／动作记录，见 [SSE 与初始同步](/v2/api/events)。
