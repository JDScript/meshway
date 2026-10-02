# 账户会话与 App 浏览器接入兼容（Web）

本切片只补齐合并账户后端（`/device-authorizations` 改为 `userSession`）所必需的 Web 消费者：真实账户登录、首个 Owner 引导、最小邀请入口、App 浏览器批准与身份展示。全部调用生成 SDK / Zod，没有新增或手写 endpoint，没有修改 Rust / protocol / generated / 根 manifest / lockfile。

## 会话

- 一个浏览器一次只有一种 console 凭据，cookie `meshway_console_session[_<namespace>]`（HttpOnly、SameSite=Strict、`MESHWAY_WEB_COOKIE_SECURE` 的 auto/true/false 语义不变）：
  `v2.<kind>.<expires_at>.<session_id>.<credential>`，`kind` 为 `user`（仅 `mws_` + 64 hex）或 `bootstrap`（管理员令牌，不得以 `mws_` 开头）。解析时类型与格式不符即视为过期并清除；两种凭据不能互相冒充（`lib/session-cookie.ts`，有单元测试）。
- 用户会话的 Max-Age 与内部过期时间都取 controller 返回的绝对 `expires_at`，Web 不续期、不滑动；管理员令牌会话保持原 8 小时。旧 `meshway_admin_session` cookie 视为过期，登录 / 退出时清除。
- `session_id` 是 Web 随机生成的非秘密标识，只用于给客户端状态加键，不是凭据。
- 账户登录：密码仅 浏览器 → Web server action → `loginAccount`；不进 localStorage、日志或 ActionState（失败时只回显邮箱）。管理员令牌登录改用 bootstrap-only 的 `listAccountPasswordResets` 证明，用户会话贴进令牌框会被拒绝。新登录前会撤销仍存在的旧用户会话（尽力而为）。
- 退出分两步：`signOutAction`（受登录栅栏约束）只对渲染时的登录调 `logoutAccount`，不改 cookie；随后的独立请求 `discardSignInAction` 仅当浏览器此刻仍是同一登录时清 cookie。controller 不可达时仍清本地 cookie 与草稿，并跳转 `/login?reason=logout-unconfirmed` 明确说明“未确认撤销，原到期前可能仍有效”，不伪称已撤销。401 视为已失效。

## 权限与错误

- 所有页面读取与 server action 仍在服务端取 cookie 凭据，并以 URL 中的固定 network 调用；授权由 controller 在写事务内复核。用户的网络列表只有其活动 membership，其他网络为 404，选择 cookie 只影响落地页。
- 公共 helper：`isCredentialRejected`（401，结束会话并转登录）与 `isPermissionDenied`（403，保留会话）。`describeError` 对 403 前缀“当前账户没有此操作的权限”。现有各 action 只在 401 时注销，所以 Member 的 403 显示为错误、不登出。
- 顶部 NavUser 由 `fetchConsoleIdentity`（`getAccountSession`）实时显示邮箱与本网络角色；管理员令牌显示“实例管理员”，并标明不能批准用户设备。无法读取时显示真实错误，不回退为管理员。

## 渲染登录栅栏（服务端）

cookie envelope 未签名，浏览器内多个标签页共享同一 cookie。为保证“为 A 渲染的页面 / 草稿 / 已排队请求不会借 B 的 cookie 执行”，由服务端强制：

- **登录键**：`<kind>:<marker>`，`marker` = SHA-256（域分隔 `meshway-web-sign-in-fence-v1`，含 kind、Web sign-in ID 与 credential 本身）截取 128 bit（`lib/session-cookie.ts::signInKeyOf`）。不可反推 credential；改 cookie 中的 session ID 或 kind 不能让一个 credential 冒充另一个登录；同一用户重新登录（新 credential）也得到新键。它只是浏览器操作归属栅栏，不是授权凭证：controller 仍对 credential 鉴权并复核绝对期限。
- **传递**：`(console)` 布局渲染时计算登录键，作为 `DraftVaultProvider` 的 key 与 `useSignIn()` / `useActionScope()` 的值。每个网络 action 第一个参数为 typed `ActionScope { networkId, signIn }`（Access、routing、connections、tags、network、enrollment / managed enrollment、managed listener、节点撤销及全部只读刷新：runtime、node status、endpoint discovery、latency history、reload）；全局 action（切换 / 创建网络、退出）显式带 `signIn`。独立页面（App 批准 / 拒绝、首 Owner 认领、邀请接受与创建）由服务端渲染写入隐藏字段 `sign_in`。
- **校验**：`requireNetworkActionContext(scope)` / `requireRenderedSignIn(signIn)` 在转发 controller 前比较请求所带 cookie 的登录键；缺失或不符即跳转 `/session-changed`（“登录已变化”，不执行、不提交草稿、不清当前 cookie）。App 批准页不符时返回错误并提供刷新，不批准也不拒绝；页面 GET 仍不批准。网络 scope、CAS 期望版本与 controller 的 Read/Manage 事务复核相互独立、全部保留。登录 / 注册等建立新登录的入口不受栅栏约束。
- **迟到的 401 / 登出**：action 中的 401 不再发送删除 cookie 的 Set-Cookie，只跳转 `/login?reason=expired`；登录页用独立请求按浏览器当时的 cookie 向 controller 复核，被拒才清除，仍有效则直接回到目标页。本地过期也不在 action 中删除。因此 A 的被挂起请求晚到的 401 或登出结果不会抹掉 B 的 cookie。
- `SessionSync` 的广播 / 焦点刷新仍保留，只是尽早丢弃旧草稿的体验优化，不是安全边界。

