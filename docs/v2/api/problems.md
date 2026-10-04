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
| 注册 | registration_disabled、password_weak、email_taken | password_weak 携带 PasswordRules；email_taken 不返回账户详情 |
| 并发 | request_id_conflict、revision_conflict、version_conflict | 幂等内容、配置 CAS、对象版本分别冲突 |
| 生命周期 | state_conflict、last_owner、already_member | 当前操作不符合资源或成员约束 |
| 一次性链接 | join_link_used、join_link_revoked、join_link_expired、join_link_issuer_unavailable | 仅验证真实 token 后区分，不返回原值 |
| 配置 | configuration_invalid | ConfigurationValidationParams：issues 和 requirements |
| 查询／预算 | invalid_cursor、state_limit、observation_limit、busy | 不泄露其他用户的活动或隐藏范围 |

revision_conflict 不携带当前配置差异；客户端重新读取基准后保留并验证原草稿。公开注册提供 email_taken，因此能得知邮箱占用；登录仍使用统一错误。

## 配置与运行问题

| 类型 | 结构与含义 |
| --- | --- |
| ConfigurationIssue | path、message：固定配置版本内的字段定位与安全文本 |
| PreparationRequirement | path、kind：材料、地址或其他准备需求 |
| StateProblem | 按 code 判别，携带相应 resource／target／measurementId 与 observedAt |

这些类型不与 HTTP Problem 共用同一个 envelope。配置字段路径不是长期规则 ID；历史运行问题也不会因读取变成新事件。StateProblem 不接受任意内部异常文本。

`cursor_too_old`／`view_changed` 属于 SSE reset 原因，见[事件流](/v2/api/events)，不混作 HTTP Problem code。

SDK 先校验错误 body 再返回类型化结果。无法解码时保留 HTTP status 并丢弃原始 body；日志、配置问题和运行问题均不得回显密码、密钥、完整配置或内部异常。
