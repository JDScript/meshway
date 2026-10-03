# API 设计

**V2 设计，尚未实现。** API 采用 `/api/v1` 与 `/agent-api/v1` 两个入口；资源路径和字段仍为草案。V2 是文档版本，不代表 HTTP 路径必须使用 v2。

## 已确定的边界

| 部分 | 选择 |
| --- | --- |
| 账户 | 默认开放邮箱／密码注册；Controller 配置可关闭注册；不提供邀请注册分支 |
| 网络 | 有效登录用户可创建自己的网络，创建者成为该网络 Owner |
| 用户认证 | 不透明会话，通过 Authorization 携带；继续由服务端检查当前权限 |
| Node 认证 | 本机生成并持久保存密钥，注册绑定公钥，后续证明持有私钥 |
| Console 同步 | SSE 首次发送完整可见 Config／State，再发送后续变化 |
| 配置应用 | Console 一次提交，Node 独立拉取、校验、保存并正常应用；展示真实逐节点进度 |
| API 分工 | `/api/v1` 提供 Console API，`/agent-api/v1` 提供节点接入／运行 API |

## API 分区

使用两个清楚的 API 根路径，共用同一 Controller、数据库与领域服务：

| 前缀 | 调用方 | 用途 |
| --- | --- | --- |
| `/api/v1` | Console；登录后使用用户会话 | 账户、网络、成员、配置、观测及设备接入批准；其中 auth 提供公开注册／登录 |
| `/agent-api/v1` | 接入中的设备或已认证 Node | 设备接入、运行实例、执行配置、State 上报／读取与获准任务 |

Linux Agent 与终端 App 使用同一 Agent API。路径划分不产生两套 User／Node 模型；Controller 为两个 router 安装各自的认证策略，共享有明确身份上下文的领域操作。

### Controller 启动

空数据目录正常初始化数据库结构与必要材料密钥，启动后即可注册、登录和创建网络。没有独立的实例初始化 API、首用户认领或默认网络创建流程；第一个用户与其他用户相同，创建网络才成为该网络 Owner。

关闭注册由部署配置控制。若空实例已关闭注册，部署者可显式调整配置后使用普通注册流程。账户恢复属于单独的运维能力设计，不因它引入日常实例管理员 HTTP 分区。数据库损坏／未知结构仍报错，不能借“自动初始化”静默清库。

用户操作显式指定 `networkId`，逐请求检查 membership／role。已接入 Node 的网络和自身 ID 从认证绑定推导，不接受请求随意指定另一网络／节点。接入前的临时请求没有 Node 权限。

## 操作索引

### 用户与网络

| 方法与路径提案 | 主体 | 结果 |
| --- | --- | --- |
| `GET /auth/options` | 匿名 | 公开 registrationEnabled 等登录选项，不含运维秘密 |
| `POST /auth/register` | 匿名 | 注册账户并建立用户会话；关闭注册时拒绝 |
| `POST /auth/login` | 匿名 | 校验密码并建立用户会话 |
| `GET /auth/session` | 用户 | 当前有效用户与会话期限 |
| `POST /auth/logout` | 用户 | 撤销当前会话，包括其活动订阅 |
| `GET /networks` | 用户 | 仅自己的有效网络成员关系 |
| `POST /networks` | 用户 | 原子创建网络、Owner 成员关系和初始配置 |
| `GET /networks/{networkId}/configuration` | 用户 | 按权限读取配置视图及 revision；编辑原文／材料使用显式管理读取 |
| `POST /networks/{networkId}/configuration` | 网络管理权限 | 以 expectedRevision 提交目标，返回新期望 revision 和 publication 引用；节点异步应用 |
| `GET /networks/{networkId}/publications/{publicationId}` | 用户 | 读取有权查看的目标及逐节点应用结果／进度 |
| `GET /networks/{networkId}/events` | 用户 | 首次同步与持续事件流 |
| `POST /networks/{networkId}/observations` | 有权诊断的用户 | 创建有租约的 path observation |
| `POST /networks/{networkId}/observations/{observationId}/renew` | 任务使用者 | 检查当前权限并续租 |
| `POST /networks/{networkId}/observations/{observationId}/cancel` | 任务使用者 | 幂等取消自己的观察订阅／需求 |

