# 注册与认证

**V2 注册与认证合同。** 新 Controller 的隔离开发基线已接通账户 options／register／login／session／logout；其余操作和 Node 持钥流程按下面的设计继续实现。用户使用不透明会话，Node 使用本地持久密钥身份；账户、网络成员身份和设备接入分别建立。

## Controller 注册设置

配置结构与环境变量名提案：

```json
{
  "accounts": {
    "registrationEnabled": true
  }
}
```

```dotenv
MESHWAY_REGISTRATION_ENABLED=false
```

| 字段 | 默认 | 语义 |
| --- | --- | --- |
| `accounts.registrationEnabled` | `true` | 是否允许创建新的自助注册账户 |

这是 Controller 实例配置，不属于某张网络的 NetworkConfiguration。建议环境变量覆盖配置文件，启动时加载；精确加载规则随 Controller 配置接口定版。

- 启动后默认任何人均可注册。关闭时服务端拒绝注册，不只是隐藏 Console 按钮。
- **不提供邀请注册，也不通过邀请绕过开关。** 关闭注册不影响已有账户登录、创建网络或已授权 Node 运行。
- Controller 正常启动即初始化空库所需结构；首个用户走普通注册，不需要实例初始化／认领 API。空实例关闭注册时由部署者显式调整注册配置。
- 首版继续不依赖发邮件。邮箱作为登录标识，不声称已验证邮箱所有权，不因邮箱后缀自动加入网络；已有账户通过下面的加入链接加入其他网络。

## 账户注册与登录

```text
浏览器读取公开登录选项
  → POST /api/v1/auth/register：email + password
  → Controller 校验开关、输入和唯一性，安全哈希密码
  → 保存 User 和会话，返回用户信息与不透明 token
  → POST /api/v1/networks：创建自己的网络
  → 同事务创建 Network + 本人 Owner membership + 初始配置
```

注册请求示意：

```json
{
  "email": "alice@example.com",
  "password": "<user-supplied-password>"
}
```

注册与登录成功返回相同的 SessionIssuedResponse；以下 ID 和 token 为占位：

```json
{
  "requestId": "<request-id>",
  "user": { "id": "<user-id>", "email": "alice@example.com" },
  "session": {
    "id": "<session-id>",
    "createdAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-04T12:00:00Z"
  },
  "token": "<opaque-session-token>"
}
```

示例时间不规定会话 TTL。密码只进入 HTTPS 请求与有界哈希流程，使用 Argon2id 等既有安全哈希，不存明文。注册响应丢失后可用本人密码登录，不通过再次注册覆盖既有账户。

注册返回 201，登录返回 200；GET auth/session 返回 requestId、user 和同类型 session 元数据，不再次交付 token。退出成功返回 204。requestId 是服务端生成的请求／活动关联，不作为注册幂等键或秘密恢复凭据。

登录用户可创建多张隔离网络，网络创建者只获得新网络 Owner。初始配置使用 access 默认拒绝＋显式 Mesh 互访模板；没有因创建账户而自动开放已有网络或公网出口。

公开 auth/options 同时提供密码规则及其计量方式，前端与服务端使用同一生成约束，不把 UTF-8 bytes 和 Unicode 字符数混用。注册的邮箱占用错误按[Problem 合同](/v2/api/problems)明确表达。

新账户实现采用以下输入边界，HTTP／生成客户端按同一规则接线：

- 邮箱先去除边缘空白；local part 使用未加引号的 ASCII dot-atom 并折叠大小写，域名经 IDNA 转为 ASCII。规范值最多 254 bytes；拒绝控制字符、quoted local 和 SMTPUTF8 local，不要求已验证邮箱所有权。HTTP 输入是待规范化的字符串，不能用更窄的第三方 email 正则提前排除允许输入。
- 密码为 12–128 个 Unicode codepoints（Unicode scalar values），最多 512 UTF-8 bytes；不 trim 或改写。JavaScript 的 UTF-16 `String.length` 不是这里的计数方式。字段结构解码与 password_weak 语义校验分开，规则来自 auth/options。
- 当前 Argon2id v19 使用 19456 KiB、2 次迭代、1 lane 与独立随机盐。既有 PHC 先经过有界格式／参数校验，不能让数据库中的参数任意放大验证工作；KDF 预算跟随实际 blocking 任务，调用取消不会提前释放预算。
- 会话默认 24 小时，部署设置可在 1 分钟至 30 天内按整秒覆盖。到期瞬间即失效；浏览器显示期限不替代服务端对账户、凭据版本和会话的再次检查。

