# 协议与类型安全

**已确认的要求：接口自动生成并强制类型安全。** 当前实现选择 Rust 类型与实际路由关联的 OpenAPI 作为 HTTP 契约来源。Agent 使用生成的 Rust 客户端，Next.js 使用生成的 TypeScript SDK 及运行时验证。不能分别手写三份“看起来相同”的协议。

## 契约生成链

```text
Rust 请求 / 响应类型 + 实际 API 路由声明
                    │
                    ▼
                 OpenAPI
                /       \
               ▼         ▼
      Rust control client  TypeScript SDK + 运行时 schema
               │                         │
               ▼                         ▼
           Rust Agent                 Next.js Web
```

路由声明关联 OpenAPI 很重要：一个与运行中服务无关的手工文档，即使可以生成客户端，也无法证明 endpoint、状态码和响应一致。

`crates/protocol` 承载共享 Rust 契约基础；本轮在 `routing.rs` 和 `config.rs` 增加策略、公共清单、配置快照和回执类型。当前生成位置为：

- `packages/protocol/openapi.json`：从 controller 实际路由导出的契约。
- `apps/agent/src/generated`：Agent 的 Rust control client。
- `packages/control-client/src/generated`：TypeScript 类型、客户端与 Zod schema。

生成工具与路径由 `scripts/generate-protocol.mjs` 统一维护；修改源类型 / 路由后再生成，不直接编辑生成文件。

### 当前生成器与支持范围

- Utoipa 6 与 utoipa-axum 0.3 从 Rust DTO 和路由声明导出原生 OpenAPI 3.1。
- `apps/controller/src/bin/generate-rust-client.rs` 使用 `openapi-to-rust` 0.19，基线生成 `enrollAgent` 与 `heartbeatAgent`。本轮选择范围扩展到 `getAgentConfiguration` / `acknowledgeConfiguration` 及其关联模型，认证配置始终从所选实际操作的 bearer 声明派生；Agent 不需要管理 API 的权限。
- 上游 Rust 生成器 0.19 没有在客户端构造中落实超时，且默认跟随重定向；仓库内的生成适配显式设置 **10 秒超时、禁止重定向**。适配前检查构造模板，模板变化会使生成失败，防止把注册凭据请求体转发给重定向目标。封闭对象另按 OpenAPI `additionalProperties: false` 生成 `deny_unknown_fields`，在 serde 丢弃异常字段之前拒绝响应；不支持的开放对象、flatten 或模板变化使生成失败。
- Rust 客户端收到非 2xx 状态头后立即返回类型化 HTTP 错误，保留状态与 headers、丢弃错误 body。已知 `401/403/409` 不能因错误响应体挂起或截断而被延迟处理或误判成可重试网络错误；成功响应仍受有界读取和严格解码约束。该适配逐项核对所选操作的模板数量和形状。
- Hey API 0.99 生成 TypeScript DTO；`scripts/generate-protocol.mjs` 从同一 OpenAPI 生成严格 Zod schema 与操作函数，再调用共享 HTTP transport。字符串长度按 Unicode code point 计算，与 Rust 的 Unicode scalar 计数一致；已修复原先 UTF-16 长度差异。
- TS 操作生成支持 JSON GET / POST、组件 schema、必填 POST body、单一 HTTP 200 成功响应，以及单个必需的 HTTP bearer 认证方案。schema 生成覆盖配置 / 策略所需的封闭对象、组件引用、数组、整数、布尔值、字符串枚举、日期时间及 `T | null`。不支持的 schema 关键字、形状或认证要求直接使生成失败。

