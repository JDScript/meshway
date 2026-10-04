# Actions 与 Problem

字段使用 camelCase，错误 code 使用稳定 snake_case。HTTP Problem 与资源 actions 已由服务端提供；配置诊断的完整 code／params／severity 整理仍待完成，精确合同以生成 schema 为准。

## Actions

资源视图提供服务端计算的固定字段，例如 NetworkActions.joinLinkRoles、成员的 assignableRoles／remove、加入链接和接入密钥的 actions.revoke，以及节点库存的 canDelete／分密钥吊销资格。不能从一个管理标志推导全部操作权限。

- 普通成员／Tag owner 的局部操作能力由真实用户、membership、Node identity 和当前配置计算。
- actions 只驱动 UI，不是授权票据；每个 mutation 在服务端重新校验。
- 权限变化使 actions、可见范围、订阅和草稿提交资格重新计算，旧 actions 不延长权限。
- 不返回秘密、任意服务器函数名或动态执行代码。

## Problem 核心

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `code` | ProblemCode | 每个操作／场景的封闭枚举 |
| `params` | 由 code 决定的类型 | 例如冲突 revision、节点或资源引用；无参数分支明确为空 |
| `message` | string | 脱敏的后备文本，不用于程序分支或主要本地化 |

外层保留各自上下文：

| 位置 | 额外内容 |
| --- | --- |
| HTTP ErrorResponse | requestId、HTTP status、可用的字段 pointer |
| ConfigurationDiagnostic | severity、pointer、受影响的已授权资源 |
| StateProblem | 资源／实例引用、observedAt、实际阶段 |

因此配置警告不会变成 HTTP 失败，历史运行错误也不会因读取刷新为当前事件。pointer 是特定配置 revision 内的位置，不能单独作为长期规则 ID。

## 错误范围

| 范围 | code 示例 | 参数边界 |
| --- | --- | --- |
| 身份 | `invalid_credentials`、`invalid_session`、`permission_denied` | 不回显凭据或隐藏对象是否存在 |
| 注册 | `registration_disabled`、`password_weak`、`email_taken` | 密码规则可公开；不返回已有账户详情 |
| 并发／生命周期 | `request_id_conflict`、`revision_conflict`、`version_conflict`、`state_conflict` | 分别表示幂等输入、配置基准、对象版本和状态冲突 |
| 一次性链接 | `join_link_used`、`join_link_revoked`、`join_link_expired`、`join_link_issuer_unavailable` | 只在匹配真实 token 后区分，不输出原值 |
| 配置 | `configuration_invalid` | issues／requirements；更细诊断类型另行完善 |
| 查询 | `invalid_cursor` | 范围、期限或编码不合法的游标 |
| 诊断预算 | `observation_limit` | 当前适用预算，不泄漏其他用户活动 |

`cursor_too_old`／`view_changed` 等是 SSE reset 原因，不能混作 HTTP Problem code。上表是语义示例，完整枚举及每个操作允许的状态由实际路由导出。

精确枚举及参数由 Rust 定义、生成 schema 和 SDK，新增场景明确扩展。公开注册首版给出 email_taken，意味着注册端可以知道邮箱占用；登录错误仍统一。加入链接解决的是按邮箱找人／直接加人的流程，不宣称实例完全隐藏账户存在性。

SDK 校验错误 body 后返回结构化 Problem；无法解码时仍保留 HTTP status，丢弃原始 body。diagnostics／State 的文本同样不得包含密码、密钥、完整配置或原始内部异常。
