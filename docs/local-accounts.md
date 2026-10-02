# P2b 本地账户 Controller / API 接线

## 状态与边界

本文完整 API handoff 从原账户 `7ceb936` 同步到最终组合分支 `integration/account-p2a`，并追加 AR1/AR2/SG1 修复语义。现有账户领域已接入 Controller 的真实 HTTP 路由、原 Store、网络管理权限与 App PKCE ownership 事务。权威 DTO 位于 `crates/protocol/src/accounts.rs`，生成 OpenAPI / Rust / TypeScript / Zod；没有另造账户领域或数据库。以下能力是此分支交付，**不是 P2b / Web / App / CLI 的全部完成声明**。

账户按 controller 全局唯一；membership 按网络隔离。同一用户可属于多网，Owner 不等于实例管理员。账户已经复用原 `Store::NetworkId`，不存在第二套 `AccountNetwork`。网络 ID 是资源作用域，不是授权。App 回调、exchange、heartbeat 请求 / 响应合同兼容，批准入口改用真正的用户 session。

## 分支安装与合并边界

- `store/account_schema.rs::install` 接收原 Store 的迁移事务，安装现有账户片段及 `store/ownership.sql`，检查真实 FK，**不写 `user_version`**。现有 v9 数据保留，旧节点 ownership 为 `membership_id=null`（legacy-unassigned）；不生成账户，不转换 tags，不认领设备。
- 本分支的 `AppState::open_with_network` 显式调用 `install_account_integration_schema`，用于隔离 fixture / API 验收；这是一处**待协调者替换的 integration hook**，不是已确定的生产迁移。正式合并应把片段与 P2a DDL 放在统一版本迁移的同一事务，并移除分支 hook。不能直接把带无版本账户表的实验数据库冒充正式升级产物。
- 共用文件的接线包括 `lib.rs`、`store.rs`、`store/networks.rs`、既有管理 API 与 Store 事务创建点、`main.rs` 的真实 peer `ConnectInfo`、protocol module 注册和 SDK public exports。它们随独立 integration commit 交付，协调者与 P2a / underlay / latency API 合并后重新生成；未修改 generator 模板、Web 或 Agent 数据面。
- P2a 冻结 `56da5ad` 的 schema **v10 保留给托管 transport**，账户片段不争用该版本，正式后续版本由协调者统一。在 `.worktree/account-p2a-integration` / `integration/account-p2a` 独立副本叠加账户提交，保留 P2a `transport-material.key`、初次 enrollment 的 listener/material 同事务安装及 `18c4313` secret Debug adapter。新 managed listener / managed enrollment 管理入口须接同一 `Read/Manage` 根事务；node-only material / capability 接口继续只接受节点身份。此副本是组合验证候选，不覆盖 main 或作者活跃 worktree。

## 合并前 review 修复

- **AR1**：`account_transaction`、`authorize_user_device` 与既有 network wrapper 均先取得根 `BEGIN IMMEDIATE` 写锁，再生成新的绝对过期时钟、从该事务认证 principal 并复核 session/credential version/当前角色。等待期间过期的 session 不会签发邀请或设备 code；KDF 仍使用锁外两阶段工作。修复覆盖共用账户 wrapper 的所有调用，不局限复现端点。
- **AR2**：移除/退出与这批撤销节点的 `node_tags` 清理处于同根事务，仅清理当前网络、该用户已撤销 membership 所有的已撤销节点；不删除 tag catalog，不清别的活动设备。真实 Owner claim 仍无损保留 tags。重复 remove 需当前角色具备历史撤销角色的管理上限（最严格角色优先）；重复 leave 需本人有效 session 和该网既有撤销 membership。两者只修复旧孤儿、不恢复身份，无变更的重试不推进 revision；仍有活动 assignment 或 Access 引用的 tag 不可删。
- **SG1**：`account_schema::validate(&Connection)` 是协调者可在迁移事务中调用的完整已安装 gate。它从权威账户/ownership SQL及真实 v9父合同构建已知 schema manifest，比较七张表、命名索引、全部账户/ownership guards 的定义（保留引用/字符串内容，只规范化外部空白），并比较解析后的列/PK、复合 FK、索引列及 partial/unique 标志，执行 FK check，以及每个活动或撤销 node 恰有一条同网 ownership 的覆盖检查。不存在“七个表名就完备”的捷径。
- `account_schema::install(&Transaction)` 安装无版本片段后立即调用上述 validate，失败整个调用方事务 rollback。当前 branch startup hook 同一 IMMEDIATE 事务中：若任何账户相关对象已经存在，必须完整 validate；全部不存在才 install。未知/partial/弱化 guard/缺 ownership 数据明确拒绝，不通过 `IF NOT EXISTS` 补壳。**正式 user_version 与删除/替换 branch startup hook 仍由协调者统一**，不抢 P2a v10，不宣称生产迁移已部署。
- 本轮只收尾上述 review 修复与验收；Web SSO、CLI request/poll 不在本轮新增范围。