## 首个 Owner、邀请

- `/onboarding`（仅管理员令牌会话）：列出网络及已有 Owner；对没有 Owner 的网络显式 `claimFirstOwner`：`new_account`（本人邮箱 + 本人在此设置的密码，二次确认）或 `existing_account`（本人邮箱密码登录取得临时会话作为证明，认领后立即撤销）。不认领旧节点，不改管理员会话；成功后可“退出管理员会话，使用该账户登录”。Web 不为现网生成邮箱或密码。
- `/invitations/new?network=`：Owner / Admin / 管理员令牌为单个邮箱创建一次邀请（`createAccountInvitation`），链接 `/<origin>/invite#token=mwi_…` 把秘密放在 fragment，只在创建结果中显示一次。角色选项只是提示，role ceiling 由 controller 执行。
- `/invite`：客户端读取 fragment 后立即从地址栏移除；未登录 / 管理员令牌会话用 `registerInvitedAccount` 注册并自动登录；已登录用户用 `acceptAccountInvitation` 接受，不改密码。没有无邀请的公开注册。

## App 浏览器接入（契约不变）

- `/authorize-device` 的参数、`meshway://enroll/callback` allowlist、state 往返、`returnTo` 规范化以及四个节点 proxy（`devices/exchange`、`agents/heartbeat`、`agents/configuration`、`agent/dns`）完全保持；App 只拿 code。
- 用户会话：显示登录邮箱与“切换账户”，只列出该用户的活动 membership 网络及角色（Member 也可批准本人设备），批准时显式提交 network，调用 `authorizeDevice(userSession)`；403 显示“不是该网络成员”，401 结束会话。GET 与页面查看不批准任何东西；拒绝也先校验用户会话。
- 管理员令牌会话：不显示批准表单，提示“需要账户登录”，可退出后用账户登录（保留 returnTo）或去认领首个 Owner；server action 被重放也只返回错误，不产生 code。

## 验证（本 worktree，`9c02e22` 之上）

- `pnpm --filter @meshway/web check`（strict typecheck + eslint 0 warning）、Web 单元测试 **156** 项、`pnpm build:web`、`tsc -p tsconfig.e2e.json`。
- Playwright 全部 **14** 个 spec（原 13 个 + 新 `account-session.spec.ts`），真实临时 controller + `next start`，未注入 cookie：
  - `device-login.spec.ts`：管理员令牌登录后不能批准（UI 与直接 API 均拒绝）→ UI 认领首个 Owner → 切换为账户登录 → 批准 → PKCE 兑换经 Web proxy → 节点归属 Owner 的 membership；proxy、拒绝、非法 redirect 原有断言保留。
  - `account-session.spec.ts`：错误密码不建 cookie、任意 returnTo 落回网络、cookie HttpOnly/Strict/≤controller 期限；Owner 发邀请 → 受邀人经链接注册；Member 读页面、写入 403 不注销、跨网 404、替换 network 的重放被拒、批准本人设备并归属本人；退出后先前批准的 code 不能首次兑换；密码 reset 后 code 失效且浏览器转登录；管理员令牌仍能管理、不能批准、用户会话不能当令牌；跨 origin / 匿名 / 管理员令牌重放批准均不产生 callback；已有账户在 UI 认领另一网络 Owner；另一标签切换账户后原标签草稿被丢弃；密码、会话、管理员令牌不出现在 HTML / Flight，所有秘密不出现在 Web / controller 日志；390 / 1440 无横向溢出。
  - `multi-network.spec.ts` 的浏览器批准改为真实账户（两网 Owner）批准；其余 spec 未改，管理员令牌入口保持原选择器。

- `session-fence.spec.ts`（新增）：同一 cookie jar 内真实 UI 登录 A、渲染并捕获 App 批准与 Access 保存，另一标签真实登录同网 Owner B（A 的标签禁用 BroadcastChannel）；以浏览器当前 B cookie 重放旧载荷不产生 code / ownership / revision，A 标签真实点击保存跳到“登录已变化”，B 会话仍有效；B 自己渲染的批准（节点归 B）与保存成功；把 A cookie 的 session ID 改成 B 的再重放 B 的渲染载荷被拒；A 被挂起请求的迟到 401 与迟到登出均不清 B 的 cookie。修复前同一 spec 两项测试均失败（code 签发、revision 1→2、草稿保存、B cookie 被清）。

## 未做（明确边界）

- 完整成员管理（列表、改角色、移除）、批量邀请、邀请撤销 / 列表 UI、密码修改与重置 UI、账户中心、旧节点 ownership 认领 UI。
- CLI request / poll 链接登录、外部 OIDC。
- 会话绝对过期的端到端等待未做（7 天）；过期竞态由 controller AR1 测试覆盖，Web 侧以 cookie 内的绝对期限 + 401 处理。
