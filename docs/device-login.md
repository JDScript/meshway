# App 浏览器登录与节点接入

App 登录本质是 SSO：App 打开 controller Web，由 Web 完成人类登录与接入授权，再通过回调交付一次性兑换码。App 不对接外部 IdP，也不持有管理员会话。这是 authorization-code + S256 PKCE 形状的 Meshway 专用协议，**不是通用 OAuth 授权服务器**。

## 本切片与后续边界

已实现 controller 契约、事务兑换、SQLite v8 升级、生成 OpenAPI / Rust / TypeScript 客户端与 Zod、Web 授权页及登录返回。当前人类登录是已有管理员会话；批准 / 拒绝都在服务端重新验证登录，批准经管理员 bearer 调用 controller。未来替换 controller 侧的人类认证 / 授权，不改变 App 请求与回调契约。

**外部 OIDC、接入资格、用户权限、节点接入有效期及审批策略：controller 侧后续决定，App 不感知。** 本切片不新增 users、principals、owners、节点到期或 App 节点类别。兑换码的五分钟期限仅限制首次消费，不是节点有效期。

**已确认产品目标**已包括 controller 本地邮箱 + 密码账户、安全哈希、仅邀请加入指定网络、Owner 与设备归属；Agent 也须提供浏览器链接登录。OIDC 后续接入。这里“后续”指该切片尚未接入这些能力，不表示用户身份仍是未确认提案；App 保持浏览器 / 兑换契约，不持有管理权限，见[产品概念](/concepts#网络、用户与设备)。

Apple 已接线 `ASWebAuthenticationSession`、Keychain、Rust 生成客户端兑换和真实心跳，见 [Apple App](./apple-app.md)。注册 / 心跳成功不代表隧道已建立；X25519 发布、agent 生命周期、NE 包流及真机验证仍待后续。

## 单一服务器地址

App 只要求 **Web 控制台 origin**（HTTPS；回环 HTTP 仅开发），浏览器授权页与 App API 使用相同 origin。Next.js 仅为 `POST /api/v1/devices/exchange`、`POST /api/v1/agents/heartbeat`、`GET /api/v1/agents/configuration`、`GET /api/v1/agent/dns` 提供显式 route-handler 代理到 `controllerUrl()`，严格限制各路径的方法，运行时读取 `MESHWAY_CONTROLLER_URL`，避免 rewrites 的 build-time 配置固化。原请求 bearer / body、上游 body / status / 认证响应头透传，无缓存、不读取 console cookie、不注入管理员凭据、不记录秘密。移除 hop-by-hop / host 头，流式转发；fetch 解压后的响应去掉编码 / 长度头。其他 API（含 admin routes）不由此代理暴露。生产也可由反向代理精确路由这四个路径，保持同源入口。App GET 库存用于显示，不发送配置 ACK。

## App 接入步骤

1. 用户选择可信 Web 控制台 origin。App 在网络请求前私有、持久保存随机节点 bearer（64 个小写十六进制字符）。失败与恢复不得重新生成身份。
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

Apple 将 credential + 完整 pending binding 作为单个 Keychain journal 原子更新，先于浏览器 / 兑换请求提交；非敏感快照另写原子 JSON。崩溃后由 journal 修复镜像，`redeeming` 重试原 code / verifier / credential / 名称 / 版本，`awaitingBrowser` 可重新打开原授权页。成功保存 node 后清 pending；Keychain 失败无明文 fallback。前台限时、无重叠心跳显示结果与时间，403 保留撤销状态。只有无已创建节点且收到原绑定 401 才提供显式过期重开（同 credential、新 state / verifier）。显式「重置此设备」只清本地身份，不撤销服务端；需在控制台另行撤销。

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

背景参考：[RFC 8252](https://www.rfc-editor.org/rfc/rfc8252) 的系统浏览器原生登录与 [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636) 的 S256 PKCE。Tailscale 是常规产品基线；具体 Meshway 确认项见[产品对齐](/product-alignment)，固定研究实现不自动定义本项目的 wire 契约。