## 已有账户加入网络

**已确认：使用一次性、有期限、绑定网络与角色的加入链接。** 不提供按邮箱直接添加用户或全局用户搜索。链接持有人须先登录；没有账户时只能走正常注册，注册关闭时链接不提供例外。

registrationEnabled=false 仍允许已有账户接受加入链接，只阻止新账户注册。inspect 通过 POST body 验证链接后展示网络／角色，redeem 才执行消费；检查结果不代替消费时的再次校验。

```text
有权管理成员的用户签发 join link
  → 手动转交给对方
  → 对方登录已有账户并接受
  → 同事务消费链接、创建新的 Membership、记录活动并通知 SSE
```

- 链接绑定网络、角色和签发时的 membership；角色不能由领取者提高，签发和消费都检查当前权限。
- 链接不是实名收件人邀请，任何持有人均可能消费。前端明确显示将加入的网络／角色。
- 已有有效 membership 时返回 already_member，不消费新链接、不通过链接改变角色。
- 相同领取者对已成功消费结果的重试仅恢复同一仍有效 membership；移除后重新加入不能复活旧链接。撤销已消费链接也不替代移除成员。
- 签发者失去对应权限时废止未消费链接，不因以后恢复角色重新有效。
- 链接 token 可由 SPA fragment 接收，捕获后清除地址栏残留，以 POST body 兑换；不进入 API 查询参数、Referrer、日志、通用缓存或持久草稿。

## 无邮件账户恢复

**已确认：Controller 主机的授权运维通过 CLI 生成一次性重置链接，Console 提供 `/reset-password` 兑换页。** 不增加网络 Owner 的全局账户管理权限，即使目标用户只属于本网也不能由 Owner 重置其密码。

CLI 使用受控本地维护访问，遵守数据库锁和私有目录边界；具体命令／本地 IPC 随实现定版。重置链接绑定 User、credentialVersion、期限与一次性消费；兑换验证密码策略并撤销旧会话，不恢复旧 Node 身份或改写配置权限。

原值仅由明确运维操作输出后手动转交，不发送邮件、不写普通日志。公开兑换端只接受本次重置凭据，响应丢失后可使用新密码正常登录，不重复重置账户。

## 创建请求的幂等与秘密交付

创建类操作使用 requestId，绑定操作主体、网络和原请求；同 ID 不同请求报冲突。它是去重标识，不是认证凭证。

接入密钥和 join link 首次成功创建响应交付原值；服务端长期只存摘要。重试已完成创建返回相同资源元数据和 `secretDelivery: "unavailable"`，不会签发第二项，也不声称能恢复原值。需要新原值时明确撤销旧项再重新签发。首次交付使用 `secretDelivery: "available"` 和必填 secret 的独立响应分支。

列表、SSE、活动记录均不返回 secret／tokenHash。浏览器批准后的设备 PKCE 结果是另一种已定义的受保护重交付流程，不能把其短期 delivery 密文扩展成普通密钥列表的“查看原值”。

## 用户会话

**继续使用随机不透明会话，不改 JWT。** token 只存摘要，响应后由 Console 按既定会话存储方式管理，逐请求通过 `Authorization: Bearer …` 发送。

JWT 是可验证的声明格式，不自动解决账户停用、即时退出或角色变化。本项目仍需实时读取当前账户／成员权限，不通过 JWT 内缓存的角色代替真实授权。

