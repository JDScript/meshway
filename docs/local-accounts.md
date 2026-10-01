# P2b 本地账户领域切片

## 状态与边界

已实现 `apps/controller/src/accounts/` 内部 Rust 领域、显式 SQLite DDL、Argon2id worker、邀请 / 会话 / 密码恢复与网络权限行为测试。**尚未安装生产 schema、接入 HTTP / Web 或改变当前管理员认证。用户登录尚未上线。** 没有新增 crate、账户数据库、公开 HTTP DTO 或手写客户端。

账户按 controller 全局唯一；membership 按网络隔离。同一用户可属于多网，Owner 不等于实例管理员。网络 ID 是资源作用域，不是授权。统一节点与现有 App PKCE / 回调合同保持现状，本模块不修改节点身份、地址、公钥、tags 或生产接入事务。

## 模型与权限

- `UserId` 是随机 256-bit opaque ID，邮箱去首尾空白并小写，唯一；不做提供商特定的 `+` 或 `.` 合并。当前内部 parser 支持 ASCII mailbox / IDNA ASCII domain，Unicode 邮箱需后续明确国际化支持。
- 用户资料与密码证明类型分开。密码输入、PHC、会话 / 邀请 / reset 原值及认证快照均不实现 `Debug` / `Serialize`。`User` 不包含密码或哈希；`email_verified=false`，复制链接不能证明邮件验证。
- 最小网络角色：Owner 管理成员 / 邀请及所有网络角色；Admin 管理网络配置、邀请 / 移除 Member；Member 只读本网。Owner / Admin 不能操作其他网络。Admin 不能授予 Admin / Owner 或管理这些成员。
- `Principal` 私有构造，区分已证明现有 `Store::authorize_admin` bearer 的 bootstrap、已验证用户 session、节点 marker。节点 marker 总是拒绝人类账户操作。实例 bootstrap 管理权限是兼容入口，不从网络 Owner 反向推导。
- 网络管理 mutation 在调用方事务内重新验证 session、credential version 和当前 role。`authorize_network(Read/Manage)` 供下一切片保护既有资源；之前一次 HTTP 验证不是长期授权证明。membership 有独立随机 ID，移除后重新邀请加入会生成新 ID，旧 device approval 不会因重新入网复活。
- 最后一个 Owner 不得移除或降级，包括 bootstrap 操作。转移采用同一外层事务先将已存在 Member 晋升 Owner，再降级原 Owner；后续 transfer API 必须将两步整体提交。没有从链接直接提升已有 membership 的捷径。
- 邀请绑定本网、精确归一化邮箱、role、签发者、期限，最多 7 天，一次消费。`set_membership` 在同一 savepoint 内**持久撤销**签发者在本网已失去授予资格的未消费邀请：Owner→Admin 只撤销 Owner/Admin 邀请，保留 Member 邀请；变为 Member 或被移除则撤销其全部未消费邀请。角色恢复、移除后以新 membership 重新入网都不复活旧链接。其他网络邀请、已消费记录及 bootstrap 签发不受影响。role 变更失败或外层事务回滚，同时回滚邀请失效。已消费绑定不可更新，跨网撤销拒绝。
- 已存在账户必须通过本人的有效 session 接受邀请；邀请注册入口拒绝该邮箱，不覆盖全局密码。不提供无邀请 signup。账户 + membership + 消费邀请在同一次事务保存。
- 全局密码恢复只允许实例 bootstrap；某网 Owner 不能接管跨网用户的密码。reset 最多 1 小时，单次、可撤销，并绑定 credential version。重置 / 本人密码变更增加 version、废止全部旧 session 及其他未消费 reset。
- session 最多 7 天，绝对过期，无自动滑动延期。退出撤销当前 session；本人 / bootstrap 可撤销某用户全部 session。会话失效不换节点身份，本切片不声称已撤回设备数据面。
- `mwi_`、`mwr_`、`mws_` 前缀分别对应 invitation / reset / session，后接 OS 随机 256-bit 原值。只保存 SHA-256 摘要；严格类型前缀和独立表拒绝混用，与无前缀旧节点 / enrollment bearer 不兼容。

