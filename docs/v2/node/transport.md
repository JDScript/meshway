# 承载会话与发送边界

当前 Rust 运行库提供 UDP／TCP／VLESS／Trojan 承载、公共 datagram framing、有界发送与完成回收。`meshway-connectivity::Manager` 已装配到 daemon 的 stream 与 Native UDP 静态端点运行路径；Linux Native TCP、Native UDP 承载上的双栈 UDP／TCP 包流分别验收。Mesh circuit、端到端加密和 TUN 接线见[数据面组件](/v2/node/dataplane)，动态 ABI 和连接池仍待实现。业务入口见 [L3 网络模式](/v2/node/runtime)，插件装载见[插件与能力](/v2/node/plugins)。

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
| `meshway-transport-spec` | 纯声明、材料结构和已选定拨号参数；`PreparedListener` 只保证结构完整，证书验证由 Server 对精确值执行 |
| `meshway-transport-tcp::TcpTransport::adopt` | 接收已连接的 Tokio TCP socket，应用 socket／TCP 参数；复用 `AsyncRead`／`AsyncWrite` |
| `meshway-transport-udp::UdpTransport::adopt` | 接收已绑定、未 connect 的 Tokio UDP socket，保留源／目的地址与真实截断信息；当前支持 Linux／macOS |
| `meshway-transport-vless::{Connector,Acceptor}` | 既有流上的 VLESS v0、空 flow、TCP CONNECT，支持 TLS 与 WS/TLS |
| `meshway-transport-trojan::{Connector,Acceptor}` | 既有流上的 Trojan TCP CONNECT／TLS |
| `meshway-tls::{Client,Server}` | 复用 Server 同一材料验证，执行精确叶证书 pin 与完整 WebPKI 验证的 TLS 握手 |
| `meshway-transport::Session` | 将 stream 或 datagram I/O 适配为完整帧收发，提供 sender／receiver 和显式 shutdown／join |

socket 的打开、捕获豁免和候选授权归宿主 underlay／connectivity。适配器使用调用者交付的 socket；字段应用失败会报错。TCP keepalive 在当前平台按整秒校验，不能静默截断亚秒配置。

显式 listener／dialer 与协调 UDP 穿透共用承载接口，provider 的连接需求和双方角色见[连接获取](/v2/node/connectivity)。

## 邻接管理

`meshway-connectivity::Manager` 持有有界 listener、拨号／重连任务、认证通道和事实快照，通过 `SocketFactory` 使用 underlay 的用途受限 socket。它复用正式 Native UDP、Native TCP、VLESS TCP/TLS／WebSocket/TLS、Trojan TCP/TLS provider；VLESS／Trojan 使用固定专用 Mesh CONNECT 目标，收到任意其他代理目标时拒绝，不提供通用代理。

每条物理 stream 先完成三消息 `MWADJv1\0` 握手，再交给 Mesh 帧处理：

| 消息 | 绑定与证明 |
| --- | --- |
| INIT | 新 client nonce、network、实际 dialer／listener、精确 Carrier、双方控制／数据面 key ID／generation／公钥摘要；发起方签名 |
| Challenge | 同一 context 与 client nonce、新 server nonce；监听方签名 |
| Ack | 同一 context 和双方 nonce；发起方签名确认 |

