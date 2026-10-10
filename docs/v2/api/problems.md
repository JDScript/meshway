# Actions 与 Problem

字段使用 camelCase，错误 code 使用稳定 snake_case。精确枚举、参数和各操作状态码以[生成契约](/v2/contracts/openapi.json)为准。

## Actions

资源视图返回 Server 计算的操作提示，例如 NetworkActions.readConfiguration／editConfiguration／joinLinkRoles、MemberActions.assignableRoles／remove、加入链接及接入密钥的 actions.revoke，Node 库存的 canDelete 和分密钥吊销资格。readConfiguration 为 false 时客户端直接读成员视图（本人节点设置），不用一次被拒的完整配置请求探测角色。

- actions 只驱动界面，不是授权票据；mutation 在最终事务重检真实身份和权限。
- 不能从一个管理标志推导全部权限。Tag owner、本人设备、目标角色和最后 Owner 约束分别计算。
- 权限变化会重新裁剪 actions、视图和订阅，旧 actions 不延长提交资格。
- 不返回秘密、任意服务器函数名或动态执行代码。

## HTTP ErrorResponse

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `requestId` | RequestId 或 null | 请求关联；仅安全 ID 生成不可用时为 null |
| `status` | integer | HTTP 状态码 |
| `problem` | Problem | 按 code 判别的错误 |

Problem 的每个分支均有 code、params、message。params 由 code 决定，多数分支为 EmptyParams；message 是安全后备文本，不能用作程序分支。

每个操作在契约中逐状态声明可能返回的 code：ErrorResponse 响应的 `description` 是逗号分隔的 code 列表，[API 参考](/v2/reference/)的响应表列为「Problem codes」。未声明的（状态, code）组合不会由该操作返回。

<!-- schema: ErrorResponse -->
```json
{
  "requestId": "00000000-0000-4000-8000-000000000001",
  "status": 409,
  "problem": { "code": "revision_conflict", "params": {}, "message": "Configuration revision conflict." }
}
```

该例仅说明结构；错误文本不构成稳定合同。

| 范围 | code 示例 | 参数与行为 |
| --- | --- | --- |
| 身份 | invalid_credentials、invalid_session、permission_denied | 不回显凭据或隐藏对象详情 |
| Node 身份撤销 | node_identity_rejected | HTTP 403；已验证的 Node 身份或密钥不再有效。Node 仅以成功解码的此错误或 `netmap_withdrawn` 撤回已签名 netmap；裸 401／403／404、证明过期和普通资源拒绝不撤回租约 |
| netmap 已撤回 | netmap_withdrawn | HTTP 403；Node 已认证，但 Server 不再为它持有 netmap（例如数据面钥被吊销、移出可信库存）。Node 撤回已持有的 netmap 并继续 sync，等待新的 netmap |
| 注册 | registration_disabled、password_weak、email_taken | password_weak 携带 PasswordRules；email_taken 不返回账户详情 |
| 并发 | request_id_conflict、revision_conflict、version_conflict | 幂等内容、配置 CAS、对象版本分别冲突 |
| 绑定 | binding_conflict | 当前 generation／绑定不匹配；不冒充请求 ID 冲突 |
| 生命周期 | state_conflict、last_owner、already_member | 当前操作不符合资源或成员约束 |
| 一次性链接 | join_link_used、join_link_revoked、join_link_expired、join_link_issuer_unavailable | 仅验证真实 token 后区分，不返回原值 |
| 配置 | configuration_invalid | ConfigurationValidationParams：issues 和 requirements |
| 查询／预算 | invalid_cursor、state_limit、observation_limit、busy | 不泄露其他用户的活动或隐藏范围 |

revision_conflict 不携带当前配置差异；客户端重新读取基准后保留并验证原草稿。公开注册提供 email_taken，因此能得知邮箱占用；登录仍使用统一错误。

### Node 配置回执冲突

`submitNodeConfigurationReceipt` 的以下 Problem 均为 HTTP 409、`params={}`。HTTP `requestId` 仅用于关联日志；回执幂等身份是 instance／publication／attempt／sequence，不是请求 ID。

