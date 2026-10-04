# 配置版本与操作历史

配置历史记录完整目标，操作历史记录已完成的管理动作，观测历史记录真实节点事件。三者使用关联 ID，权限和语义分别处理；具体字段以生成合同为准。

## 配置版本接口

下面路径相对 `/api/v1/networks/{networkId}`，需要 Owner／Admin 管理读取权限。

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| `GET /configuration/revisions` | cursor、limit | 固定上界的版本摘要列表 |
| `GET /configuration/revisions/{revision}` | revision | ConfigurationResponse：networkId、revision 与非秘密 ConfigurationView |
| `GET /configuration/source` | 可选 revision，省略为当前 | 获准管理员读取原始 JSONC／完整规范化值及必要材料，no-store |
| `POST /configuration/compare` | fromRevision、toRevision | 字段路径级变化摘要，区分语义／原文／绑定变化 |
| `POST /configuration` | expectedRevision＋新草稿，或 restoreFrom 历史引用 | 按普通 apply 保存新版本，返回 revision／publication 引用 |

同一配置提交入口用 `type: "restore"` 和 `restoreFrom: Revision` 恢复历史。可选 resourcePublication 必须同网且对应该源版本，指定某次历史数据集绑定；省略／null 使用源版本初始绑定。恢复按当前身份、材料、地址与授权重验，不默认换成最新规则集。

回滚输入由生成契约区分普通编辑和历史恢复。稳定 UserId／MembershipId 和材料／数据集依据在服务端解析；不能仅复制旧邮箱字符串后按当前账户重新匹配。失效依赖或需要设备证明的身份转换按正常规则拒绝。

Node 删除不可恢复；恢复目标含已删除 Node 时拒绝。最终差异移除当前 Node 则按永久删除执行，应用对照明确展示，不能把历史恢复当作不经过预览的特殊通道。见[整网草稿](/v2/configuration-reference/editing)。

### ConfigurationRevisionSummary

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `revision` | Revision | 历史版本 |
| `createdAt` | Timestamp | 持久提交时间 |
| `createdBy` | ActivityActor | 真实提交主体 |
| `cause` | ConfigurationChangeCause | networkCreated／edit／restore／enrollment／nodeDeleted／emailSync／memberRemoved／identityTransition |
| `rollbackFrom` | Revision? | 回滚来源；不是倒退后的版本号 |
| `current` | boolean | 是否等于本次读取截点的期望版本 |

列表不携带整份 JSONC 或秘密。节点是否应用从 publication／NodeState 查询，不能在 revision 列表写一个缺少范围与来源的全网 applied=true。

cause 表示主触发来源，具体影响由 compare 的 changes 表达。系统动作实际改变配置内容或固定绑定就形成版本；纯观测不推进版本。

### Compare

比较默认返回路径／变化类型摘要，不返回任意 JSON before／after 值：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `requestId` | RequestId | 此次比较请求关联 |
| `fromRevision` / `toRevision` | Revision | 被比较的确切版本 |
| `sourceChanged` | boolean | 原始 JSONC 文本或注释是否改变 |
| `configurationChanged` | boolean | 规范化配置是否改变 |
| `bindingsChanged` | boolean | 固定用户／资源／数据集绑定是否改变 |
| `changes` | ConfigurationChange[] | 合法字段路径的变化摘要 |

ConfigurationChange 字段为 `path: JsonPointer`、`kind: added | removed | changed`。有序数组重排按数组变化说明，不凭 remark 当主键；秘密字段只报告“已变更”，不把密码或私钥写进差异摘要。完整文本 diff 由获准管理员显式读取两个 source 后查看，脱敏视图不能整份回写。

## 操作历史接口

| 路径 | 范围与权限 |
| --- | --- |
| `GET /api/v1/networks/{networkId}/activity` | 本网络管理操作；Owner／Admin |
| `GET /api/v1/networks/{networkId}/activity/{activityId}` | 同网单条记录；仍按当前权限检查 |
| `GET /api/v1/account/activity` | 当前用户的全局账户操作，不列出别人的账户历史 |

列表接受 limit 和 cursor。历史 ID、游标或当时拥有的权限，都不能绕过当前网络授权。

### ActivityEvent

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ActivityId | 不透明记录 ID |
| `at` | Timestamp | Server 完成该动作的时间 |
| `actor` | ActivityActor | 用户或系统 |
| `action` | ActivityAction | 已知管理动作枚举 |
| `target` | ActivityTarget | 类型化目标引用，不是任意 metadata |
| `beforeRevision` | Revision? | 配置变更前版本；不涉及配置则 null |
| `afterRevision` | Revision? | 配置变更后版本；不涉及配置则 null |
| `requestId` | RequestId | 同一次逻辑操作的关联，不是授权凭证 |

scope 由所属记录及路径确定。返回时可以按权限投影主体／目标的可读名称，稳定 ID 始终保留；删除或改名不把记录改指向另一个对象。

用户 actor 返回稳定 `userId` 与发生时的 `email` 快照，成功活动及配置版本在同一事务记录。后续改邮箱或离网不改写历史，也不向原网络披露离网后的新邮箱。没有快照时 `email` 明确为 `null`，展示 User ID，不用当前邮箱猜补，也不推断为已删除用户。

ActivityAction 覆盖账户注册／登录／密码／邮箱／会话撤销、网络创建／改名、成员加入／角色／移除、加入链接创建／撤销以及 configuration.apply。配置系统变更的具体来源使用 ConfigurationChangeCause，不能把它当成另一组活动枚举。

ActivityTarget 分支为 account(userId, previousEmail)、session(sessionId)、network(networkId)、membership(membershipId)、joinLink(linkId)、configuration(revision)。`previousEmail` 只在 account.email 上携带修改前的地址，其他账户活动为 null；actor 的 email 快照是提交时刻的新地址。完整枚举见[生成契约](/v2/contracts/openapi.json)。

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
| ObservationHistory | 固定测量时间窗口及接收记录上界，按既定迟到／去重规则查询 |

ActivityListResponse 包含 requestId、items、nextCursor、asOf；ConfigurationRevisionList 另有 currentRevision。cursor 不透明，绑定身份／网络／读取上界；每页重新鉴权，变更身份不能沿用旧 cursor。并发新增记录通过新一轮查询获取。

完整配置版本保留用于恢复。观测历史的裁剪通过 HistoryCoverage 表达，不能用空列表暗示从未有过记录；ActivityListResponse 没有 coverage 字段，不从观测历史类型推导活动保留合同。

## SSE 通知

- `configuration.changed` 附新 revision，使版本列表失效；按需重新读取摘要。
- 操作历史通过 GET 按需重读；不从管理动作名称推断存在对应 SSE 事件。
- `configuration.progress` 继续表示 Node 应用结果，不当作管理动作提交确认。

SSE 是有界实时通知，持久历史通过 GET 读取；断流不丢失已提交的配置版本／动作记录，重放也不制造新记录。
