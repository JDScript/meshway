# P1a 多网络：控制台重做对接

本切片只给旧 UI 加了最小网络选择 / 创建及批准选网入口。重做控制台时应沿用下面的服务端边界，UI 可替换。

## 可直接移植的服务端层

- `lib/controller-config.ts`：运行时 controller origin，server-only。
- `lib/session.ts`：管理员 session、每个 action 的登录检查、401 清理、Secure cookie 选择。不能只依赖 layout 做 mutation 鉴权。
- `lib/network-context.ts`：选择 cookie、请求内缓存的 `selectedNetworkId(token)`、`requireNetworkActionContext(networkId)`。后者先检查登录，再用生成 Zod 校验显式 network ID，返回服务端 token / base URL / scope；网络存在性与权限由 controller 再检查。
- `lib/controller.ts`：统一认证读取、网络作用域读取、真实失败 / 401 处理；`fetchNetworks` 是全局读取。网络选择失效时仍可读取列表恢复选择。
- `lib/*-actions.ts`：所有网络 action 的**第一个参数是渲染时的 network ID**；后续参数保留原 request / expected revision / JSONC。每次均调用 `requireNetworkActionContext`。创建 / 切换网络是全局 action，重新鉴权后操作。
- `lib/device-login-actions.ts`：批准 / 拒绝都在服务端重新验证会话。批准提交表单中的 `network_id` 到生成的 `authorizeDevice`，controller 校验网络并绑定短期兑换码；拒绝仍不创建节点。
- `lib/node-proxy.ts` 与四个 `app/api/v1/**/route.ts`：原生 App 同源接口，保持透明 bearer / body 转发、方法限制、no-store 与错误状态。**不读 console cookie、不注入管理员 bearer**。

网络选择 cookie 不是权限。多标签页下 cookie 可被改写，不能在保存时用它重新决定目标。页面 / 编辑器要固定自己的 network ID，并将其与草稿 / CAS 一起保存；刷新或切换网络后重新挂载新的上下文。旧切片的 `NetworkWorkspace` 以逐网络草稿 registry 验证了无效 JSONC、注释、未应用表单和原 CAS 的恢复，也验证了另一标签页切换后旧页面保存仍写入原网络。

## API 形状

权威源：Rust 路由与 DTO → `packages/protocol/openapi.json` → 生成 TypeScript SDK / Zod。所有操作成功为 HTTP 200；closed DTO、运行时 schema、超时和状态优先错误处理继续使用生成客户端。

### 全局管理员资源

| 方法 / 路径 | SDK | 请求 | 响应 |
| --- | --- | --- | --- |
| GET `/api/v1/networks` | `listNetworks(base, token)` | — | `NetworkList { networks: NetworkSnapshot[] }`，原网络第一 |
| POST `/api/v1/networks` | `createNetwork(base, token, body)` | `CreateNetworkRequest { name }` | `NetworkSnapshot`，revision 1、真实空网络 |

`NetworkSnapshot` 保留 `{ revision, network_id, name, network: { ipv4_prefix, ipv6_prefix }, dns: { enabled, zone, ttl_seconds } }`。没有删除网络 API。名称不要求唯一，stable ID / 默认 DNS zone 不随改名变化。

### 网络内管理员资源

下表路径统一加前缀 `/api/v1/networks/{network_id}`；SDK 参数统一为 `(baseUrl, adminToken, networkId[, request])`。

