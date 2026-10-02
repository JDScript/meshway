# App 浏览器登录与节点接入

App 登录本质是 SSO：App 打开 controller Web，由 Web 完成人类登录与接入授权，再通过回调交付一次性兑换码。App 不对接外部 IdP，也不持有管理员会话。这是 authorization-code + S256 PKCE 形状的 Meshway 专用协议，**不是通用 OAuth 授权服务器**。

## 当前实现与后续边界

已实现controller事务兑换、生成客户端、Web授权页与登录返回，并接入真实用户session、当前网络membership和设备归属。批准/拒绝逐次检查登录，批准使用用户session；bootstrap不能签发user-owned设备批准。App请求与回调保持兼容。

兑换码五分钟只限制首次消费，不是节点有效期；controller检查账户/网络资格，App不持有人类会话。外部OIDC后续，不另建App节点类别。

本地邮箱/密码、仅邀请、Owner与设备归属已实现；Agent浏览器链接登录及结果领取仍待交付，见[账户合同](/local-accounts)。

Apple 已接线 `ASWebAuthenticationSession`、Keychain、Rust 生成客户端兑换和真实心跳，见 [Apple App](./apple-app.md)。注册 / 心跳成功不代表隧道已建立；X25519 发布、agent 生命周期、NE 包流及真机验证仍待后续。

## 本地账户下的统一登录

本地账户是认证来源，SSO 是 Web、App 与 CLI 共用登录的方式。用户只在 controller
Web 输入邮箱和密码；浏览器已有有效会话时直接进入设备授权页。controller 校验用户
及所选网络的成员权限，把接入结果与该用户、网络和本次设备请求绑定，设备继续用
自己的持久化节点身份接入，不获得人类会话或管理权限。

- **Web**：建立用户会话，按用户在各网络的权限提供管理与使用入口。
- **App**：系统浏览器登录 / 授权，经已有回调与 S256 PKCE 兑换取得节点接入结果。
- **CLI**：输出浏览器授权链接，复用相同用户认证与网络授权服务；无本机回调的场景
  需增加受本次设备请求绑定的结果领取机制。具体协议与生成客户端在接入切片实现，
  不把现有 App 专用 callback 直接当作已经支持 CLI。

本地账户先落地，外部 OIDC 后续只扩展 controller 的用户认证来源。现阶段不需要为
其他第三方应用提供通用IdP、ID token或客户端注册服务。App用户会话/membership批准已接线，CLI仍待实现。

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
5. 用户选择自己有活动membership的网络并批准本人设备，Web调用该网络的`authorizeDevice`，服务端绑定session/version/membership和网络；回调 `meshway://enroll/callback?code=…&state=…`。拒绝回调 `?error=access_denied&state=…`。controller故障留页显示真实错误。
6. App 使用 `ASWebAuthenticationSession(callbackURLScheme: "meshway")`，核对完整 callback URI、state 与选定 controller；取消 / 拒绝不创建节点。
7. 校验回调后，App 先把兑换码写入私有待注册状态，再使用它作为 bearer，通过生成的 `exchangeDevice` 发送原 verifier、redirect URI、已保存节点凭据及相同名称 / 版本；成功返回既有 `AgentNodeResponse`，原子保存服务端节点 ID 后清除待注册秘密。响应丢失时复用整份绑定重试。
8. 后续使用既有节点 bearer 接口。X25519 公钥继续经 `publishConnectionIdentity` 单独不可替换地绑定，本次兑换不改变其语义。

Apple 将 credential + 完整 pending binding 作为单个 Keychain journal 原子更新，先于浏览器 / 兑换请求提交；非敏感快照另写原子 JSON。崩溃后由 journal 修复镜像，`redeeming` 重试原 code / verifier / credential / 名称 / 版本，`awaitingBrowser` 可重新打开原授权页。成功保存 node 后清 pending；Keychain 失败无明文 fallback。前台限时、无重叠心跳显示结果与时间，403 保留撤销状态。只有无已创建节点且收到原绑定 401 才提供显式过期重开（同 credential、新 state / verifier）。显式「重置此设备」只清本地身份，不撤销服务端；需在控制台另行撤销。

