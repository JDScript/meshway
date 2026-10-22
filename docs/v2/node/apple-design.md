# macOS App 设计

> **实验实现，Apple Host 已有 VM Mesh 客户端、IPv4 出口使用与接口中断恢复证据。**本页记录 macOS App 的已定方向、组件边界、分阶段计划与待验证项。现行 Node 行为的唯一定义仍是[运行状态](/v2/node/runtime)、[数据面](/v2/node/dataplane)与[连接获取](/v2/node/connectivity)；精确验收范围只以[验收范围](/v2/node/validation#macos)为准。本页不加入文档导航。
>
> 归档 `archive/apple-007e9deb`（`apps/apple`、`docs/apple-app.md`）对接已退役的 agent 协议，且由 App 自行联系 Server。本设计只按[复用白名单](#reuse)取用其中的工程外壳，不恢复其控制流程。

## 已定方向 {#decisions}

| 项 | 决定 |
| --- | --- |
| 平台 | macOS 26 及以上，仅 arm64。iOS 不在本期范围，但 extension 侧代码不使用 System Extension 专有能力之外的假设，保留以后装进 App Extension 的可能 |
| 分发形态 | NetworkExtension Packet Tunnel Provider，以 **System Extension** 形式随 App 安装，Developer ID 签名、不经 App Store。不提供 launchd root daemon 直接操作 utun 的形态 |
| 进程分工 | extension 内嵌唯一 Node 运行时（Control、Engine、Manager、SecureNode、DNS）；App 与 macOS 上的 `meshway` CLI 只是本机 IPC 客户端，不联系 Server、不持有控制状态 |
| 包接入 | 从 NE 取得 utun 文件描述符交给 Rust 批处理路径；`NEPacketTunnelFlow` 作为退路保留在包设备接口后面 |
| 第一版能力 | Mesh 客户端：接入（浏览器批准／enrollment key）、组网、Mesh DNS、使用出口与切换出口方案、`status`／`ping`／`netcheck`、换钥、自助注销。**不**做本机出口角色，不做多网络切换 |
| 验收环境 | 本机 Tart（Virtualization.framework）macOS 26 VM 作为常驻验收机；NE 会改动宿主路由与 DNS，不在开发机上安装 extension |
| 签名 | Apple Developer Program 会员续费后申请 Packet Tunnel 的 System Extension／Developer ID entitlement；续费前在关闭 SIP 与 AMFI 校验的验收 VM 中以开发签名运行（[待验证](#unverified)） |

## 组件与依赖 {#components}

```text
apps/meshway-apple/                 # Xcode 工程（非 Cargo 包）
  App/                              # SwiftUI：菜单栏面板 + 主窗口，IPC 客户端
  PacketTunnel/                     # System Extension principal class，薄壳
  meshway-apple-bridge/             # Cargo 包：UniFFI 桥接（唯一允许生成 ABI 的 unsafe 边界）
crates/meshway-node/                # 从 apps/meshway-daemon 抽出的 Node 运行时库
apps/meshway-daemon/                # meshwayd：参数、日志、信号、Unix IPC、systemd、Linux Host 装配
```

```text
meshwayd ──► meshway-node (+ Linux Host)
meshway-apple-bridge ──► meshway-node (+ Apple Host)
App / CLI(macOS) ──XPC──► extension ──FFI──► meshway-apple-bridge
```

### Node 运行时库 {#meshway-node}

`crates/meshway-node` 持有运行协调器、持久意图、状态投影、控制调度、网络 plane／worker、DNS 装配、诊断与 IPC 分派。Apple 桥接与 daemon 共同装配这个库：

- **移入库**：`runtime/` 全部（intent、status、control、network、peers、ping、netcheck、local_endpoints）与 IPC 的类型化分派（当前 `ipc/mod.rs` 的 `Channels`、请求 ID、期望 revision、有界队列、ping 流取消）。分派与传输、对端认证分离，库只接受已认证的命令流。
- **留在 meshwayd**：clap 参数、`meshway-logging` 安装、信号、musl 分配器、Unix socket 服务与 UID 认证、systemd notify、stdout 就绪输出、`/var/lib/meshway` 默认路径。
- **平台 Host 接口**：库定义，Linux 实现包装现有 `meshway_underlay::Host`、`meshway_tun::Tun`、`meshway_dns::SystemDnsOwner` 与出口 driver；Apple 实现在 `crates/meshway-node/src/host/apple/`，经桥接包向 Swift 回调 NE 设置。接口面按实际消费者分组：

| 接口 | 职责 | Linux | Apple |
| --- | --- | --- | --- |
| 生命周期／capture | 按 `Desired` 安装、挂起为负向边界、down、带保留／移除语义的 join 关闭，报告真实完成与归属失效 | underlay Host | `setTunnelNetworkSettings` |
| 包设备 | MTU、可取消的有界批量读、保留已提交前缀语义的批量写；dataplane carrier 的 `deliver*`／`TunPending` 改为依赖此接口而非具体 `Tun` | `meshway_tun::Tun` | utun fd（4 字节 AF 头） |
| 受保护 socket | 沿用现有 `meshway_connectivity::SocketFactory` 与穿透授权全集，不缩成 connect／bind | underlay `SocketFactory` | extension 内普通 socket + `IP_BOUND_IF` 绑定当前主接口 |
| Control 传输 | 固定 Server origin 的 HTTP client builder，路径变化时中断 | Unix 中继 + 平台信任 | extension 直连 + 平台信任 |
| 网络观察 | path／refresh 分类事件、本机接口与 localEndpoint 清单 | rtnetlink monitor | `NWPathMonitor` 与 sleep／wake 回调 |
| 系统 DNS | 负向捕获、安装、观察／恢复、恢复先于撤除 | resolved／resolv.conf | `NEDNSSettings`（[Apple Host](#apple-host)） |
| MSS／MTU | 应用 `SecureNode::clamps` | nft `meshway_mss_*` | 不安装内核 clamp，按[待定](#open) |
| 出口角色 | 能力探测、投影、ACK／conntrack | 现有 driver | 不支持，能力不声明 |
| 本地诊断 | 系统解析、直接 ICMP | raw socket | 第一版不支持非出口外部 ping，后续用 `SOCK_DGRAM` ICMP |

抽取以 Linux 行为不变为验收：`mise run check` 加[验收范围](/v2/node/validation)的现有矩阵回归，不改 IPC wire（仍为 IPC 13）与 HTTP。

### Apple Host {#apple-host}

- **路由**：Mesh 网段放入 `includedRoutes`；出口捕获只把 `Desired::capture` 的 `managedIpv4`、`managedIpv6` 与 `sinkIpv6` 区间转换成最小 CIDR 覆盖，不重新解释规则。默认范围包括 IPv4 `0.0.0.0/0`；IPv6 `::/0` 在 Swift NE 适配层表达为 `::/1` 与 `8000::/1`，因为 VM 的 ULA-only 隧道虽获设置 completion，单独 `::/0` 不会成为系统 IPv6 默认服务。静态 IPv6 sink 在 utun 内丢弃；运行时无就绪 IPv6 出口沿用数据面本机错误路径。`apply(empty)` 与租约到期保留捕获与 DNS 设置，不落回物理网络；显式新方案的 direct／unmanaged 可收缩出口范围，Mesh 历史边界保留至 down。
- **本地网络**：默认捕获要求已保存的 `NETunnelProviderProtocol.excludeLocalNetworks=true`；provider 检查该前提，lab 保存已有 manager 时设置它。`includeAllNetworks` 保持 false。NE 的本地网络排除是当前 Apple 平台限制，不能宣称 Linux 的“只有管理员 direct CIDR 才允许源 LAN”完全等价；SSH 管理通道的实测范围见验收行。
- **underlay 绕行**：依赖 NE 不把 provider 自身流量送回本隧道，不添加 Server／peer IP 的全局 excluded route。Rust HTTPS Control、Native TCP 与 Native UDP 在默认捕获下已有实际 IPv4 外层承载／出口包流证据；不外推到外层 IPv6（[平台验证](#unverified)）。
- **DNS**：`NEDNSSettings.matchDomains` 为 Mesh 区域；当[出口 DNS 判定](/v2/node/dataplane)要求普通查询经出口时加入空匹配域。DNS 服务地址是隧道内一个保留地址，worker 把发往该地址的 UDP／TCP 53 交给现有 `meshway-dns` OwnerDispatcher，不另写解析实现。
- **漫游**：`NWPathMonitor` 的接口／网关变化映射为现有 `NetworkChange` path／refresh 分类，复用[漫游](/v2/node/connectivity#roaming)的通道跟随与重新穿透。

#### 当前接线与限制

- `Apple::new(OwnedFd, utun_name, TunnelSettingsSink)` 构造平台附件。`TunnelSettingsSink::apply(Option<TunnelSettings>)` 的完成表示 NE 设置完成；capture 从 `Desired` 的本机地址、完整 Mesh 池与出口区间计算，使用 `/32`／`/128` 本机地址、默认内层 MTU 1366。历史 Mesh 前缀至多 128 条；出口路由至多 32,768 条（含静态 IPv6 sink 的 CIDR 覆盖，超过则先安装全范围负向捕获并拒绝 applied）。切换先持久化、安装旧新出口范围并集，再安装新范围；暂存并集至多 65,536 条，超出用全范围负向边界。`suspend` 保留 NE 设置，关闭物理描述符，包平面由同一 Engine 撤回。显式 `down` 才移除设置。
- 私有 `apple-capture.json` 仅记录负向捕获资源，先于 NE 变更原子写入；设置失败或中断时保留旧新并集，新范围 completion 成功后才收缩记录。重新打开 Host 时先恢复捕获，再等待 Engine 安装有效 netmap；该记录不授权任何包。Mesh 池与出口前缀分别记录，只有 Mesh 池参与本机地址 overlap 检查。接口地址每秒重枚举，重叠时关闭描述符并报告 overlap，重叠消失后按现有 worker 流程重装。
- NE 设置串行提交，只在系统 completion 成功后确认；5 秒未完成则报错并撤回包输出，原调用完成前拒绝下一次设置变更，不把超时当作已回滚或已安装。
- `meshway_tun::Utun::from_owned_fd` 以 rustix＋Tokio AsyncFd 读写 Darwin 的 4 字节 AF 头，无后台包发送者；每次写只提交一个报文并返回已提交前缀，读批沿用 32 包／64 KiB 上限。桥接只用 `F_DUPFD_CLOEXEC` 复制 NE 的原 fd，成功结果才转入 `OwnedFd`；Swift 不调用 packetFlow 的读写。
- Apple socket 工厂实现完整 `SocketFactory`，TCP／UDP 普通 socket 绑定当前物理接口（`IP_BOUND_IF`／`IPV6_BOUND_IF`）。资源差量沿用同一 `netmap_resource_compatible`；STUN／attempt／learned／channel 授权与 Linux 共用 `meshway_underlay::traversal`，没有 Apple 专用协调权限实现。实际 UDP 截断／缓冲与批量 I/O 复用 `meshway-transport` 的 macOS 后端。
- Control 使用直连、平台信任、无代理／空闲连接池。Host 的 `RequestInterrupt` 在 path 事件时中断有限 HTTP 请求（包括 body 与 sync 长轮询）；refresh 不中断。该钩子不改变 Linux UDS relay。Control 与承载 bootstrap 各有独立的最多 8 个系统名称解析工作；已交给 libc 的解析不能强行中止，取消后不使用结果、保留工作预算，Host shutdown 等待 join。真实 provider 的 Rust Native TCP／UDP 已有默认捕获包流证据；吞吐未测。
- `NWPathMonitor` 取当前物理接口、状态、网关与地址族支持组成指纹；Rust 比较该接口实际地址，变化发 path，其余回调与 sleep／wake 发 refresh（如果同时发现地址变化仍为 path）。VM 只验过同一 en0 中断 3 秒并恢复：同 provider、原 TCP 继续，UDP 在中断期丢包后恢复。换 NAT、跨接口／地址的通道跟随、重新穿透与真实 sleep／wake 尚未验收。
- MSS clamp 返回明确的不支持错误，使用隧道 MTU 与既有 PTB；不宣称安装了 clamp。出口角色不声明能力；外部直接 ICMP 不支持。源端 IPv4 出口、单出口 DNS 与可信域名关联已验；IPv6 捕获／拒绝已验，运行时不可用时 utun 可见本机合成 ICMPv6 1/1，但 macOS nc 不立即以 Linux EACCES 语义失败。实际 IPv6 出口转发尚未验收。

#### 本机 DNS 服务地址（已定） {#local-dns}

macOS 与 Linux 共用本机 `SystemDnsSettings.anchor_address`，默认 **198.18.255.254**，可通过相同的本机 DNS 锚点参数覆盖。桥接字段为 `StartConfig.dns_anchor_address`；lab 用 `MESHWAY_DNS_ANCHOR_ADDRESS` 写入 NE preferences，未设置时沿用已保存值或库默认值。该地址不是 Server 分配的节点地址，不改 wire／云端配置，也不借用出口 `DnsService.address`。

- NE 只给该地址添加 `/32` included route，并将其作为 `NEDNSSettings` 服务器，不把服务地址赋给内核接口。域匹配与搜索域来自现有 `SystemDnsPlan`；root 匹配使用空字符串。
- 与任一本机接口地址或 Mesh 池冲突时 DNS 安装失败、阻断整体 applied。错误分别为 `apple_dns_interface_address`／`apple_dns_mesh_pool`，由现有 DNS health／状态路径报告；恢复时重新检查。恢复旧 journal 时也不安装冲突的 DNS 设置。
- worker 将本机 TUN 中发往该地址的包交给 `meshway-dns` 的有界 DNS 包服务。smoltcp 只处理该虚拟 IPv4 端点 UDP／TCP 53 的可靠传输；DNS 编解码、选择、查询许可与回答取消复用同一 `OwnerDispatcher`、`TcpCodec` 和 `Reply`。这不是通用 Mesh netstack／SOCKS 入口，其他端口与转发均不开放。A／AAAA 查询都可通过该 IPv4 DNS 端点。
- 入口与出口队列各至多 32 包，至多 128 个查询、32 条 TCP 连接；TCP 每连接顺序处理查询，保留回答许可至 ACK 或结束。每次实际 TUN 写 poll 都检查原回答许可；租约到期发布空 DNS scope，正向回答失效，保留的服务可回 SERVFAIL，DNS 捕获不撤到物理网络。显式 down 才恢复系统 DNS 并移除捕获。
- 配置上游 DNS 的事务随机化、来源／问题校验、截断与 TCP framing 只有 `meshway-dns::upstream` 一份实现，Linux 与 Apple 只提供各自的受保护 socket。未捕获名称仍由系统处理；需要将原系统分域规则压平作 fallback 的搜索域组合仍不支持，与现有分域后端限制一致。

上述 UDP／TCP 查询、TTL 0 MagicDNS、排队回答撤回与安装冲突已有组件测试；VM 系统 resolver、真实 utun DNS、租约到期 SERVFAIL 与 Down 后系统 DNS 清理已验，见[验收范围](/v2/node/validation#macos)。

### 状态目录与身份

System Extension 以 root 运行。状态目录为 `/Library/Application Support/Meshway`（root、0700），extension 的 Sandbox entitlement 只为该目录声明 absolute-path read-write exception；VM 已实测创建目录、写入身份／签名 netmap 与离线重启载入。沿用 Control 与 `meshway-private-storage` 的私有文件与原子发布，不为 Apple 另写 Keychain 身份存储。`netmap.json` 离线载入与租约语义与 Linux 相同，续租保存步长见 [Control](/v2/node/control)。

### App ↔ extension IPC {#ipc}

- 传输为 extension 提供的 XPC Mach service，对端以代码签名要求（同 Team 的 App 与内置 CLI）认证；不用 UID 认证，也不把 Unix socket 放进 App Group 容器（root extension 与用户 App 的容器路径和 UID 均不同）。
- 负载是现有 `meshway_protocol::ipc` 的帧编码（IPC 13），由 `meshway-node` 的同一分派处理；ping 等流式命令在同一 XPC 连接上按帧推送，断开即取消订阅。
- macOS 的 `meshway` CLI 随 App 打包（`Meshway.app/Contents/MacOS/meshway`），复用现有 `meshway-cli` 的参数、命令与输出，只替换传输。

`Meshway.app/Contents/MacOS/Meshway --probe` 复用 App 签名验证连接，不是正式 `meshway` CLI。lab probe 支持 `status`／`up`／`down`、`exit PROFILE`／`exit --default` 和 `ping NAME`（请求 1000 次，5 秒后关闭以验取消）。`Shared/FrameProtocol.swift` 只传 `Data`，`FrameSession.dispatchFrame` 交给 Rust `Session::send`；每个请求使用一个内存 duplex，经同一 `Node::ipc().submit` 分派 IPC 13。echo／定时 fixture 已退出当前源码。服务端至多 8 连接，每帧 64 KiB＋4 字节长度前缀、每连接一个未 ACK push；5 秒未 ACK 或断线取消该连接，Rust transport join 完成另记同 peer PID 的日志。VM 已验真实 sample 后 SIGTERM 客户端、反复超过连接预算仍可接纳。停止 Node 时 join 所有会话及运行任务。

桥接导出 `start(config, logs, settings, utun_fd, utun_name)`、显式 `start_control_only`、`NodeHandle::stop/path_changed/open_ipc_session`、`Session::send/close`。日志只在桥接入口安装一次，经 `meshway-logging` 的平台 writer 回调写入 os_log。Swift 绑定与 XCFramework 由 `scripts/build-core.sh` 生成并忽略，不手改生成物。工程、VM 与命令见 [macOS lab](/v2/node/macos-lab)。

App 与 extension 链接同一 XCFramework，但运行在不同进程。App 只调用新增 `client` 模块：`ClientRequest(request_id, command)`、`frame()`、`decode(frame)`；它复用 `meshway_protocol::ipc::write/read`，不启动 Node、不联系 Server、不加载私有身份。解码先验证完整 IPC 13 封闭对象，再投影为 UniFFI record／enum；未知字段、未知枚举、缺失必填可空字段、错误版本／请求绑定／结果类型、重复或跳序 ping 帧均报错。每个请求一个 XPC 连接，ping 逐帧解码、消费后 ACK，取消关闭连接。App 的 `FrameClient` 只传 `Data`，`IPCExchange` 负责有界等待与连接生命周期；正式界面与 `--probe` 共用该客户端。`--probe` 的 stdout 由 Rust 对已验证响应序列化，不由 Swift 解 JSON。

桥接中的可空字段保留协议字段语义：例如 `node`／RTT 为必填可空，`idlePath`／`networkError` 为可省略观察；未报告与显式 `unknown` 枚举不互换。类型投影不补测量、不生成状态时间。客户端超时不撤销已被 Node 接纳的持久命令，须先读取 Status 再决定是否重试。

### 生命周期

| 系统事件 | 运行时行为 |
| --- | --- |
| `startTunnel` | 启动 `meshway-node` 运行时，读持久意图；无身份时等待 App 发起 `Up` |
| App 的 Connect／`meshway up` | 现有 `Up` 意图；浏览器批准时 App 用 `NSWorkspace` 打开 Status 中的 `enrollmentUrl`，不做回调与 PKCE |
| App 的 Disconnect／`meshway down` | 现有 `Down` 意图，随后停止 tunnel |
| `stopTunnel`（系统或用户在设置中关闭） | 与 meshwayd 收到 SIGTERM 相同：保留身份与 `netmap.json` 的有序关闭 |
| `sleep`／`wake` | 视为网络 refresh；不延长租约 |
| 注销 | 现有 `Logout`（`POST /agent-api/v1/leave`） |

### App 界面

SwiftUI 菜单栏面板与主窗口已接入类型化客户端（macOS 26、Swift 6 严格并发）。`AppModel` 只持有界面快照与在途操作，Node 状态每秒从 IPC 读取；读取失败后不继续把旧快照显示为当前运行状态。真实 GUI／XPC 操作仍须在隔离 VM 人工验收。

| 界面 | 行为 |
| --- | --- |
| 状态岛 | 316 pt 菜单面板顶部与主窗口工具栏共用 `StatusIsland`：状态点、单行状态、可复制 Mesh IPv4 与连接开关。错误为琥珀色单行原因；点按打开 popover，完整保留独立的 Control／网络错误，不撑开布局。原始 Node／macOS VPN 状态在 popover 与设置中分别显示 |
| 连接 | 开关读 `desiredUp`，不以操作提交成功推断已连接。Connect 先启动既有 VPN manager，读取 revision 后提交 `Up`。Disconnect 提交 `Down`，等 Status 为 down 后停止 tunnel |
| 接入 | 首次输入 Server URL（extension 启动参数），支持高级 enrollment key；不保存该密钥。`awaitingApproval` 的 `enrollmentUrl` 用浏览器打开一次，并提供重新打开按钮；没有 URL 时显示联系 Server 运维的提示。既有 VPN 的 Server 不提供切网入口 |
| 菜单面板 | Liquid Glass 外壳内使用系统式紧凑行，悬停高亮；出口勾选列表、设备列表、打开 Meshway／设置／退出。设备超过五台折叠，展开后最多十行高并滚动；点设备复制 IPv4，缺失时复制 IPv6。没有地址时禁用并说明原因 |
| 主窗口 | 无侧栏，工具栏左侧状态岛、右侧胶囊分段「设备／出口／诊断」。设备表搜索名称／地址／Node ID，行内为状态点、名称、等宽地址与路径／RTT。选中时详情从右侧进入；固定 210 pt 详情列保持表列位置稳定，未选中时该列为空 |
| Peers | 直连为绿点、中继为靛蓝点、空闲为灰点、离线为淡灰点、不可达为琥珀点；右侧显示 RTT、经哪个节点或空闲。详情为地址、路径与承载、延迟、Ping。没有测量的 RTT 不填零，idle 不显示为已建立直连；完整 endpoint／via 可在行的 help 中读取 |
| 出口方案 | 面板与主窗口共用勾选列表；仅本机 netmap 与 applied 一致时显示勾选。当前方案非默认时，在其下显示「恢复默认（方案名）」小链接；清除仍要求存在持久请求。右侧出口名称来自方案规则／fallback 引用的出口列表，不代表当前某条流的实测出口。`SelectProfile`／`ClearProfile` 的 Server 提交结果只显示等待提示；请求／默认／锁定／auto 详情移到设置 |
| 设置 | 标准 SwiftUI `Settings` scene，⌘, 打开。通用放图标；账户与本机放名称、IPv4／IPv6、Server、换钥与注销确认；详细信息放租约、sync、上次同步、dataPlane、网络变化、key generation、NE／VPN、netmap 与完整错误；诊断放 Netcheck。相对时间每秒更新，悬停给出绝对时间 |
| 诊断与身份 | Ping 卡包含目标输入／建议设备、decision 的路径胶囊与方案／规则／动作、最近 24 个样本的 RTT 柱图与中位／最低最高／收到／丢包。规则缺失时省略；失败槽不填 RTT，含建连 echo 以浅色柱区分且不进入 RTT 汇总。收到／丢包优先用 completed 计数，否则显示已观测终态样本；丢包为超时比例，执行失败不算超时。unsupported、策略拒绝等原因在固定单行区显示。网络检查卡从 Netcheck 报告生成 Server／RTT、UDP、IPv4／IPv6、每 socket 的 NAT 映射、公网映射小片；只用 fresh 映射，缺测显示未测。运行中与结果区尺寸不变。设置复用 Netcheck；换钥／注销先读 revision，不绕过 CAS |
| 空状态 | 共用玻璃小图标、标题、说明与可选动作，固定尺寸。无设备、搜索无结果、只有 Mesh 方案、Server 暂不可达分别解释。只有 sync 非 connected、错误为网络／繁忙或未报告、租约未到期、netmap 已 applied 且 Node running／数据面 ready 时，才说明正在使用缓存网络；到期时间来自租约并每秒更新。不能证明缓存可用时不声称离线运行 |
| 系统引导 | 读取 extension 属性与 VPN preferences，显示未安装／待批准／未启用／缺少配置；沿用 TunnelController 显式激活与创建 VPN。lab 命令与 Debug 菜单独立于正式主路径 |
| 应用图标 | 设置 → 通用 → 应用图标提供 Ink／Frost／Indigo／Graphite 四种缩略图，未接入时也可使用；默认 Frost（方案 2，浅色磨砂＋青绿）。选择提示占用固定空间，动画尊重减少动态效果 |

状态岛只做呈现投影，优先显示已报告的操作／IPC／Node／网络／VPN preferences 错误；否则按以下顺序映射。NE 连接成功本身不等于 Node 已运行。

| 来源 | 呈现 |
| --- | --- |
| 已报告错误，或 Node `blocked`／`retrying` | 出错；popover 保留原始阶段、原因与独立错误 |
| Node `awaitingApproval`，或 extension 明确要求批准 | 需要批准 |
| Node `running` 且 dataPlane `ready` | 已连接 |
| Node `running` 且 dataPlane `preparing` | 连接中，正在应用 |
| Node `running` 且 dataPlane `failed`／`suspended` | 出错，说明数据面失败／暂停 |
| Node `running` 但数据面为其他或未知状态 | 状态未知 |
| Node `starting`／`synchronizing`／`controlReady` | 连接中 |
| Node `down`／`stopping`／`leaving` | 已断开／正在断开／正在注销 |
| 无 Node 快照，NE `connecting`／`reasserting`／`connected` | 连接中；不声称 Node 已运行 |
| 无 Node 快照，NE `disconnecting` | 正在断开 |
| 无 Node 快照，NE `disconnected`／`invalid` | 已断开 |
| Node 与 NE 均未检查或未知 | 状态未知 |

开关状态文字使用 `numericText` 过渡，状态点颜色渐变；分段内容滑移、设备详情右侧进入、列表增删淡入，复制反馈短暂变色／勾号后复原。连接中状态点脉动、出口与设备位置显示 shimmer 骨架，固定面板区域避免就绪后推动底部操作。四段点分别按 starting（正在认证）、synchronizing（正在同步网络）、controlReady／running+preparing（正在应用）、running+ready（完成）投影；现有字段不能可靠区分「正在建立通道」，不从空闲 peer 推断进度。`accessibilityReduceMotion` 下脉动／shimmer 静止，其余自定义过渡取消。连接开关、方案选择与身份操作沿用 Status 就绪条件及在途禁用，禁用原因放在 help；预览禁用时开关仍保留真实 desiredUp 的青绿背景。

App target 的 `Assets.xcassets/AppIcon.appiconset` 提供完整 macOS 16／32／128／256／512 pt 的 1×／2×默认图标；四份处理后母版作为 `AppIconChoice-*` 普通图片资源打包。`IconPreferences` 以 `UserDefaults` 保存本机偏好，启动及选择时通过 `NSApplication.shared.applicationIconImage` 更新运行中的 Dock 图标。Finder／Launchpad 保持默认 Frost，选择器明确说明这一限制；不调用 `NSWorkspace.setIcon`，不修改已签名 App bundle。源图处理与全套资源由 `apps/meshway-apple/scripts/generate-icons.py` 生成：清除圆角外残留、保留比例，图块最大 824×824 居中于 1024×1024 透明画布，不增加外阴影。

`--preview unenrolled|approval|running|error|connecting-auth|connecting-sync|connecting-apply|connecting-plane|no-peers|mesh-only|server-cached|lease-expired --snapshot-dir DIR` 只存在于 Debug 构建，使用 Rust codec 生成的样例 Status，经同一客户端解码后驱动正式视图；主窗口、面板和设置窗口都有小型 SAMPLE 角标，不轮询 XPC、不打开批准页、不创建 VPN 或激活 extension。`--preview-section peers|exit|diagnostics` 选择主窗口页面，`--preview-selected` 展开第一台设备详情，`--preview-settings general|account|details|diagnostics` 选择设置页面；预览中的图标选择仅存内存，不修改 Dock 或真实偏好；`--dark` 切换深色预览。`--model-tests` 检查同一组桥接样例、Node／NE 状态岛映射、缺测与就绪条件。running 样例含六台设备（直连、中继、空闲直连、空闲中继、离线）；`--preview-diagnostics ping|ping-exit|ping-denied|ping-unsupported|netcheck`（逗号分隔可组合）把 Rust 按 Node 流序生成的 Ping 事件帧和 Netcheck 报告逐帧经同一请求解码后填入诊断页。截图仅供人工视觉检查，不能代替真实 Node 操作与包流验收。

`--preview-search QUERY` 对样例执行真实搜索过滤。`server-cached` 只有 sync 失败（`reconnecting`＋`network`，无 `lastError`），已应用的 netmap 仍在租约内；`lease-expired` 无 netmap、phase `retrying`、数据面暂停。样例只填 IPC 已有字段，Release core 不含样例值。Swift 模型测试另以类型化单元输入验证连接阶段、缓存租约边界、空状态选择、含建连 RTT／丢包与 Netcheck 缺测；这些单元输入不作为预览协议数据。

## 复用白名单 {#reuse}

自 `archive/apple-007e9deb` 只取：Xcode 工程与 synchronized folder 结构、`Config/` 的 xcconfig／entitlements／Info.plist 模板（改为 System Extension）、`scripts/build-core.sh` 的 XCFramework 组装流程、桥接包 `unsafe_code = "deny"` 只放开生成 ABI 的审计约定、`MeshwayUI` 中的面板与设备行视图外壳。不取：`core/`（旧 agent 客户端）、`MeshwayModel` 的接入／库存／心跳／多 profile、浏览器 PKCE 回调、`MeshwayIPC`。

## 分阶段计划 {#phases}

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| A1 | 抽出 `crates/meshway-node` 与平台 Host 接口；dataplane carrier 依赖包设备接口；IPC 分派与传输分离 | Linux 行为不变：`mise run check`、现有矩阵回归；workspace 在 macOS 原生编译 |
| A2 | Tart 验收 VM；`apps/meshway-apple` 工程、System Extension 激活、XPC 通道与帧转发、桥接包 start／stop／IPC | 平台激活、root XPC、签名拒绝、薄壳推送／取消与同身份非 GUI 替换已验；A3 补真实 Rust IPC 13 Status／Up／Down 与有序关闭，B1 补真实流式取消，见[验收范围](/v2/node/validation#macos) |
| A3 | Apple Host：utun fd、socket 绑定、NE 路由与负向边界、DNS、`NWPathMonitor` | Mesh 客户端 VM 验收通过：Native TCP／UDP、内层双栈双向 ping／TCP／UDP、系统 Mesh DNS、Server 停机租约内通信、到期 fail closed、自动恢复、stop/start 离线载入。出口使用、漫游与性能验收留后续包 |
| B1 | 源端出口捕获、出口 DNS、漫游与流式 IPC 取消 | VM IPv4 出口、DNS／关联、撤回／到期负向捕获、en0 中断恢复、Rust ping 取消已有证据；仅覆盖[macOS 验收行](/v2/node/validation#macos)明列场景 |
| A4 | App 界面 | 人工验收 |
| A5 | Developer ID 签名、公证、安装与升级 extension | 未关闭 SIP 的干净 VM 上安装、升级、卸载 |

## 待验证 {#unverified}

以下记录本设计的平台依赖与实测边界。Swift 薄壳证据不替代 Rust Node 接线与包流验收；实验环境、命令与管理通道保护见 [macOS lab](/v2/node/macos-lab)。

1. Packet Tunnel System Extension 自身创建的 socket 不进入本隧道，包括 `includeAllNetworks` 打开时。**URLSession IPv4 HTTPS 范围成立**：私网 capture，以及默认 IPv4 路由配 `(includeAllNetworks, excludeLocalNetworks)=(false,true)/(true,true)/(true,false)`，provider 在 NE 设置前后请求 `https://1.1.1.1/cdn-cgi/trace` 均为 HTTP 200。三个默认路由用例中，普通来宾进程同目标请求在 capture 期间均 8 秒超时（HTTP 000），无 VPN 前置对照及独立回滚后均为 200。A3 已验 Mesh-only capture 下 Rust IPv4 外层；B1 另已验 `(false,true)` 默认捕获下 Rust HTTPS Control、Native TCP／UDP 与真实 IPv4 出口回显，不需要 Server／peer excluded route。B1 的接口中断范围单列；不外推到 Rust `includeAllNetworks=true` 或外层 IPv6。
2. 从 NE 进程中找到 utun 描述符并以 Rust 批量读写的可行性与性能，以及 `NEPacketTunnelFlow` 退路的吞吐。**A3 Rust fd 包路径与 stop/start 已验，吞吐未测量**：`getsockopt(SYSPROTO_CONTROL, UTUN_OPT_IFNAME)` 扫描最多 4096 个 fd，桥接复制后交唯一 `Utun`；VM `utun4` 的真实双栈包流、DNS、租约撤回与离线重新打开均通过。TCP 每方向校验 32768 B 不代表吞吐或批处理性能结论；packetFlow 退路尚未实现／验收。
3. 关闭 SIP 与 AMFI 校验的 VM 中，未经 provisioning profile 的 NE entitlement 能否激活 System Extension。**本 VM 开发形态成立（2026-10-18 UTC）**：macOS 26.6.2 (25G83) arm64，SIP disabled、`kern.bootargs=amfi_get_out_of_my_way=1`、developer mode on；真实 Apple Development 签名、无 embedded provisioning profile 的 App／extension 0.1/2，在用户批准 extension 和 VPN 配置后实际 `activated enabled`，tunnel 为 Connected、地址 `10.203.0.2`、接口 `utun4`，provider 以 UID 0 运行。停止／重启、扩大 capture 与最终移除 VPN 配置已执行；不代表 SIP 开启时的 Developer ID 分发或 A5 通过。
4. root System Extension 提供的 XPC Mach service 能被用户 App 与 CLI 连接，并能以代码签名要求认证对端。**Swift 帧通道范围成立**：root provider 的 `CDWF84L4TA.dev.meshway.lab.meshway.ipc` 分别通过 App 控制器路径（`--lab app-frame`，复用调试视图的 send／状态更新）和 `--probe` 取得 echo、ACK 与至少两个独立连续 push。真实开发证书、同 Team、错误 signing identifier 的客户端退出 2，服务端记录 `Dropping check-in message due to code signing requirement`，授权客户端随后成功。正常关闭和客户端 SIGTERM 后均记录对应 peer PID 的 session cancelled，重新启动 tunnel 后两入口仍成功。这里验证的是 shell fixture 和连接级取消；不是 Rust IPC 13 分派／ping 流，也不替代 GUI 人工视觉验收。

## 待定 {#open}

- **MSS clamp**：第一版设置隧道 MTU 并依赖已有 PTB，不安装内核 clamp，`clamp_mss` 明确报告不支持。未来是否按路径进一步下调隧道 MTU，或在 dataplane 中改写 TCP SYN MSS，仍需确定归属与验收。
- 是否在第一版开放本机 listener（macOS 多在 NAT 后，默认只拨出）。
