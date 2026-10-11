# 连接获取与 UDP 穿透

Meshway 共用一套承载会话与受保护数据面，将显式端点建连和协调 UDP 穿透作为独立的连接获取实现。连接如何建立、协议如何封装、业务是否获准分别判断。拨号／数据／业务三个方向与两类建连机制的产品模型见[承载与邻接模型](/v2/node/carriers)。

## 分层与所有权

```text
显式端点：listener／dialer ────────┐
                                  ├→ peer 认证／会话选择 → SecureNode → TUN／业务入口
协调穿透：绑定 UDP／发现／互探 ────┘
```

| 归属 | 职责 |
| --- | --- |
| `meshway-control` | 唯一身份、netmap 验证与发布、协调上下文与 Server 请求 owner；租约计时归 `meshway-engine` |
| `meshway-policy` | 业务、完整路径与获准连接范围的共同判定 |
| `meshway-underlay` | 实际 socket、捕获豁免、平台资源与释放 |
| `meshway-connectivity` | 两类连接获取、peer 认证、会话选择／重连与真实路径事实 |
| transport provider | 在要求的底层连接上完成协议握手、封装与 I/O |
| `meshway-dataplane` | 唯一业务流表、端到端保护、逐跳证明与原路径回复 |

crate 职责总表见[跨入口复用](/v2/architecture#跨入口复用)。两类入口不分别维护身份、权限、业务流表或加密状态。候选观测变化不自行修改业务授权；更换物理承载仍须符合当前许可及完整逻辑路径。

## Provider 的连接需求

连接需求描述的是 provider 接收什么底层资源，不代表网络一定可达，也不代表业务发起方必须是拨号方。各承载的建连需求与反向拨号示例见[承载与邻接模型](/v2/node/carriers#按承载类型的建连需求)。

| Provider | 外层资源／连接需求 | 连接获取范围 |
| --- | --- | --- |
| Native TCP | 已建立 TCP stream | 正常拨号或接受远端拨入；可反向拨号 |
| VLESS TCP/TLS、WS/TLS | 已建立 TCP stream，再执行 TLS／HTTP／VLESS 角色 | 同上 |
| Trojan TCP/TLS | 已建立 TCP stream，再执行 TLS／Trojan 角色 | 同上 |
| Native UDP | 已绑定 UDP socket、获准 peer 与 datagram 关联 | 显式端点与协调 UDP 穿透 |
| MASQUE／HTTP/3 | UDP 上的 QUIC／HTTP 连接 | 后续 provider；可复用 UDP 连通性，但仍需协议 client/server 角色 |

当前产品穿透范围限 UDP，不实现 TCP simultaneous-open。现有 TCP 系 provider 的“需要拨号”也包括对端拨入得到连接；接受连接不意味着创建第二种业务身份。

TCP／VLESS／Trojan 会话建立后同样可双向收发。UDP 打洞解决路径可达性，不能据此推导双向业务授权。内层 TCP／UDP 业务独立于外层承载；VLESS-only 等协议限制不会静默启用 UDP 业务承载。

上表是资源与行为边界。当前 Rust 接口见[承载入口](/v2/node/transport#当前-rust-入口)。协调穿透复用现有 Native UDP 声明与 Carrier，不增加 `requiresDialing` 等推测能力字段。MASQUE 尚未接入；穿透的验收状态见[验收范围](/v2/node/validation)。

## STUN 与 Server 协调

所有承载的直接拨号候选由 Server 合并：管理员的额外公告地址（`configured`）优先，本机 listener 上报地址（`local`）次之；同地址保留优先来源。流式承载也可使用 `local`。原始字段、过滤和每 listener 八地址上限见[本机 listener 候选](/v2/state-reference/endpoints#本机-listener-候选)。选定通道的 acquisition 分别为 `publishedEndpoint`／`localEndpoint`；接受端仍为 `acceptedInbound`。

Linux 的 `localEndpoints` 只从实际 listener 的地址族、bind 地址及已启用本机接口枚举。除 Mesh 地址池、loopback、未指定、多播、广播及不可用 link-local／scoped 地址外，还排除 Underlay journal 明确持有的 TUN 与独立 DNS 锚点接口上的全部地址；不按 `mwd*` 名称或 `198.18/15` 等地址前缀猜测归属。外部 link／address／route 事件唤醒现有 State 报告调度，最多每秒一次，20 秒例行报告继续作为兜底。新报告重新枚举，删除已消失的地址；待确认旧报告仍重试原内容，其成功不确认后来发生的接口变化。peer 在后续获准连接获取时使用更新后的候选，候选变化不授予新权限或延长旧 lease。

### 最后已知事实与新鲜度 {#last-known-facts}

Node 拨号候选的顺序为：netmap 中的 configured 端点，然后是 `peerFacts` 中该 listener 的 `localEndpoints`，最后（仅 UDP）为 `stunCandidates`；同地址只保留第一个。

`PeerFacts.expiresAt` 只表示新鲜度，不表示撤回：

| 事实 | 过了 `PeerFacts.expiresAt` 后 | 何时消失 |
| --- | --- | --- |
| `localEndpoints` | 作为最后已知地址继续拨号 | 被同一 peer 的新事实替换、出现在 `removed`、peer 不在当前投影内，或 netmap 结束（`apply(empty)`） |
| `stunCandidates` | 各候选按自己的 `expiresAt` 停用（NAT 绑定确实会失效） | 同上，或候选自身到期 |
| 出口就绪 `exit` | 不再作为 ready 出口的依据 | 同上 |

账本唯一 owner 是 Control 的最后已知事实（按 peer 保存最后一份，见 [Node 控制组件](/v2/node/control#netmap-发布)）；Manager 的 `manager/facts.rs` 只读取 Control 发布的 `KnownPeers` 快照、唤醒事实变化的 peer worker，不另存账本；`plan.rs::fact_endpoints` 是唯一的候选展开。Server 停机时事实不再刷新，但已安装租约内仍可用最后已知地址重建空闲关闭的邻接。Control 把同一份最后已知事实保存为 `peer-facts.json`，节点离线重启后随 netmap 载入，见[私有存储](/v2/node/control#私有存储与传输)。事实只提供尝试地址：错误或过时的地址最多导致连不上；准入仍只由 netmap 与认证握手决定。

STUN 继续使用原有同 socket 观测与双边接受的协调路径，不把 UDP 映射加入 TCP／VLESS／Trojan 目标。每个 Native UDP socket 都[持续观测](#continuous-stun)自己的映射，与是否有 configured 地址或 interest 无关；本机候选或 STUN 候选都不意味着 NAT 穿透已成功。

STUN 使用 RFC 8489 Binding 交换观察 socket 的外部映射；Meshway Server 负责当前主体／网络权限、获准 peer 的候选交换和有期限的尝试协调；Node 使用实际 socket 互发探测并完成认证。

Server 的 STUN 监听与发布端点由 `--stun-listen`／`--stun-server` 分别配置，参数见 [Server 运行参数](/v2/server#运行参数)。两者默认均为空；监听地址与对外端点分开声明，不从 wildcard bind 或 HTTP 地址猜测。

`meshway-stun` 提供有界 RFC 8489 Binding 编解码与 UDP Service。它校验事务、消息／属性长度、XOR-MAPPED-ADDRESS 和可选 FINGERPRINT；客户端还验证实际响应来源。Binding 结果是未认证的映射观测，服务按来源 IP、接收包数、输出字节和有限来源表限流。STUN 不承担管理 HTTP 或 Mesh 业务转发。

- 观测必须来自将用于该连接的 UDP socket，保留本地端口、socket 身份与运行代际。另一临时 socket 的结果不能冒充当前承载映射。
- 候选须绑定网络、peer、外层 socket、来源和观测有效期；它只提供尝试地址，不授予身份、Access 或路径权限。
- HTTPS 请求来源地址不能代替 UDP 映射。STUN 结果也不保证对另一目的地址使用相同映射；观测有效期不等于 NAT 设备保证的映射寿命。
- 外部 STUN 候选属于经过身份认证的 Node 自报事实，Server 不把它视作地址所有权证明。双方当前获准的精确邻接可使用这类候选进行有界探测；目标必须是合法的数字 UDP 单播地址，排除 loopback、未指定、多播、广播、不可用的 link-local／scoped 地址和 Mesh 捕获目标。Node 还检查本机不可作为探测目标的地址。探测按原期限限制包数、字节数和并发，不能扫描相邻端口或由候选授予业务权限。
- 尝试与缓冲有界，过期、撤回、取消和 owner 停止后不再迟到安装。实际包发送仍检查原操作期限。

## Node 协调契约

以下端点使用 Node 凭据，定义来自同一[公开 OpenAPI](/v2/contracts/openapi.json)。普通候选变化／尝试过期通过封闭的 `Unavailable` 结果表达；它们不伪装成凭据失效。

| 操作 | 路由 | 行为 |
| --- | --- | --- |
| `queryUdpTraversal` | `POST /agent-api/v1/connectivity/udp/query` | 按当前 instance／configuration 返回 State epoch、明确的 STUN 端点和仅属于本节点的有效尝试；查询不续期。 |
| `prepareUdpTraversal` | `POST /agent-api/v1/connectivity/udp/attempts` | 使用稳定 request ID、绝对期限、精确 Carrier 与双方候选引用创建不可变 offer。 |
| `acceptUdpTraversal` | `POST /agent-api/v1/connectivity/udp/attempts/{attemptId}/accept` | 检查 offer version／digest；双方各自预留真实接收槽并接受后，才返回 Active。 |
| `cancelUdpTraversal` | `POST /agent-api/v1/connectivity/udp/attempts/{attemptId}/cancel` | 幂等取消未完成的获取过程；不据此撤回已经独立认证的业务通道。 |

`SocketReference` 绑定 node、instance／generation、完整 configuration 引用、实际 socket ID／generation 和 resource。`CandidateReference` 进一步绑定原始地址、来源、观测／到期时间及 bind 信息的规范摘要。摘要不包含外层 report sequence 或接收时间；重复上报／读取不会刷新观测。State 是候选的唯一 owner，peer 视图携带引用并继续隐藏不必要的本机接口／observer 细节。

每次准备、接受及获取授权均检查双方当前身份、实例、精确 UDP 邻接及共同的业务或诊断完整路径许可。诊断 witness 另绑定同一 ObservationId、source instance、service 和完整路径，只用于获准诊断连接，不授予业务包权限；可见 peer 不自动获得非相邻 socket 端点。offer 的 nonce、候选对、粗粒度 endpoint source、digest 和期限固定；寿命最多 10 秒，且不超过原候选、报告、session／instance／execution 与所选许可期限。每节点最多八个活动尝试，每网络最多 256 条记录、进程最多 4,096 条记录；同一无序 peer 对／地址族不并行安装不同活动尝试。

Node 先在实际 UDP owner 预留 STUN 事务，再经同一 Session sender 发送，由唯一 receiver 分流响应。候选的真实 State 上报获确认后，才参与 offer。双方 Active 后从同一 socket 发包，签名握手额外绑定 offer 的 nonce／digest；接入前再次验证原尝试与接收槽，旧尝试不能借新 channel scope 延期。获取成功后，通道由当前执行许可和认证健康检查维持，不依赖过期的 STUN 观测续命。

候选协调由 Manager 按需调度：有实际 UDP endpoint，且存在有需求（interest 或[常驻](/v2/configuration-reference/topology#邻接的建立与常驻)）、任一方向获准 Native UDP 承载却缺少健康选定通道的 peer 时查询，包括原 offer 等待双边接受的阶段。需求方以本机拨号承载 prepare；不按节点 ID 限定由哪一方发起，两方同时发起时 Server 对同一 peer 对／地址族只接纳一个活动尝试，另一方经查询取得该 offer 后接受。没有需求的一方由[穿透信令](#traversal-signalling)获知并参与。健康的 Native UDP／TCP 选定通道抑制重复邻接获取；其他地址族的等待 offer 和旧候选过期不会替换健康通道。失联后的下一次获取重新读取候选和期限。

每个 Manager 的邻接获取查询在**任意半开 60 秒窗口最多 30 次**，发起至少相隔 250 ms。只为 STUN 取元数据的查询仅在有到期 socket 且没有仍有效的 discovery context 时发起，与邻接获取共用同一次 `queryUdpTraversal`，不消耗邻接获取预算。失败、取消、publication／资源交接不重置预算，健康选链不消耗或抑制 STUN 观测。查询、prepare 和候选观测均不延长原授权。

### 穿透信令 {#traversal-signalling}

NAT 后两端都能拨出时，邻接按需建立：可能只有一方有需求（例如中继 R 本身没有业务，只有 A 有经 R 的首包）。Server **只传信令、不转发流量**：

1. 需求方 prepare 成功后，Server 在同一提交之后唤醒另一参与方挂起的 `sync`。
2. 该节点的 `SyncResponse.traversalRequests` 列出所有以它为参与方、由他方准备、它尚未接受且仍有效的尝试：

   | 字段 | 含义 |
   | --- | --- |
   | `attemptId` | 尝试 ID；节点仍须经 `queryUdpTraversal` 取得并验证不可变 offer |
   | `peerId` | 准备该尝试的节点 |
   | `carrier` | 精确 Native UDP 承载（含 listener key） |
   | `expiresAt` | 尝试的固定截止，之后请求作废 |

   至多 32 项，不含候选或任何授权，也不在签名租约内。下一请求的 `traversalRequests` 回显已持有的尝试 ID；挂起只为未回显的新尝试立即回答，已持有的尝试不会让长轮询空转。
3. Control 随 `KnownPeers` 发布请求；connectivity 对其中未到期请求的 `peerId` 登记 interest（与首包 interest 同一机制，仍只限当前投影的 peer），随后按上文查询、预留真实接收槽并接受该 offer。interest 本身不授权：offer、承载与双方投影仍由 Server 和 Node 各自复核。

因此目标在一个 sync hold 内得知请求，不需要本地业务 interest；Server 不持有数据面状态，尝试记录本身仍是有期限的协调状态（至多 10 秒）。双方同时 prepare 时仍由 Server 的“同一 peer 对／地址族只接纳一个活动尝试”决胜，被拒一方经信令或查询接受对方的 offer。Server 停机期间没有信令，只有静态端点与已有通道可用。

### 持续 STUN 观测 {#continuous-stun}

每个 Native UDP socket（listener 与 dial）按**自己的固定节拍**观测映射，不依赖 interest：

| 状态 | 节拍 |
| --- | --- |
| socket 承载健康的 Native UDP 选定通道或有在途尝试 | ~10 秒 |
| 其他 | ~30 秒 |

节拍带按 socket 固定的 ±10% 抖动，从最后一次经验证的响应起算；新 socket 立即观测。一轮未取得经验证响应时 2 秒后重试，连续失败倍增至 30 秒；没有可用 STUN 元数据（Server 未配置 STUN 或查询不可用）时，只为观测而发的元数据查询同样从 2 秒倍增至 30 秒。有效 discovery context 期间每轮观测只是同 socket 的 UDP 往返，不发 HTTP。

观测结果的上报规则：

- 观测寿命 120 秒，独立于授权该 STUN 请求的元数据期限（它是映射事实，不是授权）。
- 同一 observer、同一映射的确认**不触发 State 上报**；剩余寿命不足 60 秒时才以新观测续期并上报，续期时保留上一观测至其自然到期，使依据上一次已确认报告准备的 offer 仍可预留。
- 映射变化（地址或端口不同）立即替换该 observer 的观测并唤醒 State 报告调度。
- Server 把同一 listener 中地址与来源相同的 STUN 候选合并为一个事实（取较晚的期限）：续期不改变 peer facts 内容，只经 `refreshed` 更新新鲜度，不唤醒其他节点。

按实际解析出的 STUN 地址族判断每个 socket 是否可观测；仅有 IPv4 STUN 目标时，空的 IPv6 socket 不持续重试。这个能力判断只保留至原元数据截止，新元数据会重新解析；解析失败仍可重试，不缓存为永久不支持。

已取得且仍有效的 discovery context 不必等待下一次元数据查询才准备候选对；缺失邻接的 prepare 至少相隔 1 秒。已经预留的同一 offer 按至少 500 ms 间隔重试双边接受，最多运行至原 offer 截止；接受成功后停止重试。context／offer 在执行交接、撤回或期限结束后立即失效，旧操作不换绑到新上下文。

选定通道的健康失效与合法交接由同一物理 owner 完成。旧 scope 在 publication 交接时被撤回，不代表物理选链已空缺；邻接获取与 prepare 等待原 owner 续接或退役，STUN 刷新仍检查新上下文与实际 socket 的绑定。该等待不允许旧 scope 继续发送，也不延长通道的健康／执行期限。

### UDP 控制消息预算

UDP 接收 owner 在每个就绪批次内先分流 health、邻接握手、STUN 和穿透控制，再交付业务帧；控制消息不进入业务事件队列。签名验证继续受独立的已认证 peer／全局控制速率预算限制。每批至多 32 帧，并受更小的事件槽配置约束。200 µs dispatch 墙钟预算耗尽时让出调度，保留这个有界批次的未处理后缀；恢复后重新检查原 channel，调度延迟本身不成为丢包理由。

未准入业务事件默认至多 **1,024 帧／8 MiB 原始 payload capacity**，两个上限同时生效；元数据另受帧数限制。增加小包槽数不增加原默认 64 个最大帧的 8 MiB 存储暴露。自定义事件数量仍为 1–1,024，存储上限为 `max(maxFrameBytes, min(maxFrameBytes × min(events, 64), 8 MiB))`。这个队列不是 SecureNode 已准入输出的 credit：出队后才由业务 owner 申请其独立的全生命周期额度。

同一已读批次的业务事件以一个有界队列消息发布，保留 FIFO，每批一次通知；消费者直接移交整段就绪前缀，每轮至多 32 事件，部分消费保留原后缀，帧／字节预约在整组出队或丢弃时归还。发布新业务批次前，若已有至少两批（64 帧）预约积压或槽数不足，先给已唤醒消费者一次调度机会，然后只接纳可用前缀；低负载立即发布，不等待凑批。此调度提示不改变硬上限，槽数／字节不足仍丢弃未接纳事件，不等待消费者恢复。每批结束继续让出调度。控制／诊断帧仍经独立控制队列逐事件发布，不借用业务组的批量接纳规则。

Manager→SecureNode owner 的 circuit 控制／诊断帧和 lifecycle 事件另有 32 个保留槽。帧头只作调度提示，实际认证、路径与许可检查仍由 SecureNode 执行。`recv_batch_with_data(false)` 暂停业务事件消费，继续交付控制事件；换代／停止同时清理两条队列。该分流不会让已停止消费的业务队列无限增长，也不承诺在共享 socket 的任意业务洪泛下完全避免控制丢包。

发送仍由同一实际 socket 的正式 Session owner 执行。Manager 在已有发送上限内保留两帧、两个操作和 4 KiB，业务 `submit_batch` 不能消耗该控制预留；控制逐帧入口继续使用原 scope、截止与完成检查，不增加总资源上限。健康期限仍只由匹配本机新鲜 nonce 的认证 Pong 更新，业务包或远端 Ping 不延长健康窗口；间隔与失效规则见[邻接管理](/v2/node/transport#邻接管理)。

### Manager 批次边界

默认 Session 预算为 34 帧、512 KiB capacity、16 个操作，每批至多 32 帧；两帧控制预留使一个完整业务窗口无需因帧槽拆成多个操作。UDP 的最大帧长再收紧至 65,507 B。字节／操作预算与控制预留独立，所有原 packet guard 和冻结 ChannelScope 仍随已接纳批次保留到最终 I/O 完成。

发送入口同步冻结实际 link，同批只检查一次每条选定 channel；每包原 guard 在正式 `release_batch` 和 syscall 边界检查。接收批次在当前选链表下逐 channel 检查 scope／health，跨调度让出不缓存检查结果；SecureNode 继续独立验证实际 carrier、MAC、路径和准入。合法交接保留物理 socket 时仍创建新批量 scope，旧输出不能借新选择重新绑定。空闲 Session 在提交调用内推进有界发送，只有真实 Pending 才交给原后台 owner，完成仍仅表示本机 I/O。

## 路径选择

普通拨号、反向拨号、协调 UDP 打洞、中继和真正直连分别报告。双方认证成功、真实双向往返与实际承载路径一起构成切换依据；STUN 返回值或已发送探测包不构成成功证明。

打洞失败时仅可使用已允许的其他连接／中继路径。例如逻辑路径要求 A→R→B，A 与 B 的候选即使能直连，也不能绕过 R。新通道和旧通道竞争时，由同一连接 owner 完成选择，业务流保持其原完整路径。

已实际选定的流式通道结束后，拨号 owner 经原重试间隔重新按候选来源优先级获取连接，不继续此前候选列表尚未尝试的后缀。因此已认证的 configured TCP 退路短暂断开后，会先重试该来源，再考虑本机候选；未完成认证或未获选的尝试仍继续当前候选轮次。对端仍健康的更优通道不会因此被替换。

穿透、NAT 与 TCP 退路的验收状态见[验收范围](/v2/node/validation)。

隔离双 NAT fixture 的 R 使用空 dialers，A 与 B 分别向 R 的私网 listener 获取通道；两条边仍要求原始 STUN offer、双边接受及真实安装／包流证据。这样不会因 B 的公网侧 local 候选可达而把普通拨号误当作穿透验收；业务和回复仍沿完整 A→R→B 路径。

## 规划：MASQUE

MASQUE 尚未实现。后续优先评估 CONNECT-UDP 与现有 datagram 会话的衔接，再决定 CONNECT-IP 的必要范围；HTTP/3 DATAGRAM 与 TCP 上的 Capsule 承载分别报告，经代理传输不标记为 P2P 直连。HTTP/3 不承诺与浏览器流量不可区分，代理／CDN 的支持需真实互通验证；QUIC 或代理 TLS 不替代 Mesh 的端到端保护与业务授权。

参考：[RFC 8489](https://www.rfc-editor.org/rfc/rfc8489)、[RFC 9297](https://www.rfc-editor.org/rfc/rfc9297)、[RFC 9298](https://www.rfc-editor.org/rfc/rfc9298)、[RFC 9484](https://www.rfc-editor.org/rfc/rfc9484)。