## 两阶段密码工作与竞态

设备授权存储 binding 同时记录稳定 membership ID，兑换核对当前 membership ID；仅检查 user / network 二元组不足以阻止被移除又重新加入后复活旧批准。

采用维护的 RustCrypto `argon2` 0.5.3，Argon2id v19，`m=65536 KiB, t=3, p=1`、128-bit OS 随机 salt、256-bit output，以 salted PHC 保存。不同账户 / 每次密码变更重新生成 salt；持久输入参数仅接受本模块的有界参数集，不能从损坏的库读取任意巨大 KDF 参数。

`PasswordWorker::new` 在启动时初始化 dummy PHC；默认建议 `new(2)`，最多 2 个 KDF，约 128 MiB KDF working memory，构造上限 8。`try_acquire_owned` 耗尽立即 `Busy`，不排无界队列。permit 随 `spawn_blocking` 任务持有，HTTP future 取消不会提前释放 CPU 预算。不存在账户也执行 dummy verify；外部登录错误应统一 `Unauthorized`，不要映射邮箱存在 / 密码错误。

调用顺序必须如下，任何 SQLite / Store mutex 不得跨 `await` KDF：

1. 获取数据库锁读取 `credential(email)` / `prepare_registration(token,email,now)` / `prepare_reset(token,now)`，复制私有快照或 intent，释放锁。
2. 由 `PasswordWorker::verify` / `hash` 执行有界 blocking 工作。请求体提前限制，密码 12–1024 bytes；注册 / 登录 / reset 还须在 HTTP 入口按来源 + 账户进行速率预算，不能仅依赖 CPU 并发限制。初始化 worker 只做一次。
3. 重新获取锁开启 `IMMEDIATE` 事务，`finish_login` 复核用户 credential version；注册复核邀请、邮箱与账户不存在；reset 复核 token 期限 / 状态 / version。密码变更同时复核当前 session 和旧密码证明。过期 / 撤销 / reset 发生在 KDF 中时拒绝旧证明。
4. 提交外层事务成功后才将一次返回的 token 写入安全 cookie / 复制链接响应。外层 commit 失败不得把 token 当作有效结果。

`AccountError` Display / Debug 均是净化分类，不携带 SQLite 错误文本、原请求、邮箱、PHC 或秘密。HTTP 接线不得打印私有 intent / password / token / cookie，也不得把原数据库错误注入响应。

## 迁移与 Store handoff

`accounts::SCHEMA_FRAGMENT` / `accounts/schema.sql` 是**待集成迁移片段，不分配 schema 版本**。由协调者在 P2a 整合后的统一下一版本迁移中复制 / 调用一次，不在 `Store::open` 本切片新增入口，不在 read 时建表，不写 `PRAGMA user_version`，不打开独立长期库。

DDL 的网络 FK 使用真实 v9 `networks(network_id)`，不是概念草案的 `networks.id`。测试加载整个 `store/schema_v9.sql` 到 tempfile SQLite 后再安装片段，验证 FK 与 DDL rollback。表包括 `account_users`、`account_memberships`、`account_invitations`、`account_sessions`、`account_password_resets`。独立 snapshot / credential version 不推进旧网络 publication revision；后续 membership 撤回影响设备准入 / 编译时，必须在同一根事务推进对应网络 revision 并 fence 旧 runtime。

根 Store 整合建议：

