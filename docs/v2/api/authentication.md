# 注册与认证

**已确认的 V2 方向，尚未实现新接口。** 用户使用不透明会话，Node 使用本地持久密钥身份；账户、网络成员身份和设备接入分别建立。

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
- 首版继续不依赖发邮件。邮箱作为登录标识，不声称已验证邮箱所有权，不因邮箱后缀自动加入网络；已有账户加入其他网络另由成员管理授权。

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

注册与登录成功返回相同的会话结构提案：

```json
{
  "user": { "id": "<user-id>", "email": "alice@example.com" },
  "session": { "token": "<opaque-session-token>", "expiresAt": "2026-10-04T12:00:00Z" }
}
```

示例时间不规定会话 TTL。密码只进入 HTTPS 请求与有界哈希流程，使用 Argon2id 等既有安全哈希，不存明文。注册响应丢失后可用本人密码登录，不通过再次注册覆盖既有账户。

登录用户可创建多张隔离网络，网络创建者只获得新网络 Owner。初始配置使用 access 默认拒绝＋显式 Mesh 互访模板；没有因创建账户而自动开放已有网络或公网出口。

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

### 无交互接入

管理员在 Web 为指定网络签发带预分配 Tag 的接入密钥，设备同时提交自己的公钥和持有证明。服务端将密钥消费和 Node 绑定放在同一事务内；本轮以一次性接入为基线。

这不是账户邀请注册。它只接入一个设备，不创建 User，不赋予网络管理权限。Tag 及材料由平台管理，节点接入后仍无须本地业务配置。

### 已注册 Node 认证

确定的身份语义是：**持有已绑定公钥的私钥，且该 Node 当前仍有效。** 单独提交 Node ID、公钥或自签声明不能通过认证。

HTTP 实现还需选择成熟的持钥证明／认证通道方案。可以在密钥认证后派生短期控制面凭据，但它不是另一份长期身份；若采用 bearer，泄漏者在其有效期内仍能冒用，不能宣称每个请求都证明了私钥持有。是否采用请求／通道绑定必须在认证合同中明确。

握手需覆盖 Controller 身份与 TLS、挑战唯一性／期限、请求上下文、防重放及密钥轮换；算法、签名输入和凭据编码尚未定版，不凭本页直接实现自制握手。经反向代理部署时，也不能默认已有客户端证书直通能力。

认证后的 Node API 从绑定中推导网络／节点，并检查永久凭据吊销、密钥代际及当前实例。User／Tag 授权身份从当前 Configuration 解析；enabled=false 仍允许同步／停用状态上报，不允许业务执行。认证成功只证明设备身份，访问许可、执行发布和 lease 继续独立检查。

### 重试、重新登录与撤销

- 网络请求前保存密钥及 pending 绑定；响应丢失使用原绑定恢复同一个 Node，不能每次重试生成新身份。
- 多网络接入分别保存密钥／绑定，一次只激活一个网络，不用客户端提交的 networkId 切换已有凭据作用域。
- 运行实例变化不等于换 Node；被新实例替代后旧实例不能继续上报或领取任务。
- 重新以用户身份登录可提交已确认的 Tag→User 配置转换，仍须证明是该设备；不从创建者记录自动恢复权限。
- 永久吊销节点接入或密钥使相应认证及派生凭据失效，并撤回执行许可；重放旧注册、配置回滚均不能复活。普通业务停用使用配置 enabled。
- 丢失私钥后不能靠公钥重新取得身份；恢复／轮换需要明确的再授权流程，不静默创建替代身份冒充恢复。

## 实现边界与参考

当前实现仍为仅邀请账户注册、用户 `mws_` 会话、节点随机 bearer 加独立数据面公钥。本页确定下一版方向；现有 App 接入消费者的协调切换需单独安排。

- [现行账户与会话](/local-accounts)
- [现行 App PKCE 接入](/device-login)
- [Tailscale：Node keys 与 machine keys](https://tailscale.com/kb/1010/node-keys)
- [RFC 7636：PKCE](https://www.rfc-editor.org/rfc/rfc7636)
