# macOS App 设计

> **实验实现，Apple Host 与 Rust 桥接已接线，VM Node 验收未完成。**本页记录 macOS App 的已定方向、组件边界、分阶段计划与待验证项。现行 Node 行为的唯一定义仍是[运行状态](/v2/node/runtime)、[数据面](/v2/node/dataplane)与[连接获取](/v2/node/connectivity)；验收状态只以[验收范围](/v2/node/validation)为准。本页不加入文档导航。
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

- **路由**：Mesh 网段放入 `includedRoutes`；选择出口方案时加入默认路由，必要时 `includeAllNetworks`。`apply(empty)` 与租约到期时保留 Mesh 网段（及已选出口的默认路由）的捕获并丢弃或回 ICMP admin-prohibited，作为负向边界，不撤 NE 设置让流量落回物理网络。
- **underlay 绕行**：依赖 NE 不把 provider 自身流量送回本隧道，因此方案不引入 Linux 的 route／rule journal。URLSession IPv4 HTTPS 在默认路由／`includeAllNetworks` 下已验证，Rust 普通 socket／UDP 仍须补证据（[平台验证](#unverified)）。
- **DNS**：`NEDNSSettings.matchDomains` 为 Mesh 区域；当[出口 DNS 判定](/v2/node/dataplane)要求普通查询经出口时加入空匹配域。DNS 服务地址是隧道内一个保留地址，worker 把发往该地址的 UDP／TCP 53 交给现有 `meshway-dns` OwnerDispatcher，不另写解析实现。
- **漫游**：`NWPathMonitor` 的接口／网关变化映射为现有 `NetworkChange` path／refresh 分类，复用[漫游](/v2/node/connectivity#roaming)的通道跟随与重新穿透。

#### 当前接线与限制

- `Apple::new(OwnedFd, utun_name, TunnelSettingsSink)` 构造平台附件。`TunnelSettingsSink::apply(Option<TunnelSettings>)` 的完成表示 NE 设置完成；capture 从 `Desired` 的本机地址与完整 Mesh 池计算，使用 `/32`／`/128` 本机地址、默认内层 MTU 1366。历史捕获前缀并集至多 128 条；`suspend` 保留 NE 设置，关闭物理描述符，包平面由同一 Engine 撤回。显式 `down` 才移除设置。
- 私有 `apple-capture.json` 仅记录负向捕获资源，先于 NE 变更原子写入。重新打开 Host 时先恢复捕获，再等待 Engine 安装有效 netmap；该记录不授权任何包。接口地址每秒重枚举，发现与 Mesh 前缀重叠时关闭描述符并报告 overlap，重叠消失后按现有 worker 流程重装。
- NE 设置串行提交，只在系统 completion 成功后确认；5 秒未完成则报错并撤回包输出，原调用完成前拒绝下一次设置变更，不把超时当作已回滚或已安装。
- `meshway_tun::Utun::from_owned_fd` 以 rustix＋Tokio AsyncFd 读写 Darwin 的 4 字节 AF 头，无后台包发送者；每次写只提交一个报文并返回已提交前缀，读批沿用 32 包／64 KiB 上限。桥接只用 `F_DUPFD_CLOEXEC` 复制 NE 的原 fd，成功结果才转入 `OwnedFd`；Swift 不调用 packetFlow 的读写。
- Apple socket 工厂实现完整 `SocketFactory`，TCP／UDP 普通 socket 绑定当前物理接口（`IP_BOUND_IF`／`IPV6_BOUND_IF`）。资源差量沿用同一 `netmap_resource_compatible`；STUN／attempt／learned／channel 授权与 Linux 共用 `meshway_underlay::traversal`，没有 Apple 专用协调权限实现。实际 UDP 截断／缓冲与批量 I/O 复用 `meshway-transport` 的 macOS 后端。
- Control 使用直连、平台信任、无代理／空闲连接池。Host 的 `RequestInterrupt` 在 path 事件时中断有限 HTTP 请求（包括 body 与 sync 长轮询）；refresh 不中断。该钩子不改变 Linux UDS relay。Control 与承载 bootstrap 各有独立的最多 8 个系统名称解析工作；已交给 libc 的解析不能强行中止，取消后不使用结果、保留工作预算，Host shutdown 等待 join。真实 provider 中的 Rust TCP／UDP 绕行、吞吐与漫游尚未验收。
- MSS clamp 返回明确的不支持错误，使用隧道 MTU 与既有 PTB；不宣称安装了 clamp。出口角色不声明能力；外部直接 ICMP 不支持。出口使用仍未启用：出现外部受管范围时只安装负向默认捕获并拒绝 applied，不直出；该保留边界需显式 down 才移除。

#### DNS 地址决定待确认

现行 netmap 没有本机 DNS 虚拟服务地址。`DnsService.address` 是出口节点的 IPv4 Mesh 地址，不能用作本机 NE DNS 地址；Linux 的 `198.18.255.254` 是本机安装参数中的 DNS 锚点，不是已约定的跨平台保留地址。候选为：

1. 采用平台本地安装地址（例如现有锚点默认值），在 NE 中捕获并检查冲突；无需修改配置／wire，但须确认本地保留范围与冲突时行为。
2. 由配置／Server 分配正式 DNS 服务地址；需要约定分配／保留语义并更新契约，避免与节点 Mesh 地址重叠。

决定前不安装 `NEDNSSettings` 或第二份 resolver。存在 DNS 需求时 Host 返回 `apple_dns_service_address_pending`，阻断整体 applied。仅 Mesh IP 验收须按[无 DNS 配置](/v2/node/runtime#安装范围与状态)关闭所有 DNS 需求；之后的隧道 UDP／TCP 53 应交给现有 OwnerDispatcher。

### 状态目录与身份

System Extension 以 root 运行。状态目录为 `/Library/Application Support/Meshway`（root、0700），extension 的 Sandbox entitlement 只为该目录声明 absolute-path read-write exception；实际 VM 沙箱放行尚未验收。沿用 Control 与 `meshway-private-storage` 的私有文件与原子发布，不为 Apple 另写 Keychain 身份存储。`netmap.json` 离线载入与租约语义与 Linux 相同。

### App ↔ extension IPC {#ipc}

- 传输为 extension 提供的 XPC Mach service，对端以代码签名要求（同 Team 的 App 与内置 CLI）认证；不用 UID 认证，也不把 Unix socket 放进 App Group 容器（root extension 与用户 App 的容器路径和 UID 均不同）。
- 负载是现有 `meshway_protocol::ipc` 的帧编码（IPC 13），由 `meshway-node` 的同一分派处理；ping 等流式命令在同一 XPC 连接上按帧推送，断开即取消订阅。
- macOS 的 `meshway` CLI 随 App 打包（`Meshway.app/Contents/MacOS/meshway`），复用现有 `meshway-cli` 的参数、命令与输出，只替换传输。

`Meshway.app/Contents/MacOS/Meshway --probe` 复用 App 签名验证连接，不是正式 `meshway` CLI。`Shared/FrameProtocol.swift` 只传 `Data`，`FrameSession.dispatchFrame` 交给 Rust `Session::send`；每个请求使用一个内存 duplex，经同一 `Node::ipc().submit` 分派 IPC 13。echo／定时 fixture 已退出当前源码。服务端至多 8 连接，每帧 64 KiB＋4 字节长度前缀、每连接一个未 ACK push；5 秒未 ACK 或断线取消该连接。停止 Node 时 join 所有会话及运行任务。

桥接导出 `start(config, logs, settings, utun_fd, utun_name)`、显式 `start_control_only`、`NodeHandle::stop/path_changed/open_ipc_session`、`Session::send/close`。日志只在桥接入口安装一次，经 `meshway-logging` 的平台 writer 回调写入 os_log。Swift 绑定与 XCFramework 由 `scripts/build-core.sh` 生成并忽略，不手改生成物。工程、VM 与命令见 [macOS lab](/v2/node/macos-lab)。

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

菜单栏面板 + 主窗口，数据只来自 IPC Status 与命令：连接开关与 phase、本节点名称／Mesh IP、peers（名称、地址、`idlePath` 与实际通道）、出口方案选择（`SelectProfile`／`ClearProfile`）、netcheck、换钥、注销、接入等待批准页。不展示 IPC 未提供的事实，缺测不填零。

## 复用白名单 {#reuse}

自 `archive/apple-007e9deb` 只取：Xcode 工程与 synchronized folder 结构、`Config/` 的 xcconfig／entitlements／Info.plist 模板（改为 System Extension）、`scripts/build-core.sh` 的 XCFramework 组装流程、桥接包 `unsafe_code = "deny"` 只放开生成 ABI 的审计约定、`MeshwayUI` 中的面板与设备行视图外壳。不取：`core/`（旧 agent 客户端）、`MeshwayModel` 的接入／库存／心跳／多 profile、浏览器 PKCE 回调、`MeshwayIPC`。

## 分阶段计划 {#phases}

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| A1 | 抽出 `crates/meshway-node` 与平台 Host 接口；dataplane carrier 依赖包设备接口；IPC 分派与传输分离 | Linux 行为不变：`mise run check`、现有矩阵回归；workspace 在 macOS 原生编译 |
| A2 | Tart 验收 VM；`apps/meshway-apple` 工程、System Extension 激活、XPC 通道与帧转发、桥接包 start／stop／IPC | **Swift／平台部分已验，VM Rust 分派待验**：VM 无 profile 激活、真实 tunnel、root XPC、签名拒绝、连续推送与断开取消通过；桥接的真实 control-only IPC 单测、Swift 编译／签名已通过，升级 VM 需 GUI 授权 |
| A3 | Apple Host：utun fd、socket 绑定、NE 路由与负向边界、DNS、`NWPathMonitor` | 实现已接线，VM 与 Linux peer Mesh 双向 TCP／UDP、Mesh DNS、租约到期 fail closed、离线载入均待验；DNS 地址待决定。出口使用与漫游完整验收留后续包 |
| A4 | App 界面 | 人工验收 |
| A5 | Developer ID 签名、公证、安装与升级 extension | 未关闭 SIP 的干净 VM 上安装、升级、卸载 |

## 待验证 {#unverified}

以下记录本设计的平台依赖与实测边界。Swift 薄壳证据不替代 Rust Node 接线与包流验收；实验环境、命令与管理通道保护见 [macOS lab](/v2/node/macos-lab)。

1. Packet Tunnel System Extension 自身创建的 socket 不进入本隧道，包括 `includeAllNetworks` 打开时。**本次 URLSession IPv4 HTTPS 范围成立**：私网 capture，以及默认 IPv4 路由配 `(includeAllNetworks, excludeLocalNetworks)=(false,true)/(true,true)/(true,false)`，provider 在 NE 设置前后请求 `https://1.1.1.1/cdn-cgi/trace` 均为 HTTP 200。三个默认路由用例中，普通来宾进程同目标请求在 capture 期间均 8 秒超时（HTTP 000），无 VPN 前置对照及独立回滚后均为 200，证明 capture 生效而 provider 外连可用。此结论不外推到 Rust 普通 TCP／UDP socket、IPv6、漫游或长期连接。
2. 从 NE 进程中找到 utun 描述符并以 Rust 批量读写的可行性与性能，以及 `NEPacketTunnelFlow` 退路的吞吐。**fd 发现前置成立，批量 I/O／性能未验证**：provider PID 915、UID 0 在 NE 设置后以 `getsockopt(SYSPROTO_CONTROL, UTUN_OPT_IFNAME)` 实际取得 `utun4`，首次 fd 5，后续 stop/start 为 fd 7；`lsof` 独立显示 `com.apple.net.utun_control`。扫描最多 4096 个 fd，不依赖 `getsockname` 的地址族过滤，不复制、接管或读写 fd。描述符生命周期／所有权、Rust 批处理与吞吐仍属 A3。
3. 关闭 SIP 与 AMFI 校验的 VM 中，未经 provisioning profile 的 NE entitlement 能否激活 System Extension。**本 VM 开发形态成立（2026-10-18 UTC）**：macOS 26.6.2 (25G83) arm64，SIP disabled、`kern.bootargs=amfi_get_out_of_my_way=1`、developer mode on；真实 Apple Development 签名、无 embedded provisioning profile 的 App／extension 0.1/2，在用户批准 extension 和 VPN 配置后实际 `activated enabled`，tunnel 为 Connected、地址 `10.203.0.2`、接口 `utun4`，provider 以 UID 0 运行。停止／重启、扩大 capture 与最终移除 VPN 配置已执行；不代表 SIP 开启时的 Developer ID 分发或 A5 通过。
4. root System Extension 提供的 XPC Mach service 能被用户 App 与 CLI 连接，并能以代码签名要求认证对端。**Swift 帧通道范围成立**：root provider 的 `CDWF84L4TA.dev.meshway.lab.meshway.ipc` 分别通过 App 控制器路径（`--lab app-frame`，复用调试视图的 send／状态更新）和 `--probe` 取得 echo、ACK 与至少两个独立连续 push。真实开发证书、同 Team、错误 signing identifier 的客户端退出 2，服务端记录 `Dropping check-in message due to code signing requirement`，授权客户端随后成功。正常关闭和客户端 SIGTERM 后均记录对应 peer PID 的 session cancelled，重新启动 tunnel 后两入口仍成功。这里验证的是 shell fixture 和连接级取消；不是 Rust IPC 13 分派／ping 流，也不替代 GUI 人工视觉验收。

## 待定 {#open}

- **MSS clamp**：第一版设置隧道 MTU 并依赖已有 PTB，不安装内核 clamp，`clamp_mss` 明确报告不支持。未来是否按路径进一步下调隧道 MTU，或在 dataplane 中改写 TCP SYN MSS，仍需确定归属与验收。
- 是否在第一版开放本机 listener（macOS 多在 NAT 后，默认只拨出）。