1. 沿用调用方原 Connection、FK ON 和单一迁移事务；片段、旧身份映射元数据、完整 FK check、统一 `user_version` 更新在同一 `IMMEDIATE` 事务。任一步失败全部回滚。DDL 无 `IF NOT EXISTS`，防止掩盖不完整安装。
2. `Accounts::new(&Connection)` 只读；所有 mutation 接收调用方 `&mut Transaction`，内部 savepoint 保证 Err 时本操作账户 / membership / token 均回滚，即使调用方继续提交其他工作。调用方事务可以组合节点接入、用户归属和 revision 更新；任何组合步骤失败必须回滚**外层**，不能捕获 Err 后提交半个组合操作。
3. 当前 Store `NetworkId::as_str` 只在 `store` 内可见，本切片没有改中心文件。暂用私有 `AccountNetwork` 从相同 `networks` 查询验证，绝不反序列化 / 任意构造，也不提供 HTTP scope 类型。协调者整合时开放 `NetworkId` 的 crate 内只读 accessor，将该内部适配替换为现有 `NetworkId`，不要维护第二套网络身份。根 Store 封装账户事务方法，不把原连接公开给 API。
4. `prove_bootstrap(&Store,bearer)` 必须在真实当前实例证明旧 admin bearer；当前它不可轮换，故可短期携带 proof。未来引入 bootstrap rotation 时在事务重新验证其版本，不把 proof 缓存为永久管理 session。
5. 第一 Owner 只能在无 Owner 的目标网络显式认领，需真实邮箱 + 密码，不能生成虚假邮箱、把 bearer 当密码或改旧 node 身份。已存在用户使用 `claim_existing_owner`，同时出示 bootstrap 和本人 session；不重置密码。网络创建 / 首 Owner assignment 后续必须同一外层事务。
6. 同一全局用户跨网的 membership 才是设备归属候选，未来 nodes 归属 FK 应引用 `(network_id,membership_id)`，不直接用任意全局 user_id。membership 撤销保留带 revoked_at 的旧行，不复活旧 ID；重新邀请创建新行，partial unique index 只保证每网每用户一个活动 membership。保留行便于后续旧节点 ownership FK / 审计；正式集成须在撤销同根事务停止其用户节点准入、发布撤回并终止旧 runtime。

### 旧节点与 tag 映射方案（待接线）

保留既有 node_id、network_id、bearer hash、公钥、双栈地址、enrollment 消费绑定和所有 tags 字节。旧节点先显式标 `legacy-unassigned`，由已认证网络 Owner 在 Web 选择本网络 membership 认领；不因认领 first Owner 自动给所有旧设备换归属。现有 tag 是管理员分配属性，不自动改成 tag-owned 身份。后续 tag-owned 接入记录预分配 tag identity 与签发者 audit，须解决“身份 tag 最后一个移除”的产品冲突，不能静默转换。所有权 / tag 迁移和节点准入撤回是后续独立集成事务。

## 后续 API 草案（不是生成契约）

### 可管理的链接资源

已补齐内部公有资源引用 `InvitationId` / `PasswordResetId`（独立随机 256-bit opaque ID，可从字符串显式校验）。ID 不是 bearer、不是授权凭证；取得 ID 后仍须校验 principal 与 network。原 invitation ID 现在类型化返回；reset ID 只加入尚未上线的 schema 片段，无 `user_version` 分配。

- `create_invitation` 返回 `IssuedInvitation { metadata, secret }`，`issue_reset` 返回 `IssuedPasswordReset { metadata, secret }`。包含 secret 的结果不实现 Debug / Serialize；secret 只在创建成功且根事务提交后返回一次，刷新页无法重新获取。库内仍只保存 token hash。
- `InvitationMetadata` 仅有 ID、network、目标 email、role、issuer user、expires_at、revoked_at、consumed_by / consumed_at；不含 secret / hash。`Accounts::list_invitations(principal, network, now)` 与 `invitation_metadata(principal, network, id, now)` 是按真实身份授权的管理投影。Owner / bootstrap 可见本网全部，Admin 仅可见其能管理的 Member 邀请，Member / node 无权限。过期、撤销、已消费记录仍可查看，跨网 ID 查询拒绝。
- `revoke_invitation(tx, principal, network, &InvitationId, now)` 按公有资源 ID 撤销；不再需要原链接。与查询使用同一个 role 上限，Admin 不能撤销 Owner / Admin 邀请。未知或跨网 ID 返回净化拒绝；重复撤销保留第一次 revoked_at。
- `PasswordResetMetadata` 仅有 ID、目标 user、expires_at、revoked_at、consumed_at、credential_current；最后一项表示签发时 credential version 是否仍当前，不暴露版本、PHC 或 token hash。`list_password_resets(principal)` / `password_reset_metadata(principal, &PasswordResetId)` / `revoke_reset(tx, principal, &PasswordResetId, now)` 均只允许实例 bootstrap，网络 Owner 不能查看或撤销全局恢复资源。未知或错误类别资源 ID 拒绝。
- metadata 类型只用于本模块内部管理投影，尚不是手写 HTTP DTO。未来按权威协议生成契约；管理列表 / 详情也需在调用方 read transaction 内完成身份与资源查询，不能信任页面缓存的角色。

