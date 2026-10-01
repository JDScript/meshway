# App 节点 OAuth 登录（提案）

::: warning 提案 · 未确认 · 未实现
2026-10-01。用户仅确认 **App 使用 OAuth** 与 **统一节点模型**；下述 IdP、用户归属、审批、到期、登出及交付切片均为建议，须逐项确认。本文不定义已发布 API，也不表示 Apple App 已登录、注册或建立隧道。
:::

## 建议摘要与已有边界

建议采用 **外部 OIDC IdP + Meshway 登录校验 / 注册绑定**，不自建账号密码及通用 OAuth 授权服务器。Apple 原生 App 使用 authorization code + PKCE，通过 `ASWebAuthenticationSession` 登录；人类身份只授权一次节点接入，长期运行仍使用现有节点凭据和 X25519 身份。

- 无头服务器保留管理员创建的一次性 enrollment token；OAuth 与 token 两条路径最终创建**同一种节点**，共用配置、能力、Access、Routing 与撤销机制。
- 不新增 App / server 节点类别、默认中继资格或独立 relay-role 开关。owner 与接入来源是归属 / 审计信息，不决定节点路径职责。
- 当前只有管理员 bearer、Web 8 小时 HttpOnly 管理会话、注册令牌、节点 bearer；没有 users、teams、OIDC 或 App 登录服务。详见[注册基线](/enrollment)、[架构](/architecture)与[产品对齐](/product-alignment)。
- 注册事务绑定一次性令牌与节点凭据；已消费的同身份重试可恢复，撤销不可恢复。当前 X25519 公钥通过独立的 `POST /api/v1/agent/connection-identity` 不可替换地绑定，并非注册事务的一部分。[M1][M2]
- 登录 / 注册 / 心跳 / 配置 ACK 分别是不同证据，均不能单独证明 TUN 或隧道已生效；现有[里程碑](/milestones)仍按真实数据面验收推进。

## 上游研究：可借鉴与不能等同的语义

研究使用 `references/README.md` 固定版本：Tailscale `ab2380a6312858ccf95451787f33acc9c9fca066`，Headscale `eeaac680bef26585c8cc1569f949e26d0d6aba80`。Headscale 是独立控制服务器，不能据此推断 Tailscale 官方服务端实现。

| 主题 | 固定源码 / 文档结论 | 对 Meshway 的启示 |
| --- | --- | --- |
| 交互登录 URL | Tailscale `RegisterResponse.AuthURL` 表示授权待完成；客户端以 `Followup` / `WaitLoginURL` 等待浏览器操作。Headscale 缓存 machine key、node key 与注册信息，再生成 `/register/{auth_id}`。[T1][T2][H1] | 借鉴短期尝试与原身份绑定。此 URL + 长轮询**不是 RFC 8628 device grant**，不能只改字段名宣称兼容。 |
| Headscale OIDC | 外部 issuer discovery、client ID / secret、服务端 callback；state、nonce 与可选 PKCE；交换 code 后验证 ID token，再按 allowed users / domains / groups 过滤并更新 user。注册另有显式确认页。[H2][H3] | 人类认证、接入资格与节点绑定分层；不以邮箱作为稳定身份，不从 IdP group 自动授予管理或 tag 权限。 |
| machine key / node key | Tailscale machine key 用于协调服务器通信，node key 用于 WireGuard / DERP；Headscale 分别保存，两者不匹配时拒绝对既有节点操作。[T3][H4] | Meshway bearer 是控制面凭据，X25519 是邻接 / E2E 公钥绑定；功能边界可类比，协议与轮换规则不等价。OAuth token 不替换它们。 |
| 到期与重认证 | Tailscale 协议有 `KeyExpiry` / `NodeKeyExpired`，客户端到期或 interactive login 会生成新 node key。Headscale 可设 node expiry，`use_expiry_from_token` 实际读取 **ID token** 到期值。[T1][T2][H5] | 不把短期 OAuth token 到期直接当节点撤销，也不照搬自动换 key：Meshway 当前公钥不可替换，轮换须另立契约。 |
| user-owned / tagged | Headscale 用 `UserID` 表示 user-owned；有 tags 时 tags 定义身份，`IsUserOwned = !IsTagged`。其注册文档分别描述用户与 tagged-devices。[H4][H6] | Meshway 保持统一节点；管理员 tag 仍是属性，可以和 owner 共存，不引入用户身份被 tag 替换的模型。 |
| pre-auth key | Headscale 默认一次性、1 小时；可选 reusable / ephemeral。校验依次拒绝 revoked、expired，再处理 reusable / used；ephemeral 节点有不活跃回收，登出可删除。[H6][H7][H8] | 注册 key 到期 / 撤销与已经注册节点的生命周期不同。首片保留 Meshway 一次性 token；reusable 与 ephemeral 都是另行产品决策。 |
| 节点批准 / 授权 | Tailscale wire 分开 `AuthURL`、`MachineAuthorized`、key expired。Headscale 支持 CLI / API 批准注册或委托 OIDC，但源码明确**未实现独立 machine authorization**，通常返回 true。[T1][H6][H8] | 人已认证、节点获准接入、Access 允许业务、运行已连通必须分开。不能把 Headscale 注册批准描述为同款独立设备审批层。 |
| 登出 / 撤销 | Tailscale 登出提交过去的 key expiry 并清空客户端 persist；Headscale 普通节点设置 expiry，ephemeral 删除，当前 tagged logout 不设置 expiry。pre-auth key revoke 是另一对象。[T2][H7][H8] | 不复制上游 tagged 例外。Meshway 暂停、重认证、永久 revoke 与清除本地凭据需明确分开；IdP 登出不自动撤销节点。 |

