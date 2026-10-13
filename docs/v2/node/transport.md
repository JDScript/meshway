# 承载会话与发送边界

本页定义承载层的分层、Rust 入口、邻接管理、帧与资源预算，以及发送接纳、完成与取消语义。Mesh circuit、端到端加密和 TUN 见[数据面组件](/v2/node/dataplane)，连接获取见[连接获取](/v2/node/connectivity)，业务入口见 [L3 网络模式](/v2/node/runtime)；动态 ABI 和连接池尚未实现。

## 分层

| 层 | 唯一职责 |
| --- | --- |
| dataplane | IP 准入、Mesh 帧、端到端保护、完整逻辑路径与业务流状态 |
| connectivity | 在获准邻接内选择／复用承载会话，管理拨号、重试、竞速与切换 |
| transport 公共适配层 | Stream／Datagram 接口、公共长度 framing、有界帧收发与完成语义 |
| transport provider | 具体协议的建连、监听、握手、封装和底层 I/O |

UDP／TCP／VLESS 的实现复用公共帧适配；provider 不解析业务 IP、重编译路由或自授 Mesh 邻接资格。协议入口连接成功只证明承载可用，仍须完成 Mesh 身份及邻接验证后才能交付业务。

VLESS／Trojan 的协议实现参考 mihomo／sing-box 的握手、编解码、TLS／WebSocket 组合及错误处理；参考代码使用 `references/README.md` 记录的固定提交。Meshway 的支持范围按[传输配置](/v2/configuration-reference/transports)实施，共同运行接口仍按本页的所有权与生命周期约定定义。

## 当前 Rust 入口

| 包／入口 | 行为 |
| --- | --- |
| `meshway_transport::spec` | 纯声明、材料结构和已选定拨号参数；`PreparedListener` 只保证结构完整，证书验证由 Server 对精确值执行 |
| `meshway_transport::tcp::TcpTransport::adopt` | 接收已连接的 Tokio TCP socket，应用 socket／TCP 参数；复用 `AsyncRead`／`AsyncWrite` |
| `meshway_transport::udp::UdpTransport::adopt` | 接收已绑定、未 connect 的 Tokio UDP socket，保留源／目的地址与真实截断信息；当前支持 Linux／macOS |
| `meshway_transport::udp::UdpTransport::adopt_batch` | Linux 探测 UDP GSO／GRO，使用安全 nix 封装；不支持时按 mmsg、逐报文回退，macOS 保留逐报文路径 |
| `meshway_transport_stream::vless::{Connector,Acceptor}` | 既有流上的 VLESS v0、空 flow、TCP CONNECT，支持 TLS 与 WS/TLS |
| `meshway_transport_stream::trojan::{Connector,Acceptor}` | 既有流上的 Trojan TCP CONNECT／TLS |
| `meshway_transport_stream::tls::{Client,Server}` | 复用 Server 同一材料验证，执行精确叶证书 pin 与完整 WebPKI 验证的 TLS 握手 |
| `meshway-transport::Session` | 将 stream 或 datagram I/O 适配为完整帧收发，提供 sender／receiver 和显式 shutdown／join |

socket 的打开、捕获豁免和候选授权归宿主 underlay／connectivity。适配器使用调用者交付的 socket；字段应用失败会报错。TCP keepalive 在当前平台按整秒校验，不能静默截断亚秒配置。

显式 listener／dialer 与协调 UDP 穿透共用承载接口，provider 的连接需求和双方角色见[连接获取](/v2/node/connectivity)；拨号方向、数据方向与业务方向的关系见[承载与邻接模型](/v2/node/carriers)。

## 邻接管理

`meshway-connectivity::Manager` 持有有界 listener、拨号／重连任务、认证通道和事实快照，通过 `SocketFactory` 使用 underlay 的用途受限 socket。它复用正式 Native UDP、Native TCP、VLESS TCP/TLS／WebSocket/TLS、Trojan TCP/TLS provider；VLESS／Trojan 使用固定专用 Mesh CONNECT 目标，收到任意其他代理目标时拒绝，不提供通用代理。

每条物理 stream 先完成三消息 `MWADJv1\0` 握手，再交给 Mesh 帧处理：

| 消息 | 绑定与证明 |
| --- | --- |
| INIT | 新 client nonce、network、实际 dialer／listener、精确 Carrier、双方控制／数据面 key ID／generation／公钥摘要；发起方签名 |
| Challenge | 同一 context 与 client nonce、新 server nonce；监听方签名 |
| Ack | 同一 context 和双方 nonce；发起方签名确认 |