注册基线生成物已通过漂移检查及真实 Rust / TS 客户端往返测试；配置 / 策略契约也已生成并通过客户端、类型检查和真实服务闭环验证，具体测试批次见[验收记录](/development#配置与路由切片)。所有管理操作都需要管理员 bearer；例如 SDK 读取概览的签名为 `getOverview(baseUrl, token)`。

接口来源为 `crates/protocol/src/` 的 DTO、`apps/controller/src/lib.rs` 的 `contract_router` 及关联处理器；本轮配置路由位于 `apps/controller/src/configuration_api.rs` 并合入同一 router。认证安全方案由 `describe` 导出为 `adminBearer`、`enrollmentBearer`、`nodeBearer`，每个操作声明相应 `security`。生成客户端必须消费这些信息，不能在 Web 或 Agent 中手写 endpoint 或另建 DTO。

## 注册与心跳 API

默认本地地址：`http://127.0.0.1:8420`。以下路由、认证流程及概览 / 节点响应的配置字段已通过生成客户端和集成验收。

| 方法与路径 | bearer 权限 | JSON 请求 | HTTP 200 响应 |
| --- | --- | --- | --- |
| `GET /api/v1/overview` | 管理员 | 无 | `Overview`，本轮增加 `configuration_revision` |
| `GET /api/v1/nodes` | 管理员 | 无 | `{ nodes: [NodeSummary] }` |
| `POST /api/v1/enrollment-tokens` | 管理员 | `{ expires_in_seconds }` | `{ token_id, token, expires_at }` |
| `POST /api/v1/nodes/revoke` | 管理员 | `{ node_id }` | `{ node: NodeSummary }` |
| `POST /api/v1/agents/enroll` | 注册令牌 | `{ node_credential, name, agent_version }` | `{ node: NodeSummary }` |
| `POST /api/v1/agents/heartbeat` | 节点 | `{ agent_version }` | `{ node: NodeSummary }` |

`NodeSummary` 保留 `id`、`name`、`agent_version`、`last_seen`、`registered_at`、`revoked_at`，本轮增加 `mesh_ipv4`、`mesh_ipv6`、必填可空 `country`、`configuration_revision` 与 `acknowledged_revision`。时间使用 RFC 3339；`revoked_at` 必须出现，未撤销时为 `null`。令牌有效期仍为整数 `60..86400` 秒。完整字段和约束以生成 schema 为准，不在 Web 手抄这一类型。

注册 DTO 中的 `node_credential` 约束为 64 个小写十六进制字符，代表 Agent 预先随机生成的私有 bearer，**不是公钥**。`name` 最长 128 个 Unicode 字符、`agent_version` 最长 64 个，均须包含非空白文本。生成 schema 与 Rust 都按 Unicode code point / scalar 计数；真实往返测试覆盖 128 个 emoji 可接受、129 个被拒绝及全空白文本拒绝。

表格仅是字段概览。请求与错误响应、约束及状态码的机器可读参考是 `packages/protocol/openapi.json`；它必须由实际 Rust 路由重生成并通过漂移检查。HTTP API schema 不是 mesh 数据面协议或网络连通性证明。当前路由没有公开健康 endpoint；若之后提供，也仅表示服务进程存活，不能暴露受保护清单或代表节点在线。

### 记录和权限语义

- 心跳不接收调用方任选的节点 ID；controller 从节点 bearer 派生身份。
- `name` 与 `agent_version` 是元数据，不是 mesh 密钥认证或可信能力声明。
- `last_seen` 是最近注册 / 心跳联系时间，不是在线、隧道健康或业务路径探测结果。
- `node_count` 统计所有持久化节点，包括已撤销节点；controller 重启保留这些记录。
- 节点凭据不能读取管理清单、签发注册令牌或撤销节点；注册令牌也不授予管理权限。
- 注册绑定与幂等恢复、撤销不能复活等语义见[注册与持久化](/enrollment)。

## 配置与策略 API

以下路径和 DTO 由 Rust 源契约生成，已用于真实 Agent / SDK 往返和 Web 路由编辑闭环。

| 方法与路径 | bearer 权限 | 生成请求类型 | HTTP 200 响应 |
| --- | --- | --- | --- |
| `GET /api/v1/policy` | 管理员 | 无 | `PolicySnapshot`：保存的源策略、网络、当前全局 revision 和诊断 |
| `POST /api/v1/policy/preview` | 管理员 | `PreviewPolicyRequest`：策略草稿与 `FlowQuery` | `PreviewPolicyResponse`：当前 revision、有效性、诊断及逻辑决策 |
| `POST /api/v1/policy/save` | 管理员 | `SavePolicyRequest`：`expected_revision` 与策略 | `PolicySnapshot` |
| `POST /api/v1/nodes/country` | 管理员 | `SetNodeCountryRequest`：节点 ID、必填可空国家、`expected_revision` | `{ node: NodeSummary }` |
| `GET /api/v1/agents/configuration` | 节点 | 无 | `ConfigurationSnapshot`：摘要与本节点公共配置 |
| `POST /api/v1/agents/configuration/ack` | 节点 | `AcknowledgeConfigurationRequest`：`revision` 与 `digest` | `AcknowledgeConfigurationResponse` |

条件保存和国家修改比较**全局** revision，不只比较上次策略保存版本；冲突返回 `409`。无效策略保存返回 `422` 且不改变数据库；纯预览不写库，合法请求中的策略 / 流错误通过 `valid: false`、诊断与 `invalid` 决策表达。IPv4-mapped IPv6 的点分 / 十六进制流输入在分类前直接返回 `invalid`，不隐式转为 IPv4。Web 保留冲突草稿，保存基准只在成功保存或显式重新读取后推进，预览不会自动更新基准。

节点配置读取 / ACK 均从 bearer 派生自身身份，不能自行选择节点。ACK 只接受当前自身配置的 revision 与 SHA-256 摘要；旧版本、未来版本或错误摘要返回 `409`，已撤销节点返回 `403`。Agent 每次 ACK 前都须确认私有缓存文件和父目录 fsync 成功，包括未变化缓存及重启后加载；发布后同步失败返回错误、不 ACK，即使新文件已经可见。ACK 不增加 revision，也不代表网络已应用。公共配置排除凭据和瞬时时间，规范 JSON 摘要规则见[配置分发](/configuration#公共配置与稳定摘要)。

策略源文档因节点撤销而失效时，管理 API 仍返回源文档及诊断；节点配置分发的是 deny-all 有效策略，并带 `policy_valid: false`。不能把坏引用丢掉后继续使用默认 direct。

### 有向业务授权与一致数据面快照

| 方法与路径 | bearer 权限 | 生成操作 / 类型 |
| --- | --- | --- |
| `GET /api/v1/access` | 管理员 | `getMeshAccess` → `MeshAccessSnapshot` |
| `POST /api/v1/access` | 管理员 | `saveMeshAccess`，请求 `SaveMeshAccessRequest`，返回 `MeshAccessSnapshot` |
| `GET /api/v1/agent/dataplane` | 节点 | `getAgentDataplane` → `AgentDataplaneSnapshot` |

授权保存使用全局 CAS，相同内容不增加 revision。节点快照在一个 SQLite 读事务中组合节点配置、连接、授权及所需公钥，所有部分使用同一 revision，调用方身份只从节点 bearer 派生。摘要覆盖完整公共 payload；它不包含节点 bearer、管理员令牌或本地 profile 密钥。

路径授权最多 128 条，每条包含 2–8 个不同的活动节点，完整路径有方向，不隐式允许反向或子路径。每个参与节点必须已有合法绑定公钥。单节点的数据面公钥集合最多 64 个；超出容量拒绝分发，不截断路径。授权源上限 128 KiB，一致快照上限 2 MiB。具体执行范围见[托管 Mesh IPv4 数据面](/managed-dataplane)。

旧的配置与连接响应不增加字段，仍保留各自的保存 ACK 语义。新数据面快照不是新的“已生效”回执；本地 TUN 代际证据绑定其 revision 和摘要，实际可达性另用普通 IP 流量验证。

## 错误与运行时验证

新增连接契约通过同一生成链提供 `publishConnectionIdentity`、`getConnections`、`saveConnections`、`getAgentConnections` 和 `acknowledgeAgentConnections`。精确路径与字段见[公钥与连接配置](/connections)及 Rust 源契约。只有管理读取 / 保存导出为 Web 的公共 SDK 操作；Agent 的三个节点操作由实际路由声明选择生成。连接回执独立于 IP / 路由回执。

Rust 路由声明包括：无效或缺失凭据的 HTTP `401`、已撤销节点访问的 `403`、非法请求的 `400`、未知节点的 `404`、冲突的 `409`，以及请求体 / 存储错误；本轮新增无效策略 / 容量失败的 `422`。每个 endpoint 的完整响应集合以 OpenAPI 为准。不同凭据复用已消费注册令牌按无效注册权限处理；不能把它当作相同身份的成功重试。

TypeScript SDK 导出 `ControlApiError`，其 `status` 保存非成功 HTTP 状态。Web 使用 `error instanceof ControlApiError && error.status === 401` 判断管理员凭据被拒绝，再进入登录流程。非成功响应体被丢弃，不把任意服务端内容回显给用户；JSON、媒体类型、网络及 schema 失败仍为普通 `Error`，不会伪装成认证拒绝。

控制面输入 DTO 使用 serde 拒绝未知字段。生成的 Rust 响应模型校验必填字段、类型、时间及必填可空语义，并按封闭 schema 拒绝额外字段；Agent 再校验摘要、节点绑定及业务约束。TS 操作在请求前验证 body，HTTP transport 在成功响应边界使用生成的严格 Zod schema，校验额外 / 缺失字段、数值、`null` 与 RFC 3339 时间。transport 不缓存、禁止重定向，设置 10 秒超时及 2 MiB 响应上限。类型声明不能替代这些运行时检查。

Rust 生成适配器从 schema 的 `required` 与 nullable 属性生成 serde 规则：`revoked_at` 可以明确为 `null`，但不能缺失；序列化 `None` 时保留该字段。`country`、`exit_node_id`、预览上下文和诊断中的必填可空字段也保持这一语义。配置 / 策略的嵌套类型、枚举与可空字段已纳入契约和客户端测试；生成器不支持的形状应直接失败，不能以宽泛 JSON 或未验证断言绕过。

## Web 如何消费真实契约

Web 在 Next.js 服务端通过 `@meshway/control-client` 调用管理员操作。登录验证成功后将管理员令牌保存在 `HttpOnly` cookie 中；读取与 server action 均从服务端获取凭据，逐次检查权限，再交给生成 SDK。未登录不读取或展示受保护数据，登出清除 cookie。

包公开的管理员操作直接导出生成函数：

| SDK 调用 | 返回类型 |
| --- | --- |
| `getOverview(baseUrl, token)` | `Promise<Overview>` |
| `listNodes(baseUrl, token)` | `Promise<NodeList>` |
| `createEnrollmentToken(baseUrl, token, body)` | `Promise<CreateEnrollmentTokenResponse>` |
| `revokeNode(baseUrl, token, body)` | `Promise<RevokeNodeResponse>` |
| `getPolicy(baseUrl, token)` | `Promise<PolicySnapshot>`，本轮新增 |
| `previewPolicy(baseUrl, token, body)` | `Promise<PreviewPolicyResponse>`，本轮新增 |
| `savePolicy(baseUrl, token, body)` | `Promise<PolicySnapshot>`，本轮新增 |
| `setNodeCountry(baseUrl, token, body)` | `Promise<RevokeNodeResponse>`，本轮新增，复用单节点响应包装 |

请求类型和响应类型均来自生成声明。策略保存、国家修改以及预览都在 Next.js 服务端检查登录身份，再调用生成 SDK。页面只消费验证通过的数据；不能用 `any`、未验证的类型断言或“缺字段就补假数据”掩盖协议漂移。会话期限、POST 清理及 Secure 配置见[Web 边界](/enrollment#web-管理员边界)。

## 前后端一起交付的含义

每个功能以一条可运行的纵向切片推进：

1. 定义 Rust 契约、路由与明确的业务语义。
2. 实现 controller 行为与必要的行为测试。
3. 生成 OpenAPI、Rust 客户端、TS SDK 与运行时 schema。
4. Agent / Web 消费生成接口，呈现真实的结果、空状态和错误状态。
5. 验证真实请求往返、生成物无漂移，并同步文档中的状态。

已验证范围包括注册令牌、持久身份、心跳、撤销、地址、配置回执、国家和策略预览 / 保存，以及管理员 Web 闭环。逻辑路径与“已校验保存”回执如实区分；未来 trace 页面需随真实探测 API 和数据面能力交付。

## 漂移与兼容性检查

在仓库根目录执行：

```sh
pnpm protocol:generate
pnpm protocol:check
pnpm test:protocol
```

`protocol:check` 调用客户端包的 `check:generated`，将结果生成到临时目录，再与仓库产物逐文件比较，不依赖已有 Git 提交。完整检查链还应包括：

- 重新导出 OpenAPI 并重新生成全部客户端，与仓库产物应无差异。
- Rust 与 TypeScript 编译通过，Agent 和 Web 不依赖手写 DTO。
- 使用生成客户端对真实测试服务执行管理认证、令牌创建、注册、心跳、清单读取和撤销往返。
- 对缺失 / 额外字段、非法时间和可空性验证严格 schema 拒绝，并验证类型化 HTTP 401 能向上传递。
- 路由、方法、认证要求、成功与错误响应契约都能关联到实际处理器。
- [注册验收矩阵](/development#注册与持久化验收矩阵)中的权限、并发与持久化行为有实际证据。
- 本轮新增配置读取 / ACK、策略预览 / CAS 保存、国家 CAS 修改的真实生成客户端往返及必填可空 / 枚举校验。
- [新切片验收清单](/development#配置与路由切片)中的升级保留身份、版本稳定性、失效策略整体拒绝与缓存失败行为。

首版使用 `/api/v1` 命名空间，但开发阶段的契约仍可能变更。本轮特定的开发 v1 SQLite 增量升级保留注册数据，Agent v1 身份文件继续使用；这不代表混合版本客户端兼容或生产版本长期迁移保证。兼容窗口、协商与废弃流程需后续明确。

## WASM 的位置

如果后续 Web 与 Rust 必须运行完全相同的策略编译 / 解释逻辑，可以评估共享 Rust 库编译为 WASM。它是可选的业务逻辑复用方式；HTTP 协议一致性由 Rust 契约、OpenAPI、生成客户端及边界验证解决。