## 谁提供 OAuth / OIDC

OAuth 提供授权框架，**识别人类采用 OIDC**；仅有 OAuth access token 不能被当成通用“已登录用户”证明。

| 选项 | 建议与代价 |
| --- | --- |
| 外部 IdP 是 OP / authorization server，Meshway 是 RP | **推荐**。复用现成的登录、MFA、账号恢复和 issuer / JWKS；可选自托管 Keycloak、Authentik 等或托管 OIDC 服务，具体供应商未定。控制面仍自行决定谁能接入以及绑定哪个节点。 |
| controller 自己充当完整 authorization server | 暂不推荐。需自行承担账号、认证、MFA、授权码 / token、client registry、恢复与注销等生命周期；若只代理外部 IdP，仍要维护第二层 OAuth 协议。可在独立产品需求成立后考虑。 |

### 推荐原生流程：外部 IdP public client

App 与 controller 构成同一 Meshway RP 的原生 / 服务端部分：**App 是外部 IdP 的 public OAuth client**，controller 校验该登录事务的 OIDC 断言并执行接入授权。这里 controller 不提供 OAuth token endpoint；节点注册兑换不是新的通用 OAuth grant。

1. 用户选定可信 controller。App 先在 Keychain 保存随机节点凭据与 X25519 私钥，建立持久的待注册状态；恢复时复用这份身份，不能随失败重新生成。
2. 向 controller 发起短期 login attempt：绑定 network、节点凭据摘要、公钥、原注册元数据及客户端恢复证明。服务端返回随机 attempt ID、OIDC nonce、到期时间和管理员配置的 issuer / native client ID / redirect allowlist；不接受调用者任选 issuer。
3. App 生成随机 `state` 和 PKCE verifier，以 S256 challenge、服务端 nonce、`openid` 最小 scope 在 `ASWebAuthenticationSession` 打开外部 IdP。**不嵌入 WKWebView 登录，不使用 implicit / password grant，不携带 client secret**。[R1]
4. IdP 将短期 authorization code 返回已登记的 App callback。App 校验 state、issuer / redirect 与本次尝试一致，再用 verifier 向 IdP token endpoint 兑换；优先经域名证明的 HTTPS callback，或严格登记的反向域名 scheme，PKCE 均必需。首次接入明确展示 controller / network、登录账号及本机名称 / 公钥指纹，用户确认后才提交，避免浏览器 SSO 自动完成陌生注册。
5. App 仅将短期 ID token 作为该 attempt 的**一次性登录断言**提交给 controller，同时证明原节点凭据的持有。controller 校验可信 JWKS / 算法、`iss`、登记的 native client `aud`（多 audience 时含 `azp`）、`exp` / `iat`、服务端 nonce 及接入资格；重认证可要求 `auth_time` / `max_age`。
6. controller 将通过验证的 principal 绑定为一次性 login grant；消费 grant 与创建节点 / owner / 凭据摘要 / 公钥关联在同一事务完成。App 获得普通节点记录，随后使用生成接口心跳与拉取配置；OAuth token 不用于日常节点 API。

