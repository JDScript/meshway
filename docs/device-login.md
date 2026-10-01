# App 浏览器登录与节点接入

App 登录本质是 SSO：App 打开 controller Web，由 Web 完成人类登录与接入授权，再通过回调交付一次性兑换码。App 不对接外部 IdP，也不持有管理员会话。这是 authorization-code + S256 PKCE 形状的 Meshway 专用协议，**不是通用 OAuth 授权服务器**。

## 本切片与后续边界

已实现 controller 契约、事务兑换、SQLite v8 升级、生成 OpenAPI / Rust / TypeScript 客户端与 Zod、Web 授权页及登录返回。当前人类登录是已有管理员会话；批准 / 拒绝都在服务端重新验证登录，批准经管理员 bearer 调用 controller。未来替换 controller 侧的人类认证 / 授权，不改变 App 请求与回调契约。

**外部 OIDC、接入资格、用户权限、节点接入有效期及审批策略：controller 侧后续决定，App 不感知。** 本切片不新增 users、principals、owners、节点到期或 App 节点类别。兑换码的五分钟期限仅限制首次消费，不是节点有效期。

Apple 原生代码尚未接线：`apps/apple` 后续实现 `ASWebAuthenticationSession`、Keychain 与 Rust core 调用。本切片注册成功不代表 App 已接入真实隧道；真机 NetworkExtension 验证仍待后续条件具备。

## App 接入步骤

1. 用户选择可信 controller / Web 地址。App 在网络请求前私有、持久保存随机节点 bearer（64 个小写十六进制字符）。失败与恢复不得重新生成身份。
2. App 生成随机 `state` 与 PKCE `code_verifier`（43–128 个 ASCII unreserved 字符），计算 `code_challenge = BASE64URL(SHA256(verifier))`，不带 padding；保存本次尝试的 verifier、state 与原名称 / 版本。
3. 在系统浏览器会话打开 Web `/authorize-device`，查询参数如下。URL 不携带节点凭据或 verifier。

| 参数 | 约束 |
| --- | --- |
| `redirect_uri` | 精确 `meshway://enroll/callback` |
| `state` | 不透明，1–256 个 Unicode 字符，不含控制字符 |
| `code_challenge` | 规范 S256 base64url，43 字符 |
| `name` | 1–128 个 Unicode 字符，包含非空白文本 |
| `agent_version` | 1–64 个 Unicode 字符，包含非空白文本 |

4. Web 先验证请求；非法回调仅显示错误页，不跳转。未登录时进入现有登录页，成功后返回**重新验证并规范化的授权路径**。确认页展示设备名称 / 版本、网络名称与 controller 地址；GET 不创建兑换码或节点。
5. 管理员明确批准后，Web 调用 `authorizeDevice`，回调 `meshway://enroll/callback?code=…&state=…`；拒绝回调 `?error=access_denied&state=…`。controller 故障留在页面显示真实错误。
6. App 使用 `ASWebAuthenticationSession(callbackURLScheme: "meshway")`，核对完整 callback URI、state 与选定 controller；取消 / 拒绝不创建节点。
7. 校验回调后，App 先把兑换码写入私有待注册状态，再使用它作为 bearer，通过生成的 `exchangeDevice` 发送原 verifier、redirect URI、已保存节点凭据及相同名称 / 版本；成功返回既有 `AgentNodeResponse`，原子保存服务端节点 ID 后清除待注册秘密。响应丢失时复用整份绑定重试。
8. 后续使用既有节点 bearer 接口。X25519 公钥继续经 `publishConnectionIdentity` 单独不可替换地绑定，本次兑换不改变其语义。

远程浏览器、Web→controller 与 App→controller 通道均使用 HTTPS；回环 HTTP 仅用于开发。App 不保存管理员 bearer、浏览器 cookie 或 IdP token；兑换码 / verifier / 节点凭据不写普通日志。

## Controller 契约

Rust DTO 位于 `crates/protocol/src/device_login.rs`，实际路由位于 `apps/controller/src/device_login_api.rs`，约束与错误以生成 OpenAPI 为准。

| 路由 / 操作 | 认证 | 请求 | HTTP 200 |
| --- | --- | --- | --- |
| `POST /api/v1/device-authorizations` / `authorizeDevice` | 管理员 bearer | `{ redirect_uri, code_challenge, name, agent_version }` | `{ code, expires_at }` |
| `POST /api/v1/devices/exchange` / `exchangeDevice` | 兑换码 bearer（`enrollmentBearer` scheme） | `{ code_verifier, redirect_uri, node_credential, name, agent_version }` | 既有 `{ node: NodeSummary }` |

`state` 只在 Web / App 往返中回传，controller 不需要它来验证 PKCE。回调 allowlist 采用协议常量，首片只允许一个精确 URI，避免增加尚无消费者的部署配置或任意客户端注册。Web 校验来自 Rust 约束生成的 Zod schema。

错误为净化的 `ErrorResponse`：`400` 格式错误、`401` 无效 / 过期 / 不同绑定、`403` 已撤销、`409` 凭据已绑定、`422` 容量不足；body 限制沿用 `413` / `415`。错误不包含原始秘密。

## 存储、恢复与统一节点

SQLite v8 只给 `enrollment_tokens` 增加可空 `device_binding`，存储公开的 redirect URI、challenge、名称 / 版本。旧 token 该列为 null；原身份、凭据摘要、IP、公钥、策略及 revision 保留。

兑换码为随机 256-bit bearer，服务端仅保存摘要，首次消费期限固定 300 秒。新增绑定复用原注册事务：校验 PKCE / 元数据 / 期限，分配地址、创建相同 nodes 记录、绑定节点 ID 与请求摘要、推进配置 revision，全部一起提交。首次校验失败或容量不足不消费代码。新节点没有 App 标志、owner 或另一套身份系统，Access / Routing / 撤销语义与 token 注册一致。

同代码 + 同 verifier / callback / 凭据 / 原名称 / 版本重试返回同一个仍有效节点，原首次期限已过也能恢复；不同绑定拒绝，撤销永远不恢复。已消费记录保留去重依据。摘要比较使用固定长度的累积差异比较；不宣称 SQLite 查询整体恒定时间。

## 验证与背景

- RFC 7636 标准 S256 向量；controller 的错误 PKCE、回调 / 元数据绑定、首次过期、并发单节点、期限后恢复、换凭据拒绝、撤销不可恢复与匿名签发拒绝测试。
- v7→v8 迁移逐表比较既有行，再验证原节点凭据、旧 token 与重启。
- Web 参数 / allowlist / 登录安全返回单测；隔离端口与数据目录的 Playwright 实测登录→批准→回调 Location→真实兑换、匿名 action 重放、拒绝与非法回调页。
- 生成漂移与工作区 Rust / TypeScript 检查；具体运行结果见本切片提交报告。

背景参考：[RFC 8252](https://www.rfc-editor.org/rfc/rfc8252) 的系统浏览器原生登录与 [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636) 的 S256 PKCE。Tailscale / Headscale 研究仅说明人类认证与节点身份应分层；其 OIDC、owner、tag 与到期规则不构成 Meshway 需求。
