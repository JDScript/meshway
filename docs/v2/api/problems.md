# Actions 与 Problem

**V2 接口规则，尚未实现。** 字段使用 camelCase；错误 code 使用稳定 snake_case 枚举，HTTP／配置诊断／运行问题共享核心定义。

## Actions

资源视图返回服务端计算的、按资源类型固定的 actions。例如 NodeActions 可包含 editSettings、assignTags、delete、diagnose 和 selectProfiles，值分别为布尔或有权使用的明确集合。

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

## 首批 code 范围

| 范围 | code 示例 | 参数边界 |
| --- | --- | --- |
| 身份 | `invalid_credentials`、`session_expired`、`permission_denied` | 不回显凭据或隐藏对象是否存在 |
| 注册 | `registration_disabled`、`password_weak`、`email_taken` | 密码规则可公开；不返回已有账户详情 |
| 并发 | `revision_conflict`、`version_conflict` | 当前版本及允许展示的冲突路径 |
| 成员 | `last_owner`、`already_member`、`membership_revoked` | 当前网络关系，不暴露全局账户清单 |
| 一次性链接 | `link_expired`、`link_consumed`、`link_revoked` | 有效请求绑定内的状态；不输出原 token |
| 配置 | `hostname_taken`、`invalid_configuration`、`rule_shadowed` | pointer、规则位置与类型化冲突依据 |
| 执行／资源 | `unsupported_capability`、`resource_conflict`、`dependency_unavailable` | 必需能力或已授权资源引用 |
| Node | `node_deleted`、`device_login_required`、`address_conflict` | 当前网络的目标与变更位置 |
| 预算 | `request_too_large`、`rate_limited`、`observation_limit` | 已声明预算／重试时间，不泄漏其他用户活动 |
| 流同步 | `cursor_too_old`、`view_changed` | reset／重新读取原因 |

精确枚举及参数由 Rust 定义、生成 schema 和 SDK，新增场景明确扩展。公开注册首版给出 email_taken，意味着注册端可以知道邮箱占用；登录错误仍统一。加入链接解决的是按邮箱找人／直接加人的流程，不宣称实例完全隐藏账户存在性。

SDK 校验错误 body 后返回结构化 Problem；无法解码时仍保留 HTTP status，丢弃原始 body。diagnostics／State 的文本同样不得包含密码、密钥、完整配置或原始内部异常。