ID token 的 audience 是登记的 Meshway 原生 RP client，**不是 controller 管理 API 的 access-token audience**；服务端只在 nonce 绑定的登录入口接受它，不把任意 App token 当 bearer。每个部署需在 IdP 登记兼容的原生 public client 与准确 callback，并由 controller 固定允许的 client ID；共享客户端注册、跨 controller audience 与部署域名尚需选定方案。所有跨网络登录 / 兑换 / 控制请求使用 HTTPS；token / verifier 不进入日志、回调 URL 或 Web 页面载荷。

首版不请求 `offline_access`、不持久化 OAuth refresh / access / ID token。长期仅保存既有节点身份材料（节点凭据与必要的 X25519 私钥）；管理员 bearer、IdP client secret、密码与管理 cookie 均不得进入 App。Keychain 建议 `ThisDeviceOnly`，按 App / NetworkExtension 所需的最小 access group 共享；私钥不得进入普通日志、URL 或 Web 客户端。

**兼容备选**：IdP 不支持原生 public client 时，可采用 Headscale 式 controller confidential RP 浏览器回调，secret 仅留服务端；App 接收独立、短期、一次性且绑定 App challenge 的注册兑换码。须分别定义上游 OIDC code + PKCE 与下游 Meshway 兑换，不能将后者冒充 RFC 8252 OAuth token flow，亦不能静默降级。

### tvOS / 无浏览器 CLI 的备选

仅在确有需求、外部 IdP 支持且能完成 Meshway attempt 绑定时使用 **RFC 8628 device authorization grant**：IdP 发 `device_code`、`user_code`、verification URI、expiry / interval；另一设备确认，原设备按 `authorization_pending` / `slow_down` 轮询，拒绝 / 到期停止。[R2]

`device_code` 不展示或写入日志；确认页面核对设备与 user code。RFC 8628 本身不保证 OIDC nonce 绑定：后续契约须确认 IdP 的 OIDC 支持及一次性登录结果与原节点身份的服务端绑定办法，不支持时不开放。此功能不替代 iOS / macOS code + PKCE，也不替代无头服务器现有 enrollment token；tvOS 不属于本次已确认平台范围。

## 身份绑定与权限不变量

- **认证人，授权节点**：principal 唯一键为 `(issuer, subject)`；email / name 只作展示或经验证的接入过滤，不按相同邮箱合并账号。登录不能改 owner、抢占既有 node ID，或接管其他人的节点。
- **单次提交**：验证 OIDC 在事务外完成；事务内再次检查 grant 期限 / 消费状态、principal 禁用、审批、network、身份唯一性与 revoke，消费 grant 并建立完整节点绑定。浏览器访问或 GET 回调本身不直接创建节点。
- **同身份恢复**：grant 的消费 tombstone 记录 node ID 与完整 binding digest（network、owner、节点凭据、公钥、原元数据）；成功响应丢失后，凭原恢复证明与相同绑定返回同一仍有效节点。不得为了重试更换 principal、凭据、公钥或元数据；不同绑定冲突，revoked 一律拒绝。首次消费期限与已提交恢复分开，保留必要 tombstone，不能因 attempt TTL 清理而失去去重。
- **持有与公钥**：节点凭据证明控制面身份；公钥关联不自动证明私钥持有，真实连接仍需现有认证握手。OAuth 路径可在新注册事务中初次绑定公钥，但既有 token 路径和不可替换 / 全局唯一语义须保留，不能把本提案写成现有行为。
- **权限隔离**：人类登录默认不获得任何 admin 能力，不可创建 enrollment token、分配 tag、改 Access / Routing / IdP、读取全局管理清单。App 首片只注册、查看自身接入结果和以节点凭据执行现有 node-scoped 操作；未来 owner 管理 API 需独立授权契约。
- **接入不等于 Access**：owner 不附赠互访许可。沿用当前 network 的 `*`、管理员 tag、node selectors；初始空策略 reject。显式已有 `*` allow 规则可能覆盖获准的新节点，应在接入确认中真实呈现，不能额外偷偷添加 OAuth allow。
- **职责一致**：OAuth 来源或 owner 不改变节点能力、连接方向、必经路径与 E2E 要求；公钥、连接、业务授权来自同 revision 快照，撤回 / lease 独立终止旧代际。