## 正式生成接口与前端 handoff

所有接口前缀 `/api/v1`，返回 `Cache-Control: no-store`。人类 API 不使用 Controller cookie：Web 服务端调用生成 SDK，私有保存 session，浏览器安全 cookie / CSRF / Origin 校验由 Web 主线接线。Controller 不接受 cookie 登录，不把 admin.token 返回给 App / CLI。不要把 login 返回的原 session 写入普通日志、localStorage 或 App 回调。

| 操作 ID（SDK 同名导出） | HTTP 路径 | 主体 / 结果 |
| --- | --- | --- |
| `loginAccount` | POST `/auth/login` | 邮箱密码；durable commit 后单次返回 `mws_` session 与 Unix 秒绝对 expiry |
| `getAccountSession` / `logoutAccount` | GET `/auth/session` / POST `/auth/logout` | 本人有效 session；资料与 live memberships / 撤销当前 session |
| `listNetworks` | GET `/networks` | 用户只见活动 membership 的网络，bootstrap 保留全局列表 |
| `claimFirstOwner` | POST `/networks/{network_id}/owner-claim` | bootstrap；`kind=new_account` + 真实 email/password，或 `kind=existing_account` + 本人有效 `session_token`，两组互斥 |
| `createAccountInvitation` / `listAccountInvitations` | POST / GET `/networks/{network_id}/invitations` | Owner / Admin role ceiling / bootstrap；secret 仅创建且 commit 后返回 |
| `getAccountInvitation` / `revokeAccountInvitation` | GET `/networks/{network_id}/invitations/{invitation_id}` / POST `/networks/{network_id}/invitations/revoke` | 仅公有 ID + 当前主体授权；读取 metadata 不返回 secret/hash |
| `registerInvitedAccount` / `acceptAccountInvitation` | POST `/auth/invitations/register` / `/auth/invitations/accept` | 新账户 KDF + 单次邀请原子保存；已有账户本人 session 接受，不覆盖密码 |
| `listAccountMembers` / `saveAccountMember` | GET `/networks/{network_id}/members` / POST `/networks/{network_id}/members/save` | read / role ceiling；`role` 必填 `owner/admin/member/remove`，移除不是缺省字段 |
| `leaveAccountNetwork` | POST `/networks/{network_id}/members/leave` | 本人退出；最后 Owner 保护，所属设备撤回同根事务 |
| `revokeAccountSessions` | POST `/auth/sessions/revoke` | 本人或实例 bootstrap 撤销指定用户全部 session |
| `changeAccountPassword` | POST `/me/password` | 本人有效 session + 原密码证明；version CAS，撤销全部旧 session/reset |
| `createAccountPasswordReset` / `listAccountPasswordResets` | POST / GET `/auth/password-resets` | 仅实例 bootstrap；手动转交链接，无发邮件 |
| `getAccountPasswordReset` / `revokeAccountPasswordReset` | GET `/auth/password-resets/{reset_id}` / POST `/auth/password-resets/revoke` | 仅 bootstrap，公有 ID 管理 |
| `redeemAccountPasswordReset` | POST `/auth/password-resets/redeem` | 独立 `mwr_` secret，KDF 前读取 intent，消费/version/password/session 同事务 |
| `listNodeOwnership` / `claimNodeOwnership` | GET `/networks/{network_id}/node-ownership` / POST `/networks/{network_id}/node-ownership/claim` | read / 真实 Owner；只认领本网活动 membership 与未认领、未撤销的旧节点 |
| `authorizeDevice` | POST `/networks/{network_id}/device-authorizations` | `userSession`，Member 可批准本人设备，bootstrap/node/enrollment 不可产生 user-owned 接入 |
| `exchangeDevice` | POST `/devices/exchange` | 原 code/PKCE；同事务 revalidate + 原 node 创建/地址/ownership/code消费/revision |

OpenAPI 三类人类认证明确区分：`userSession` 只接受 `mws_`；`adminBearer` 只用于 bootstrap；`managementBearer` 支持用户 session 或原 bootstrap 兼容入口。节点和 enrollment 仍是 `nodeBearer` / `enrollmentBearer`，均不能取得人类权限。网络 Owner 无法通过任何网络角色反向成为实例管理员。