| code | 条件 | Node 恢复 |
| --- | --- | --- |
| `receipt_instance_mismatch` | instance、generation、Server boot 或 instance 截止不再匹配 | fencing 后重新取得有效资格 |
| `receipt_target_mismatch` | 目标既非当前目标，也非该节点曾持有的合法历史引用 | 绑定错误，fencing 并进入 blocked |
| `receipt_publication_superseded` | 当前 publication 的执行资格已撤回或改变 | 丢弃被明确拒绝的回执，刷新目标并创建新安装尝试 |
| `receipt_lease_expired` | 当前启用目标的 applied 缺少未到期 execution lease | 同上；不延长旧许可 |
| `receipt_precondition` | 观测时间越界，或 applied 没有对应更早的 saved | 同上；新尝试重新确认 saved |
| `receipt_content_conflict` | 相同幂等身份的内容不同，或 phase／failure 不相容 | 同上；不覆盖或改写冲突回执 |

上述后四类通过同一 instance 非 fencing 恢复，1 秒后重试控制刷新；不因回执拒绝调用网络 suspend。当前身份鉴权仍先于回执分类；401／403、永久吊销及绑定错误不进入此路径。历史完成回执仍可记录为历史，不能提升当前目标。`request_id_conflict` 仅表示真正的请求 ID 被不同输入复用。

## 配置与运行问题

| 类型 | 结构与含义 |
| --- | --- |
| ConfigurationIssue | path、code、params、message：见下节 |
| PreparationRequirement | path、kind：材料、地址或其他准备需求 |
| StateProblem | 按 code 判别，携带相应 resource／target／measurementId 与 observedAt |

这些类型不与 HTTP Problem 共用同一个 envelope。配置字段路径不是长期规则 ID；历史运行问题也不会因读取变成新事件。StateProblem 不接受任意内部异常文本。

### ConfigurationIssue {#configuration-issue}

| 字段 | 含义 |
| --- | --- |
| `path` | 指向**输入**的 JSON Pointer，精确到出错的元素（如 `/access/1/src/0`、`/routing/egress/profiles/work/rules/0`）；数组下标按作者书写顺序，规范化排序在定位之后进行。整份文档的问题为空串 |
| `code` | 封闭枚举 `IssueCode`（snake_case），客户端按它本地化；完整列表见[生成契约](/v2/contracts/openapi.json) |
| `params` | 该 code 文档声明的参数，始终存在（可为空对象），值均为字符串 |
| `message` | `code` 的英文渲染，不含参数值，不供程序判断 |

参数约定：引用类问题带被引用的名字（`user`、`name`、`node`、`key`、`profile`、`value`、`address`、`hostname`）；节点级的出口授权问题（`exit_outside_access`、`global_exit_requires_internet`、`exit_fallback_requires_internet`）挂在**规则／分配／fallback** 的 pointer 上，受影响节点在 `params.node`；传输声明问题 `invalid_transport` 的 `params.reason` 是传输层声明错误码（如 `invalid_host`、`websocket_alpn`）；源文本问题 `invalid_jsonc`／`invalid_shape` 带 1 起算的 `line`、`column`，不回显文本；远程规则集问题 `rule_set_unavailable` 的 `params.reason` 是 `ResourceProblem` code。`netmap_limit` 表示该版本会使某节点的 netmap 超过 8 MiB 下发安全上界，路径为 `/nodes/<id>`，参数 `node`、`bytes`、`maxBytes`。`netmap_budget` 表示编译整网 netmap 超过固定的编译预算（节点数或每边承载候选），路径为空、无参数；它是确定性的，重试同一版本不会成功（Node 触发的同类情况不报配置错误，netmap 记为不可用）。

同一类型也用于 `warnings`（接受但可能非本意的配置，如 `shadowed_rule`：该规则的全部目标已被前面的规则匹配，`params.by` 是补齐覆盖的那条规则下标）和 `unavailableDependencies`（保留但已失效的依赖）。

`cursor_too_old`／`view_changed` 属于 SSE reset 原因，见[事件流](/v2/api/events)，不混作 HTTP Problem code。

SDK 先校验错误 body 再返回类型化结果。无法解码时保留 HTTP status 并丢弃原始 body；日志、配置问题和运行问题均不得回显密码、密钥、完整配置或内部异常。