## 最小数据模型与生命周期（建议）

| 数据 | 首片建议 | 边界 |
| --- | --- | --- |
| principal | 内部 ID、issuer、subject 唯一组合、可选展示名 / 已验证邮箱、disabled 状态 | 不新增 teams / groups / 多租户，也不自动产生管理员角色；删除宜保留审计关联。 |
| node owner | nullable `owner_principal_id` | OAuth 节点有 owner；旧节点 / token 节点可为 null。null 不代表节点类别或特殊权限，tag 不清除 owner。 |
| login attempt / grant | ID、nonce、恢复证明摘要、期限、network、binding digest、principal、消费 node ID 与终态 | 短期原始 token 不保存；消费记录与节点撤销 tombstone 保留。可共用一张状态表，不必制造两套身份。 |
| 接入批准 / 重认证状态 | 仅在用户选定审批 / 到期方案后增加 typed 状态与时间 | pending / approved / expired 与不可逆 revoked 分开；用户状态 / 期限变化须撤回配置，不靠 UI 标记。 |

是否需要 `user:` Access selector 留作开放问题：首片 owner 仅归属；以后若需要“某人的所有节点”，须独立定义 selector、network scope、归属转移及快照编译，并同步 Rust schema / 编辑器 / 生成产物。不能用 hidden tag 或邮箱字符串绕开现有 selector 契约。

推荐将 OAuth token 到期与 **Meshway 接入认证有效期** 分开；后者可建议 30 天、允许同 owner + 原节点凭据重认证，保留 node ID / 地址 / 公钥。到期撤回业务运行，续期只允许 expired / logged-out 节点，永远不能清除 revoked。expired / logged-out 的原凭据仅可用于受限恢复 / 重认证，不能靠心跳、旧 grant 重放或拉取配置恢复授权；已消费 grant 的幂等读取不延长接入期限。30 天与是否实施到期均未确认；不据此改变无头服务器 token 路径的生命周期。

推荐区分以下用户操作，具体命名待确认：

- **断开连接**：停止本地运行，保留身份；Mesh 网段仍 fail-closed，不等于登出。
- **退出 Meshway 登录**：终止本节点接入认证、停隧道并撤回配置，保留 Keychain 身份以供同 owner 重认证；不代表全局 IdP / 浏览器 SSO 已注销。离线时先停止本地，明确服务端尚未确认，保留持久待同步退出操作。
- **移除此节点 / 永久撤销**：服务端保留 revoked tombstone，终止旧代际；确认后清理本地身份。再次接入需用户显式建立新身份和新 grant，旧凭据 / 公钥 / grant 不可复活或迁移到新节点。
- **IdP 账号禁用 / 注销**：不天然等于 Meshway 节点撤销。推荐 controller 禁用 principal 时立即终止其接入授权；IdP 禁用的即时同步需 webhook / back-channel 等另片，没有集成时明确只能在下次认证或 Meshway 期限到期发现。

## Web 控制台选项

以下均为可选后续 UI，管理 mutation 继续逐次在服务端检查管理员身份并使用生成 SDK / 运行时 schema：

- **IdP 配置**：推荐首片由部署侧配置 issuer、native client ID、callbacks 与接入 allowlist；Web 只展示已配置状态 / 真实诊断。备选 Web 编辑（CAS、审计；若采用 confidential RP，secret 写入后不可回读）。
- **归属与状态**：既有节点详情增加 owner、接入来源、批准 / 重认证状态及期限；同一节点清单呈现，不另建“设备”类别页面。不展示 token 或公钥关联以外的秘密材料。
- **待批准列表**：仅当确认手动批准时，在节点管理中聚合申请；显示可信 principal、目标 network、设备声明与公钥指纹、期限，批准 / 拒绝不可改变原绑定。自动批准时不制造虚假 pending。
- **人员 / 权限**：如需禁用 principal，提供最小人员清单；owner 自助管理、用户 Web 登录及 OIDC 管理员登录均需后续独立产品确认。

