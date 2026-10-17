# macOS App 设计

> **设计稿，A2 Swift 薄壳已建立，Node 接线与平台验收未完成。**本页记录 macOS App 的已定方向、组件边界、分阶段计划与待验证项。现行 Node 行为的唯一定义仍是[运行状态](/v2/node/runtime)、[数据面](/v2/node/dataplane)与[连接获取](/v2/node/connectivity)；验收状态只以[验收范围](/v2/node/validation)为准。本页不加入文档导航。
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

`apps/meshway-daemon` 现在持有运行协调器、持久意图、状态投影、控制调度、网络 plane／worker、DNS 装配、诊断与 IPC 分派。按“库不依赖 App”的约束，Apple 桥接不能链接它，因此先抽出 `crates/meshway-node`：

- **移入库**：`runtime/` 全部（intent、status、control、network、peers、ping、netcheck、local_endpoints）与 IPC 的类型化分派（当前 `ipc/mod.rs` 的 `Channels`、请求 ID、期望 revision、有界队列、ping 流取消）。分派与传输、对端认证分离，库只接受已认证的命令流。
- **留在 meshwayd**：clap 参数、`meshway-logging` 安装、信号、musl 分配器、Unix socket 服务与 UID 认证、systemd notify、stdout 就绪输出、`/var/lib/meshway` 默认路径。
- **平台 Host 接口**：库定义，Linux 实现包装现有 `meshway_underlay::Host`、`meshway_tun::Tun`、`meshway_dns::SystemDnsOwner` 与出口 driver；Apple 实现位于桥接包，向 Swift 回调 NE 设置。接口面按实际消费者分组：

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
- **underlay 绕行**：依赖 NE 不把 provider 自身流量送回本隧道（[待验证](#unverified)），因此 Apple Host 不需要 Linux 的 route／rule journal。
- **DNS**：`NEDNSSettings.matchDomains` 为 Mesh 区域；当[出口 DNS 判定](/v2/node/dataplane)要求普通查询经出口时加入空匹配域。DNS 服务地址是隧道内一个保留地址，worker 把发往该地址的 UDP／TCP 53 交给现有 `meshway-dns` OwnerDispatcher，不另写解析实现。
- **漫游**：`NWPathMonitor` 的接口／网关变化映射为现有 `NetworkChange` path／refresh 分类，复用[漫游](/v2/node/connectivity#roaming)的通道跟随与重新穿透。

### 状态目录与身份

System Extension 以 root 运行。状态目录为 `/Library/Application Support/Meshway`（root、0700），沿用 Control 与 `meshway-private-storage` 的私有文件与原子发布，不为 Apple 另写 Keychain 身份存储。`netmap.json` 离线载入与租约语义与 Linux 相同。

### App ↔ extension IPC {#ipc}

- 传输为 extension 提供的 XPC Mach service，对端以代码签名要求（同 Team 的 App 与内置 CLI）认证；不用 UID 认证，也不把 Unix socket 放进 App Group 容器（root extension 与用户 App 的容器路径和 UID 均不同）。
- 负载是现有 `meshway_protocol::ipc` 的帧编码（IPC 13），由 `meshway-node` 的同一分派处理；ping 等流式命令在同一 XPC 连接上按帧推送，断开即取消订阅。
- macOS 的 `meshway` CLI 随 App 打包（`Meshway.app/Contents/MacOS/meshway`），复用现有 `meshway-cli` 的参数、命令与输出，只替换传输。

A2 薄壳当前以 `Meshway.app/Contents/MacOS/Meshway --probe` 复用 App 签名验证连接；不是正式 `meshway` CLI。`Shared/FrameProtocol.swift` 只传 `Data`，`FrameSession.dispatchFrame` 是待接 Rust 的唯一入口。当前 echo 和 `shell-fixture: runtime=absent` 连续推送不冒充 IPC 13 Status。服务端限制为 8 连接、每帧 1 MiB、每连接一个未 ACK push；5 秒未 ACK 或断线取消该连接。App 消费帧后再 ACK，避免把推送无限堆入 UI 队列。工程、VM 与命令见 [macOS lab](/v2/node/macos-lab)。

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
| A2 | Tart 验收 VM；`apps/meshway-apple` 工程、System Extension 激活、XPC 通道与帧转发、桥接包 start／stop／IPC | **未完成**：工程 unsigned 编译与开发签名通过；VM SIP／AMFI／developer 设置已实际读回，无 profile extension 已通过系统校验并安装，但等待用户在 Network Extensions 批准；Rust bridge 未接。仍须启动 tunnel、CLI 经 XPC 取得 control-only Status |
| A3 | Apple Host：utun fd、socket 绑定、NE 路由与负向边界、DNS、`NWPathMonitor` | VM 与 Linux lab 节点 Mesh 双向 TCP／UDP、Mesh DNS、租约到期 fail closed、离线载入、出口使用、漫游；新增 `macos/*` 验收行 |
| A4 | App 界面 | 人工验收 |
| A5 | Developer ID 签名、公证、安装与升级 extension | 未关闭 SIP 的干净 VM 上安装、升级、卸载 |

## 待验证 {#unverified}

以下是本设计依赖、但尚未在本项目中实测的平台行为；A2／A3 首先验证，不成立时回到本页修改方案：

1. Packet Tunnel System Extension 自身创建的 socket 不进入本隧道，包括 `includeAllNetworks` 打开时。**未能验证（A2 薄壳）**：已提供私网／默认 IPv4 路由及 `includeAllNetworks` 开关，并在 extension 的 NE 设置前后分别执行有截止时间的 URLSession HTTPS 探测；provider 加载受第 3 项的用户批准阻塞。此外，未启用 VPN 的 VM 中 `curl` 对 `https://1.1.1.1/cdn-cgi/trace`、`https://example.com`、`https://www.apple.com` 均在 10 秒超时（HTTP 000，默认路由仍为 en0→192.168.64.1），必须先取得有效公网对照。即使 URLSession 后续通过，仍须补普通 Rust socket／UDP 证据。
2. 从 NE 进程中找到 utun 描述符并以 Rust 批量读写的可行性与性能，以及 `NEPacketTunnelFlow` 退路的吞吐。**未能验证**：薄壳已加入只读前置观察，NE 设置完成后在最多 4096 个 fd 中筛选 AF_SYSTEM socket，以 `getsockopt(SYSPROTO_CONTROL, UTUN_OPT_IFNAME)` 读取接口名并记录 fd／PID／UID；不复制、接管或读写 fd。因 provider 尚未启动，没有枚举结果；Rust 批处理与性能仍属 A3。
3. 关闭 SIP 与 AMFI 校验的 VM 中，未经 provisioning profile 的 NE entitlement 能否激活 System Extension。**未完成验证，已证明无 profile 注册／安装通过（2026-10-17）**：VM 实际为 macOS 26.6.2 (25G83) arm64，SIP disabled、`kern.bootargs=amfi_get_out_of_my_way=1`、developer mode on。真实 Apple Development 签名、无 embedded provisioning profile 的 App 已在来宾运行；`sysextd` 通过签名和 Network Extension category 校验，生成预期 MachServices 并复制到 `/Library/SystemExtensions`，最终状态为 `activated waiting for user`。`--lab activate` 收到 `requestNeedsUserApproval`，退出 77；需要用户在 **System Settings → General → Login Items & Extensions → Network Extensions** 允许 **Meshway Packet Tunnel**。仍未证明获批后 provider 能运行或 tunnel 能启动；不能把 staging／安装视为 A2 通过。环境与批准步骤见 [macOS lab](/v2/node/macos-lab)。
4. root System Extension 提供的 XPC Mach service 能被用户 App 与 CLI 连接，并能以代码签名要求认证对端。**未能验证（A2 薄壳）**：`NEMachServiceName`、App Group／Team 前缀、listener/client 双向签名 requirement、echo／连续推送与断线取消已编译；签名产物满足 Team OU＋精确 identifier 的静态 requirement 校验，来宾 `sysextd` 已接受对应 Mach service 声明。真实 root provider→用户 App／`--probe` 连接、拒绝不匹配签名及取消证据受第 3 项的用户批准阻塞，不能据声明注册或编译宣称成立。

## 待定 {#open}

- **MSS clamp**：macOS 无对应内核表。候选为按 `SecureNode::clamps` 下调隧道 MTU 并依赖已有的 PTB 合成，或在 dataplane 中对隧道内 TCP SYN 改写 MSS（会成为与 Linux nft 并列的第二份实现，需要先定归属）。
- 是否在第一版开放本机 listener（macOS 多在 NAT 后，默认只拨出）。