签名验证使用当前执行发布登记的控制公钥。Carrier 包含实际拨号方、监听方、listener key 和协议；连接方向独立于业务方向。邻接握手只证明当前获准连接，不替代 [Noise 端到端保护与逐跳包证明](/v2/node/dataplane#候选受保护-circuit)。

Native UDP 复用同一 context、codec 和签名实现，并增加监听方签名的 READY 确认。INIT、Challenge、Ack、READY 均有有界重传；确认绑定实际来源地址、当前 endpoint 与双方 nonce。每个绑定 endpoint 只有一个接收 owner 和一个正式发送 owner，多个 peer 共享实际 socket；退役单个 peer 不关闭其他 peer 的通道。未知来源、截断报文和旧握手不能安装或替换活动邻接。

重启／端口重绑定通过新鲜认证替换精确的旧 channel ID，不能等待旧健康窗口结束才开始建连。只有拨号能力的 peer 可用当前通道签名 Ping 触发监听端重新发起 source challenge；旧探测本身不恢复通道或延长许可。完成 Ack／READY 后仍检查捕获的旧通道或空槽代际，迟到确认不能覆盖后来建立的通道；被替换 peer 的旧排队输出立即失效。

UDP 通道使用新鲜 nonce 的认证 Ping／Pong 检查存活，默认间隔 2 秒、健康窗口 6 秒；`send_to` 成功不构成可达证明。失效后按获准候选尝试其他承载。业务帧的实际目的地址由已认证通道固定，其后台发送同时检查原包期限、peer 通道和 endpoint 生命周期；外层等待者未继续被轮询也不能产生失效后的迟到输出。

默认预算为 64 连接、16 listener、128 候选和 64 事件；握手期限 5 秒，重试退避 1 秒。事件与 snapshot 来自真实 listener／link 生命周期，不制造 RTT／丢包数据。停止取消并 join 任务，guard 在阻塞 I/O 中持续检查。

当前 publication／续租更新会重建 Manager 的物理承载；符合连续性条件的 SecureNode 逻辑流／circuit 可保留。TCP 建连与重连、Native UDP 静态端点均不构成打洞证明；STUN、候选交换、协调 UDP 穿透和连接池尚未实现，范围见[连接获取](/v2/node/connectivity)。

## 帧与资源

`Frame` 持有原始 payload `Vec` 和可选 peer。Stream 的 peer 为固定会话端点；UDP 必须明确目的地，接收帧携带真实来源。payload 是不解释的字节，不代表已经通过 Mesh 身份、IP 或路径检查。帧及完成结果的 Debug 只显示长度／容量等元数据。

`Limits::new` 明确指定帧长、帧数、payload 容量、操作数和 batch 帧数。工程上限为 16 MiB／帧、1 GiB 已接纳容量、65,536 帧／操作；全部为正，至少能容纳一个最大帧，操作数及 batch 帧数不超过帧预算。这些是运行库边界，实际 MTU 和宿主预算另行选择。

发送预算按原始 Vec 的 **capacity** 计费，覆盖排队及在途操作。发送完成后原始 buffer 交回结果持有者；接收帧和已完成结果的保留由调用者计入自己的预算。Stream 编码组上限为 `max(maxFrameBytes + 4, min(maxBytes + 4 × maxBatchFrames, 256 KiB))`，接收预读最多为该值与 64 KiB 的较小值，解码单个大帧可增长至配置帧长。

## 两类 I/O

| 接口 | 语义 |
| --- | --- |
| Datagram | 保留消息边界，接收结果说明实际长度与截断；允许丢失、重复和乱序 |
| Stream | 有序字节流，允许部分读写；本身不保证 Mesh 帧边界 |

公共适配层交付完整、不解释内容的 transport frame，并保留承载的可靠性、顺序、尺寸限制和关闭信息。TCP／VLESS／Trojan 字节流使用同一套长度 framing；WebSocket binary 消息先适配为连续字节流，Mesh datagram 边界由公共 framing 恢复。截断报文、非法长度和不完整帧不能作为成功接收交给业务。

VLESS／Trojan 的 `Connector::connect` 与 `Acceptor::accept` 接收宿主已打开的流；CONNECT 目的地作为类型化 `Endpoint` 元数据交付，宿主决定是否为自己的 Mesh 服务，不据此自动拨号。协议凭据验证和外层 TLS 均不代替 Mesh Node 认证。

VLESS 客户端先交付 CONNECT 请求，再延迟读取首个响应头，允许上游等到业务请求后才响应。原始连接截止覆盖响应头确认前的读、写与 flush；确认后成为普通字节流。WS 使用 tungstenite 的 binary message 协议，并适配为同一连续字节流：消息可切分／合并，控制帧不成为 payload，关闭应答必须完成底层 flush。

当前 WS 运行边界为 16 KiB 输出分块、32 KiB 写缓冲、1 MiB frame/message、16 KiB Upgrade 输入、每方向 64 个／8 KiB 额外 header 和 2,048-byte path。路径中的 UTF-8 与既有 percent escape 原样匹配；额外 header 值受当前握手库限制，非 ASCII 值在构造时拒绝。未经请求的扩展／子协议会在发送 VLESS 凭据前拒绝。当前日志入口不桥接依赖的 `log` facade；嵌入宿主须避免开启 tungstenite 会输出完整握手／frame 的 payload trace。

当前 stream framing 为大端 u32 长度前缀，使用 Tokio `LengthDelimitedCodec`；拒绝空帧和超过配置上限的长度。UDP 每个报文承载一个完整 payload，保留原有报文边界。Linux 截断结果可提供原长度，macOS 无法可靠获取时返回未知。UDP 当前仍逐报文执行系统调用，发送 batch 不等于已实现 `sendmmsg`／`recvmmsg`。

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

编码组开始 I/O 前失效可清空 staging 并复用 stream；flush 已尝试后的失败／取消将整组标为不确定并关闭 stream，后续尚未处理的组／操作标为未开始。接收采用 pull API；取消一次接收等待保留已读半帧。丢弃任一会话半部会关闭承载并停止 owner，显式 shutdown 还会完成已接纳操作的收尾并 join。捕获的回调 panic 关闭承载并以不含原始 panic 内容的 `ProviderPanicked` 表达；owner 意外丢失通过 `CompletionLost` 表达。

## 连接复用与切换

每个获准邻接使用单连接作为正确性基线；小连接池作为性能对照，池大小尚未成为配置字段或默认值。同一业务流稳定映射到一个承载通道，避免逐包随机选择；不同连接之间没有全局有序保证。

一条 TCP 连接的丢包会阻塞该连接上的后续字节。小连接池可以缩小影响范围，但不会消除各连接自身的队头阻塞；多路复用字段本身不能提供独立的可靠性。

替换底层连接保持原获准逻辑邻接及完整路径。旧连接的排队／在途帧按发送结果处理，不由 provider 自动复制到新连接重放。业务流、加密会话和重放状态是否可继续，由数据面依据身份、路径、代际和许可判断。

publication 是执行目标引用，不直接充当所有连接、业务流或加密会话的 ID。相容目标的资源复用与实际新目标应用分别验证；诊断续租连续性的现行限制见[实时观察](/v2/state-reference/realtime)。

### 流映射与乱序隔离

以下是尚未实现的连接池约束；映射由 connectivity 持有，数据面提供本机调度所需的稳定流标识。当前 Manager 与逻辑 circuit 的关系见[邻接管理](#邻接管理)。

- 新流选择承载通道后保存映射；增加通道只影响新分配，不能按新的连接数量重新散列全部存量流。
- 通道失效时只重新分配受影响的流，目标仍是原获准邻接。其余流保持映射，避免一次故障引起全池扰动。
- 连接切换不重置加密计数器或重放记录。需要创建新加密上下文时必须完成相应验证，不能把旧帧重新标记为新上下文的数据。
- 不同通道可能具有不同队列和网络延迟。排序、nonce 唯一性和重放检查的作用域分别定义，不能仅凭一个全局递增序号假定所有通道同时推进。

当前每 circuit／方向的重放窗口容量仍须结合峰值包率和允许的乱序时间验证；只扩大窗口会增加内存与状态维护成本。未来按流或逻辑子通道细分也需要有界上下文创建、正确的认证绑定和 nonce 空间隔离。物理 socket 不能直接成为端到端安全上下文的唯一身份。

## 验证与性能对照

性能测量通过专门脚本显式启动并输出机器可读结果及汇总报告，不进入常规单元／集成测试，也不由 `mise run check` 或 `cargo test --all-targets` 启动测量。常规测试验证编解码、截断、顺序、取消、权限和资源回收等正确性，不设置吞吐或耗时性能门槛。

当前入口为 `scripts/perf/transport.py` 和 `tools/meshway-transport-perf`，共用上述正式 provider／session 实现：

```sh
mise run perf:transport --protocol all --payload 64 1200 --batch 1 32 \
  --messages 200000 --warmup 1000 --repeats 3
```

脚本显式构建 release 测量程序，使用独立 `target/transport-perf` 缓存，交替执行 raw／datagram 次序。同仓库测量互斥，每次运行保存独立二进制快照，避免后续构建改变正在比较的程序。结果位于仓库公共目录 `.meshway/perf/transport/<run>/`，包括二进制、`results.json`、`summary.md` 及各次 stdout／stderr；记录源码／二进制 digest、工具链、平台、参数和有效成对结果，失败样本保留而不填零。表格为各模式中位数，比值为有效配对重复的比值中位数。

当前支持 `tcp`、`udp`、`vless-tcp`、`vless-ws`、`trojan` 的本机固定 payload 批量 echo。代理协议的两种模式均保留真实 TLS／协议握手和相同字节流实现。吞吐按发起端实际收回的 payload 计，消息速率是逻辑消息交付计数；RTT 从批次提交到各条 echo 交付，CPU 时间来自完整子进程（含 warmup／初始化）。吞吐区间排除连接建立和 warmup；分配、复制及峰值内存尚无计数器时明确为 null。两种模式使用相同 socket 参数和 stream 缓冲上限；raw 在底层不支持 vectored I/O 时采用同样有界的批量字节缓冲，避免把一批输入误变为逐消息发送。此入口测量公共 transport datagram 适配成本，Mesh 认证、端到端加密、路由、动态 ABI 与 TUN 在报告中明确标为未启用。

每种正式协议及封装都必须提供成对的性能结果，使用同一 transport 实现、连接条件、业务 payload、批量策略与资源预算：

| 协议／封装 | raw 基线 | Meshway datagram 对照 |
| --- | --- | --- |
| Native UDP | 原生 UDP 报文收发 | UDP 承载 Meshway datagram，并在对端恢复 |
| Native TCP | 原生 TCP 字节流收发 | 公共 framing 封装／恢复 Meshway datagram |
| VLESS TCP/TLS | 完整 VLESS＋TLS 协议 I/O | 同一协议栈承载并恢复 Meshway datagram |
| VLESS WebSocket/TLS | 完整 VLESS＋WS＋TLS 协议 I/O | 同一协议栈承载并恢复 Meshway datagram |
| Trojan TCP/TLS | 完整 Trojan＋TLS 协议 I/O | 同一协议栈承载并恢复 Meshway datagram |

raw 保留该协议自身的正常握手、TLS／WS 和协议封装；统计对端实际收到的有效业务 payload，不把额外封装字节计为业务吞吐。每对结果同时比较吞吐／pps、CPU、延迟及分配／复制，测量层级和已启用的处理步骤随报告记录。认证、加密、转发和 TUN 的增量成本另行分层测量。新增协议须补齐同样的 raw／Meshway datagram 对照。其他项目的整机结果作为互通或外部参考，不直接计算为 Meshway 包装损失。

| 对照轴 | 场景 |
| --- | --- |
| 连接数 | 每邻接单连接与有界小连接池；业务流保持稳定映射 |
| 负载 | 小包、常见包长、接近实际 MTU、单流和多流 |
| 网络 | 无人工损伤的基线，以及隔离环境中的延迟、丢包、乱序和断连 |
| 调用／批量 | 同一协议实现的直接调用与实际动态库适配；单帧与有界 batch |
| 生命周期 | 排队取消、半帧失败、目标更新、期限到达和承载切换 |
| 跨流干扰 | 快慢通道并行、长流与短流混合、单通道故障、允许范围内的乱序及重复帧 |

固定工具链、release 构建、协议与密码实现、主机资源、MTU 和缓冲预算。记录有效业务吞吐、pps、CPU、内存、分配／复制、RTT 分位数、队列与丢弃原因，并区分本机接纳与对端实际接收。连接池、batch 和损伤参数属于实验输入，不能隐式改变业务配置或 VLESS-only 限制。

比较连接数量时保持全局排队预算一致，记录各连接固有的额外内存与握手成本。先检查流映射稳定、重复的数据帧不会重复交付业务、未认证的序号不能推进重放窗口，以及约定乱序范围内的合法慢流不会被快流持续排挤；正确性不成立的结果不参与吞吐择优。

微基准、真实 socket 收发与完整 TUN／SOCKS5 路径分别提供证据。性能对照共用领域实现，仅替换被比较的适配／调度方式；不另写一套省略认证、权限或加密的业务数据面冒充正式结果。

MTU／分片、Mesh 帧布局、加密与重放作用域、异步 ABI、连接池和宿主默认预算仍待验证定版。运行脚本的局部 socket 结果不代表这些完整路径已经验收。
