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

## listener 绑定与重试 {#listener-bind-retry}

Manager 是 listener 生命周期的唯一 owner。安装内容时先绑定全部新 listener，再启动它们的 worker；某个 listener 的本机地址不可用（`Error::Bind`：端口被其他 socket 占用、地址尚未分配、无权限的特权端口）时，它不阻止其他 listener、拨号 socket 和邻接安装，而由该 key 自己的 owner 在当前内容下重试：

| 原因 | 重试间隔 |
| --- | --- |
| 可自行消失（`EADDRINUSE`、`EADDRNOTAVAIL` 等） | 1 s 起翻倍，上限 60 s |
| 需要运维处理（`EACCES`） | 每 60 s |

重试不会放弃：可自行消失的原因一直按上限 60 秒重试。本机 path 类[漫游](#roaming)变化（地址可能回来了）让每个正在重试的 listener 立即再试一次，倍增从 1 秒重新开始；`attempts` 仍累计。绑定指定地址的 listener 在该地址离开本机时由同一变化关闭并进入本重试（`EADDRNOTAVAIL`）。

绑定成功后 Manager 接管该 socket（Native UDP 有 live instance 时为协调 socket，进入 STUN 观测并重新发布候选），发出 `Changed`。接管时按**当时**的 instance 重新确认：绑定后、接管前才到达第一个 instance 的普通 Native UDP socket 先同步释放、再绑定为协调 socket（不会撞上自己刚占的端口），协调 socket 的 State 引用与 Underlay 授权改为当前 instance；内容变化使该 key 不再存在或声明改变时，重试 owner 随旧资源退役。

同一端口上任何改绑都是“旧 socket 关闭后再 bind”，由 `manager::rebind` 统一保证：内容变化、首个 instance 到达（普通 listener 改为协调 socket）与 Manager 重启都先 join 旧 owner（其 socket 随 owner 同步关闭）；重试 owner 已绑定、尚未被接管的 socket 在该 owner 退役时同步关闭（内容变化在重新安装前、`shutdown` 在返回前），不会留在接管队列里占住端口。重试期间：

- `Snapshot.unavailable` 列出该 listener（地址、原因、是否可自行消失、已失败次数、下次尝试时间）。
- Node State 中该 listener 为 `phase: failed`、无 socket、`problem` 为 `resource_unavailable`（`resource` 为该 listener）。
- 本地 Status 为 `lastError: transport`，`lastErrorDetail` 给出 bind 错误、占用端口的进程与临时端口范围提示，见[运行状态](/v2/node/runtime)；绑定后 daemon 以同一内容再 apply，二者清空。
- WARN 日志 `Listener unavailable; retrying its bind`（`key`、`error`、`recoverable`、`attempts`、`retry_ms`），绑定后 INFO `Listener bound after retry`。

Linux 默认临时端口范围（`net.ipv4.ip_local_port_range`，32768–60999）内的 listener 端口可能在某一刻被任意进程的出站 socket 占用；配置校验对已启用 listener 的此类端口给出警告 `ephemeral_listener_port`，节点在启动邻接时按本机实际范围记 WARN。建议选用范围外的端口（或调整该 sysctl）。绑定失败时 listener 每秒重试，成功后以同一内容重新 apply 并清空 `lastError`，无需重启（`udp/off` 行与[真机证据](/v2/node/validation#u1-startup-self-heal)）。

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
| 出口就绪 `exit` | 最后已知为就绪（IPv4 已安装）的出口仍可选，但排在该决定的所有当前就绪出口之后；最后已知为未就绪的不选。过期不拒绝 netmap 允许的新出口流，出口端仍按自己的 netmap 与角色准入（`KnownPeers::ready_exits`、`SecureNode::set_ready_exits`） | 同上 |
| 通道 `connected` | 不再作为[中继排序](#relay-fallback)的依据 | 同上 |

`connected` 是该 peer 自己 State 报告中已认证且未到期的 `links` 对端（只含其 netmap 投影内、且查看者投影内的节点，升序去重，最多 256 个）；变化即是事实内容变化，唤醒查看者的 `sync`。它只给中继候选排序，不授予邻接、路径或业务权限；省略表示 Server 未报告（旧 Server）。

### 候选节奏 {#candidate-pacing}

所有候选都会尝试，不按地址类别（公网、私网 RFC 1918、CGNAT、IPv6 ULA、STUN 映射）猜测可达性：两个节点可能共享一段私网（同一宿主上的容器与虚拟机），也可能都只能经第三方中继。错误的地址最多导致一次失败的尝试；身份与准入只由握手决定。探测的节奏由退避收敛：

| 获取方式 | 失败的单位 | 失败后暂停 | 重新开始（清零全部暂停） |
| --- | --- | --- | --- |
| 显式拨号（configured、`localEndpoints`） | 一个候选（承载 key＋地址＋端口）的一次尝试 | 第 n 次连续失败后 `1 s × 2^(n−1)`，上限 **60 秒** | 与该 peer 已有选定通道，或上次检查以来有通道建立（含已结束）；该 peer 的候选集合变化（端点事实）；本机任一 socket 观测到的映射变化；新内容中**这条边**变化 |
| 协调 UDP 穿透（STUN 映射） | 本节点准备或接受的一个尝试，结束时没有通道 | 本节点对该 peer **主动发起**的 prepare 及为它发的邻接获取查询暂停 `1 s × 2^(n−1)`，上限 60 秒 | 同显式拨号 |

“这条边”是内容对该 peer 的描述（`plan.rs::Plan::edge`）：对端注册密钥、双向的承载与拨号候选（含双方 listener／dialer 参数与 configured 端点）、该边是否常驻（决定它的 topology 规则的 `keepalive` 或单向可拨）。与它无关的内容变化（其他节点加入或改备注、Access、出口、DNS）不重置暂停；协调暂停同样跨内容与本机资源变化保留。暂停因上述原因重新开始时记一条 debug 事件 `operation=candidate_pacing`（`peer`、`cause`：`edge`／`candidates`／`own_mapping`／`channel`），每次失败记 `operation=candidate_backoff`（`failures`、`retry_ms`）。

协调暂停只约束本节点主动发起：该 peer 发来仍有效的[穿透请求](#traversal-signalling)时，本节点不论自己是否在暂停中、是否仍持有与它的选定通道（见[漫游](#roaming)“接收方的旧半边”），都立即查询并接受该尝试（不因此 prepare，也不清零自己的暂停）。因此一端重启或恢复后由它发起的尝试，另一端当场参与，两端不会因各自的暂停节奏错开。

同一 peer 一旦有选定通道就停止探测它的其他候选。暂停期间其余候选各按自己的节奏尝试；首次暂停为 1 秒（`Limits::retry_backoff`），连续失败 7 次（累计约 1 分钟）后到达每分钟一次，因此对端重启、路径恢复后至多约一分钟内重试。没有失败记录的候选立即可试。暂停只影响尝试节奏，不改变候选顺序、准入或通道的健康判定。协调暂停表在每次内容切换时剔除已不在投影里的 peer，且至多 1,024 个 peer（满时让位给新 peer 的是暂停结束最久的一个，全部仍在暂停中则不记录新的失败）。实现唯一归属 Manager 的候选调度（`manager/workers.rs::Pacing`、`manager/traversal/coordination.rs::Coordination`）。

失败的直连不阻塞业务：经第三节点的备选路径同时可用时，新流立即经中继，见[路径选择](#路径选择)。

账本唯一 owner 是 Control 的最后已知事实（按 peer 保存最后一份，见 [Node 控制组件](/v2/node/control#netmap-发布)）；Manager 的 `manager/facts.rs` 只读取 Control 发布的 `KnownPeers` 快照、唤醒事实变化的 peer worker，不另存账本；`plan.rs::fact_endpoints` 是唯一的候选展开。Server 停机时事实不再刷新，但已安装租约内仍可用最后已知地址重建空闲关闭的邻接。Control 把同一份最后已知事实保存为 `peer-facts.json`，节点离线重启后随 netmap 载入，见[私有存储](/v2/node/control#私有存储与传输)。事实只提供尝试地址：错误或过时的地址最多导致连不上；准入仍只由 netmap 与认证握手决定。

STUN 继续使用原有同 socket 观测与双边接受的协调路径，不把 UDP 映射加入 TCP／VLESS／Trojan 目标。每个 Native UDP socket 都[持续观测](#continuous-stun)自己的映射，与是否有 configured 地址或 interest 无关；本机候选或 STUN 候选都不意味着 NAT 穿透已成功。

STUN 使用 RFC 8489 Binding 交换观察 socket 的外部映射；Meshway Server 负责当前主体／网络权限、获准 peer 的候选交换和有期限的尝试协调；Node 使用实际 socket 互发探测并完成认证。

Server 的 STUN 监听与发布端点由 `--stun-listen`／`--stun-server` 分别配置，参数见 [Server 运行参数](/v2/server#运行参数)。两者默认均为空；监听地址与对外端点分开声明，不从 wildcard bind 或 HTTP 地址猜测。至少两个不同 IP 的观测点才能判断映射是否与目的相关（[netcheck](/v2/node/diagnostics#netcheck) 的 `endpointMapping`）；开发 Server 发布 Google 与 Cloudflare 两个 STUN 端点。

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

Underlay 对每个穿透 socket 按其确切 `SocketReference` 授权尝试目标。instance 换代（Server 重启、instance 租约断续后重新取得）时 Manager 在同一内容上把每个穿透 socket 的引用连同 Underlay 授权一起改绑到新 instance（`Manager::set_instance`），旧 discovery 与尝试目标随之失效，已选定通道保留自己的期限；改绑前的尝试在激活时被拒（`target_not_authorized`），不会因 instance 变化长期无法打洞。

每次准备、接受及获取授权均检查双方当前身份、实例、精确 UDP 邻接及共同的业务或诊断完整路径许可。诊断 witness 另绑定同一 ObservationId、source instance、service 和完整路径，只用于获准诊断连接，不授予业务包权限；可见 peer 不自动获得非相邻 socket 端点。offer 的 nonce、候选对、粗粒度 endpoint source、digest 和期限固定；寿命最多 10 秒（Server 按自己的时钟、自收到 prepare 起计，请求期限更远即 `notAvailable`），且不超过原候选、报告、session／instance／execution 与所选许可期限。节点以 Server 时间表达请求期限：取所用查询回答的 `asOf` 加此后的单调时钟经过时间（不会超前于 Server），而不是本机墙钟，因此本机时钟快于 Server 不会使 prepare 一律不可用。每节点最多八个活动尝试，每网络最多 256 条记录、进程最多 4,096 条记录；同一无序 peer 对／地址族不并行安装不同活动尝试。

Node 先在实际 UDP owner 预留 STUN 事务，再经同一 Session sender 发送，由唯一 receiver 分流响应。候选的真实 State 上报获确认后，才参与 offer。双方 Active 后从同一 socket 发包，签名握手额外绑定 offer 的 nonce／digest；接入前再次验证原尝试与接收槽，旧尝试不能借新 channel scope 延期。获取成功后，通道由当前执行许可和认证健康检查维持，不依赖过期的 STUN 观测续命。

候选协调由 Manager 按需调度：有实际 UDP endpoint，且存在有需求（interest 或[常驻](/v2/configuration-reference/topology#邻接的建立与常驻)）、任一方向获准 Native UDP 承载却缺少健康选定通道的 peer 时查询，包括原 offer 等待双边接受的阶段。需求方以本机拨号承载 prepare；不按节点 ID 限定由哪一方发起，两方同时发起时 Server 对同一 peer 对／地址族只接纳一个活动尝试，另一方经查询取得该 offer 后接受。没有需求的一方由[穿透信令](#traversal-signalling)获知并参与。健康的 Native UDP／TCP 选定通道抑制重复邻接获取；其他地址族的等待 offer 和旧候选过期不会替换健康通道。失联后的下一次获取重新读取候选和期限。

每个 Manager 的邻接获取查询在**任意半开 60 秒窗口最多 30 次**，发起至少相隔 250 ms；尝试没有得到通道的 peer 按[候选节奏](#candidate-pacing)暂停，不再计入“有需求”。只为 STUN 取元数据的查询仅在有到期 socket 且没有仍有效的 discovery context 时发起，与邻接获取共用同一次 `queryUdpTraversal`，不消耗邻接获取预算。失败、取消、publication／资源交接不重置预算，健康选链不消耗或抑制 STUN 观测。查询、prepare 和候选观测均不延长原授权。

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

### 重启后的恢复 {#restart-recovery}

任一端 daemon 重启、Server 重启（instance、State 与事实都随之结束）或 NAT 映射丢失后，需要打洞的边无需人工操作即重新获取：

| 事件 | 恢复依据 |
| --- | --- |
| Server 重启 | 节点取得新 instance，穿透 socket 改绑（见上文 `SocketReference`）；随即以新 instance 上报 State，Server 重建事实（见 [Netmap · 事实重建](/v2/node/netmap-design#facts-rebuild)） |
| 对端 daemon 重启 | 旧通道健康失效后退役；有需求的一端按[候选节奏](#candidate-pacing)重试（≤60 秒），另一端无论自己是否暂停都参与对方发起的尝试 |
| 本端 daemon 重启 | 新进程无暂停记录，按需求立即获取 |

不变量：任意一方重启后，在一个 sync 周期加一个 State 周期（约 35 秒）内双方取得对方当前候选，需要打洞的边在 60 秒内重新直连（有需求时）。隔离验收见 [nat/reconnect 行](/v2/node/validation#reconnect-row)。

### 漫游 {#roaming}

漫游指同一 Meshway 网络内、本节点身份不变时底层网络的变化：换 Wi-Fi／LTE、换 NAT、默认路由、承载地址或公网映射变化。session、instance、netmap、租约与 `Desired` 都不因漫游变化；新来源只是投递信息，改变发送目的前必须由对端签名并通过返回可达（return-routability）挑战。切换到另一张 Meshway 网络不是漫游，见 [CLI 与 IPC](/v2/node/runtime#cli-与-ipc)。

**分类（Underlay）。** Underlay 是唯一的 netlink owner。外部接口的 up 链路、地址，以及除本 owner 自有表（capture、源端捕获、出口）与 local 表之外任一路由表的路由（含 NetworkManager 多表、WARP／WireGuard 的 fwmark 表）的 kernel 通知只作标记；一串通知在最后一条之后静止 250 ms（trailing debounce；通知持续不断时，至迟在这串的第一条之后 1 秒）计算一次指纹并发布 `NetworkChange { epoch, path, class, at }`（`Host::network_changes`）：

| 类 | 判定 | 反应 |
| --- | --- | --- |
| path | 指纹改变：外部 up 接口的 index 与载波（`IFF_LOWER_UP`）；它们的有效地址（global，非 deprecated／tentative／dadfailed；标志按地址族解释：位 0x01 在 IPv4 是 `IFA_F_SECONDARY`，同前缀的第二地址照样有效、计入，在 IPv6 是 `IFA_F_TEMPORARY`，隐私地址每天轮换、不计入）；以本节点 mark 经完整 RPDB 查到的每个参照目的地的路由（oif、网关、prefsrc），参照为控制中继最近连到的 Server 地址（每地址族一个）与本节点当前发送的 remote（每条选定通道的 remote，及没有通道的可拨 peer 的字面 IP 候选，至多 64 个，由 Manager 报告），所有地址族都计入；指纹读不出也算 path | 控制中继关闭当前连接；Manager 漫游反应（下表）；State 报告不按每秒一次合并 |
| refresh | 有外部通知但指纹不变（无关接口、链路地址、IPv6 临时地址、不通往任何参照的路由） | 只重新采样本机端点，按现有调度报告；控制连接保留 |
| 映射 | 本机无通知，[持续 STUN](#continuous-stun) 发现同一 observer 给出另一映射 | 立即上报、清零候选节奏，该 socket 的每条通道按漫游节拍探测 |

控制中继新连到的 Server 地址与 Manager 新报告的 remote 是新的参照：只把它们的路由查入基准，基准中已有参照的值不变（与参照更新同时发生的变化仍被比较），不算变化；不再是参照的目的地从基准移除。两次指纹之间才成为参照的目的地不参与比较。例如 Server 经 IPv6、对端经 IPv4 的节点（`meshway-vm`）只改 IPv4 默认路由的 `src` 或增加第二个 IPv4 地址，都是 path 类变化（[roam/direct 行](/v2/node/validation#roam-row) 的 dual-stack 格）。不观测 Wi-Fi SSID：载波闪断、地址、路由与 STUN 已覆盖它的路径影响。

**本机漫游（Manager，path 类）：**

| 步骤 | 规则 |
| --- | --- |
| STUN | 旧路径上的 STUN 观测立即撤回（下一份 State 不再携带），每个穿透 socket 立即观测（不附 size probe；未应答按 500 ms 重发，事务期限 5 秒） |
| 漫游探测 | 每条 Native UDP 选定通道立即发健康 Ping，之后每 250 ms 一次共 4 次，再回到 2 秒节拍；匹配本机 nonce 的认证 Pong 结束。来源没变的对端照常应答，通道不变 |
| 候选节奏 | 全部 peer 的显式与协调暂停清零（`candidate_pacing` 的 `cause=own_network`），等待中的拨号立即重试 |
| 流式通道 | 本地地址不再是本节点 mark 查到其 remote 的路由首选源时退役，按[原候选来源优先级](#relay-fallback)重拨；未变的保留 |
| 报告与控制 | 路径变化后 1.5 秒内的 State 报告只间隔 100 ms（首个立即发出，随后的新 STUN 映射不等合并），之后恢复每秒至多一次；中继关闭使在途 sync 或 Control 调用失败的立即重试，不增长 1–15 秒退避 |
| PMTU | 跟随到新来源的通道，以及本节点自己的 path 类变化或 STUN 映射变化后的每条 UDP 通道，都立即重新搜索[路径 MTU](#path-mtu)：旧测量当即撤回，完成前 `inner` 未知（路由 MTU 不变而真实路径变小时不沿用旧值） |
| listener | 绑定指定地址、而该地址已不属于本机（以本节点 mark 查该地址的路由不在 local 表）的 listener 关闭并进入 [U1 重试](#listener-bind-retry)；所有正在重试的 listener 立即再试一次，倍增从 1 秒重新开始。通配地址 listener 不动 |

**对端漫游（拨号端与 listener 相同）：**

| 步骤 | 规则 |
| --- | --- |
| 新来源 | 未知来源的 Ping 按本 socket 某条已选定通道的 transcript 验签通过（该对端签名、该通道 ID） |
| 挑战 | 只向该来源发送该通道新的签名 Ping（250 ms 间隔至多 4 次，1.5 秒期限）；每通道同时至多 2 个、每 socket 至多 8 个来源。拨号与穿透 socket 只能向自己的目标发送，Underlay 只在挑战期限内准许向该来源发送（`Datagram::authorize_challenge`，每 socket 同时至多 4 项、需同地址族的合法单播地址）。通道跟随后由该通道的一份许可准许它的当前来源（`Datagram::authorize_source`，与挑战分开计数，每 socket 同时至多 4 项，同一地址只保留一份）；通道再次跟随时撤回上一份，因此任意多次迁移都只占一项，通道结束即全部失效 |
| 跟随 | 该来源回来的、对这个 nonce 的认证 Pong 使同一通道（ID、transcript、对端 boot、acquisition 均不变）的 remote 改为新来源，健康期限续期；旧来源同时丢弃，不保留双来源宽限 |
| 拒绝 | 无挑战的 Pong、其他 nonce 或从其他来源到达的 Pong 都不改变目的；被重放的旧 Ping 至多让本节点向它的来源发出上述有界挑战，未认证报文不能让本节点向任意地址发送业务 |

没有通道能验签的签名 Ping（对端 listener 已重启）仍走[重启后的恢复](#restart-recovery)的新握手。对端事实改变时：本机拨号的 Native UDP 通道的 remote 原是对端已发布（configured／local）候选、而新事实不再发布它并发布了同承载的其他字面地址候选，不等健康窗口，在原通道旁拨号新候选，认证成功即精确替换该通道；经返回可达跟随的映射地址从不是候选，不触发拨号，主机名候选不比较。circuit 只绑定相邻对端 boot，跟随与替换都是同一 boot 的承载更换，CID、nonce 与重放窗口保留（[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)）。

**跟随不成（对端在 NAT 后）：** 对端的 NAT 只放行发往本机旧映射的流的回包（端口相关过滤），漫游探测到不了对端，对端也就无从挑战。此时重新协调穿透，由漫游方发起：

| 步骤 | 规则 |
| --- | --- |
| 结束旧通道 | 本机可以再次获取的通道（acquisition 不是 `acceptedInbound`）在路径变化或其 socket 的映射变化后 1.5 秒内没有对漫游探测的认证 Pong：它的健康窗口当即结束（debug `Connectivity channel transition` 的 `roam`／`unanswered`），不等 6 秒健康窗口。接受得来的通道由对端重拨或跟随，本机只继续探测（每个 Ping 都让对端有机会挑战新来源）。每次路径变化重新计这 1.5 秒 |
| 重新协调 | 漫游方的候选节奏已清零（`own_network`），新 STUN 映射经 State 确认后即 prepare（debug `Traversal prepared`）；接收方对漫游方的暂停只因漫游方的候选变化而清零，对其他 peer 的暂停不变 |
| 接收方的旧半边 | peer 只在没有通道时 prepare，所以它仍有效的[穿透请求](#traversal-signalling)说明接收方手上的选定通道已是对方放弃的一半：接收方不等该通道的健康窗口，立即查询、预留并接受（debug `Traversal accepted beside a selected channel`），认证完成的新通道精确替换它（`Replacement::Channel`）；offer 的承载方向可以与旧通道相反（旧通道由接收方准备） |
| 空窗 | 同 boot 的承载替换，CID 保留；已有流在空窗内等待，空窗即重新穿透的时间。空窗内的新流见下文 |

两端都在端口相关过滤 NAT 后、漫游方换到另一台端口保持型 NAT：首个内层回复 2.6–3.7 秒（main 5.2–6.5 秒，等健康窗口失效），见 [roam/nat 行](/v2/node/validation#roam-nat-row)。端点相关映射一侧漫游后仍不可直连（[来源学习](#source-learning)的限制）。

**漫游中的策略中继：** 已有流不迁移到中继（保持原完整路径与 circuit，等直连恢复）。本机重新获取直连通道期间，下一跳没有通道的已建立 circuit 不接纳新流（排序中其后有带通道的路径时），新流在空窗内经策略中继新建 circuit，并一直留在那条路径上；直连恢复后，新流重新加入直连 circuit（[中继兜底](#relay-fallback)）。没有策略中继时空窗内的新流与已有流一样等待。“新流”指本机流表中没有的 5 元组：应用的新 socket 若复用了流表中仍活着（UDP 默认空闲 60 秒，见[数据面资源](/v2/node/dataplane)）的 5 元组，就是那条已有流，留在原 circuit 上等待；它与该目标其他等待的包共用每目标 32 个包的首包队列，队列满时丢弃。验收见 [roam/relay 行](/v2/node/validation#roam-relay-row)。

**快速往返：** 每次 path 类变化各触发一次完整反应（撤回 STUN、立即观测、漫游探测、候选节奏清零、报告间隔 100 ms）；通知静止前 250 ms 内的多次切换只比较一次指纹，回到原状态即 refresh 类。通道只跟随对该来源挑战 nonce 的、从该来源到达的 Pong，所以最终跟随最后一次切换后的来源，途中旧来源的 Pong 不会把它拉回。每次 path 类变化后的 1.5 秒内 State 报告至多 16 份（100 ms 间隔），实测每次 2 份（撤回、带新映射）。验收见 [roam/flap 行](/v2/node/validation#roam-flap-row)。

**宿主资源：**

| 资源 | 漫游时 |
| --- | --- |
| Mesh TUN、地址、capture | 不变，不重建 |
| 新网络分配的地址落在 Mesh 池内 | 已安装的数据会话与 descriptor 立即结束（fail closed），网络 owner 撤回安装；Status `lastError: underlay`，`lastErrorDetail` 以 `meshPoolOverlap:` 开头并给出接口与地址。地址离开后无需 `up` 即重新安装。宿主内核归属的新一期（进程启动或[重证](/v2/node/runtime#ownership-end)）按当时读到的地址重建并发布 overlap：地址仍在则以 overlap 开始（不是冲突），期间已离开则撤回上一期的 overlap 并重装。外来路由覆盖 Mesh 池仍是永久冲突 |
| 自有 capture 规则或私有表路由被他人删除（网络管理器清理外来规则） | 按 end、bypass、sink、lookup 的顺序重装并回读，1 分钟内至多 3 次；第 4 次或槽位已被外来对象占据为冲突。重装前读不到现状（netlink 失败）时这次重装仍然欠着：1 秒后再试，不计入 3 次，连续 5 次读不到即以 `repair_unavailable` 暂态结束归属；不会在缺对象时仍当作健康。外来对象新增仍为冲突 |
| 出口节点 | 路径变化后出口能力先报 `probing`，1 秒后重新探测前置条件（实测新路由后 1.5–1.7 秒报 `ready`）；源端只选就绪出口。经该出口的已有外部连接：内核 masquerade 在旧出口地址离开时删除其 conntrack，流的下一个包按新出接口地址重新转换——UDP 流以新来源继续（服务端看到新地址），TCP 连接在服务端是另一 4 元组、不再前进（实测无 RST）；新流从新上联建立。出口 owner 把内核为同一原始 tuple 新建的标记条目视为该投影的后继：退役时新旧两个回复 tuple 都进隔离、后继被精确删除。经出口节点的 Mesh 流不受影响。验收见 [roam/exit 行](/v2/node/validation#roam-exit-row) |
| resolv.conf 文件后端 | 不支持漫游：保存的原 nameserver 属于旧网络；漫游节点使用 resolved 后端（[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装)） |
| bootstrap resolver（Server 名称） | 每次解析比对 `/etc/resolv.conf`（及其指向）与 resolved 上游文件 `/run/systemd/resolve/resolv.conf` 的身份与修改时间，变了即重读，新网络的 DHCP／resolved 上游无需重启即用；文件不存在为没有上游，读取出错（I/O 错误）为未知：沿用正在用的上游、不记下这次的文件身份，下次解析再读；显式 `--bootstrap-nameserver` 不变 |
| 指定地址的 listener | 地址消失后关闭并重试绑定，地址回来的 path 类变化时重绑（见上表 listener 行） |
| 显式 `bindInterface`／`localAddress` 的拨号 | 地址消失后不自动改绑 |

**可观测：** Status `lastNetworkChange`（最近一次分类时间）。debug 事件 `Underlay network change classified`（class、epoch）、`Connectivity roam`（`path_changed` 的计数、`peer_candidates_moved`，以及 daemon 对每次通道跟随／替换的 `channel_moved`／`channel_replaced` 记录，含经该 peer 的业务 circuit 的 CID）、`Connectivity channel transition` 的 `roam`／`source_challenged`、`source_moved`、`unanswered`，`Traversal prepared`、`Traversal accepted beside a selected channel`。不新增 acquisition 值，漫游不进 Server State 与 Console。

每次 UDP 通道健康窗口结束都另记 debug `Connectivity channel health ended`：`unanswered`（上次本机 nonce 得到 Pong 以来发出的健康 Ping）、`peer_pings`（同期收到的对端认证 Ping）、`answered_ms`（距上次 Pong）与 `cause`——`no_peer_traffic`（整条路径不通）或 `own_probes_unanswered`（对端的 Ping 仍到达，只是本机的探测或对端的 Pong 丢失）。

实测与验收见 [roam/direct 行](/v2/node/validation#roam-row) 及其后四行。未覆盖：外层 IPv6 承载的漫游（`roam/direct` 只让 A 的控制路径走 IPv6，承载与对端仍为 IPv4）、端点相关映射一侧的漫游；指定地址 listener 与 bootstrap resolver 只有单元／组件测试（`manager::tests::netmap::roam`、`dns::tests`），没有矩阵行；只支持 Linux。设计与后续切片见[漫游设计](/v2/node/roaming-design)。

### 端点相关映射与来源学习 {#source-learning}

对称（端点相关映射）NAT 为每个目的地分配不同的外部端口：STUN 观测到的映射只对 STUN 服务器有效，对端从另一端口到达。双方 Active 后，若收到本 offer 的对端**开场消息**（响应方的签名 punch `k=0`、拨号方的签名 INIT `k=1`），其签名按 offer digest 验证通过，而来源地址与 offer 登记的候选不同（只换端口，或 NAT／VPN 对本节点使用了另一出口地址），则：

- 来源可以是任意合法单播地址（与候选同样排除 loopback、未指定、多播、广播与 Mesh 地址）；每个尝试最多学习 2 个来源，随尝试期限结束。被重放的签名开场消息最多让本尝试向 2 个额外地址发出其原有界输出。
- 学到的来源加入本尝试有界输出的目的（原候选继续发送；每尝试 32 次的总发送上限不变），当前输出的节拍重新开始一次。Underlay 的目标授权（`DatagramAuthorization::authorize_learned`）独立核对同一 IP、同一尝试与上限。
- 后续 Challenge／Ack／READY 可来自学到的来源；安装的通道以 offer 与签名 transcript 为身份依据，以握手实际完成的地址为通道端点（`LinkFact.remote`），acquisition 仍为 `coordinatedUdp`。
- 未通过签名或属于其他 offer 的报文不被学习，也不消耗尝试；源地址只是投递信息，身份只由签名证明。

一方在公网（无 NAT）或其 NAT 的过滤只与地址相关、另一方在端点相关映射 NAT 后时由此直连：前者从对方 punch 的实际来源学习端口，回复经该映射进入 NAT。端点相关映射的一方遇到**端口相关过滤**（address-and-port-dependent，如 Linux conntrack 的 masquerade：只放行与已发出流 5 元组完全一致的回包）的一方，或双方都是端点相关映射时仍不可达：前者的 punch 从未被对方观测到的新端口发出、被对方过滤丢弃，对方也就无从学习（需要端口预测，不实现），此时经第三节点[中继](#relay-fallback)。

本机的映射变化（另一出口地址或端口，例如 VPN 更换公网 IP）由同 socket 的[持续 STUN 观测](#continuous-stun)发现、立即上报，并使全部[候选节奏](#candidate-pacing)重新开始，该 socket 的通道按[漫游](#roaming)节拍探测。

### 外层路径 MTU {#path-mtu}

Linux 的 Mesh 外层 UDP socket 设 `IP_PMTUDISC_DO`／`IPV6_MTU_DISCOVER=DO`：外层报文从不在本机分片，超过路由 MTU 的发送返回 `EMSGSIZE`；只有连 1,500 B 路径也装不下的单个控制报文（出口 DNS 消息）允许分片，见 [UDP 批量接口](/v2/node/transport#udp-批量接口)。很多路径丢弃超大 DF 报文却不回 ICMP（实测 Cloudflare WARP），内核路由因此停留在接口 MTU，所以每条 Native UDP 选定通道自己搜索路径 MTU（RFC 8899 式）：

| 项 | 规则 |
| --- | --- |
| 探测 | 独立的 `Probe` 消息（与健康 Ping 区分、单独限速应答），签名覆盖通道、新鲜 nonce 与**整个报文长度**，以零填充到目标大小，带 DF；接收方按实际收到的长度验签，长度不符（被截短）不应答；对这个 nonce 的认证 Pong 证明该大小可达。UDP payload 最多 2,048 B（报文编解码上限），所以搜索上限是 `min(路由 MTU, 2,048 + 28／48)`：路由 MTU 更大（巨型帧）时报告该上限，任何内层 MTU 都在其内。探测无法构造时只结束这一次测量。本版本之前的对端不认识 `Probe`，结果为未知，不影响健康检查 |
| 顺序 | 先试路由 MTU（通过即结束）；否则在 1,280 与失败大小之间二分；每个大小最多 2 次、每次等待 2 秒 |
| 结果 | `path` = 已确认的最大大小与当前路由 MTU（受保护 socket 同 mark、同本地地址的瞬时 connected socket 读 `IP_MTU`，不发包，上限同上）的较小者；`inner = path − 28`（IPv6 为 48）`− 86`（Data 帧固定开销）。搜索完成前或连 1,280 都未确认时为未知 |
| 更新 | 每 10 分钟重新搜索：先重验已确认的大小（期间旧值仍作为暂存事实报告），通过才向上搜索；连续 2 次丢失即放弃旧值、在 1,280 与它之间重新二分，连 1,280 都不通过时撤回为未知。路由学到更小的 PMTU 时立即取较小值 |

`path`／`inner` 是 channel 事实（`Snapshot.mtu`、`Manager::path_mtu`、IPC `peers[].mtu`），连接组件自己不据此改变发送。数据面据此对本节点超过 `inner` 的包回 fragmentation needed／packet too big，Underlay 据此限制 TCP MSS，见[通道 MTU](/v2/node/dataplane#channel-mtu)。探测丢失会让结果偏小（保守方向）。

**限制：中途剥离 DF 的路径。** 中途 NAT 剥离 DF 并在更窄的链路上分片、且远端在报文到达对端节点前重组时，认证探测无法发现分片：对端收到的是完整、长度正确的 Probe，结果偏大（最大为路由 MTU）。症状是该方向外层报文大量乱序（重组顺序），内层 TCP 窗口被压低、单流吞吐远低于同路径其他 TCP；ping 与健康检查不受影响。节点不做"DF 被剥离"的启发式判定；处置是在宿主侧把通往该路径的路由 MTU 限到窄链路的大小，通道即按[结果](#path-mtu)规则立即取较小值，数据面与 TCP MSS 随之收窄。实例：OrbStack machine 的出网 NAT 不保留 DF，Mac 经 Cloudflare WARP（MASQUE，utun MTU 1,300）出网，见[常驻 systemd 验收 VM](/v2/install#orbstack-warp)。

真机上搜索结果与实际路径一致（us-lax-it7 → WARP 后的 meshway-vm 为 `1444/1330`，外层 1,444 B 通过、1,445 B 不通）；不保留 DF 的 NAT 会使结果偏大，处置见 [OrbStack 与 WARP](/v2/install#orbstack-warp)，证据见[真机证据](/v2/node/validation#real-nat)。

### 持续 STUN 观测 {#continuous-stun}

每个 Native UDP socket（listener 与 dial）按**自己的固定节拍**观测映射，不依赖 interest：

| 状态 | 节拍 |
| --- | --- |
| socket 承载健康的 Native UDP 选定通道或有在途尝试 | ~10 秒 |
| 其他 | ~30 秒 |

节拍带按 socket 固定的 ±10% 抖动，从最后一次经验证的响应起算；新 socket 与本机路径变化（[漫游](#roaming)）后的每个 socket 立即观测。一轮未取得经验证响应时 2 秒后重试，连续失败倍增至 30 秒；没有可用 STUN 元数据（Server 未配置 STUN 或查询不可用）时，只为观测而发的元数据查询同样从 2 秒倍增至 30 秒。有效 discovery context 期间每轮观测只是同 socket 的 UDP 往返，不发 HTTP。

观测结果的上报规则：

- 观测寿命 120 秒，独立于授权该 STUN 请求的元数据期限（它是映射事实，不是授权）。
- 同一 observer、同一映射的确认**不触发 State 上报**；剩余寿命不足 60 秒时才以新观测续期并上报，续期时保留上一观测至其自然到期，使依据上一次已确认报告准备的 offer 仍可预留。
- 映射变化（地址或端口不同）立即替换该 observer 的观测并唤醒 State 报告调度。
- Server 把同一 listener 中地址与来源相同的 STUN 候选合并为一个事实（取较晚的期限），并按（地址, observer）排序：续期不改变 peer facts 内容，只经 `refreshed` 更新新鲜度，不唤醒其他节点。两个 observer 的续期相隔数秒时（例如 observer 各见不同映射端口），上一观测先后到期使报告中的候选顺序暂时互换，这也不是内容变化。

按实际解析出的 STUN 地址族判断每个 socket 是否可观测；仅有 IPv4 STUN 目标时，空的 IPv6 socket 不持续重试。这个能力判断只保留至原元数据截止，新元数据会重新解析；解析失败仍可重试，不缓存为永久不支持。

已取得且仍有效的 discovery context 不必等待下一次元数据查询才准备候选对；缺失邻接的 prepare 至少相隔 1 秒。已经预留的同一 offer 按至少 500 ms 间隔重试双边接受，最多运行至原 offer 截止；接受成功后停止重试。context／offer 在执行交接、撤回或期限结束后立即失效，旧操作不换绑到新上下文。

选定通道的健康失效与合法交接由同一物理 owner 完成。旧 scope 在 publication 交接时被撤回，不代表物理选链已空缺；邻接获取与 prepare 等待原 owner 续接或退役，STUN 刷新仍检查新上下文与实际 socket 的绑定。该等待不允许旧 scope 继续发送，也不延长通道的健康／执行期限。

### UDP 控制消息预算

UDP 接收 owner 在每个就绪批次内先分流 health、邻接握手、STUN 和穿透控制，再交付业务帧；控制消息不进入业务事件队列。签名验证继续受独立的已认证 peer／全局控制速率预算限制。每批至多 32 帧，并受更小的事件槽配置约束。200 µs dispatch 墙钟预算耗尽时让出调度，保留这个有界批次的未处理后缀；恢复后重新检查原 channel，调度延迟本身不成为丢包理由。

未准入业务事件默认至多 **1,024 帧／8 MiB 原始 payload capacity**，两个上限同时生效；元数据另受帧数限制。增加小包槽数不增加原默认 64 个最大帧的 8 MiB 存储暴露。自定义事件数量仍为 1–1,024，存储上限为 `max(maxFrameBytes, min(maxFrameBytes × min(events, 64), 8 MiB))`。这个队列不是 SecureNode 已准入输出的 credit：出队后才由业务 owner 申请其独立的全生命周期额度。

同一已读批次的业务事件以一个有界队列消息发布，保留 FIFO，每批一次通知；消费者直接移交整段就绪前缀，每轮至多 32 事件，部分消费保留原后缀，帧／字节预约在整组出队或丢弃时归还。发布新业务批次前，若已有至少两批（64 帧）预约积压或槽数不足，先给已唤醒消费者一次调度机会，然后只接纳可用前缀；低负载立即发布，不等待凑批。此调度提示不改变硬上限。槽数／字节不足时，未接纳的业务事件（至多这一已读批次）按原顺序留在接收 owner 内，owner 在此期间**不再读取该 socket**：后续数据报留在内核接收缓冲（默认读回 8 MiB，见[接收缓冲](/v2/node/performance#接收缓冲)），溢出由内核计数（`Udp RcvbufErrors`、`ss` 的 `d`）。恢复读取只由消费者出队腾出的槽位驱动：owner 等待并持有恰好这一组的帧与字节额度，不轮询；其余分支（内容变化、health 节拍、穿透命令）照常运行。每批结束继续让出调度。

同一 socket 上排在这些数据报之后的 health、邻接握手、尺寸探测与 STUN 控制数据报一起等待，最坏为内核缓冲中排在前面的数据报数 × 消费者每包处理时间：8 MiB 读回值约容纳 3,600 个满长数据报（按每个约 2.3 KiB 内核记账估计），Neoverse N1 上每包约 23 µs 即约 85 ms，Ryzen 9950X 上约 9 µs 即约 33 ms（每包成本见[公网 ARM 实测](/v2/node/performance#field-arm64)）。消费者 **250 ms**（`RECEIVE_STALL`，等于握手重发间隔）内没有腾出这一组的槽位时，owner 恢复读取并丢弃无槽位的业务事件（profile `manager.udp.drop event_full`），直到队列再次接纳；该停滞结束时记一条 warn `Connectivity event queue made no room; business frames dropped while it was stalled`（`dropped`、`stalled_ms`，受诊断事件预算限制）。因此控制数据报至多等待 250 ms 加上按读取速度排空内核缓冲的时间，远小于 health 窗口 6 秒、Ping 间隔 2 秒、握手 8 次 × 250 ms 重发与漫游应答 1.5 秒；控制数据报不单独保留业务队列余量（它们本来不进入该队列）。实现唯一归属 Manager 的 datagram 接收 owner（`manager/datagram/receive.rs::Backlog`）。控制／诊断帧仍经独立控制队列逐事件发布，不借用业务组的批量接纳规则。

Manager→SecureNode owner 的 circuit 控制／诊断帧和 lifecycle 事件另有 32 个保留槽。帧头只作调度提示，实际认证、路径与许可检查仍由 SecureNode 执行。`recv_batch_with_data(false)` 暂停业务事件消费，继续交付控制事件；换代／停止同时清理两条队列。该分流不会让已停止消费的业务队列无限增长，也不承诺在共享 socket 的任意业务洪泛下完全避免控制丢包。

发送仍由同一实际 socket 的正式 Session owner 执行。Manager 在已有发送上限内保留两帧、两个操作和 4 KiB，业务 `submit_batch` 不能消耗该控制预留；控制逐帧入口继续使用原 scope、截止与完成检查，不增加总资源上限。健康期限仍只由匹配本机新鲜 nonce 的认证 Pong 更新，业务包或远端 Ping 不延长健康窗口；间隔与失效规则见[邻接管理](/v2/node/transport#邻接管理)。

### Manager 批次边界

默认 Session 预算为 34 帧、512 KiB capacity、16 个操作，每批至多 32 帧；两帧控制预留使一个完整业务窗口无需因帧槽拆成多个操作。UDP 的最大帧长再收紧至 65,507 B。字节／操作预算与控制预留独立，所有原 packet guard 和冻结 ChannelScope 仍随已接纳批次保留到最终 I/O 完成。

发送入口同步冻结实际 link，同批只检查一次每条选定 channel；每包原 guard 在正式 `release_batch` 和 syscall 边界检查。接收批次在当前选链表下逐 channel 检查 scope／health，跨调度让出不缓存检查结果；SecureNode 继续独立验证实际 carrier、MAC、路径和准入。合法交接保留物理 socket 时仍创建新批量 scope，旧输出不能借新选择重新绑定。空闲 Session 在提交调用内推进有界发送，只有真实 Pending 才交给原后台 owner，完成仍仅表示本机 I/O。

### 通道稳定性 {#channel-stability}

健康的选定通道不因观测或新鲜度刷新而重建：持续 STUN 观测、peer 事实或候选过期、同一 link 的 netmap 重签与续租、业务帧和远端 Ping 都不替换它，也不改变其源端口。只有下列情况退役或替换通道：

| 原因 | 结果 |
| --- | --- |
| 承载失败：UDP 健康窗口内没有匹配本机新鲜 nonce 的认证 Pong，或流式连接结束 | 退役；有需求时按获准候选重新获取 |
| 对端代际变化：transcript 中的 peer boot 改变（对端重启） | 新通道替换旧通道，数据面结束相关 circuit |
| 授权变化：执行许可、拓扑边或承载被撤回 | 退役，不等待健康窗口 |
| 更优的新鲜认证通道获选 | 精确替换旧 channel ID |

拨号方每次获取使用新的本地 socket，因此重建后的通道在监听端表现为新的源端口；源端口持续变化说明通道在反复失效，不是刷新本身引起的。

只能单向拨入的 Native UDP 边（监听端 dialers 关闭）上，100 次 `meshway ping` 全部回复且同一 link ID 与源端口，跨多个 STUN 节拍与健康周期没有通道转移。承载本身每隔一个报文丢弃时（OrbStack 2.2.4 之前容器→machine 的 UDP 路径）健康 Pong 确定性丢失、通道按健康窗口正确退役，这是真实承载失败，见[常驻 systemd 验收 VM](/v2/node/docker-lab#systemd-vm)。

## 路径选择

普通拨号、反向拨号、协调 UDP 打洞、中继和真正直连分别报告。双方认证成功、真实双向往返与实际承载路径一起构成切换依据；STUN 返回值或已发送探测包不构成成功证明。

打洞失败时仅可使用已允许的其他连接／中继路径。例如逻辑路径要求 A→R→B，A 与 B 的候选即使能直连，也不能绕过 R。新通道和旧通道竞争时，由同一连接 owner 完成选择，业务流保持其原完整路径。

### 中继兜底 {#relay-fallback}

Policy 给出候选集合，源节点按本机已知的连通事实排序：

- **集合**（Policy，[routing](/v2/configuration-reference/routing#可行性、循环与失败) 的搜索）：必经点展开后最短优先、同长按节点 ID；同一长度的路径全部列出后才看更长的，列满 `MAX_PATHS`（8）条的那个长度结束搜索。拓扑允许 A–B 直连而没有 `via` 时，A→B 与拓扑允许的**每一个** A→R→B 都是候选，不按节点 ID 截断。中继与端点各自在自己的投影上用同一搜索准入，因此源选中的任一候选在中继和目标处都被接受。
- **排序**（`SecureNode::choose_path`，`secure/netmap/selection.rs::ranked`）：直连路径在前；经中继的路径按（本机与其下一跳有通道，路径上每个中继在[最后已知事实](#last-known-facts)的 `connected` 中都与其后的节点有通道）字典序排列，即整条链已知连通的路径最先、只与本机有通道的次之、其余在后，同类保持 Policy 顺序。只看第一个中继的下一节点不够：多中继路径的第一个中继与第二个中继相连并不说明能到目标（目标离线时它会排到所有单中继路径之前）。取前 8 条（含直连）；超出的中继不尝试，并记一条 debug 事件 `operation=path_candidates`（`kept`、`truncated_relays`）。

新 circuit（业务首包、`meshway ping`、诊断任务）在排序后的候选上选路：

1. 取第一条下一跳已有选定通道、且最近一次源端握手没有无应答结束的路径；都在暂停中时取第一条有通道的路径。
2. 排在所选路径之前、下一跳没有通道的路径（通常是直连）登记邻接需求，Manager 按[候选节奏](#candidate-pacing)继续获取；直连建立后，新流走直连。
3. 没有任何路径的下一跳有通道时，为保留的全部下一跳登记需求并排队首包，任一通道就绪即在其路径上握手。

新流只加入排在第 1 条所选路径及其之前的已建立 circuit：已失去远端的中继（事实中不再与目标有通道）或直连已有通道时的中继上的旧 circuit 不再接纳新流，新流在所选路径上新建 circuit。下一跳当前没有通道的已建立 circuit（本机漫游后正在重新获取直连）在排序中其后有带通道的路径时也不接纳新流；没有任何带通道的路径时新流仍加入它并等待（`SecureNode::joinable`）。已经建立的流始终保持自己的原完整路径。

候选计算有界：一次目标搜索共享 16,384 工作项预算、队列 2,048 项，每目标最多 2,048 条（`MAX_SEARCH_PATHS`）。先只朝下一必经点扩展、再扩展绕行，使 1,000 个节点的全连通投影中直连＋998 个两跳中继约用 5,000 个工作项列出，随后因已满 8 条而结束，不扩展三跳；排序只看前 8 条，每个新流的 circuit 查找也只在这 8 条上进行。

源端握手到期（`handshake_timeout`，10 秒）仍无应答时，该路径暂停 `10 s × 2^(n−1)`（上限 60 秒），下一个 circuit 换用其他有通道的路径；`n` 是连续无应答次数，暂停结束只让路径重新可试、不清零 `n`，得到应答、该路径内容变化或有界回收（至多 256 条路径，满时让位给新路径的是暂停结束最久的一条）才清零。新 netmap 内容只清除自身内容变化了的路径的暂停：路径内容是路径上各节点的数据面密钥与相邻两跳之间双向的获准承载（`secure/netmap/pause.rs::PathContent`），其他节点的备注、Access、DNS、出口等变化保留暂停。这覆盖“中继有通道但到不了目标”：中继缺下一跳时只登记需求并拒绝 INIT（不消耗 CID），源不会因此永久固定在这条路径上。已建立的 circuit 与流始终保持原完整路径，切换承载或路径只作用于新 circuit。

`meshway status` 对经中继的 peer 显示 `relay … via <R>`；有邻接需求、当前没有通道也没有 circuit，但按上述排序的第一条中继路径首跳有通道、且其上每个中继在最后已知事实中都与下一节点有通道时状态为 `relay (idle)`（`SecureNode::relay_route`），`unreachable` 只用于没有任何可用路径；任一无通道、无 circuit 的 peer 另显示新 circuit 会走的路径（`direct (idle)`／`relay (idle) via <R>`／`no path`，`SecureNode::idle_route`，与首包同一选择；`unreachable`／`offline` 的 peer 没有可走的路径，总是 `no path`），见[运行状态](/v2/node/runtime)。`meshway ping` 每个回复显示 `via relay <R>`。直连不可能时（两端都在端点相关映射 NAT 后、无共同私网），流量经与双方都有通道的第三节点中继；R 不需要 listener，只要它与两端各有通道。

真机上两端都在端点相关映射 NAT 后时，流量经唯一与两端都有通道的节点中继，见[真机证据](/v2/node/validation#n4-real-network)。

已实际选定的流式通道结束后，拨号 owner 经原重试间隔重新按候选来源优先级获取连接，不继续此前候选列表尚未尝试的后缀。因此已认证的 configured TCP 退路短暂断开后，会先重试该来源，再考虑本机候选；未完成认证或未获选的尝试仍继续当前候选轮次。对端仍健康的更优通道不会因此被替换。

穿透、NAT 与 TCP 退路的验收状态见[验收范围](/v2/node/validation)。

隔离双 NAT fixture 的 R 使用空 dialers，A 与 B 分别向 R 的私网 listener 获取通道；两条边仍要求原始 STUN offer、双边接受及真实安装／包流证据。这样不会因 B 的公网侧 local 候选可达而把普通拨号误当作穿透验收；业务和回复仍沿完整 A→R→B 路径。

## 规划：MASQUE

MASQUE 尚未实现。后续优先评估 CONNECT-UDP 与现有 datagram 会话的衔接，再决定 CONNECT-IP 的必要范围；HTTP/3 DATAGRAM 与 TCP 上的 Capsule 承载分别报告，经代理传输不标记为 P2P 直连。HTTP/3 不承诺与浏览器流量不可区分，代理／CDN 的支持需真实互通验证；QUIC 或代理 TLS 不替代 Mesh 的端到端保护与业务授权。

参考：[RFC 8489](https://www.rfc-editor.org/rfc/rfc8489)、[RFC 9297](https://www.rfc-editor.org/rfc/rfc9297)、[RFC 9298](https://www.rfc-editor.org/rfc/rfc9298)、[RFC 9484](https://www.rfc-editor.org/rfc/rfc9484)。
