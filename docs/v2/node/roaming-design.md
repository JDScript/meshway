# 漫游（底层网络切换）设计

> **第一切片（R1）已实现**，现行行为的唯一定义是[连接获取 · 漫游](/v2/node/connectivity#roaming)，验收见 [roam/direct 行](/v2/node/validation#roam-row)；D9、D10、D12 随 R1 实现。**第二切片（R2）中 NAT↔NAT 重新穿透、漫游中的策略中继与快速往返已实现**（[roam/nat](/v2/node/validation#roam-nat-row)、[roam/relay](/v2/node/validation#roam-relay-row)、[roam/flap](/v2/node/validation#roam-flap-row)，`roam/nat` 门槛按 D14 收紧到 5 秒）；**R2 剩余项已实现**（R2 剩余，`r3/roaming-fingerprint`）：`roam/exit`（[roam/exit 行](/v2/node/validation#roam-exit-row)）、指定地址 listener 的地址消失与 U1 倍增清零、bootstrap resolver 上游重读，同时修正指纹（按地址族解释地址标志、路由参照含当前 remote，见[漫游](/v2/node/connectivity#roaming)）。本页其余部分仍是设计稿：节点在运行中更换底层网络（Wi-Fi→LTE、换 NAT、地址或默认路由变化）时连接、数据面与宿主资源应有的行为、可选方案与验收办法。现行行为的唯一定义仍是[连接获取](/v2/node/connectivity)、[数据面](/v2/node/dataplane)与[运行状态](/v2/node/runtime)；验收状态只以[验收范围](/v2/node/validation)为准。本页不加入文档导航。
>
> 术语：本页的“漫游”指**同一 Meshway 网络内、本节点身份不变**时底层网络的变化。[CLI 表](/v2/node/runtime#cli-与-ipc)中“切网”一行指的是切换到另一张 Meshway 网络，与本页无关（见[待用户决定](#decisions) D1）。

## R1 前基线 {#baseline}

> 以下两表是 R1 实现前（设计稿提交 `f007bde1` 时）的行为与推算，只作为本页方案的出发点保留，**不是现行行为**：R1–R3 已取代其中的变化来源、拨号端来源变化、STUN 提前观测、NAT 后重新穿透与验收各项，现行定义见[连接获取 · 漫游](/v2/node/connectivity#roaming)，验收见[漫游行](/v2/node/validation#roam-row)及其后四行。表中的源码位置是当时的位置。

| 项 | R1 前行为 | 当时的定义位置 |
| --- | --- | --- |
| 变化来源 | Underlay rtnetlink monitor 只发一个单调计数 `network_changes`：外部接口的 up 链路增删、地址增删，以及 **main 表（254）**中出接口为外部接口的路由增删；未 up 链路、自有 TUN／出口 TUN／DNS 锚点、自有规则与私有表不计 | `crates/meshway-underlay/src/linux/kernel/monitor/classify.rs::network_changed`；[控制调度与恢复](/v2/node/runtime#控制调度与恢复) |
| 消费者 | ① 控制中继关闭当前连接（挂起的 `sync` 随之中断，按 1–15 s 有界退避重连）；② 网络 worker 重新采样并唤醒 State 报告（最多每秒一次，重新枚举 `localEndpoints`） | `linux/proxy.rs::relay`；`runtime/network/worker` `Work::NetworkChanged` |
| STUN | 每 socket 固定节拍（有通道 ~10 s，否则 ~30 s）；**不因网络变化提前观测**；映射变化立即上报并清零全部候选节奏 | [持续 STUN 观测](/v2/node/connectivity#continuous-stun) |
| UDP 通道 | 以对端 `SocketAddr` 为键；未知来源的业务帧丢弃；认证 Ping 每 2 s、健康窗口 6 s | [邻接管理](/v2/node/transport#邻接管理) |
| 来源变化的接住 | **listener 端**收到未知来源、按现有通道签名的 Ping 时，向新来源发起新的 source challenge，完成后以新 channel 精确替换旧 channel（`probe_recovery`）；**拨号端没有对应路径**；穿透尝试内的[来源学习](/v2/node/connectivity#source-learning)（≤2 个来源）只在建立期有效 | `manager/datagram/session.rs::probe_recovery` |
| 流式通道 | 只在连接结束时退役，没有应用层健康检查；本机地址消失后可能长时间挂起（内核 TCP 重传上限） | [通道稳定性](/v2/node/connectivity#channel-stability) |
| circuit | 绑定相邻对端的 **boot**，不绑 link／地址；同 boot 的承载更换保留 circuit；源端连续 15 s 无认证帧、双向 90 s 无帧时结束 | [受保护 circuit](/v2/node/dataplane#候选受保护-circuit) |
| 候选节奏 | 有选定通道即停止探测该 peer 其他候选；候选集合变化、本机映射变化、该边内容变化清零暂停 | [候选节奏](/v2/node/connectivity#candidate-pacing) |
| 事实传播 | State 报告写 Server State，候选内容变化立即唤醒可见它的节点挂起的 `sync` | [Netmap · 传输](/v2/node/netmap-design#sync-transport) |
| 验收 | 无漫游行；[nat/reconnect](/v2/node/validation#reconnect-row) 只覆盖重启与映射清空，[N3](/v2/node/validation#n3-real-network) 记“WARP 更换出口 IP 后的恢复”未观测 | [验收范围](/v2/node/validation) |

据此推算的 R1 前结果（当时未实测；R1–R3 的实测见上述验收行）：

| 情形 | 预计 |
| --- | --- |
| 漫游方是 UDP **拨号方**，对端公网 listener | 下一次健康 Ping（≤2 s）触发对端 `probe_recovery`，约 2–3 s 恢复；circuit 预计保留。无证据 |
| 漫游方是 **listener**，对端拨入 | 对端拨号 socket 不接受新来源；对端健康窗口 6 s 失效后按新 `localEndpoints`／configured 重拨，约 6–10 s；configured 地址已失效时更久 |
| 双方在 NAT 后（协调穿透） | 等下一次 STUN 节拍（≤10 s）→ State → prepare／accept，约 10–20 s；超过 circuit 的 15 s 无回复上限时 circuit 重建 |
| 流式承载（TCP／VLESS／Trojan） | 直到内核报告连接结束才重拨，可达数分钟 |

不变量（所有方案都必须满足）：

- **漫游不改变身份与授权**。session、instance、netmap、租约与 `Desired` 都不因漫游变化；候选变化不授予权限、不延长租约（[STUN 与 Server 协调](/v2/node/connectivity#stun-与-server-协调)）。
- **源地址只是投递信息**。任何“换到新地址”都必须先由对端签名证明身份，并对新地址完成返回可达（return-routability）挑战，才改变发送目的；未认证报文不能让本节点向任意地址发包。
- **不绕必经点**。漫游期间的承载替换只发生在同一相邻对、同一获准 carrier 内；已建立的流保持原完整路径（[路径选择](/v2/node/connectivity#路径选择)）。
- **一个 owner**。网络变化的分类归 Underlay（唯一 netlink owner），连接反应归 Manager，报告调度归 daemon／Control；不新增第二个 monitor、第二份候选账本或第二份流表。
- **有界**。漫游触发的 STUN、Ping、挑战与查询都受现有预算（控制消息预算、每 60 s 30 次查询）约束，接口抖动不能放大成无界发送。

## 1. 什么算网络变化 {#what-changes}

| 事件 | 能否观测（Linux） | 影响 |
| --- | --- | --- |
| 默认路由变化（出接口／网关／prefsrc） | rtnetlink；**非 main 表**（NetworkManager 多表、WARP／WireGuard 的 fwmark 表）当前不计 | 外层报文的出接口与源地址改变 → 所有 UDP 通道的对端看到新来源 |
| 承载接口地址增删 | rtnetlink | 正在用的源地址消失 → 流式通道失效、指定地址的 listener 失效；新地址进入 `localEndpoints` |
| 接口 up／down、载波变化 | rtnetlink（`IFF_UP`、`IFF_LOWER_UP`／operstate） | 关联（Wi-Fi 重连）多伴随载波闪断，地址可能不变 |
| Wi-Fi SSID／BSS 变化 | rtnetlink 看不到；nl80211 可见（本设计不使用） | 只能经载波闪断、地址或 STUN 结果间接得知 |
| 公网映射变化（NAT 重启、CGNAT 轮换、VPN 换出口 IP） | 只有持续 STUN | 本地无任何事件；对端看到新来源端口／地址 |
| IPv6 前缀变化、临时地址轮换 | rtnetlink | 前缀变化等同地址增删；临时（privacy）地址每天轮换，属于噪声 |

### 选项

| | (a) 维持单一计数 | (b) 底层指纹（推荐的一层） | (c) 逐通道路由指纹（推荐的二层） |
| --- | --- | --- | --- |
| 判定 | 任何外部事件 = 变化 | Underlay 在事件后（250 ms 合并）计算指纹：各外部 up 接口的（index、载波、首选全局地址集合）＋每地址族“以本节点 mark 查询到 Server 首个地址的路由”（oif、网关、prefsrc）；指纹变才算 **path** 类变化，其余为 **refresh** 类 | 对每条选定 UDP／流式通道，以该通道 socket 的 mark 与本地地址做一次 `RTM_GETROUTE` 到其 remote，比较（oif、prefsrc、网关）；变化的通道才主动探测 |
| 非 main 表 | 看不到 | 路由查询走真实 RPDB，覆盖多表与 fwmark 表 | 同左 |
| 临时地址轮换 | 每次都触发 | 只比较首选（非 deprecated、非 tentative）地址；临时地址不入指纹 | 不受影响，除非某通道正使用该地址 |
| 成本 | 每次都全量反应 | 每次事件一次 dump＋两次路由查询 | 每条通道一次路由查询（上限 = 通道数，≤64） |
| 缺点 | 接口抖动放大成全量重探；漏掉多表 | 不知道哪些通道受影响 | 需要逐通道知道本地地址 |

### 推荐：(b)＋(c) 分层

- **path 类**（指纹变）：立即重探——全部 UDP socket 立即 STUN、受影响通道立即探测（[第 2 节](#local-roam)）、立即 State 报告。
- **refresh 类**（指纹不变，例如无关接口地址、临时地址）：只按现有调度刷新 State 报告，不重探。
- **映射类**（持续 STUN 发现映射变化，本地无事件）：沿用现有“立即上报、清零候选节奏”，另加：该 socket 上的全部通道立即发一次漫游探测（同第 2 节）。
- 分类结果由 Underlay 作为 `NetworkChange { class: Path | Refresh, fingerprint }` 发布（替代裸计数，控制中继只在 Path 类时关闭连接）；不新增 monitor。

### 待用户决定

- D2：refresh 类变化是否仍关闭控制中继连接（推荐：不关闭；只有 path 类关闭）。
- D3：是否引入 nl80211 观测 SSID（推荐：不引入；载波闪断＋路由指纹＋STUN 已足够，SSID 不是路径事实）。

## 2. 本机漫游后的恢复序列 {#local-roam}

本机是漫游方。目标：Wi-Fi→LTE（先断后连）时，对端公网可达的情形下首个内层回复 ≤3 s，**circuit 不断（同一 CID）**，内层 TCP 不断开。

### 选项

| | (a) 被动：等对端接住 | (b) 主动漫游探测（推荐） | (c) 全部重建 |
| --- | --- | --- | --- |
| 行为 | 不做额外动作；下一次例行 Ping 从新来源到达对端，由对端 `probe_recovery` 接住 | path 类变化后立即对每条受影响 UDP 通道发一次签名 Ping（不等 2 s 节拍）；对端（**两种角色都支持**）对新来源发 challenge，完成后替换通道；流式通道本地地址消失或路由变化即退役并按原候选优先级重拨 | 退役全部通道，重新获取 |
| 拨号方漫游 | ≤2 s | ≤1 RTT＋挑战 1.5 RTT | 握手＋候选节奏 |
| listener 漫游 | 不恢复（对端拨号 socket 不接受新来源），等对端 6 s 健康失效 | 同拨号方（需对端拨号端也接受漫游，见第 3 节） | 依赖对端重拨 |
| 未受影响的通道（另一接口上的 LAN peer） | 不受影响 | 不受影响（逐通道判定） | 被无谓重建 |
| circuit | 同 boot 保留 | 同 boot 保留 | 同 boot 保留，但空窗更长，可能越过 15 s |

### 推荐：(b)，失败后对该通道退到 (c)

恢复序列（t0 = Underlay 发布 path 类变化）：

| 步骤 | 时刻 | Owner | 说明 |
| --- | --- | --- | --- |
| 1 合并 | t0 ≤ 事件 +250 ms | Underlay | DHCP／RA 的一串事件合成一次 |
| 2 STUN | t0 起并行 | Manager | 全部 Native UDP socket 立即观测；未响应时 500 ms 重试，至多 3 次后回到常规 2 s 倍增；旧指纹下的观测**立即作废**（不再等 120 s 自然到期），续期保留规则只对同一指纹生效 |
| 3 漫游探测 | t0 起并行 | Manager | 受影响 UDP 通道立即各发一次签名 Ping，250 ms 间隔至多 4 次（计入现有控制预算）；任一认证 Pong（经挑战后的新通道）即完成 |
| 4 流式通道 | t0 | Manager | 本地地址已不存在或路由（oif／prefsrc）改变的流式通道立即退役，按[原候选来源优先级](/v2/node/connectivity#relay-fallback)重拨；未改变的保留 |
| 5 候选节奏 | t0 | Manager | 全部 peer 的显式与协调暂停清零（`candidate_pacing` 的 `cause=own_network`） |
| 6 State 报告 | 步骤 2 首个结果或 t0+1 s 的较早者 | daemon／Control | 首个报告不受“每秒一次”合并延迟（合并只限制其后的报告）；控制连接因本机关闭而失败时第一次重试立即进行，不计入 1–15 s 退避 |
| 7 PMTU | 新通道建立时 | Manager | 替换后的通道重新搜索[路径 MTU](/v2/node/connectivity#path-mtu)：先取新路由 MTU，搜索完成前 `inner` 为未知；不能沿用旧路径的值 |
| 8 失败退路 | 某通道 1 s 内未完成 | Manager | 该通道按 (c) 退役并重新获取；对端在 NAT 后时进入[第 3 节](#peer-roam)的协调穿透 |

时间预算（对端公网，LTE RTT 按 60–100 ms 估）：

| 段 | 预算 |
| --- | --- |
| 事件合并 | ≤250 ms |
| 漫游 Ping → 对端 challenge → 应答 → 新通道 | 1.5–2 RTT ≈ 150–300 ms；丢一次加 250 ms |
| 首个内层回复 | 新通道安装后即走原 circuit（同 CID），≤1 RTT |
| 合计 | **≈0.5–1 s**；目标上限 3 s（覆盖 LTE 承载建立前的黑洞期不计入，计时从新链路有路由起算） |
| 内层 TCP | Mesh 地址不变，应用连接不受底层地址影响；空窗 <3 s 时内核 TCP 只经历 1–2 次 RTO，可见停顿 ≈ 空窗＋一个 RTO（≤4 s） |

circuit 连续性的依据：circuit 只绑定相邻对端 boot，漫游方与对端都未重启，替换通道是同 boot 的承载更换；只要首个认证帧在源端 15 s 无回复上限内到达，CID、nonce 与重放窗口全部保留，流表不变。

### 待用户决定

- D4：恢复目标是否采纳为验收门槛——对端可直达时首个回复 ≤3 s 且同一 CID（推荐：采纳）。
- D5：漫游后是否给通道新的 acquisition 值（如 `roamed`）在 status 中显示（推荐：不新增；保留原 acquisition，另记 debug 事件 `operation=roam` 与通道替换计数）。

## 3. 对端漫游（本机是接收方） {#peer-roam}

### 已有机制够不够

| 机制 | 适用 | 不够之处 |
| --- | --- | --- |
| 来源学习 N2a（≤2 个来源、签名 punch／INIT、随尝试期限结束） | 协调穿透**建立期** | 已建立通道不经过它；不能用于漫游 |
| `probe_recovery`（未知来源的签名 Ping → 新 challenge → 替换） | 本机是 **listener**，对端拨号方漫游 | 拨号端没有；一次只处理一个 peer 的挑战 |
| 候选更新（`peerFacts`） | 对端是 listener 且本机可拨 | 本机仍有“健康”通道时（≤6 s 窗口）不试新候选 |

### 选项

| | (a) 只在 listener 端接受漫游（现状） | (b) 两端都接受漫游（推荐） | (c) 接受任意签名帧的新来源（WireGuard 式） |
| --- | --- | --- | --- |
| 拨号端收到对端从新来源发来的签名 Ping | 丢弃 | 用该通道 transcript 验签通过后，对新来源发 challenge（拨号端发起的返回可达挑战），对端在新地址应答后替换 | 直接改发送目的 |
| 安全 | — | 新目的必须先经挑战证明可达且由对端签名；重放的旧 Ping 因 nonce／replay 集合被拒；每 peer 同时至多一个挑战 | 被重放的签名帧可把流量导向任意地址（放大／劫持），**拒绝** |
| 对称 NAT（端点相关映射）漫游方 | — | 对端公网或地址相关过滤时，漫游方的 Ping 从新映射到达，挑战回复经同一映射返回，可恢复 | — |

### 推荐：(b)

- **旧来源何时丢弃**：挑战完成、新通道替换旧通道的同一时刻。不保留双来源宽限期——先断后连时旧地址已不可达，先连后断（双上联）时旧通道照常工作到替换。
- **候选更新加速**：对端 `peerFacts` 中某 listener 的候选变化，且本机当前通道的 remote 已不在新候选集合中时，不等健康窗口失效，立即对新候选并行获取；成功即按“更优的新鲜认证通道”精确替换旧 channel（[通道稳定性](/v2/node/connectivity#channel-stability)）。
- **NAT↔NAT（两端都在端口相关过滤的 NAT 后）**：只能重新协调穿透。**由漫游方发起**：它最先知道变化，也最先得到新的 STUN 事实；它的候选节奏已在第 2 节清零。接收方按现有规则不论暂停与否立即参与对方的尝试（[候选节奏](/v2/node/connectivity#candidate-pacing)），因此接收方不需要重置自己的暂停。接收方只在“该 peer 的候选集合变化”时清零对该 peer 的暂停（现有规则），不因自己没有变化而清零全部。
- 预算：漫游方 STUN（0.3 s）→ 报告并确认（0.5 s）→ prepare（0.3 s）→ Server 唤醒对端 hold → 对端 query＋accept（0.5–1 s）→ punch（0.3 s），合计约 2–3 s，目标上限 10 s，低于 circuit 的 15 s 上限。

### 待用户决定

- D6：是否允许拨号端接受对端漫游（推荐：允许，必须经挑战；拒绝 (c)）。

## 4. 中继与路径 {#relay}

Tailscale 漫游时直连失效即退回 DERP，任何对端都有兜底；Meshway 只有[策略定义的路径](/v2/node/connectivity#relay-fallback)。

### 选项

| | (a) 不迁移已有流（推荐，R1） | (b) 已有流临时迁到中继再迁回 | (c) 先连后断（双上联保活） |
| --- | --- | --- | --- |
| 已有流 | 保持原完整路径与原 circuit，等直连恢复（≤15 s 内保留 circuit） | 换到同样被 Policy 选中的中继路径上的新 circuit，直连恢复后迁回 | 两个上联同时在线时先在新上联建通道，再撤旧上联 |
| 新流 | 现有 `choose_path`：首跳无通道时选有通道的中继路径（已实现） | 同左 | 同左 |
| 与不变量 | 符合“已建立的流保持原完整路径” | 违反该不变量；需要流表在两条路径间重绑 CID，端点、中继各自重新准入，属于新的路径语义 | 符合；依赖宿主保留旧上联 |
| 无中继时 | 空窗期断，直连恢复后续上 | 同左 | 不受影响 |

### 推荐：(a)

R1 不迁移已有流；漫游空窗内的新流按现有排序可经中继（`nat/relay` 已验证首包选路）。漫游方本身是中继 R（A→R→B）时，R 与两侧各自按第 2、3 节恢复通道，经过 R 的 circuit 绑定的是 A、B 各自对 R 的 boot，不受影响。方案中没有中继且直连恢复不了时，漫游期间就是断：这是策略路径的结果，不另造兜底。

### 待用户决定

- D7：是否接受“没有策略中继时漫游空窗即断、已有流不迁移到中继”（推荐：接受；(b) 作为后续单独的产品决定）。

## 5. Server 侧 {#server}

| 项 | 行为 |
| --- | --- |
| session／instance | 不变。漫游只是 HTTP 连接重建；Server 不得把 session、instance 或 sync hold 绑定到来源 IP。实施前需核对 Server 没有这类绑定 |
| sync hold | 漫游方：控制中继关闭使挂起的请求中断，新请求即续；Server 侧“被取代的回答不计入”规则保证租约记账正确（[传输](/v2/node/netmap-design#sync-transport)）。对端：漫游方的候选变化是事实**内容**变化，立即唤醒可见它的 hold，不等 60 s |
| 候选 | State 报告只写 State；新候选不授予权限、不延长任何期限；旧指纹的 STUN 候选在新报告中直接不再出现（第 2 节步骤 2），Server 按 `removed`／替换下发，peer 不再拨过期映射 |
| 对端收到新候选 | 第 3 节“候选更新加速” |
| 穿透信令 | 不变：漫游方 prepare 后 Server 唤醒对端 hold，对端经 `traversalRequests` 参与 |

传播时延目标：漫游方 State 报告被 Server 接受到对端 `KnownPeers` 更新 ≤1 s（对端 hold 在途时）。

### 待用户决定

- D8：是否把漫游作为可见事件（Server State／Console 观察中记录“节点网络变化”时间与新公网地址类别）（推荐：R1 不加，只有 Node debug 事件与 Status 中的 `lastNetworkChange` 时间；Console 呈现留给后续）。

## 6. TUN、路由、DNS 与出口的不变量 {#host-invariants}

| 资源 | 漫游时 | 风险与处理 |
| --- | --- | --- |
| Mesh TUN、地址 | 不变，不重建 | 无 |
| capture（私有表＋RPDB 块） | 不变；受保护 socket 经 own-mark 跳过本块后按原 RPDB 走新的默认路由 | 新网络分配的地址落在 Mesh 地址池内（运营商 CGNAT 使用 `100.64.0.0/10` 时可能发生）→ 现为整个 Host 保护冲突、fail closed。建议保留 fail closed，但给出专门原因（`meshPoolOverlap`，含接口与地址），地址离开后自动恢复（D9） |
| 自有规则／路由 | 不变 | 网络管理器在重新配置链路时删除“外来”策略规则（systemd-networkd `ManageForeignRoutingPolicyRules=yes` 为默认值；NetworkManager 视配置而定）→ 现为 `unexpected_del_rule`、fail closed。见 D10 |
| DNS 锚点（resolved 后端） | 不变；新链路的 DNS 服务器由 resolved 自己管理 | Wi-Fi 重认证造成的宿主 RTNL 停顿已由 [D3](/v2/node/validation#d3-dns-health) 的观察期限分类覆盖 |
| resolv.conf 文件后端 | 不变 | DHCP 客户端改写 `/etc/resolv.conf` → `resolv_conf_mode` foreign；保存的“原 nameserver”属于旧网络，漫游后不可达。见 D11 |
| bootstrap resolver | — | 已实现（R2 剩余，`r3/roaming-fingerprint`）：原为启动时取快照；现每次解析按文件身份比对并重读 resolved 上游文件或系统 resolv.conf 真实上游 |
| 源端 egress capture | 不变 | LAN 例外只来自显式 CIDR，新 LAN 不自动放行（现有语义） |
| MSS clamp | 随通道 MTU 重新测量更新 | 第 2 节步骤 7 |
| 出口 TUN／NAT（出口节点自己漫游） | masquerade 使用实际出接口地址，新流自动用新地址 | 已验证（R2 剩余，`r3/roaming-fingerprint`），见 [roam/exit 行](/v2/node/validation#roam-exit-row)：旧地址的 conntrack 由内核清除，UDP 流以新来源继续、TCP 连接停滞；内核为同一原始 tuple 新建的条目原被出口 owner 判为外来、角色无法撤回，现视为该投影的后继。path 类变化后 1 秒重跑出口前置探测，结果经 State 上报 |
| listener | 通配地址 listener 无需重绑 | 已实现（R2 剩余，`r3/roaming-fingerprint`）：绑定**指定地址**的 listener 在 path 类变化时若地址已不在本机即关闭并进入 [U1 重试](/v2/node/connectivity#listener-bind-retry)；path 类变化让重试立即再试并清零倍增。临时端口范围冲突（`ephemeral_listener_port`）仍由 U1 重试兜底 |
| 拨号 socket | 默认不绑定本地地址，内核按每次发送选源地址 | 显式 `bindInterface`／`localAddress` 的拨号在该接口／地址消失后失败，属于配置结果，不自动改绑 |

journal 与“外来变更”判定：漫游本身不触碰任何自有对象，现有分类（只有 kernel 来源通知参与归属；未 up 链路不算变化；自有对象排除）不会把普通漫游当作冲突。会被判为冲突的只有上表两种：地址落入 Mesh 池，以及网络管理器删除自有规则／路由。

### 待用户决定

- D9：新网络地址与 Mesh 池重叠时保持 fail closed＋专门原因＋自动恢复（推荐：是）。
- D10：自有规则／路由被网络管理器删除（且没有外来对象占位）时，改为有界自愈重装而不是永久冲突（推荐：是；外来对象覆盖 Mesh 池仍为冲突）。
- D11：resolv.conf 文件后端是否声明“不支持漫游”（推荐：声明不支持，漫游节点使用 resolved 后端；不实现上游重新快照）。
- D12：出口节点漫游时，是否在重新探测完成前把出口能力报为 `probing`（推荐：是，源端按出口就绪事实暂不选它）。

## 7. 验收 {#validation}

### 隔离 fixture

在 `verify_runtime.py` 的 NAT 拓扑上增加 `--netmap-scenario roam-*`，资源全部为 fixture 自有 Docker 网络与容器，不改宿主网络、不用 host network 或 privileged：

```text
               ┌ uplink1 ─ router-a1（SNAT，公网地址 P1）┐
A（两个上联）─┤                                         ├─ internet ─ B（公网）／router-b ─ B（NAT 后）
               └ uplink2 ─ router-a2（SNAT，公网地址 P2）┘          └ R2（公网中继，roam/relay）
```

| 扰动 | 做法 | 模拟 |
| --- | --- | --- |
| soft | A 的 netns 内 `ip route replace default via router-a2`，再删除 uplink1 地址 | 默认路由切换＋地址删除 |
| hard | `docker network disconnect uplink1 A`，随后 `connect uplink2`（或反序：先连后断） | 链路消失（Wi-Fi 断开）／双上联 |
| nat-reboot | 清空 router-a1 的 conntrack 并改用 `--random-fully` | 映射变化而本地无事件 |
| role | A 为拨号方（B listener），以及 A 为 listener（B 拨入）两组 | 第 3 节两种角色 |

### 测量

| 量 | 方法 | R1 门槛 |
| --- | --- | --- |
| 首个内层回复 | 100 pps 带序号 UDP 双向流与 10/s `ping`，取扰动后首个回复时刻 | `roam/direct` ≤3 s；`roam/nat` ≤10 s（R2 起 ≤5 s） |
| UDP 丢包 | 双向序号缺口计数 | ≤ 门槛时长 × 速率 |
| TCP | 长连接 echo（每 50 ms 一条带时间戳记录），取最大到达间隔；连接不得重置 | 同一连接存活，最大间隔 ≤5 s |
| circuit 保持 | 扰动前后 A、B 的 CID（debug 事件或 IPC 字段，见 D13） | 同一 CID |
| 通道 | 替换前后 channel ID、`LinkFact.remote`、boot | channel 替换一次，boot 不变 |
| STUN→候选发布 | A 的观测时刻 → Server State 中新候选的接受时刻 | ≤1.5 s |
| 对端收到候选 | Server 接受 → B 的 `KnownPeers` 更新（debug 事件） | ≤1 s |
| 无关通道 | A 另一接口上的 LAN peer 的 channel ID | 不变 |
| 路由器证据 | 两个路由器的 SNAT 计数、uplink1 断开后无 A 的报文 | 计数 > 0 |

### 矩阵行

| 行 | 拓扑 | 覆盖 |
| --- | --- | --- |
| `roam/direct` | A 两上联（端口保持 SNAT）↔ 公网 B | soft／hard × 拨号方／listener；同 CID；TCP 存活 |
| `roam/nat` | A 两上联 ↔ NAT 后 B（端口相关过滤），uplink2 为 `--random-fully` | 漫游方发起的重新协调穿透；nat-reboot |
| `roam/relay` | A 两上联，uplink2 丢弃 A↔B；R2 与两端常驻 | 新流经 R2；已有直连流在直连恢复前不迁移（D7）；uplink1 恢复后新流回到直连 |

其后实现：`roam/flap`（R2）与 `roam/exit`（E 漫游，出口能力重新探测、旧外部连接结束、新流用新地址；R2 剩余），定义见[验收范围](/v2/node/validation#roam-exit-row)。

### 真机

| 环境 | 做法 | 覆盖 |
| --- | --- | --- |
| 常驻 VM `meshway-vm` | Mac 切换 Wi-Fi 或开关 WARP；VM 内接口不变 | 只有映射变化（nat-reboot 的实网形态），补 [N3](/v2/node/validation#n3-real-network) 未观测项 |
| 常驻 lab 容器 | 对 lab 自有的第二个 Docker 网络 `docker network connect／disconnect` | hard 扰动；不改宿主与未知网络（[Docker lab](/v2/node/docker-lab)） |
| 实体 Linux 笔记本（若有） | Wi-Fi ↔ 手机热点 | 真实先断后连、DHCP、resolved 链路 DNS |

### 待用户决定

- D13：circuit 连续性的证据通过 debug 事件取得，还是在 IPC Status 中暴露 circuit 标识摘要（推荐：debug 事件，避免为验收升 IPC 版本）。
- D14：上表门槛作为 R1 合入条件（推荐：是；`roam/nat` 的 10 s 在 R2 收紧到 5 s）。

## 8. 切片 {#slices}

| 切片 | 范围 | 矩阵行 |
| --- | --- | --- |
| **R1**（已实现，见[漫游](/v2/node/connectivity#roaming)；另含 D9、D10、D12） | Underlay：path／refresh 分类、指纹（含非 main 表的路由查询）与 250 ms 合并，替代裸计数；控制中继只在 path 类关闭。Manager：path 类后立即 STUN（旧指纹观测作废）、清零全部候选节奏、受影响 UDP 通道立即漫游探测、拨号端接受漫游（经挑战）、流式通道按本地地址／路由退役重拨、替换后重新搜索 PMTU、对端候选变化且当前 remote 不在新集合时并行获取。daemon／Control：首个 State 报告不等合并、本机关闭引起的控制失败立即重试。文档与 `roam/direct` | `roam/direct` |
| R2 | **已实现**：NAT↔NAT 由漫游方发起的重新穿透与时延收紧（漫游探测 1.5 秒无应答即结束可再获取的通道；peer 的请求在接收方旧通道旁接受并替换）；漫游空窗内新流经策略中继（下一跳无通道的 circuit 不接纳新流）；快速往返。D9、D10、D12 已随 R1 实现。指定地址 listener 的地址消失处理与 U1 倍增清零、出口节点漫游的验收、bootstrap resolver 上游重读已实现（R2 剩余，`r3/roaming-fingerprint`） | `roam/nat`、`roam/relay`、`roam/flap`、`roam/exit` |
| R3 | 已有流迁移到中继（D7 的 (b)，需单独产品决定）；双上联先连后断／多路径；IPv6 临时地址与前缀委派细化；非 Linux 平台 | 另定 |

R1 不改 wire（漫游探测复用现有签名 Ping 与 challenge 消息），不改 HTTP 契约、不改 IPC 版本（D13 推荐时）。

## 参照：Tailscale {#tailscale}

| Tailscale（magicsock／netmon） | Meshway 对应 |
| --- | --- |
| netmon 区分“major”变化，忽略无关接口 | 第 1 节 path／refresh 分类 |
| 链路变化后 `ReSTUN("link-change")` 并 rebind socket | 第 2 节步骤 2；通配 socket 无需 rebind |
| 端点立即经 control 更新给 peer | State 报告 → 事实内容变化唤醒 peer 的 hold |
| 清除 bestAddr 信任，disco ping 全部候选，选出新路径 | 漫游探测＋候选更新加速；身份由签名、可达由挑战证明 |
| WireGuard 接收任意已认证包的新来源即改端点 | 拒绝无挑战的改目的（第 3 节 (c)）；只在挑战完成后替换 |
| 直连失效期间经 DERP，任何 peer 都有兜底 | 没有全员中继：只有策略路径中的中继承接新流，已有流不迁移（第 4 节） |
| WireGuard session 与端点无关，切换不断 | circuit 绑定相邻 boot 而非地址，同 boot 的承载替换保留 CID |

参考：[How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)、[tailscale/wgengine/magicsock](https://github.com/tailscale/tailscale/tree/main/wgengine/magicsock)、[tailscale/net/netmon](https://github.com/tailscale/tailscale/tree/main/net/netmon)。

## 待用户决定（汇总） {#decisions}

| 编号 | 问题 | 推荐 |
| --- | --- | --- |
| D1 | 文档术语：底层网络变化统一称“漫游”，CLI 表的“切网”改为“切换 Meshway 网络” | 采纳 |
| D2 | refresh 类变化是否关闭控制中继连接 | 不关闭 |
| D3 | 是否引入 nl80211 观测 SSID | 不引入 |
| D4 | 对端可直达时首个回复 ≤3 s、同一 CID 作为验收门槛 | 采纳 |
| D5 | 是否为漫游后的通道新增 acquisition 值 | 不新增，记 debug 事件与 `lastNetworkChange` |
| D6 | 拨号端是否接受对端漫游 | 接受，必须经挑战 |
| D7 | 无策略中继时漫游空窗即断、已有流不迁移 | 接受 |
| D8 | 漫游是否进入 Server State／Console | R1 不进入 |
| D9 | 新地址与 Mesh 池重叠 | fail closed＋专门原因＋自动恢复 |
| D10 | 自有规则被网络管理器删除 | 有界自愈重装 |
| D11 | resolv.conf 文件后端与漫游 | 声明不支持 |
| D12 | 出口节点漫游时能力先报 `probing` | 是 |
| D13 | circuit 连续性证据来源 | debug 事件 |
| D14 | 第 7 节门槛作为 R1 合入条件 | 是 |

## 限制

- 只针对 Linux；只观测 rtnetlink 与 STUN，不观测 SSID、蜂窝状态或系统网络管理器的意图。
- 推算的“今天的结果”和全部时间预算都没有实测；R1 的第一步是在 `roam/direct` fixture 上测量现状作为基线。
- 两端都在端点相关映射＋端口相关过滤 NAT 后的边漫游后仍不可直连（与现状相同，需要中继）。
- 漫游方的公网地址变化必然结束经它出口的外部连接；Mesh 内连接因 Mesh 地址不变而可以保持。