## 增量切片与验收（均待批准开工）

每片先定义 Rust 契约、真实路由和权限，再生成 OpenAPI / Rust / TS 客户端及运行时 schema，通过 drift check 后接入消费者；本文件中的概念名不是冻结 endpoint / DTO。

| 切片 | 交付 | 验收条件 |
| --- | --- | --- |
| D0 · 决策与契约 | 确认下列问题；typed attempt / principal / grant 终态、错误、迁移及权限矩阵 | 升级保留旧 token、凭据、公钥、地址、revoked；匿名 / user / node 不能调用 admin；生成漂移检查通过。 |
| D1 · Controller 真登录 / 事务注册 | 对接独立测试 OIDC IdP；nonce 绑定断言、资格校验、统一节点创建及同身份恢复 | 拒绝错误 issuer / aud / azp / nonce、过期 / 篡改 token；并发消费只建一个节点；断网丢响应 / 重启可恢复；换身份、公钥或 owner 失败；撤销与消费竞态不能复活。原 enrollment 行为回归通过。 |
| D2 · Apple 原生接入 | ASWebAuthenticationSession、code + S256 PKCE、Keychain、真实状态 / 错误及生成接口桥接 | 用真实测试 IdP 完成浏览器往返；取消、错误 callback / state、兑换超时、进程中断均可解释且不换身份；无长期 OAuth token / secret 泄漏；显示“注册完成”而非“隧道已连接”。先模拟器 / macOS，真机 NE 验收另待会员恢复。 |
| D3 · 生命周期与所选管理 UI | 实现已选批准 / 到期 / 同 owner 重认证 / 退出 / revoke；Web owner 和实际审批状态 | pending / expired / logged-out 无业务授权；到期、principal 禁用、revoke、lease 撤回能终止既有代际；重认证保留身份但不能复活 revoked；离线退出真实显示待确认；匿名重放管理 mutation 被拒绝。 |
| D4 · 可选扩展 | 如另行确认，再做 RFC 8628、owner 自助、`user:` selector 或 key rotation | device flow 的轮询 / 超时 / 拒绝及原身份绑定实测；selector / rotation 各有独立契约与撤回验收，不能由已登录或配置 ACK 推断完成。 |

## 待用户确认的问题

1. **谁提供人类登录？** 推荐默认：单一外部 OIDC IdP，Meshway 作为 RP，不自建授权服务器；备选：controller 自建 / 代理 OAuth AS，或后续多 IdP。
2. **Apple 如何对接 IdP？** 推荐默认：每个部署登记原生 public client，ASWebAuthenticationSession + code / S256 PKCE，controller 验证 nonce 绑定的登录断言；备选：controller confidential RP 浏览器回调 + 独立绑定兑换码。tvOS / CLI device grant 后置。
3. **谁能接入，是否审批？** 推荐默认：管理员显式配置 principal 接入 allowlist，名单内自动批准，未配置时拒绝 OAuth 接入；备选：可信域 / group 过滤，或每节点人工批准。审批不替代 Access。
4. **用户归属需要什么权限？** 推荐默认：最小 principal + nullable node owner，登录无 admin 权限，tag 仍由管理员分配，首片只操作自身节点接入；备选：owner 自助管理 / 用户 Web，或单独设计 OIDC 管理员角色。
5. **何时到期和怎样退出？** 推荐默认：OAuth 接入认证有效期 30 天，与 IdP token 到期分开；退出终止接入并保留身份供同 owner 重认证，永久移除走不可逆 revoke；备选：无接入到期、另定期限，或退出即永久 revoke。无头 token 节点生命周期另行保持。
6. **是否需要按用户授权业务？** 推荐默认：owner 仅归属，Access 保留 `*` / tag / node，暂不增加 `user:`；备选：后续独立设计 network-scoped user / group selectors，明确归属变化与撤回。
7. **首片 Web 管到哪一步？** 推荐默认：部署侧 IdP 配置，Web 展示真实状态 / owner 并管理 principal 禁用；备选：Web 编辑 IdP、人工审批列表和 owner 自助入口，分别按已选产品需求交付。

## 证据索引

以下 `file:line` 均在上述固定 checkout 实际核对；链接指向固定提交。规范用章节号，Meshway 行号对应本提案基于的主线快照。