新 reset ID 与 invitation ID 均由显式不可变 binding guard 保护；管理刷新、详情和撤销查询完全不选择 token hash 或原值。不保留按 secret 撤销 helper，既有行为测试同步切换到公有 ID。

协议权威仍是未来 `crates/protocol` Rust DTO + 实际路由，必须一起生成 OpenAPI / Rust / TS / Zod 并跑 drift 与全部消费者。本切片下列仅 handoff：

| 草案操作 | 主体 / 必须事务 |
| --- | --- |
| `POST /api/v1/auth/login` | 邮箱密码；KDF 后 `finish_login` version 复核事务；只返回 session 原值一次 |
| `GET /api/v1/auth/session`, `GET /api/v1/me/networks` | 用户 session；实时验证，仅自身 membership |
| `POST /api/v1/auth/logout`, `/auth/sessions/revoke` | 当前用户或实例 bootstrap；会话撤销事务 |
| `GET/POST /api/v1/networks/{id}/invitations`, `GET /invitations/{invitation_id}`, `POST /invitations/revoke` | 本网 Owner / 可授角色 Admin / bootstrap；列表 / 详情只公有 metadata，创建单次返回 secret，撤销请求为 invitation_id；事务复核 role 与网域 |
| `POST /api/v1/auth/invitations/register` | 邮箱绑定邀请 + 新密码；KDF 后账户 / membership / 消费事务，无匿名 signup |
| `POST /api/v1/auth/invitations/accept` | 已存在用户本人 session；membership / 消费事务，无密码覆盖 |
| `POST /api/v1/networks/{id}/members/save` | 本网 Owner / 限定 Admin / bootstrap；最后 Owner 保护，设备准入撤回同根事务 |
| `GET/POST /api/v1/auth/password-resets`, `GET /password-resets/{reset_id}`, `POST /password-resets/revoke` | 实例 bootstrap；列表 / 详情只公有 metadata，创建单次返回 secret，撤销请求为 reset_id；全局 user reset，不发邮件 |
| `POST /api/v1/auth/password-resets/redeem` | 独立 reset bearer；KDF 后消费 / 密码替换 / 旧 session 与 reset 失效事务 |
| `POST /api/v1/me/password` | 本人 session + 旧密码证明；新 KDF 后 version CAS 与会话废止事务 |
| `POST /api/v1/networks/{id}/owner-claim` | bootstrap + 新真实账户或本人 session；无 Owner 前提与 membership 原子提交 |

Web 服务端每次 mutation 实时校验人类身份；浏览器 session Cookie 应 Secure / HttpOnly / SameSite，登录 / mutation 做 CSRF 和 origin 检查，复制链接不进普通日志 / referrer。浏览器 session 不能封装或返回全局 admin.token；App / CLI 不取得此 cookie。

### Meshway 统一浏览器 SSO 与设备授权

本地邮箱密码也是 controller 的人类认证来源。Web / App / CLI 共用 controller 用户账户与 Web 会话；外部 OIDC 未来只替换认证来源，不是完成内部 SSO 的前提，不实现通用第三方 OAuth/OIDC 授权服务器。