每个 nonce（含 UDP 健康 Ping nonce）的前 16 字节是签名方本次 Manager 生命周期的随机 boot 标识，后 16 字节为新随机数；消息布局不变。对端从 transcript 读出 boot（拨号方取 server nonce，监听方取 client nonce），记入 `LinkFact.peerBoot`：同 boot 的新 channel 是同一对端状态的承载更换，boot 变化表示对端重启，数据面据此结束相关 circuit（见[数据面](/v2/node/dataplane#候选受保护-circuit)）。Manager 内部重启沿用原 boot。

签名验证使用当前执行发布登记的控制公钥。Carrier 包含实际拨号方、监听方、listener key 和协议；连接方向独立于业务方向。邻接握手只证明当前获准连接，不替代 [Noise 端到端保护与逐跳包证明](/v2/node/dataplane#候选受保护-circuit)。

Native UDP 复用同一 context、codec 和签名实现，并增加监听方签名的 READY 确认。INIT、Challenge、Ack、READY 均有有界重传；确认绑定实际来源地址、当前 endpoint 与双方 nonce。每个绑定 endpoint 只有一个接收 owner 和一个正式发送 owner，多个 peer 共享实际 socket；退役单个 peer 不关闭其他 peer 的通道。未知来源、截断报文和旧握手不能安装或替换活动邻接。

重启／端口重绑定通过新鲜认证替换精确的旧 channel ID，不能等待旧健康窗口结束才开始建连。只有拨号能力的 peer 可用当前通道签名 Ping 触发监听端重新发起 source challenge；旧探测本身不恢复通道或延长许可。完成 Ack／READY 后仍检查捕获的旧通道或空槽代际，迟到确认不能覆盖后来建立的通道；被替换 peer 的旧排队输出立即失效。

UDP 健康 Ping 允许在声明长度之后以零填充（仅 Ping，仅零），用作[路径 MTU 探测](/v2/node/connectivity#path-mtu)；签名体不变，旧版本接收方丢弃填充的 Ping。UDP 通道使用新鲜 nonce 的认证 Ping／Pong 检查存活，默认间隔 2 秒、健康窗口 6 秒；`send_to` 成功不构成可达证明。失效后按获准候选尝试其他承载。业务帧的实际目的地址由已认证通道固定，其后台发送同时检查原包期限、peer 通道和 endpoint 生命周期；外层等待者未继续被轮询也不能产生失效后的迟到输出。

默认预算为 64 连接、16 listener、128 候选；事件的帧／容量双重上限与批次语义见[UDP 控制消息预算](/v2/node/connectivity#udp-控制消息预算)。握手期限 5 秒，重试退避 1 秒。事件与 snapshot 来自真实 listener／link 生命周期，不制造 RTT／丢包数据。停止取消并 join 任务，guard 在阻塞 I/O 中持续检查。

Control 认可的直接 publication／续租交接可保留 Native TCP／UDP 及材料未变化的 VLESS／Trojan TCP/TLS 的实际 socket、Session、channel ID 与认证 transcript。交接要求资源声明及原绑定可保留，并逐 peer 重验当前邻接权限、注册材料与原健康期限；失去资格的 peer 退休。socket ID／generation 随实际描述符生命周期变化，配置引用随获准交接更新；原 STUN 观测不重新计时。

Underlay 按 descriptor 的实际资源／参数绑定保留或不可逆取消 token，Manager 差量停止并 join 变化项、新增资源；移除无关 B 不拆除仍获准的 A–R 通道。共享 UDP descriptor 仍被其他合法通道使用时继续保留。原 opening、discovery、attempt 和排队输出各自保留旧 guard，不能借 descriptor 续接获得新权限。VLESS／Trojan 的 TLS、凭据与 WS 参数参与精确资源比较，内部缓冲写入仍受下文[取消与关闭](#取消与关闭)的冻结输出 scope 约束。source bind、listener 参数、材料或其他不可保留的绑定变化要求对应资源重建；新增必需资源失败仍阻断整体 applied。TLS／WS 材料轮换的整组行为仍未验收，见[验收范围](/v2/node/validation)。

物理 gate 与每次操作的冻结 scope 分离：无当前权限时只在原期限内保留失活资源，不能执行 I/O；只有合法后继可恢复未来操作。旧 packet、health、STUN 或 READY 输出仍按原 scope 失效，进行中的穿透授权不跨 publication 续命。SecureNode 的流／circuit 继续使用独立的[连续性规则](/v2/node/dataplane#候选受保护-circuit)。TCP 建连与重连、Native UDP 静态端点均不构成打洞证明；协调穿透范围与验收见[连接获取](/v2/node/connectivity)。

保留交接仍检查当前 peer／资源预算，休眠拨号任务在新 scope 下重新检查资格。候选 socket 仅复用地址族、本地地址与选项均相容的 listener；缺少兼容资源时显式创建，容量不足拒绝安装。peer 候选查询按有界批次轮转，缺少候选不会永久占住排序前缀。

UDP listener 保留最多 1,024 个历史 INIT nonce。额度耗尽后不遗忘历史；新的来源先完成有界新鲜 challenge／ACK 和通道 CAS，再触发 Manager 整代关闭，由 daemon 重试安装恢复接入。复制 INIT、旧 ACK 或失败的 CAS 不能触发重建。

## 帧与资源

`Frame` 持有 `Payload` 和可选 peer。`Frame::new` 接收 Vec，`Frame::owned` 接收实现 `PayloadStorage` 的 owned 存储；后者直接保有 dataplane PacketBuffer 的原分配与池归属。Stream 的 peer 为固定会话端点；UDP 必须明确目的地，接收帧携带真实来源。payload 是不解释的字节，不代表已经通过 Mesh 身份、IP 或路径检查。帧及完成结果的 Debug 只显示长度／容量等元数据。

`Limits::new` 明确指定帧长、帧数、payload 容量、操作数和 batch 帧数。工程上限为 16 MiB／帧、1 GiB 已接纳容量、65,536 帧／操作；全部为正，至少能容纳一个最大帧，操作数及 batch 帧数不超过帧预算。这些是运行库边界，实际 MTU 和宿主预算另行选择。

发送预算按原始存储的 **capacity** 计费，包括 PacketBuffer 的 headroom／tailroom，覆盖排队及在途操作。发送完成后原始 buffer 交回结果持有者；接收帧和已完成结果的保留由调用者计入自己的预算。Stream 编码组上限为 `max(maxFrameBytes + 4, min(maxBytes + 4 × maxBatchFrames, 256 KiB))`，接收预读最多为该值与 64 KiB 的较小值，解码单个大帧可增长至配置帧长。

## 两类 I/O

| 接口 | 语义 |
| --- | --- |
| Datagram | 保留消息边界，接收结果说明实际长度与截断；允许丢失、重复和乱序 |
| Stream | 有序字节流，允许部分读写；本身不保证 Mesh 帧边界 |

公共适配层交付完整、不解释内容的 transport frame，并保留承载的可靠性、顺序、尺寸限制和关闭信息。TCP／VLESS／Trojan 字节流使用同一套长度 framing；WebSocket binary 消息先适配为连续字节流，Mesh datagram 边界由公共 framing 恢复。截断报文、非法长度和不完整帧不能作为成功接收交给业务。

VLESS／Trojan 的 `Connector::connect` 与 `Acceptor::accept` 接收宿主已打开的流；CONNECT 目的地作为类型化 `Endpoint` 元数据交付，宿主决定是否为自己的 Mesh 服务，不据此自动拨号。协议凭据验证和外层 TLS 均不代替 Mesh Node 认证。

VLESS 客户端先交付 CONNECT 请求，再延迟读取首个响应头，允许上游等到业务请求后才响应。原始连接截止覆盖响应头确认前的读、写与 flush；确认后成为普通字节流。WS 使用 tungstenite 的 binary message 协议，并适配为同一连续字节流：消息可切分／合并，控制帧不成为 payload，关闭应答必须完成底层 flush。

当前 WS 运行边界为 16 KiB 输出分块、32 KiB 写缓冲、1 MiB frame/message、16 KiB Upgrade 输入、每方向 64 个／8 KiB 额外 header 和 2,048-byte path。路径中的 UTF-8 与既有 percent escape 原样匹配；额外 header 值受当前握手库限制，非 ASCII 值在构造时拒绝。未经请求的扩展／子协议会在发送 VLESS 凭据前拒绝。当前日志入口不桥接依赖的 `log` facade；嵌入宿主须避免开启 tungstenite 会输出完整握手／frame 的 payload trace。

当前 stream framing 为大端 u32 长度前缀，使用 Tokio `LengthDelimitedCodec`；拒绝空帧和超过配置上限的长度。UDP 每个报文承载一个完整 payload，保留原有报文边界。Linux 截断结果可提供原长度，macOS 无法可靠获取时返回未知。

### UDP 批量接口

Linux `UdpTransport::adopt_batch` 对未指定（0）的接收／发送 socket buffer 请求 **7 MiB**，吸收 GSO/GRO 突发；已有更大的 buffer 不降低，显式非零值优先。仅使用普通 `SO_RCVBUF`／`SO_SNDBUF`，内核可按自身上限裁剪，返回大小在 Linux 上通常含双倍记账；不修改系统 sysctl，也不请求 `SO_*BUFFORCE`。其它 provider 与非 Linux／逐包 UDP 的 0 仍保留系统值。buffer 不提供新的 authority，原 descriptor、scope／mark 和逐 syscall 检查继续生效。

`Datagram::poll_send_batch` 返回完整发送的前缀数量；`poll_recv_batch` 填充接收缓冲前缀并逐报文返回来源、长度和截断信息。默认实现每次处理一个报文，provider 可实现内核批量。Linux UDP 同目的、等长帧（最后一段可短）通过 `UDP_SEGMENT` 一次 `sendmsg` 发送；聚合 payload 按目的地址限制为 IPv4 65,507 字节、IPv6 65,527 字节，IPv4-mapped IPv6 按 IPv4 处理。`UDP_GRO` cmsg 拆回独立报文。不适合 GSO 时使用 `sendmmsg`；接收使用 `recvmmsg` 并解析每个消息的 cmsg；不支持 mmsg 时回退 `sendmsg`／`recvmsg`。Linux 外层 socket 一律设 DF（`IP_PMTUDISC_DO`／`IPV6_MTU_DISCOVER=DO`），从不在本机分片；超过路由 MTU 的报文返回 `EMSGSIZE`。段长加报头超过路由 MTU 时，内核以 `EINVAL`／`EMSGSIZE` 拒绝 `UDP_SEGMENT` 发送，这与 GSO 能力无关：这一批改为不用 GSO 再发一次，若同样得到 `EMSGSIZE`（段长确实超过路径），记录该目的与段长 60 秒（最多 64 个目的），期间该目的不小于此段长的批次不用 GSO，GSO 保持开启（`gsoSizeRejections` 计数）；若不用 GSO 发送成功，说明 GSO 本身被拒，才关闭 GSO。仅 `UDP_SEGMENT` 发送的 `EIO` 按驱动 Tx checksum 能力不足关闭 GSO；普通发送／接收的 `EIO` 与所有路径的 `EMSGSIZE` 仍报错。`Datagram::path_mtu(peer)` 报告该 endpoint 到 peer 的路由 MTU（含 ICMP 学到的 PMTU），受保护 socket 用同 mark 的瞬时 connected socket 读取，不发包；通道的路径 MTU 搜索见[外层路径 MTU](/v2/node/connectivity#path-mtu)。一次批量 poll 至多一个数据 syscall，能力回退在下一次 poll 执行，因此受保护 socket 每次 syscall 前仍经过 scope／mark 检查。

`capabilities()` 区分成功启用的 GSO／GRO、已由真实调用验证的 mmsg（未探测为 `None`），并记录成功的 GSO／GRO／mmsg 调用次数。批量 UDP provider 的 scratch 上限为 32 × 64 KiB，已接收剩余报文以槽位／区间元数据保留在同一 scratch 中，最多 2,048 个分段，耗尽前不执行下一次接收 syscall；交付时直接复制到调用者缓冲，不再逐段分配／复制 pending payload。逐包 API 同样保留 GRO 中未取走的报文。Session 的批量接收 scratch 按帧长取最多 32 个，容量不超过 `min(maxBytes, 2 MiB)`（允许单个更大的合法帧），使用时分配。交付批次仍不超过 32 帧／64 KiB；超过本批字节上限的已读帧保留到下一次接收，逐包入口也先消费这个前缀。截断或不一致的批量接收元数据返回错误，不交付伪造的完整帧。

Session 的一次 datagram `recv_batch` 可重复消费立即就绪的 provider 前缀，直至 32 帧／64 KiB 或更小的批量配置，不等待凑批。小 scratch 不强制产生额外任务交接；每次 provider poll 仍独立经过原 socket scope 检查，scratch 容量上限不变。已交付前缀之后遇到错误时，先返回有效前缀，再由下次单包或批量接收返回该错误，之后才允许新 I/O。

Mesh 数据帧不统一叠加自动重传／排序层。内层 TCP 和外层 TCP 各自具有其协议语义；握手及控制消息的重试单独定义。当前重放窗口见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)，性能与更大乱序范围仍须验证。

## 本机接纳与完成

发送分为三个不同事实：

1. **本机接纳**：有界发送队列取得指定帧及其 buffer 的所有权。
2. **本机 I/O 完成**：该次 I/O 已结束，相关 buffer 可以由其 owner 回收；结果可以是成功、取消或失败。
3. **对端处理**：对端实际收到、验证或交付的数据，由对应协议／测量另行证明。

本机接纳和本机写入都不代表对端处理。TCP 写入成功也不是对端应用确认。

成功完成须提交完整帧，并处理协议内部必要的 flush；部分写入或仅进入内部队列不能记为完整帧写入成功。若使用内核延迟回收的发送方式，buffer 还须等到实际释放通知后才能归还调用者。

当前 sender 为单一接纳 owner，提交入口和每会话一个后台 worker 共用同一 poll 状态机。`try_send` 返回接纳数量、停止原因和批次完成句柄；被接纳前缀从调用者队列移出，未接纳后缀原样保留。写入 owner 空闲时，提交入口可直接推进一次有界就绪 I/O；未完成操作连同游标与 buffer 交给 worker，不重复编码或发送。接纳前先预留队列容量，完成通知后才释放独占写入权。

一次直接推进最多处理 128 帧、256 KiB 工作量（或一个更大的合法帧），并尝试一次编码组 flush；该次 flush 最多调用 64 次底层 write／flush poll，每次调用前重检取消／截止。Stream 在同一操作／`SendContext` 内组成有界编码组，每组执行完整 flush 后才报告完成。完成结果逐帧区分 `LocalComplete`、`NotStarted`、`Indeterminate`，并交回原始 Vec 分配。

批量提交采用明确的接纳前缀：返回接纳数量，前缀内的帧转移所有权，其余帧仍归调用者。容量不足、尺寸超限、上下文失效或会话关闭时返回对应原因；未接纳部分不能被异步保留。

显式业务批量入口为：

```rust
Sender::submit_batch(&mut VecDeque<ScopedFrame>, SendContext) -> Admission
Receiver::recv_batch() -> Result<Vec<Frame>, ReceiveError>
BatchCompletion::outcome() -> BatchOutcome
```

`ScopedFrame` 保留 `Frame`、原 `SendContext` 与宿主提供的 `ChannelScope`。同一批接纳前缀必须拥有同一 scope 标识及目的端点，scope 不一致处返回 `ChannelScopeMismatch`；scope 标识本身不授予权限。宿主按已认证通道的冻结 scope 创建标识，换代创建新标识，不能仅按 socket／peer 复用。Session 将每帧上下文及调用上下文取交集：保留全部取消／lifetime，截止取最早值。任何成员失效均阻止后续 I/O，完成等待者停止轮询也不能导致迟发。

每批最多 32 帧／64 KiB，并服从更小的会话预算；容量按原存储 capacity、帧数和一个操作继续计费。只提交当前就绪数据，不等待凑批；握手、health、STUN 等控制消息由宿主使用独立逐帧操作，不能加入业务批次。一次批次只有一个 completion；`BatchOutcome::LocalComplete` 要求全部已接纳帧本机完成，全部未开始为 `NotStarted`，部分完成或尝试后的失败为整批 `Uncertain`。原逐帧结果仍供兼容入口使用；批次完成仍不代表对端交付。Stream 保持原长度 framing／flush／半帧失败关闭规则；批量接收只读取就绪完整帧，半帧保留在 codec。已读到但超出本批字节预算的一帧（允许单个更大的合法帧），或有效前缀后的 EOF／错误，留至下一次接收，不能吞掉有效前缀或越过错误继续读取。

daemon 的 `Manager::send_batch` 在调用时捕获已认证 link，按其冻结 `ChannelScope` 分组，调用正式 `carrier::send_batch_shared`。合法 publication 交接即使保留物理 channel ID，也创建新的批量 scope 标识；旧组仍由原 channel token 撤销。一个已接纳前缀共用包含全部原 guard 的 `OutputLifetime` 和 completion，未接纳后缀在原期限内等待容量。Manager 的 stream／UDP 接收 owner 使用 `recv_batch`，物理 scope 与受保护 socket 的 batch poll 都继续在 syscall 前检查。TUN 交付使用同一 guard 交集的 `deliver_batch`；流水线和保序边界见[数据面](/v2/node/dataplane#发送流水线与连续性)。

发送资源同时按帧数、字节数和在途操作数计入有界预算。provider 的协议内部缓冲也须有上限，不能通过另建队列绕过预算。队列满时由上层按业务类别执行有界等待或显式丢弃，不持续分配内存等待 Server 或网络恢复。

## 取消与关闭

| 时点／原因 | 处理边界 |
| --- | --- |
| 尚未接纳 | 调用者仍拥有帧，可以直接取消 |
| 已接纳，等待者退出 | 发送 owner 继续持有帧，直到完成或取消结果；退出等待不立即释放 buffer |
| 已接纳但尚未开始 I/O，业务上下文失效 | 丢弃对应排队帧并完成回收 |
| Stream 已开始写入一帧，上下文失效或 I/O 失败 | 中止该承载通道；不能在半帧后写下一帧并继续复用原流 |
| 帧已经提交到网络，结果不确定 | 如实保留“不确定”结果，不能声明从未发送；接收／转发仍检查自己的许可与期限 |

软取消等待与 down、撤回、代际失效、硬截止分别处理。后者停止新接纳并终止不再允许的 I/O；插件不能为了排空队列而继续使用过期许可。

关闭时先停止接纳，再取消／完成在途工作，最后归还 buffer、句柄和代码引用。完成通知是内存可回收的边界，必须保证之后不再访问对应 buffer。整个顺序由会话 owner 协调，不由 CLI、provider 和数据面分别启动一套关闭流程。

`SendContext` 携带宿主取消信号、单调截止时间和可选 `SendLifetime`，不构成授权凭据。已接纳操作保留完整 lifetime：每次底层 write／flush 前同步检查，阻塞 worker 自行监听失效，不依赖完成等待者继续被轮询。`wait_capacity` 提供有界预算等待，返回不代表预留，调用者仍须检查 `try_send` 结果。

stream 的冻结输出 scope 保留至该组完整 flush；读驱动的 WebSocket flush 和 TLS alert 也受同一 scope 检查。缓冲 provider 在协议栈下方使用 `raw::CheckedOutput`，每次实际输出检查同步 poll 携带的限制；provider 不在脱离 Session 的后台任务中继续输出旧缓冲。

编码组开始 I/O 前失效可清空 staging 并复用 stream；flush 已尝试后的失败／取消将整组标为不确定并关闭 stream，后续尚未处理的组／操作标为未开始。接收采用 pull API；取消一次接收等待保留已读半帧。丢弃任一会话半部会关闭承载并停止 owner，显式 shutdown 还会完成已接纳操作的收尾并 join。捕获的回调 panic 关闭承载并以不含原始 panic 内容的 `ProviderPanicked` 表达；owner 意外丢失通过 `CompletionLost` 表达。

## 规划：连接池

连接池尚未实现；当前每个获准邻接使用单连接，池大小不是配置字段或默认值。规划约束：

- 映射由 connectivity 持有，同一业务流稳定映射到一个承载通道；增加通道只影响新分配，通道失效只重新分配受影响的流，目标仍是原获准邻接与完整路径。
- 替换底层连接时，旧连接的排队／在途帧按发送结果处理，不由 provider 复制到新连接重放；连接切换不重置加密计数器或重放记录。
- 排序、nonce 唯一性和重放检查的作用域分别定义，物理 socket 不直接成为端到端安全上下文的身份；小连接池只缩小而不消除 TCP 队头阻塞。
- publication 是执行目标引用，不充当连接、业务流或加密会话的 ID；诊断续租连续性的现行限制见[实时观察](/v2/state-reference/realtime)。

## 性能测量 {#验证与性能对照}

承载层 raw／datagram 对照（L1）的入口、报告契约与对照轴见[分层性能测量](/v2/node/performance#l1-承载对照)。常规测试只验证编解码、截断、顺序、取消、权限和资源回收等正确性，不设置吞吐或耗时门槛。