- **M1** `apps/controller/src/store.rs:187–245`：令牌消费、binding digest、恢复与撤销检查；`141–151` 管理员凭据校验。`crates/protocol/src/lib.rs:110–130` 为注册请求 / 校验入口。
- **M2** `apps/controller/src/connections_api.rs:28–49`；`apps/controller/src/store/connections.rs:224–268`：节点凭据鉴权、同事务绑定公钥、不同绑定 / revoked tombstone 冲突；`crates/protocol/src/connections.rs:40–47` 为契约入口。
- **T1** [`tailcfg/tailcfg.go:1328–1377`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/tailcfg/tailcfg.go#L1328-L1377)：register、Followup、到期、ephemeral、授权状态；同文件 `389` 为 KeyExpiry、`455` 为 MachineAuthorized。
- **T2** [`control/controlclient/direct.go:505–565`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/control/controlclient/direct.go#L505-L565)、[`646–719`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/control/controlclient/direct.go#L646-L719)：logout 清 persist、等待 URL、到期 / interactive regen。
- **T3** [`types/key/machine.go:34–36`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/types/key/machine.go#L34-L36)、[`types/key/node.go:47–49`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/types/key/node.go#L47-L49)：两类密钥用途。
- **H1** [`hscontrol/auth.go:302–349`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/auth.go#L302-L349)、[`362–409`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/auth.go#L362-L409)、[`479–510`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/auth.go#L479-L510)：Followup、原身份缓存与登录 URL。
- **H2** [`hscontrol/oidc.go:83–102`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/oidc.go#L83-L102)、[`180–227`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/oidc.go#L180-L227)、[`477–521`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/oidc.go#L477-L521)：RP、PKCE、code exchange 与 ID token verifier。
- **H3** [`hscontrol/oidc.go:235–365`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/oidc.go#L235-L365)：state / nonce、同 sub UserInfo、资格过滤、显式注册确认；[`docs/ref/oidc.md:230–243`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/docs/ref/oidc.md#L230-L243) 为 claims 映射。
- **H4** [`hscontrol/types/node.go:121–168`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/types/node.go#L121-L168)、[`275–285`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/types/node.go#L275-L285)、[`hscontrol/auth.go:26–35`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/auth.go#L26-L35)：身份、owner / tag 与 machine-key mismatch。
- **H5** [`docs/ref/oidc.md:145–175`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/docs/ref/oidc.md#L145-L175)、[`hscontrol/oidc.go:448–453`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/oidc.go#L448-L453)：重认证期限；文档称 Access Token，函数实际使用 ID token expiration，本文以代码为准。
- **H6** [`docs/ref/registration.md:6–34`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/docs/ref/registration.md#L6-L34)、[`89–135`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/docs/ref/registration.md#L89-L135)：归属、注册批准、默认 pre-auth key。
- **H7** [`hscontrol/types/preauth_key.go:49–118`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/types/preauth_key.go#L49-L118)、[`hscontrol/app.go:590–598`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/app.go#L590-L598)：reusable / ephemeral / revoke、回收调度。
- **H8** [`hscontrol/auth.go:155–286`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/auth.go#L155-L286)：expired、logout、ephemeral 删除、tagged 例外及未实现独立 machine authorization。
- **R1** [RFC 8252 §§4–8](https://www.rfc-editor.org/rfc/rfc8252.html#section-4)、[RFC 7636 §4](https://www.rfc-editor.org/rfc/rfc7636.html#section-4)：external user-agent、code flow、public client、redirect / state 与 PKCE；[OIDC Core §3.1.3.7](https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation) 为 ID token 校验依据。
- **R2** [RFC 8628 §§3–5](https://www.rfc-editor.org/rfc/rfc8628.html#section-3)：device code / user code、轮询终态与跨设备确认。

[M1]: #证据索引
[M2]: #证据索引
[T1]: #证据索引
[T2]: #证据索引
[T3]: #证据索引
[H1]: #证据索引
[H2]: #证据索引
[H3]: #证据索引
[H4]: #证据索引
[H5]: #证据索引
[H6]: #证据索引
[H7]: #证据索引
[H8]: #证据索引
[R1]: #证据索引
[R2]: #证据索引