表中省略公共 `/api/v1` 前缀。成员、配置历史、局部编辑和材料读取等精确操作后续补齐；所有写入复用既有领域权限与配置 CAS。已有账户加入其他网络是成员管理操作，不附带创建账户或替对方设置密码。

### 设备接入与运行

| 方法与路径提案 | 主体 | 结果 |
| --- | --- | --- |
| `POST /agent-api/v1/enrollment-requests` | 尚未接入的设备 | 创建绑定公钥、PKCE 和返回方式的临时请求 |
| `POST /api/v1/device-authorizations/{requestId}/approve` | 用户会话 | 选择有效网络，批准该次本人设备接入 |
| `POST /agent-api/v1/enrollments` | 本次设备证明＋一次性批准或接入密钥 | 原子创建／恢复同一 Node 绑定 |
| `POST /agent-api/v1/instances` | Node | 获得／恢复当前运行实例，隔离被替代实例 |
| `GET /agent-api/v1/configuration` | Node | 拉取本节点当前有效目标配置与执行上下文，按 full／delta 下发 |
| `POST /agent-api/v1/configuration/receipts` | Node＋当前实例 | 分别提交 saved／applied／failed 类型化回执 |
| `POST /agent-api/v1/state` | Node＋当前实例 | 上报自身 State，携带序号与配置引用 |
| `POST /agent-api/v1/state/query` | Node | 查询服务端授权裁剪后的执行依赖 |
| `POST /agent-api/v1/demands` | Node | 请求获准业务目标所需的路径／材料依赖 |

Node 的认证握手、请求结果领取、配置／State watch 与任务下发的精确 HTTP 编码需继续设计。用户 SSE 不直接复用为 Node 执行通道；两者可共享事件帧处理，但不能共享授权视图或把浏览器通知当作执行许可。

配置回执与 NodeState.progress 消费同一运行状态机，saved 的含义只有持久保存；Node 在有效许可下直接安装并上报结果。客户端取消 HTTP 不取消已受理配置，恢复旧期望需显式回滚为新版本。见[普通应用流程](/v2/configuration-reference/deployment)。

## 一致性与错误

- 用户配置写入以 expectedRevision 做 CAS；冲突返回 `409` 与当前版本，不覆盖未保存草稿。SSE 通知不替代 CAS。
- 注册成功不自动创建网络；网络创建返回丢失时，绑定用户和请求内容的幂等键可恢复原结果，不能因重试创建多张网络。
- 设备注册的公钥、批准消费、身份／归属、地址及初始配置同事务绑定；响应丢失重试不能换身份。
- `401` 表示认证无效／过期；`403` 表示身份有效但该操作被禁止；`409` 表示版本或绑定冲突；`422` 表示配置语义无法满足。精确错误码进入各操作的类型化响应。
- SSE 握手前用正常 HTTP 错误；开始流式响应后用类型化终止／reset 事件及断开。网络断开不能伪装成空 State 或节点离线。

## 契约与生成

Rust DTO、真实路由、认证和流式操作声明仍为唯一权威。由同一来源导出 Console／Agent API 分组与各消费者客户端；不维护两份手写 endpoint／DTO。

生成器需要覆盖 SSE、所需 query/header 参数和新的 Node 认证方式。当前生成链仍只交付 JSON HTTP 客户端；可行路线与验证边界见 [SSE 与初始同步](/v2/api/events#openapi)。

继续阅读：[注册与认证](/v2/api/authentication)、[SSE 与初始同步](/v2/api/events)、[操作权限](/v2/permissions)。

接口使用的数据来源与代码 owner 见 [Entity](/v2/entities)、[存储与访问](/v2/storage)、[技术栈与 Shared crates](/v2/architecture)。