- session 绑定稳定 User ID、凭据版本和到期时间；邮箱修改不改变主体。
- 每次调用及管理事务检查 session、账户状态与当前网络权限。
- 退出撤销该会话，密码重置／全部会话撤销终止相应旧会话；活动 SSE 同样失效。
- Console 沿用 Authorization 与当前标签页身份隔离。SSE 使用 fetch 流，登录／网络切换时中止旧订阅并隔离缓存和草稿。
- 用户退出浏览器只结束会话，不自动注销已独立接入的 Node。账户停用／离网按设备有效授权身份处理；Tag 节点独立存续规则见[身份生命周期](/v2/configuration-reference/identity#身份切换与生命周期)。

## Node 的三层凭据

| 层 | 用途 | 生命周期 |
| --- | --- | --- |
| 一次性设备批准／接入密钥 | 允许某次注册加入指定网络、绑定 User 或 Tag 身份 | 有限期、消费绑定；不是日常 Node 凭据 |
| Node 控制面身份密钥 | 向 Controller 证明自己是已注册设备 | 私钥本地持久保存，Controller 绑定公钥与 Node ID |
| 数据面密钥 | 与 peer 认证及端到端加密 | 独立用途与代际，按可信执行发布绑定 |

控制面私钥不进入 NetworkConfiguration，也不交给 Controller／Console；数据面公钥经可信身份绑定发布。协议 UUID、password、TLS listener 材料均不能替代 Node 身份。

新 Controller 的密钥轮换把 requestId／请求摘要、原签名 challenge、生成的公钥元数据及代际写入同一事务。设备在轮换前私有持久保存新私钥和 pending 请求／challenge；成功响应丢失后重投同一轮换请求，以新私钥签名恢复仍有效的同次公钥元数据，无需预先知道新 keyId。恢复不会再次轮换，旧 challenge 的原始操作期限不限制只读结果恢复；当前 Node、成员身份及两枚目标密钥仍须有效。目标密钥已退役／吊销时拒绝恢复；同 requestId 或 challenge 的内容不符报冲突。此恢复只交付公钥元数据，不恢复旧 session 或旧执行许可。

Tailscale 也区分 machine key 与用于网络通信的 node key。本设计采用其“设备持久身份与网络通信密钥分责”的原则，不照搬 WireGuard／Noise wire 协议。尤其不能把现有 X25519 加密私钥直接当签名私钥。

### 用户设备接入

```text
App / CLI 先私有持久保存本网络接入使用的密钥与 pending 记录
  → 建立 enrollment request，绑定公钥、PKCE challenge、设备信息和返回方式
  → 打开／打印 Controller 浏览器授权链接
  → 浏览器用用户 session 登录，选择自己有资格的网络并批准
  → Controller 将一次性结果绑定 User、membership、网络与本次请求
  → 设备提交结果、PKCE verifier 和持有私钥的证明
  → Controller 原子消费批准、绑定 Node／公钥，并在配置写入 identity 与分配地址
  → 设备保存 Node ID，随后以密钥身份认证
```

App 使用系统浏览器及受限制回调；CLI 无本地回调时领取同一请求的结果。领取须证明持有本次请求秘密／密钥，知道 request ID 或浏览器链接不够。精确轮询／推送编码另定，二者复用同一授权服务。

设备不收集用户密码，不拿浏览器 session。PKCE 证明本次授权请求的绑定，不替代长期 Node 密钥。批准与首次兑换均校验当前用户及网络资格；登录后不会无条件批准外来设备链接。

尚未批准的请求没有绑定用户。网页选择“不批准／返回”只结束当前网页流程，不调用全局取消；请求由设备凭持钥证明取消，或等待到期。`denyDeviceAuthorization` 仅允许已批准请求的批准者在兑换前撤回，其他登录用户及未批准请求均被拒绝。撤回后旧一次性结果不能兑换；已接入 Node 的删除仍是独立操作。

设备侧 `EnrollmentRequestView` 包含签名请求里的 `hostname`、公钥、`existing` 绑定、phase、到期时间与 `returnMethod: "poll"`。当前接入请求没有系统／Agent 版本或 callback 注册字段，不从后续 State 倒填设备自报信息。网页 GET／approve 使用 `DeviceAuthorizationView`：`request` 复用设备视图，`approval` 仅向批准者或批准网络当前管理员返回 `{ userId, networkId }`，其他主体为 `null`；设备凭本请求的签名和 PKCE 证明进行 query 时也可读取该 approval。读取不认领请求。phase 中 `cancelled` 表示设备签名取消，`withdrawn` 表示批准者撤回，两者独立持久化；非法生命周期操作返回 `state_conflict`，证明无效仍为 401。

接入密钥列表采用 1–200 条 keyset 分页，默认 50 条、按创建顺序从新到旧；`total` 与首次分页创建上界和当前授权网络一致，每页重检管理权限。普通 view 包含 `createdBy`、`consumedAt`、`nodeId`、`initialSettings` 非秘密摘要与 `actions.revoke`。摘要复用 `ListenerView`，不包含密码、私钥或 header 值。创建时一次性返回的 secret 与该只读 view 分开。

### 无交互接入

管理员在 Web 为指定网络签发带预分配 Tag 的接入密钥，设备同时提交自己的公钥和持有证明。服务端将密钥消费和 Node 绑定放在同一事务内；本轮以一次性接入为基线。

签发时校验 `initialSettings.hostname`、listener 声明、显式材料及 TLS 证书／私钥／名称／有效期和永久吊销记录。非法值返回带字段路径的 `configuration_invalid`，不签发密钥；缺省自动材料在真实 Node 接入时生成。兑换仍按当前网络资格、资源和配置重验；hostname 冲突按首次接入规则添加后缀。

这不是账户邀请注册。它只接入一个设备，不创建 User，不赋予网络管理权限。Tag 及材料由平台管理，节点接入后仍无须本地业务配置。

### 已注册 Node 认证

确定的身份语义是：**持有已绑定公钥的私钥，且该 Node 当前仍有效。** 单独提交 Node ID、公钥或自签声明不能通过认证。

新 Controller 的 Node／执行候选使用 **Ed25519 持钥证明后签发 5 分钟不透明 Node session**。它不是另一份长期身份；后续 HTTP 使用 bearer，泄漏者在有效期内仍能冒用，不声称每个请求重新证明了私钥持有。接入与完整 Config 提交已在同一 SQLite 事务接通并通过 HTTP 验证；Agent 消费与真实包流仍须独立接线验收。

候选握手合同：

- `POST /agent-api/v1/auth/challenges` 绑定 networkId、nodeId、keyId、operation 和 requestDigest，返回服务端随机 32-byte nonce、challenge ID、当前 key generation、持久 controllerId 与 60 秒绝对期限。
- 控制密钥固定 Ed25519，数据面密钥固定 X25519，不能相互代用。签名通过 `ed25519-dalek` strict 验证，并拒绝 weak key。
- 签名输入由 `meshway_protocol::node::proof_transcript` 唯一定义：`meshway/node-proof/ed25519/v1\0`、operation 单字节（session=1、rotate=2）、controller/network/node/key/challenge 的五个 UUID 原始 16 bytes、key generation 大端 u64、nonce 32 bytes、过期时间的 Unix 毫秒大端 u64、requestDigest 32 bytes；不签 JSON 展示文本。HTTP 时间字段如 `expiresAt` 使用 UTC RFC 3339，转换须保持精确毫秒，不能以浮点舍入或更改展示时区改变签名字节。
- requestDigest 是 `SHA-256("meshway/node-intent/v1\0" || canonical-intent-JSON)`；对已通过类型校验的 intent 递归按对象 key 排序、保留数组顺序、以无空白 UTF-8 JSON 编码，密钥原值采用规范小写 hex。intent 只使用已定义的字符串、布尔、null 和安全整数，不接受浮点值或任意 JSON 扩展。签名为 64 bytes 的无 padding base64url；生成 SDK 的属性声明顺序不影响证明。
- `POST /agent-api/v1/auth/session` 验证签名、请求摘要、当前身份与期限，同事务消费 challenge 并只存 session token 摘要；同 challenge 重放拒绝。原值以 `mwns_` 开头，随后 32 随机 bytes 的无 padding base64url。
- session 逐请求读取永久 Node／key 吊销、当前 key generation、当前 Config identity 与稳定 membership。Tag 身份不依赖创建者资格；enabled=false 可继续控制同步。Controller 重启使用新 boot epoch，旧短期 session 拒绝，必须重新持钥认证。
- `POST /agent-api/v1/auth/rotate` 的固定 intent 同时绑定新控制／数据面公钥；旧密钥与新 Ed25519 密钥分别签署同一 rotate challenge。原子退休旧代际、保存新代际并撤回旧执行许可。旧 session／旧注册结果不能领取新身份。
- HTTPS 负责远端 Controller 身份和通道机密性，客户端将 enrollment 返回的 controllerId 与配置的 HTTPS origin 一并持久绑定；请求前核对 challenge 绑定，不能只见有效 Ed25519 签名就接受任意 origin。反向代理不需要转发客户端证书。

上述是隔离候选的实际 Node 认证合同，不表示现有 Agent 已切换。注册批准、持钥认证、运行实例与执行 lease 是独立边界；SSE／联系保活不续期旧执行许可。

认证后的 Node API 从绑定中推导网络／节点，并检查永久凭据吊销、密钥代际及当前实例。User／Tag 授权身份从当前 Configuration 解析；enabled=false 仍允许同步／停用状态上报，不允许业务执行。认证成功只证明设备身份，访问许可、执行发布和 lease 继续独立检查。

### 重试、重新登录与撤销

- 网络请求前保存密钥及 pending 绑定；响应丢失使用原绑定恢复同一个 Node，不能每次重试生成新身份。
- 多网络接入分别保存密钥／绑定，一次只激活一个网络，不用客户端提交的 networkId 切换已有凭据作用域。
- 运行实例变化不等于换 Node；被新实例替代后旧实例不能继续上报或领取任务。
- 重新以用户身份登录可提交已确认的 Tag→User 配置转换，仍须证明是该设备；不从创建者记录自动恢复权限。
- enrollment intent 必须包含 `existing`：首次接入为显式 `null`；重新登录为 networkId、nodeId、keyId、keyGeneration 对象。签名摘要也包含该字段，遗漏会在结构校验时拒绝。重新登录的控制／数据面公钥必须匹配该设备当前有效绑定，整个 intent 仍由控制私钥签名；批准网络不能改绑。PKCE 兑换与完整 Config 提交同事务完成 Tag→User／用户重绑定，保持 Node ID、地址和协议设置，终止旧控制 session／实例执行资格。普通 Config 文档不能替代这份设备登录证明。
- 永久吊销节点接入或密钥使相应认证及派生凭据失效，并撤回执行许可；重放旧注册、配置回滚均不能复活。普通业务停用使用配置 enabled。
- 最新密钥被吊销但尚未删除的 Node 仍出现在管理库存，分别显示控制／数据面密钥吊销时间；业务 enabled 不冒充凭据有效性。成员移除与其当前 User 身份 Node 删除、地址退出和执行撤回在同一事务提交，Tag 身份 Node 保留。其他配置对被删 Node 的显式引用若无法通过校验，整个操作失败，不提交半套结果。

Node 管理库存分别返回 `controlKeyCreatedAt`／`dataplaneKeyCreatedAt` 和 `canRevokeControlKey`／`canRevokeDataplaneKey`，与 `canDelete` 独立。`desiredExecution` 是当前期望执行引用，可能来自后续 refresh，不代表 saved／applied，也不把它的 receipt 纳入旧 apply 的目标计数。`GET /api/v1/networks/{networkId}/nodes/{nodeId}/settings` 返回相同的非秘密 `NodeSettingsView`：管理员读本网节点，普通成员只读当前自己的 User 身份设备；转为 Tag 后原创建者失去 own-only 权限。完整配置原文及管理历史权限保持独立。
- 丢失私钥后不能靠公钥重新取得身份；恢复／轮换需要明确的再授权流程，不静默创建替代身份冒充恢复。

## 实现边界与参考

当前实现仍为仅邀请账户注册、用户 `mws_` 会话、节点随机 bearer 加独立数据面公钥。本页确定下一版方向；现有 App 接入消费者的协调切换需单独安排。

- [现行账户与会话](/local-accounts)
- [现行 App PKCE 接入](/device-login)
- [Tailscale：Node keys 与 machine keys](https://tailscale.com/kb/1010/node-keys)
- [RFC 7636：PKCE](https://www.rfc-editor.org/rfc/rfc7636)
