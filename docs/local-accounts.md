# 本地账户、网络权限与设备归属

**账户、Controller HTTP、事务内网络 RBAC 与 App PKCE ownership 已实现。** 既有 Web 的邮箱登录、成员/邀请/角色/设备归属、我的账户、初始化/恢复已接通；CLI 链接登录、多接入仍待交付。Apple 新增开发暂停，隧道未实现。

账户在 controller 全局唯一，membership 按网络隔离；同用户可属多网。Owner/Admin/Member 是网络角色，Owner 不等于实例管理员。权威 DTO 在 `crates/protocol/src/accounts.rs`，路由/OpenAPI/SDK 走[统一生成链](/protocol)。

## 账户与权限

- 本地邮箱+密码，仅邀请，无开放 signup，不发送邮件；邀请/reset 链接手动转交。已有账户须用本人 session 接受邀请，不能用链接覆盖其全局密码。
- UserId、MembershipId、InvitationId、PasswordResetId 是独立 opaque 资源 ID，不是 bearer。邮箱 trim/lowercase，不做 `+`/`.` 提供商合并；邀请绑定精确邮箱、网络、角色、签发者与期限。
- Owner 管理本网角色；Admin 仅管理配置与 Member，不能授予/管理 Owner/Admin；Member 只读/预览并批准本人设备。最后 Owner 不能移除/降级，所有网络角色都不授予跨网权限。
- `mws_` session、`mwi_` invitation、`mwr_` reset 分表/前缀，只存随机原值的摘要。session 绝对过期，无滑动续期。密码变更/reset 推进 credential version 并撤销全部旧 session/reset；不换节点身份。
- 退出/移除 membership 与其设备撤销、tag assignment 清理、runtime 撤回和 publication revision 在同根事务。重加入创建新 membership，不复活旧邀请、设备批准或节点；claim 旧节点则保留身份、公钥、地址、tags 与材料。
- 旧未归属节点须由真实本网 Owner 显式认领，不自动归给首 Owner，不把 tag 转成账户身份。尚未绑定账户 membership 的 null 不统称为“历史节点”。

每个管理操作在取得根 `BEGIN IMMEDIATE` 后用新的时间校验 session/credential version/当前权限，再执行保存；等待写锁期间过期也必须拒绝。角色降级/移除持久废止签发者失去权限的未消费邀请，恢复角色不复活链接。

## 接口入口

以下仅为操作索引，精确路径、字段和错误由当前 OpenAPI 生成：

| 功能 | 生成操作 |
| --- | --- |
| 登录/当前会话/退出 | `loginAccount`、`getAccountSession`、`logoutAccount` |
| 首 Owner 初始化 | `claimFirstOwner`，实例 bootstrap+真实账户证明 |
| 邀请 | `createAccountInvitation`、`listAccountInvitations`、`getAccountInvitation`、`revokeAccountInvitation`、`registerInvitedAccount`、`acceptAccountInvitation` |
| 成员/退出 | `listAccountMembers`、`saveAccountMember`、`leaveAccountNetwork` |
| 密码/全部会话撤销 | `changeAccountPassword`、`revokeAccountSessions` |
| 实例密码恢复 | `createAccountPasswordReset`、`listAccountPasswordResets`、`getAccountPasswordReset`、`revokeAccountPasswordReset`、`redeemAccountPasswordReset` |
| 设备归属 | `listNodeOwnership`、`claimNodeOwnership` |
| 浏览器接入 | `authorizeDevice`、`exchangeDevice` |

Controller 只读 Authorization，不接受 Cookie 登录；旧 Next 服务端私有持有用户会话，新 pure SPA 的 `mws_`/LocalStorage/标签页内存与同源部署合同见[SPA 消费](/spa-api-audit)。节点/enrollment 凭据不授予人类权限；node-only 材料接口不接受 Owner、用户 session 或 bootstrap。

## 密码与竞态

Argon2id v19：`m=65536 KiB,t=3,p=1`，独立128-bit随机 salt、256-bit output，以有界参数 PHC 保存；密码12–1024 UTF-8 bytes。密码、PHC、token、私有认证 intent 不进入普通日志/响应。不存在账户也执行 dummy verify，外部登录错误统一 Unauthorized。

有界 KDF blocking worker（正常至多2并发）在 Store 锁外工作，permit 随实际任务持有，取消 HTTP 不提前释放。入口还有来源/账户/实例预算，真实 peer 不无条件相信 X-Forwarded-For。

1. 读取私有 credential/邀请/reset intent，释放 Store 锁。
2. 执行有界 hash/verify，不持 SQLite mutex 跨 await。
3. 重新取得 IMMEDIATE 事务、新时间与当前版本/状态，复核 session、邀请/reset、membership，再原子提交。
4. commit 成功才返回一次性秘密；commit 失败不能虚报有效结果。

## 浏览器 SSO 与节点接入

Web/App/未来CLI 共用 controller 用户体系；设备不收集密码或浏览器 session，只拿绑定本次接入的一次性结果。App 系统浏览器登录/选网/批准后 PKCE 兑换；code 消费、node/IP/ownership/revision 同事务。

首次兑换复核原 session/version/membership；已消费精确恢复不重新注册，却仍要求原节点未撤销、原 membership 活动且 ownership 不变。撤销和移除后重加入不能恢复旧批准。[接入合同](/device-login)保留原请求/回调兼容。

CLI 链接登录将复用授权服务，终端只打印链接并领取一次性结果；request/poll、响应丢失重交付与本地 journal 尚未实现，不将设计草案当作可调用 endpoint。OIDC 后续作为认证来源，不是内部统一浏览器登录的前提。

## 存储与迁移

正式启动迁移在单库事务中安装账户/ownership与邻接历史，再验证结构/FK并提交版本；随后 Mesh v3 迁移添加其源/执行状态，不修改旧身份/配置或自动激活。版本和 DDL 是兼容边界，业务方法使用显式 `NetworkId`。

schema gate 比较真实 columns/PK/FK/index/partial/unique、完整owned guards及每node同网ownership，按 `(type,name)` 区分同名 trigger/index；缺失、partial、未知或弱化结构拒绝，不用 `IF NOT EXISTS` 补壳。保留旧 node ID/凭据摘要/公钥/地址/JSONC/revision/材料密文，不创建假账户。

备份须同时保存 SQLite、`admin.token`、已有 `transport-material.key` 与必要私有状态；不能丢材料密钥后生成替代。网络 Owner 不获他人全局密码恢复权。当前没有逐会话清单/完整全局无membership账户枚举，UI仅显示真实已有操作。