已实现精确领域联结点：

```rust
let principal = Accounts::new(&transaction).authenticate(browser_session, now)?;
let network = Accounts::new(&transaction).network(selected_network_id)?;
let subject = Accounts::new(&transaction).authorize_device(&principal, &network, now)?;
// subject.owner() 是本网络真实 membership；Member 可接入自己的设备。
// subject.binding() 是待与原 PKCE code 同事务存储的内部授权记录。
// 后续 first exchange 在同根事务：
let subject = Accounts::new(&transaction).revalidate_device(stored_binding, &code_network, now)?;
// 创建原 node 身份，owner = subject.owner()，消费原 PKCE code 并推进 revision。
```

`DeviceAuthorizationSubject` 私有构造，`DeviceAuthorizationBinding` 只有 network、user、membership ID、session digest、credential version，无密码 / cookie / admin bearer。它是内部存储联结点，不是第二套 HTTP 请求。bootstrap / node principal 不能产生 user-owned admission；真实用户 session 可以复用，不需再走管理员登录。签发与首次兑换都检查所选 membership；logout、session 绝对过期、reset / 密码变更 / session 撤销、membership 删除均拒绝首次兑换。role 变为 Member 仍允许接入本人设备，不增加管理权限。binding 与 immutable code 的网络不符拒绝。测试直接覆盖这些接口和失效交错。

App 继续系统浏览器 → Web 登录 / 网络选择与授权 → 已有 S256 PKCE code → `meshway://enroll/callback` → 本地已持久节点 credential + code 兑换。下一切片将 `DeviceAuthorizationBinding` 与 device code 保存为同一事务记录，并在消费时调用 `revalidate_device`；创建 node、归属 membership、消费 code、地址分配和 revision 更新同事务。不能把人类 session 当 enrollment token，也不能批准任意其他网络。App 不感知邮箱密码或外部 OIDC，不取得浏览器 session / 管理员 bearer，既有回调、exchange 与 heartbeat 合同保持兼容。

CLI 链接登录的最小适配草案采用 **device-request 绑定 polling**，不增加任意 callback URI：

1. CLI 先私有持久保存原节点 credential、本次 request journal / PKCE verifier、随机 state，生成公开 S256 challenge。创建短期 request 时提交 challenge、元数据与 node credential 的承诺摘要（domain-separated hash）；服务端返回 opaque request ID、仅 CLI 持有的高熵 polling secret、5 分钟绝对期限和浏览器 URL。polling secret 仅 hash 落库，与其他 token 类别区分；浏览器 URL 不含 node credential / verifier / polling secret。
2. 浏览器使用已有用户 session，明确展示设备元数据 / controller / 所选网络，用上述 `authorize_device` 取得 user / membership。确认事务把 request ID + challenge + 原设备元数据 + node credential 承诺 + selected network + user / session / credential version 与一次性 code 不可变地绑定。用户在浏览器已登录可直接授权；GET 和 polling 不隐式批准。
3. CLI 按服务端限频（建议至少 2 秒、最多原期限内）用 polling secret 查询同一个 request，得到 pending / denied / expired 或已批准的一次性结果。为避免响应丢失造成新的身份，可在未兑换、原期限内对同 request / polling secret 幂等返回**同一**短期结果；不得签发第二个 code。若需要重交付原值，code 使用可审查的临时加密保管方案，不能为了 polling 把 raw code 明文长期落库；正式契约前选定具体机制。
4. 兑换携带 request 绑定证明、PKCE verifier、本地持久 node credential 和原元数据；校验 challenge / request / credential 承诺 / immutable network / user 以及 `revalidate_device`。一次性 code 消费 + 原 node 创建 / membership ownership + 地址 / revision 同事务。错 request、错 verifier、换节点 credential、跨网覆盖均拒绝。CLI 只得到与本次接入绑定的短期结果 / 节点响应，不得到人类密码、cookie 或管理员 bearer。
5. 批准前 session 或 membership 失效则不能批准；批准后、首次兑换前失效则 code 不得建新 node，polling 显示重新授权所需状态。首次成功后 session/logout 不自动换节点身份；已消费 code 的精确重试保持既有 App 绑定恢复语义，仍必须核对 node 未撤销及本网络 membership 资格，不得复活已撤销设备。membership 移除的节点撤回由正式 P2b 集成完成，不由 domain proof 假称已生效。