| 方法 / 后缀 | SDK | 请求类型 | 响应类型 |
| --- | --- | --- | --- |
| GET `/overview` | `getOverview` | — | `Overview`，节点数包含撤销节点 |
| GET `/nodes` | `listNodes` | — | `NodeList` |
| POST `/nodes/revoke` | `revokeNode` | `RevokeNodeRequest` | `RevokeNodeResponse` |
| POST `/nodes/country` | `setNodeCountry` | `SetNodeCountryRequest` | `RevokeNodeResponse` |
| POST `/nodes/tags` | `setNodeTags` | `SetNodeTagsRequest` | `TagSnapshot` |
| POST `/enrollment-tokens` | `createEnrollmentToken` | `CreateEnrollmentTokenRequest` | `CreateEnrollmentTokenResponse`，原始 token 只返回一次 |
| GET `/network` | `getNetwork` | — | `NetworkSnapshot` |
| POST `/network` | `saveNetwork` | `SaveNetworkRequest { expected_revision, name, dns }` | `NetworkSnapshot`，改名 / DNS，地址池只读 |
| GET `/tags` | `getTags` | — | `TagSnapshot` |
| POST `/tags` | `saveTags` | `SaveTagsRequest` | `TagSnapshot` |
| GET `/access` | `getMeshAccess` | — | `MeshAccessSnapshot` |
| POST `/access` | `saveMeshAccess` | `SaveMeshAccessRequest` | `MeshAccessSnapshot` |
| GET `/policy` | `getPolicy` | — | `PolicySnapshot` |
| POST `/policy/preview` | `previewPolicy` | `PreviewPolicyRequest` | `PreviewPolicyResponse` |
| POST `/policy/save` | `savePolicy` | `SavePolicyRequest` | `PolicySnapshot` |
| POST `/flows/preview` | `previewFlow` | `PreviewFlowRequest` | `PreviewFlowResponse`，计划而非 trace |
| GET `/connections` | `getConnections` | — | `ConnectionsSnapshot` |
| POST `/connections` | `saveConnections` | `SaveConnectionsRequest` | `ConnectionsSnapshot` |
| GET `/connections/runtime` | `getConnectionRuntimes` | — | `ConnectionRuntimesSnapshot` |
| GET `/connections/discovery` | `getEndpointDiscoveries` | — | `EndpointDiscoveriesSnapshot` |
| POST `/device-authorizations` | `authorizeDevice` | `AuthorizeDeviceRequest`，body 不增加 network 字段 | `AuthorizeDeviceResponse { code, expires_at }` |

原未限定管理员路径已删除，不应继续调用。所有 CAS 在一个网络内共享；不同网络可恰好有相同 revision，故单独保存 revision 不足以标识草稿目标。跨网络节点、selector、tag、连接引用拒绝；预览沿用 `valid:false` / invalid 决策诊断，不把未匹配外网节点补入清单。

### 错误码

- `400`：无效 JSON、未知字段、请求格式 / 参数；`413`：端点 body budget；`415`：不是 JSON。
- `401`：管理员 / 节点 / 接入凭据缺失或无效；Web 应进入原会话清理流程。管理员验证先于网络存在性检查。
- `404`：网络不存在；节点撤销 / 元数据操作的节点不在所选网络也可为 404。
- `409`：该网络的 CAS 过期、凭据 / 身份不同绑定；保留原草稿与网络 / revision 基准。
- `422`：无效配置、跨网络 / 未知 selector、tag 或连接引用、publication / revision 容量。
- `403`：Agent 身份已撤销，不能重试恢复为有效节点；管理员网络 API 当前不使用成员角色。
- `500`：净化的存储错误。所有错误 payload 是 `ErrorResponse { message }`；SDK 按状态处理，不回显任意错误 body。各操作支持的完整错误集合以 OpenAPI 为准。

### App 与 Agent 兼容边界

`POST /api/v1/devices/exchange`、`POST /api/v1/agents/heartbeat`、`GET /api/v1/agents/configuration`、`GET /api/v1/agent/dns` 的 URL / request / response 保持兼容。其他 Agent 控制 API 同样由节点 bearer 推导网络；客户端不能用 body / query 选择网络。App 的浏览器请求 / 回调 / PKCE 也不增加参数，只有 Web 批准表单增加所选网络。四个 Web 同源代理仍只暴露既有路径。

## 迁移与当前实现边界

v8→v9 将现有 ID / 名称 / DNS / IPAM / 节点 / 凭据 / 接入绑定 / tags / 策略 / 连接 / 运行 lease / report / revision 原样作为第一张网络。新网络地址可重叠：IPv4 `100.96.0.0/16`、新随机 ULA `/64`、DNS 关闭，默认 `net-<network_id>.meshway.test`，TTL 30。

本阶段仍是全局管理员、Access / Routing v2 与现有手工连接配置。新网络 Access 以 v2 default allow 表达默认全成员允许；Routing v2 仍 reject。用户 / Owner、策略 v3、托管承载、自动建连和真实出口不在 P1a 里伪装实现。客户端多接入切换后续推进；同一 controller 节点身份只能归属一张网络。

验证入口：`e2e/multi-network.spec.ts` 使用真实 controller + production Web，覆盖空网络、接入令牌、批准选网、跨网 Agent 快照、逐网络无效草稿 / CAS，以及多标签页原网络保存；其他 11 个原行为 e2e 通过生成接口在第一张网络运行。
