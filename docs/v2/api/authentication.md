# 注册与认证

用户使用不透明会话，Node 使用本地持久密钥身份。账户、网络成员关系和设备接入分别建立，精确请求与响应见[生成契约](/v2/contracts/openapi.json)。

## Server 注册设置

注册开关（`--registration-enabled`，默认开放）与用户会话寿命（`--session-ttl-seconds`，默认 86400 秒）是实例启动配置，定义见 [Server 运行参数](/v2/server#运行参数)。关闭注册后 Server 拒绝新注册，但已有账户仍可登录、建网和接受加入链接。

Server 初始化空库结构后即可提供普通注册，不需要首用户认领。首个用户不自动拥有实例管理权限。邮箱是登录标识，不表示已验证邮箱所有权，也不按后缀自动加入网络。

## 账户注册与登录

```text
GET /api/v1/auth/options
  → POST /api/v1/auth/register 或 /auth/login
  → 返回用户、会话元数据与一次交付的 token
  → Authorization: Bearer …
  → POST /api/v1/networks 创建网络及本人 Owner membership
```

注册请求格式示例；密码由用户提供，不复用文档示例值：

<!-- schema: RegisterRequest -->
```json
{
  "email": "alice@example.com",
  "password": "example-password-do-not-use"
}
```

注册返回 201，登录返回 200，均为 SessionIssuedResponse：requestId、user、session、token。`GET /api/v1/auth/session` 返回同类型用户与会话元数据，不再次交付 token。logout 成功返回 204。

requestId 是服务端请求／活动关联，不是注册幂等键或秘密恢复凭据。注册响应丢失可用本人密码登录，不能再次注册覆盖账户。

### 登录失败退避 {#login-backoff}

同一邮箱（按规范化后的形式；不存在的账户同样计数，不暴露账户是否存在）从同一来源连续 5 次登录失败后，下一次尝试被拒绝 1 秒，之后每次失败倍增，最长 5 分钟；拒绝期间的登录返回 503 `busy`（`Retry-After: 1`），不校验密码，正确密码也一样。同一键同时只处理一次尝试，并发的其余尝试同样是 `busy`。一次成功登录或 15 分钟内没有新的失败即清零，不会永久锁定。来源是 HTTP 连接的直接对端地址（IPv6 按 /64），不读取 `X-Forwarded-For` 等转发头；经反向代理部署时所有客户端的来源都是代理，退避按账户生效，按客户端的限速在代理上配置（见[安装 · Console 与 TLS](/v2/install#server-proxy)）。退避状态只在进程内存中，最多保留 10,000 个键，Server 重启即清空。

### 输入规则

- 邮箱去除边缘空白；local part 使用未加引号的 ASCII dot-atom 并折叠大小写，域名经 IDNA 转为 ASCII，规范值最多 254 UTF-8 bytes。拒绝控制字符、quoted local 和 SMTPUTF8 local。客户端不能使用更窄的第三方正则提前排除合法输入。
- 密码为 12–128 个 Unicode scalar values，最多 512 UTF-8 bytes，不 trim 或改写。JavaScript `String.length` 是 UTF-16 单元数，不能直接使用。规则通过 auth/options 发布。
- 密码以 Argon2id v19、19456 KiB、2 次迭代、1 lane 和独立随机盐保存。PHC 参数有界验证，KDF 预算跟随实际 blocking 工作，调用取消不提前释放预算。
- 会话到期瞬间失效。Server 逐请求检查账户状态、凭据版本和会话，而非信任浏览器显示期限。

登录用户可创建多张隔离网络，只成为新网络的 Owner。新网络配置以默认拒绝配合显式 Mesh 互访模板初始化，不开放已有网络或公网出口。

## 已有账户加入网络

加入链接一次性、有期限，固定网络与角色。领取者须先登录；没有账户只能正常注册，链接不绕过注册关闭。没有按邮箱直接添加用户或全局用户搜索入口。

```text
有权管理成员的用户签发 join link
  → 转交给对方
  → 对方登录并 inspect 链接
  → redeem 同事务消费链接、创建 Membership 并记录活动
```

- 签发与消费均检查当前权限；链接绑定签发时 membership，签发者失权时其未消费链接同事务写入 `revokedAt`，后续重新加入不能复活。
- JoinLinkView 以 `issuedBy` 和 `consumedBy`（userId＋email）命名签发者与领取者；email 是账户当前地址，领取者离网后仍可读。
- inspect 用 POST body 接受 token，仅展示网络／角色；不消费链接，也不替代 redeem 时重检。链接自身状态先于签发者状态报告：已撤销、已过期、已使用分别返回 join_link_revoked／join_link_expired／join_link_used，只有仍有效却找不到签发时权限关系的链接才返回 join_link_issuer_unavailable。
- 链接不绑定收件人，任何持有人均可能领取。已有有效成员返回 already_member，不消费链接或改变角色。
- 同一领取者重试仅恢复同一个仍有效 membership；已移除成员不能重放链接恢复。撤销已消费链接不替代移除成员。
- SPA 可从 fragment 捕获 token 后清理地址栏，以 POST body 兑换；token 不进入查询参数、Referrer、日志、通用缓存或持久草稿。
- 角色降级同事务吊销该成员签发、仍未使用且未到期、而新角色已无权签发的链接（降为 admin 吊销 admin／owner 链接，降为 member 吊销全部）；已到期链接保持到期状态。`GET members/{membershipId}/role-change-impact/{role}` 按与变更相同的权限和可分配角色检查返回这些链接：`joinLinks` 最多 100 条、`joinLinkCount` 为总数、`version` 为当前成员版本；只读，同角色为空影响。

## 无邮件账户恢复

Server 主机的授权运维通过 `reset-password` 签发一次性链接；命令、参数与期限见 [Server 运维命令](/v2/server#运维命令)。该命令须取得正常私有存储所有权，执行前停止占用同一数据目录的 Server。

Console 使用 `/reset-password` 页面兑换，API 为 `/api/v1/auth/password-reset/inspect` 和 `/redeem`。链接绑定 User、credentialVersion、期限与一次消费，兑换检查密码规则并撤销旧会话。原值仅通过明确维护命令输出，手动转交，不发邮件或写普通日志。

网络 Owner／Admin 无权重置他人的全局账户密码。密码重置不恢复旧 Node 身份、不改网络授权；响应丢失后使用新密码正常登录。

## 创建请求的幂等与秘密交付

管理创建操作的 requestId 绑定主体、网络与原请求；同 ID 不同内容报 request_id_conflict。它是去重标识，不是凭据。

接入密钥与 join link 只在首次成功响应交付原值，长期保存摘要。创建重试返回相同资源元数据与 `secretDelivery: "unavailable"`；首次交付使用 `secretDelivery: "available"` 和 secret。需要新秘密时明确撤销旧项再创建。

列表、SSE、活动记录不返回原值或 token 摘要。设备批准的 PKCE 结果有独立的短期加密重交付机制，不能将其扩展为普通凭据的“查看原值”。

## 用户会话

token 为随机不透明凭据，Server 只存摘要；HTTP 使用 `Authorization: Bearer …`。会话绑定稳定 UserId、credentialVersion 和期限，邮箱修改不改变主体。

- 每次调用和管理事务重检 session、账户与网络权限。
- logout 撤销本会话；密码重置或全会话撤销终止相应旧会话与 SSE。
- SSE 使用 fetch 流；身份或网络切换须中止旧订阅、隔离缓存和草稿。
- 浏览器退出不自动注销独立 Node。账户停用／离网按当前 User／Tag 身份处理，见[身份生命周期](/v2/configuration-reference/identity#身份切换与生命周期)。

### 委托登录 {#delegation}

部署可以让配置过的浏览器客户端（例如对外站点的 API 参考）以用户身份调用 `/api/v1`，而不把密码交给它：OAuth 2.0 授权码加 PKCE（S256），Console 负责登录与同意，Server 签发。

```text
站点生成 code_verifier，计算 code_challenge，带 client、redirect_uri、code_challenge、state 打开 Console 的同意页
  → 已登录用户同意；Console POST /auth/delegations（createDelegation）
  → Server 返回一次性 code；Console 跳转到 redirect_uri，查询串只带 code 与站点自己的 state
  → 站点 POST /auth/delegations/token（exchangeDelegation），带 client、code、code_verifier、redirect_uri
  → Server 校验并消费 code，签发委托会话
```

- 客户端由 Server 启动参数 `--delegation-client <name>=<origin>` 声明（可重复；见 [Server](/v2/server)）。`redirectUri` 必须是该 origin 上的绝对 `http(s)` URL，不含 fragment 与凭据；`codeChallenge` 为 43 字符 base64url，`codeChallengeMethod` 只接受 `S256`。不满足者为 400 `invalid_request`。`state` 不经过 Server。
- 授权码一次性、60 秒有效，Server 只存摘要，并绑定签发它的用户会话、client、redirect_uri 与 challenge。兑换一次即消费，无论成功与否；client、redirect_uri 或 verifier 不匹配、过期、重复兑换、签发会话已失效，一律 400 `invalid_delegation`，不区分原因。
- 委托会话 1 小时到期、不续期，`AccountSessionView.delegationClient` 为客户端名；出现在账户会话列表里，可被用户的普通会话单独撤销，自己只能 logout。普通会话 logout 不影响它；改密码、撤销全部会话使其随之失效。
- 委托会话拥有用户的全部网络权限，但不能改密码、改邮箱、撤销其他会话，也不能再签发委托：这些返回 403 `permission_denied`。
- 兑换响应带 `server`：客户端访问本 Server 的 origin，由 `--public-origin` 配置，未配置为 null。
- 委托客户端的 origin 可跨源调用 `/api/v1`（含 SSE），规则与 Console origin 相同，见 [Server · 跨源访问](/v2/server#cors)。
- 每次授权在账户活动中记为 `session.delegate`，目标为授权时使用的会话；兑换记为该委托会话的 `session.login`。

## Node 的三层凭据

| 层 | 用途 | 生命周期 |
| --- | --- | --- |
| 设备批准／接入密钥 | 允许某次注册加入网络并写入身份 | 有限期、消费绑定 |
| 控制身份密钥 | 向 Server 证明已注册设备身份 | 本地持久 Ed25519 私钥，Server 绑定公钥 |
| 数据面密钥 | peer 认证及端到端加密 | X25519，独立用途与代际 |

控制私钥不进入配置或 Server／Console。VLESS UUID、Trojan password、TLS listener 材料不能替代 Node 身份。

### 用户设备接入

```text
设备私有保存密钥和 pending 记录
  → 创建绑定公钥、PKCE challenge、hostname、existing 的接入请求
  → 打开 Server 浏览器授权页
  → 用户登录、选择有资格的网络并批准
  → 设备持钥 query 取得本次一次性结果
  → 提交结果、PKCE verifier 和持有证明
  → 同事务消费批准、绑定 Node／公钥、写入配置 identity／地址
```

设备不收集用户密码或浏览器 session。PKCE 绑定本次请求，不替代长期密钥；批准和兑换均重检当前资格。

- 返回方式为 `returnMethod: "poll"`。EnrollmentRequestView 包含 hostname、公钥、existing、phase、期限和 `authorizationUrl`，没有 callback 或系统版本字段。
- `authorizationUrl` 是用户批准该设备的浏览器入口，Console 在其路径提供批准页；生成规则与 null 语义见 [Console origin](/v2/server#运行参数)。
- DeviceAuthorizationView 复用该 request，approval 仅向批准者、批准网络当前管理员或持有本次设备证明的 query 返回；其他主体为 null。读取不认领请求。

未批准请求由设备凭证明取消或到期，网页返回不取消全局请求。denyDeviceAuthorization 只允许批准者在兑换前撤回自己的批准；cancelled 是设备取消，withdrawn 是批准撤回。非法状态操作返回 state_conflict，无效证明返回 401。

### 无交互接入

管理员为网络签发一次性、带预分配 Tag 的接入密钥。设备提交自己的公钥和持有证明，密钥消费与 Node 绑定同事务完成，不创建 User 或授予管理权限。

创建时校验 initialSettings.hostname、listeners、显式材料、TLS 名称／有效期／密钥匹配及永久吊销记录；非法输入返回 configuration_invalid。自动材料在实际注册时生成；兑换按当前资格重验，首次 hostname 冲突添加唯一后缀。

密钥列表采用 1–200 条 keyset 分页，默认 50，从新到旧，total 按当前网络权限和首次创建上界计算。view 包含 createdBy、consumedAt、nodeId、非秘密 initialSettings 和 actions.revoke；密码、私钥、header 值不进入摘要。

### 已注册 Node 认证

Ed25519 持钥证明签发 5 分钟不透明 Node session。后续请求使用 bearer，因此 token 持有人在有效期内可冒用；不能把它描述为逐请求私钥签名。

1. `POST /agent-api/v1/auth/challenges` 接收 `SignedNodeChallengeRequest`：`intent` 包含客户端生成的 challenge `id`、已绑定的 `controllerId`、绝对 `expiresAt`，以及 `request` 中的 networkId、nodeId、keyId、operation、requestDigest；`signature` 为当前控制私钥对申请 transcript 的签名。Server 先验签与重检身份，再接纳最多 60 秒的申请，返回原 ID／期限、32-byte 随机 nonce 和 key generation。
2. `POST /agent-api/v1/auth/session` 严格验证 Ed25519 签名、摘要、当前身份和期限，同事务消费 challenge；同一 proof 重放拒绝。token 为 `mwns_` 加 32-byte 不透明值的无 padding base64url，仅存摘要。
3. 每个请求检查 Node／key 吊销、密钥代际、当前配置身份与稳定 membership。Tag 不依赖创建者资格，enabled=false 仍可控制同步。Server 重启更换 boot epoch，旧 Node session 必须重新认证。

challenge 申请响应不确定时，重发同一签名申请；同 ID／绑定返回原 nonce 和期限，已消费记录也不重新变成可消费证明。session 响应丢失时，Node 保留原 session intent，以新的申请 ID 取得**新的 challenge 并重新持钥证明**。同身份、key 和 intent 的会话仍有效时，Server 返回相同 token 与原截止；不重新计入会话额度或延长 lease。已消费的 proof 不能直接取回 token。

结果恢复使用有界的原始 challenge 记录；token 由私有 HMAC 域、Server boot 和该原始 transcript 派生。会话到期、记录不存在或 Server 重启后，新的持钥认证使用新 challenge 签发不同 token，旧 bearer 不复活。待证明 challenge 和会话恢复锚点各限全局 4,096、每 Node 八条；总保留申请记录限全局 8,192、每 Node 16 条。幂等读取先于新申请预算判定。Server 将清理与认证时间高水位一并持久提交；墙钟低于该界线时拒绝认证，不因重启或回拨重建过期申请。

申请签名字节由 `meshway_protocol::challenge_request_transcript` 定义，与消费 challenge 的签名使用不同 domain：

```text
"meshway/node-challenge-request/ed25519/v1\0"
  || challenge/controller/network/node/key UUID 原始各 16 bytes
  || operation byte (session=1, rotate=2)
  || requestDigest (32 bytes)
  || expiresAt Unix milliseconds (big-endian u64)
```

签名字节由 `meshway_protocol::node::proof_transcript` 定义：

```text
"meshway/node-proof/ed25519/v1\0"
  || operation byte (session=1, rotate=2)
  || controller/network/node/key/challenge UUID 原始各 16 bytes
  || key generation (big-endian u64)
  || nonce (32 bytes)
  || expiresAt Unix milliseconds (big-endian u64)
  || requestDigest (32 bytes)
```

requestDigest 为 `SHA-256("meshway/node-intent/v1\0" || canonical-intent-JSON)`。已校验 intent 的对象 key 递归排序、数组保持顺序、无空白 UTF-8 编码，密钥采用规范小写 hex；不接受浮点或任意 JSON 扩展。签名为 64 bytes 无 padding base64url，不签 JSON 展示文本。HTTP 时间使用 UTC RFC 3339，签名转换保持精确毫秒。拒绝 weak key，控制与数据面密钥不可代用。

HTTPS 负责 Server 身份和通道机密性。设备将 enrollment 返回的 controllerId 与 HTTPS origin 一并持久绑定，请求前核对 challenge；不能仅凭 Node 签名信任任意 origin。

### 换钥、重新登录与撤销

- rotate intent 同时绑定新控制／数据面公钥，旧与新控制私钥签同一个 challenge。设备请求前持久保存新私钥、pending 请求和 challenge。
- `POST /agent-api/v1/auth/rotate` 原子退休旧代际、建立新代际、撤回旧执行。响应丢失时重投同一请求，以新私钥证明恢复仍有效的公钥元数据；不再次轮换、不恢复旧 session，也不要求预先知道新 keyId。
- 恢复不受原 challenge 操作期限限制，但仍检查当前 Node／membership 和目标密钥有效性。同 requestId／challenge 内容不符报冲突，目标已退休／吊销则拒绝。
- enrollment intent 必填 existing：首次为 null；重新登录为 networkId、nodeId、keyId、keyGeneration。重新登录公钥须匹配当前绑定，批准网络不可改绑。
- PKCE 兑换与配置提交同事务完成 Tag→User 或 User 重绑定，保留 NodeId、地址和协议设置，终止旧 session／实例资格。普通配置保存／回滚不能替代设备证明。
- 多网络分别保存密钥和接入绑定；一个凭据不能通过修改 networkId 切换作用域。实例变化不等于新 Node，被替代实例不能继续上报或领取任务。
- 永久吊销不被旧注册重试、换钥或配置回滚清除。删除 Node 和成员移除中的 User 身份设备删除见[权限](/v2/permissions)。丢失私钥不能只靠公钥恢复身份。

密钥创建／吊销时间和操作资格分别表达；Node settings 为同一 revision 的非秘密视图，普通成员仅读取当前自己的 User 身份设备。

## 参考

- [Tailscale：Node keys 与 machine keys](https://tailscale.com/kb/1010/node-keys)
- [RFC 7636：PKCE](https://www.rfc-editor.org/rfc/rfc7636)