远程浏览器、Web→controller 与 App→controller 通道均使用 HTTPS；回环 HTTP 仅用于开发。App 不保存管理员 bearer、浏览器 cookie 或 IdP token；兑换码 / verifier / 节点凭据不写普通日志。

## Controller 契约

Rust DTO 位于 `crates/protocol/src/device_login.rs`，实际路由位于 `apps/controller/src/device_login_api.rs`，约束与错误以生成 OpenAPI 为准。

| 路由 / 操作 | 认证 | 请求 | HTTP 200 |
| --- | --- | --- | --- |
| `POST /api/v1/networks/{network_id}/device-authorizations` / `authorizeDevice` | 用户session/活动membership | `{ redirect_uri, code_challenge, name, agent_version }` | `{ code, expires_at }` |
| `POST /api/v1/devices/exchange` / `exchangeDevice` | 兑换码 bearer（`enrollmentBearer` scheme） | `{ code_verifier, redirect_uri, node_credential, name, agent_version }` | 既有 `{ node: NodeSummary }` |

`state` 只在 Web / App 往返中回传，controller 不需要它来验证 PKCE。回调 allowlist 采用协议常量，首片只允许一个精确 URI，避免增加尚无消费者的部署配置或任意客户端注册。Web 校验来自 Rust 约束生成的 Zod schema。

错误为净化的 `ErrorResponse`：`400` 格式错误、`401` 无效 / 过期 / 不同绑定、`403` 已撤销、`409` 凭据已绑定、`422` 容量不足；body 限制沿用 `413` / `415`。错误不包含原始秘密。

## 存储、恢复与统一节点

SQLite v8 只给 `enrollment_tokens` 增加可空 `device_binding`，存储公开的 redirect URI、challenge、名称 / 版本。旧 token 该列为 null；原身份、凭据摘要、IP、公钥、策略及 revision 保留。

接入令牌/兑换码有不可变网络归属，App exchange/heartbeat/configuration/DNS和回调兼容保留，网络由凭据推导。Apple现已保存隔离多接入、一次一个活动接入，暂无隧道可切换；CLI待实现。详见[网络边界](/network-management)。

兑换码为随机256-bit bearer，只存摘要，首次消费300秒。PKCE/元数据/期限、当前session/version/membership、node/IP、ownership、code消费与revision同事务；失败不消费。统一nodes记录，没有App类别或另一套身份系统。

同code/verifier/callback/凭据/原名称版本的精确重试可恢复原节点，首次期限过后也不重新注册；仍检查节点未撤销、原membership活动和ownership一致。移除再加入不能复活旧批准，不同绑定拒绝，已消费记录保留去重依据。

## 验证与背景

- RFC 7636 标准 S256 向量；controller 的错误 PKCE、回调 / 元数据绑定、首次过期、并发单节点、期限后恢复、换凭据拒绝、撤销不可恢复与匿名签发拒绝测试。
- v7→v8 迁移逐表比较既有行，再验证原节点凭据、旧 token 与重启。
- Web 参数 / allowlist / 登录安全返回与会话模型单测；Controller/Apple core 保留真实兑换与身份绑定验证，浏览器登录、批准、拒绝和回调页面手工验证。
- 生成漂移与工作区 Rust / TypeScript 检查；具体运行结果见本切片提交报告。

背景参考：[RFC 8252](https://www.rfc-editor.org/rfc/rfc8252) 的系统浏览器原生登录与 [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636) 的 S256 PKCE。Tailscale 是常规产品基线；具体 Meshway 确认项见[产品对齐](/product-alignment)，固定研究实现不自动定义本项目的 wire 契约。