本切片实现用户主体和资格接口；request 生命周期 / polling / 一次性 code / ownership 事务与 CLI journal 接线由协调者后续实现，并生成正式协议。没有键盘复制密码、浏览器 cookie、admin token 或手工 profiles 流程。本切片不扩写 Agent CLI，不扩大 App redirect allowlist，不新增 OAuth provider / JWKS / 第三方 client registry。**P2b 正式完成须 Web / App / CLI 共用同一个用户认证与设备授权闭环，领域表交付不是其最终完成定义。**

## 验证与依赖

必要叶依赖：RustCrypto `argon2`、`zeroize`，以及已有 workspace `getrandom` / `sha2`；候选 `Cargo.lock` 独立提交供协调者审查。没有 HTTP / 协议变化，消费者全量生成检查留到接线切片；本切片运行 controller 行为 / 既有 tests、fmt 和 Clippy。

测试包括实际 v9 FK、DDL rollback / user_version 不变、邮箱归一化、盐与 PHC、错误密码 / 不存在用户、一次邀请过期 / 撤销 / 重放 / 绑定 / 两独立连接竞争、已有用户密码不改、role 上限 / 跨网 / 最后 Owner 与原子转移、session 过期 / 退出 / 全部撤销、reset / change 密码与旧 session 失效、两独立连接 reset 恰好一次、错误 secret 类别、stale KDF proof 被 reset version 拒绝、注入存储失败无半个账户 / membership / 消费记录、reset 存储失败保留旧密码和 session、SQLite 错误净化、有界 KDF 拒绝、真实 bootstrap bearer 证明，以及用户 session → 本网 membership → device subject 的资格、session/reset 失效与移除后重加入不复活批准。

复现命令（在该 worktree，独立 target；feature 组合串行）：

2026-10-01 验证：账户领域 **20** 项行为测试、既有 Store **40** 项单元测试、既有 HTTP **24** 项测试，合计 **84 passed / 0 failed**。Controller 全 target / 全 feature Clippy（`-D warnings`）与 `cargo fmt --all -- --check` 通过。首次真实 bootstrap fixture 因 tempfile 目录非 0700 被私有权限检查拒绝，修正测试目录权限后完整重跑通过；生产权限检查未放宽。

审查补齐一先增加两个失败回归，分别复现 Owner/Admin 邀请在角色恢复后复活、成员变更未触发持久邀请撤销。修复后覆盖 Owner→Admin 的 Member 邀请保留、Owner→Member→Owner、移除后重新加入、跨网不受影响、最后 Owner 拒绝、注入存储失败及外层 rollback 的原子性。

审查补齐二增加两项针对性测试：丢弃创建返回值并重新连接 SQLite 后，按公有 ID 读取 metadata / 撤销；验证 Admin 的 Member 上限、Member/node 拒绝、跨网 / 未知 / 错误类别 ID 拒绝、secret 不能作为 ID、metadata Debug 不含原 token / hash，以及已消费 / 撤销 / 过期状态可查和 ID 不可替换。

两项审查补齐后的完整验证：账户领域 **24**、既有 Store **40**、既有 HTTP **24**，合计 **88 passed / 0 failed**；Controller 全 target / 全 feature Clippy `-D warnings`、fmt check 通过。新增 reset ID 仅在未上线 schema 片段；本轮无依赖或 lockfile 变化，原独立 lock 候选继续供协调者审查。

```sh
CARGO_TARGET_DIR=target cargo test -p meshway-controller --tests
CARGO_TARGET_DIR=target cargo clippy -p meshway-controller --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