现有管理 API 全部通过 `network_operation(token, network_id, Read/Manage, action)`：在实际根 `BEGIN IMMEDIATE` 内验证当前 session / credential version / role，再执行资源操作。原业务事务在根事务内成为 savepoint，根失败整体 rollback；没有可变 selected-network 或仅 handler 的权限检查。网络清单按当前 membership 过滤。`Member` 可 read、预览与本人设备接入，不可写配置。

账户 ingress 采用固定一分钟窗口的有界预算：同账户最多 10 次、同真实 peer IP 最多 30 次、全实例最多 120 次；key 仅 SHA-256 摘要，最多 4096 keys。不相信 `X-Forwarded-For`；未注入真实 peer 的测试 / embedding 共享保守 unknown-source 预算。reset 在读取 intent 后另按实际 user 预算，入口与用户预算都先于 KDF；reset 的双重 reservation 会更早耗尽 source/global 预算。KDF 固定至多 2 个并发，取消 HTTP 不释放仍执行的 blocking permit；没有 Store 锁跨 KDF await。

membership 移除 / 退出在同根事务撤销其原稳定 membership 所属节点，清除这批节点 runtime reports 与 tag assignments，并推进受影响网络 revision 和 publication 校验；任何一步失败撤回 membership、邀请失效、节点与关联写入。重加入生成新 membership ID，不复活旧设备或旧批准。首次兑换检查 session/expiry/credential version/membership；精确已消费重试不依赖浏览器 session，却仍检查原节点未撤销及原 membership 活动、ownership 相同。owner / binding 存储禁止更换或删除，node ID/credential/IP/keys/material 不重写；claim 保留 tags，撤回按上述清理语义处理。

## 尚待交付

- Web 本地登录 / Cookie / CSRF / 目标网络选择与上述生成接口接线由 Opus 主线负责；此分支没有改 `apps/web`，不宣称浏览器 SSO UI 完成。
- CLI request/approve/poll/exchange、code 临时加密重交付与本地 journal 尚未实现，沿用本文末尾 polling handoff；不交付半接 CLI endpoint。
- 生产 schema 版本 / P2a 合并与迁移部署由协调者决定；Controller 撤回和 revision 已接线，真实 NE / Agent runtime 终止与跨 transport 数据面联测仍受既有主线状态约束。

下文保留账户领域原切片的审计背景及 CLI 设计；其中“待接线 / 下一切片”描述是历史 handoff，已集成项目以上述正式接口为准。

## 本轮验证（账户独立分支）

- `CARGO_TARGET_DIR=target cargo test --workspace --all-features --locked -- --test-threads=2`：**477 passed / 0 failed / 7 ignored**（含 1 doctest；既有显式外部代理 / Docker / 性能专项 ignored 未执行）。Controller **67 单元 + 29 HTTP**，其中账户领域25项；另有6项既有 generator 测试。Apple core真实Controller接入测试已改为浏览器fixture真实账户 / session，确认membership ownership后验证原exchange/heartbeat/撤销。
- `cargo clippy -p meshway-controller -p meshway-apple-core --all-targets --all-features -- -D warnings` 通过；`cargo fmt --all -- --check`、生成drift、Web/SDK/e2e TypeScript及4个 `experiments/*/tsconfig.json` 检查通过。SDK **14** tests、Web **145** tests通过。
- 新取消测试确定性占用 blocking worker，取消请求后预算保持占用，直至真实KDF任务完成。新HTTP回归先复现login按email、reset按user ID分拆预算可绕过同账户上限（200而非429），修正后以归一化邮箱共享预算，拒绝时不消费reset、不修改密码/session。
- API验收包括所有既有网络 read / mutation 的 Member与跨网隔离、公有邀请ID管理及跨重启永久失效、最后Owner、已有账户接受邀请、全局恢复权限、本人密码/会话撤销、App首次兑换的session失效、移除后重加入、节点精确retry与撤回、成员退出、ownership存储故障原子rollback，以及实际v9每个旧表/列的SQL值对照与DDL失败rollback。托管material的组合故障和撤回投影在独立P2a副本验证。
- 仅独立tempfile / loopback fixture；未使用真实控制面数据、真实管理员token或live端口，无Docker / 宿主路由 / DNS / 防火墙操作。组合P2a验证单独记录在integration候选，不把其待审查状态或生产schema部署记为已完成。

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
