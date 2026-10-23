# 数据面、TUN 与 DNS 组件

本页定义策略准入、受保护 circuit、Linux TUN、underlay 与 DNS 组件的行为。安装门槛见[网络模式](/v2/node/runtime#安装范围与状态)，验收状态见[验收范围](/v2/node/validation)。

## 策略与包准入

`meshway-policy` 是 Access、主体匹配、routing、topology 和编译规则的实现归属。Server 对可信完整输入编译；Node 使用自己的 `NodeConfiguration`／`PathPermit`，通过 `admission::admit_forward`／`admit_reply` 检查业务元数据与完整路径。

执行发布中的 `mesh_hosts` 也按该节点作为请求源的 Access 裁剪。拥有中继／回复资格、获知路径 peer 的地址，不自动获得其 MagicDNS 名称可见性；预需求发现仍通过精确名称接口完成。

判定检查当前 permit、业务源／目的地址、协议／端口、完整路径、实际相邻 peer 和出口资格。回复必须对应数据面已有的正向流，并沿原完整路径反向运行。规则语义分别见 [Access](/v2/configuration-reference/access) 和 [Routing](/v2/configuration-reference/routing)；NodeID 或 pure predicate 的调用不证明身份认证。

`meshway-dataplane::Engine` 持有 Engine 的[期望内容](/v2/node/runtime#engine)（`Desired`）和唯一有界流表。`Packet::parse(PacketBuffer)` 验证包并保留原始 buffer；`Packet::parse(&[u8])` 返回借用视图；兼容的 Vec 输入在边界转入带预留空间的存储。`process` 产生 `PendingAction`。宿主必须在最终同步发送／交付边界调用其 `release`，等待期间保留 wrapper 并监听 `ended()`。

同步操作在入口取得一次读数（期望内容、UTC 与单调时间），本次 policy 判定、流时间和 circuit／流截止计算共用。流提交、nonce／replay 消耗、circuit 状态提交及产出前仍执行最终检查；读数不读取租约，也不延长入口确定的期限。排队输出保存自身截止与取消，跨等待的 `release`／carrier I/O 边界继续独立检查。租约到期由 Engine 的空期望保证：SecureNode 被丢弃，全部 circuit、流与排队输出随之取消。本地重试 pacing 与 probe 令牌预算仍以实际扣除预算时的单调时间计时。

流表内部按双向 tuple 索引查询与冲突检查，按单调／UTC 截止索引回收；刷新和删除同步移除旧截止项，索引大小受同一流容量约束。索引只定位候选，不替代当前 permit、完整路径、协议状态和最终输出 guard 检查。读取不刷新 idle／lease，也不创建反向流。

`Ingress::Peer` 的 original origin、packet producer、previous peer 必须由受信身份层提供。网络入口使用下节的 `SecureNode::receive`，端点验证解密后进入同一流表，中继使用会话级转发；不能把未经认证的 transport payload 直接包装成明文入口输入。

### 包与流边界

| 项目 | 当前范围 |
| --- | --- |
| IPv4 | 精确长度与 checksum；暂不支持 options 或分片 |
| IPv6 | 精确长度；最多两个顺序合法、仅含 padding 的扩展头；拒绝 routing header、全部 fragment header 和 jumbogram |
| 业务协议 | TCP、UDP、ICMP echo；检查端口、协议头及适用 checksum |
| ICMP 错误 | IPv4 destination unreachable（code 0–15，含 fragmentation needed）与 time exceeded（code 0–1），引用无选项、未分片的 IPv4 头加至少 8 字节 TCP／UDP 端口或 echo request；ICMPv6 destination unreachable（code 0–6）、packet too big（code 0）与 time exceeded（code 0–1），引用无扩展头的 IPv6 头加至少 8 字节同类内容；见[ICMP 错误](#icmp-errors)。不是业务流量，不建立流 |
| UDP／ICMP | 回复匹配已建立反向 tuple；echo 另绑定 identifier／sequence |
| TCP | 三次握手、每方向 16 MiB 序号历史内的 ACK／重分段重传、前方 16 MiB 内最多 64 个已见乱序区间、FIN 与已建立 RST；支持序号回绕，状态有界。端点按空闲回收的 TCP tuple 记入有界续接集合（同一内容内有效，内容更换或撤空即清空）；其后源方向带 ACK、无 SYN／RST 的同 tuple 段按首包重新准入（续接：forward 从该段序号、reply 从其 ACK 起算），回包仍须确认源已发出的字节；回复方向的段不能建流。因撤销、撤回或租约到期结束的流不可续接，旧连接只能重新握手。由端点 TCP 处理实际重排／SACK；不缓存包体，不支持 simultaneous-open、TFO 或握手 RST |
| 资源 | 默认 4,096 流、60 秒空闲；最大 65,536 流、24 小时空闲设置；ICMP echo 流（每个 `(id, sequence)` 一条）等待回复至多 10 秒、收到回复后 2 秒结束（不长于空闲设置）；实际寿命受空闲、circuit 与 Engine 撤回约束 |

#### ICMP 错误 {#icmp-errors}

ICMP 错误是一条已准入流的 RELATED 报文：`Packet::parse` 把它解析为 `Transport::Error`（外层地址＋被引用包的元组），流表只在它**引用的正是某条活流的 forward 包**（TCP／UDP 端口、echo 的 identifier 与 sequence 完全相同）时，沿该流原路径的回复方向放行：在流的终点（出口或 Mesh 终点）发送、在源接收，且必须是该流绑定的 circuit 与完整路径。Policy（`admission::netmap::admit_related`）另要求错误发往该流的源地址，来自被引用的目的地址本身、终点节点自己的地址，或（出口流）Mesh 地址池以外的地址（出口之外的路由器）；Mesh 中其他节点的地址不能冒充。错误不刷新流期限、不推进协议状态、不创建流；排空中、已到期、出口投影未 ACK 或匹配不唯一的流都不放行。每条流在每个端点每秒至多 10 条（`RELATED_PER_SECOND`），超出即丢弃；`SecureNode::related_counters` 计数放行、拒绝（无匹配流或 Policy 不符）与限速。引用 forward 方向以外的包（例如源本机对回包产生的错误）与其他类型照旧丢弃；ICMPv6 错误按同一规则判定（出口仅 IPv4，受管 IPv6 仍 sink，只有 Mesh 内 IPv6 流有可引用的流）。中继不变，仍按会话转发。本节点为自己超限的包合成的错误不经网络，见[通道 MTU](#channel-mtu)。

同一流固定原 demand ID 与完整路径；同一节点／角色范围的活 tuple 在多个 demand ID 之间也不能被另一条路径接管。`LocalFlow` 只读查询同一 FlowTable，供本机源包／原路径回复使用，不刷新流、推进协议状态或形成第二份路由表。SecureNode 中继不创建业务流，也不因此获得本机发起资格；歧义拒绝。

SecureNode 的实际 forward 包通过认证与流准入后，CID 与协议状态在同一流表事务中提交；handshake 完成、读取状态、失败准入和重放均不能建立或改写绑定。`SecureNode::local_circuit` 据此选择本机回包的 cipher，reply 准入再次要求相同 CID，避免从相同 demand／path 的多个本地 Established circuit 中误选未完成全路径交付的旧响应。普通 Engine 流的 cipher 引用为不适用；只有业务源可在旧 cipher 结束后为同一获准路径取得新 cipher。

TCP 乱序区间只记录已准入字节的序号，不制造接收确认：缺口补齐前不推进连续 ACK 边界，不能用“最高见过的序号”接受越过缺口的 ACK。空 ACK 不创建区间或推进边界；重叠重分段可以补充新后缀。FIN 的字节位置一经准入不能改变，也不能重新解释已准入的数据；RST 仍要求精确连续前沿。区间容量、窗口边界或协议检查失败不提交状态。此前丢失的包不会因此自动重发，重传和实际接收窗口仍由两端 TCP 拥有。

`SecureNode::apply(&Desired)` 对新内容就地比较：仅保留服务、完整路径与出口选择仍由新内容决定的活流，以及授权 digest 与路径密钥不变的 circuit；本节点身份、地址或密钥变化时全部结束。同内容为空操作。

应用不延长任何流期限；只有实际获准包通过同一协议状态机才能刷新其期限。拒绝包不建立或推进流状态。

#### 通道 MTU {#channel-mtu}

连接组件为每条选定的 Native UDP 通道测得内层上限（[外层路径 MTU](/v2/node/connectivity#path-mtu) 的 `inner`，`Snapshot.mtu`），daemon 每轮与承载一起交给唯一 SecureNode（`set_path_limits`）。TUN MTU 仍是固定的 1,366（`DEFAULT_INNER_MTU`），比某条通道能装下的大时：

| 机制 | Owner | 行为 |
| --- | --- | --- |
| 超限包 | dataplane | 端点业务包准入后、消耗 nonce 之前，若它比到下一跳那条通道的 `inner` 大即丢弃（`Error::TooBig`）。本节点 TUN 是该流本机一端时（源发出的包；Mesh 流终点的回包），合成一条错误交本机 TUN：IPv4 为 fragmentation needed（3/4，next-hop MTU = `inner`，引用原包 IPv4 头＋8 字节），IPv6 为 packet too big（2/0，MTU = `inner`，引用原包至多 1,232 字节），源地址为原包目的地址，像该路径上的路由器发出。`inner` 低于 IPv6 最小链路 MTU 1,280 的通道**不承载 IPv6**：任何大小的 IPv6 包都丢弃（`Error::Ipv6Unavailable`，profile 事件 `host.reject.kind` 为 `ipv6_unavailable`），本机一端合成 ICMPv6 destination unreachable（1/0，no route），不再发出无法收敛的 packet too big 1,280；不做外层分片重组。错误绑定该流的期限与取消，经与收到的包相同的 TUN 交付（`take_local_errors` → `deliver_batch`），不进入网络、不经 `admit_related`；全节点每秒至多 32 条、待交付至多 32 条。应用栈据此学到 PMTU（Linux 路由缓存 `mtu <inner>`），TCP 降段、`ping -M do` 报告 `Frag needed` |
| 无就绪出口的 IPv6 | dataplane | 本节点 IPv6 出口流的首包，判定允许但该族没有就绪（含最后已知就绪）出口：不建流、不 direct，合成 ICMPv6 destination unreachable 1/1（administratively prohibited，源地址为原包目的）交本机 TUN，Linux 报 `EACCES`，应用立即改用 IPv4；与超限包错误同一交付路径与限额（期限 1 秒）。IPv4 静默丢弃（[IPv6 出口](/v2/node/exit-design#ipv6)） |
| TCP MSS | Underlay（nft） | 自有表 `inet meshway_mss_<installation>`（与出口捕获同一 installation、同一命令序列化），每次变化以一个 nft 事务整表替换（事务失败时同一内容按 1 s 起翻倍、上限 60 s 重试，直到成功或内容再变），随捕获退役删除，出口前置探测把它算作本节点所有。Mesh TUN 上发出的 SYN 按目的、收到的 SYN 按来源、出口 TUN 收到的源节点 SYN 按来源，把 MSS 限到上限 −40（IPv4）／−60（IPv6）；不小于 TUN 给出的 MSS 时不限，没有任何限制时表不存在 |
| 上限 | dataplane（`SecureNode::clamps`） | 直连 peer：到它那条通道的 `inner`；经中继的 peer：已知上限中最小者（首跳未知）；Mesh 以外的目的（经出口）：到各就绪出口（含最后已知就绪）的上限中最小者。daemon 每秒把结果交 Underlay（`Host::clamp_mss`），只有变化才写 nft；直连↔中继切换或通道重新测量后随之更新 |

范围：只有本节点首跳的通道上限可知；中继到下一跳更窄时帧在中继处丢弃，不会生成错误（中继不解密）。出口对来自外部、超过其首跳上限的回包只丢弃，不向外部发送错误，TCP 由出口 TUN 的 MSS 限制覆盖。不合成 ICMP 的包（ICMP 错误本身、echo reply、带扩展头的 IPv6）只丢弃。上限只改变包的大小，不授予也不撤销任何准入。

### IPv4 出口流与内核投影

出口业务复用同一 `SecureNode`／`FlowTable`：Policy 在本节点投影上检查已认证原始源、`sourceAddresses`、出口 `Decision` 的精确目的／服务、selection 和完整路径；出口端另需本地 `exitRole`（[Routing · 出口联合许可](/v2/configuration-reference/routing#出口联合许可)）。出口端只转发角色 `families` 中的族，混合族包与角色不承载的族拒绝并计数（`family_rejected`）；IPv6 的捕获、按族选择与快速拒绝见 [Exit 设计 · IPv6 出口](/v2/node/exit-design#ipv6)。出口回程由 `exit::ExitReplyIngress` 交给 `SecureNode::exit_reply`，只从已有流选择原 CID，再执行 `admit_reply` 和原路径反向 Noise 保护；未关联包丢弃并计数。出口之外路由器与出口主机自己产生的 [ICMP 错误](#icmp-errors)走同一回程，内核侧见 [Exit 设计 · ICMP 错误](/v2/node/exit-design#icmp-errors)。

出口角色是本节点为其他源提供的能力，不是本节点自身流量的必需组件：本地 `exitRole` 的前置条件不满足或角色安装失败时，本节点其余内容照常 applied，只报告 `exitCapability: unavailable`；角色未安装时出口端不建立出口流（源端按出口就绪事实不选它）。宿主条件与恢复见[出口宿主前置探测](/v2/node/runtime#出口宿主前置探测)。

`SecureNode::next_exit_projection` 按需返回一个不透明 `exit::ExitProjection`：`Install`／`Renew` 携带 flow incarnation、原 tuple（`ExitTuple` 两族同一形状）、不可换绑的 `ProjectionScope` 和一次性 ACK；`Retire` 携带原资源绑定与独立清理 ACK。**内核侧由 Underlay 执行，见 [Exit 设计](/v2/node/exit-design#数据路径与唯一-owner)**；daemon 映射类型并驱动事件和 I/O，不增加业务流表。此接口本身不证明 Linux NAT 安装或包流已验收。

首包和触发续期的输出保留原 guard，通过 `PendingAction::projection_ready`／`PendingFrame::projection_ready` 等待 ACK；每次 `release` 仍检查原截止、流／cipher 取消和 ACK。`PendingAction::exit_flow` 给出该包的精确投影 incarnation；ACK 后的 `exit_scope` 供宿主异步写适配器逐次 poll 检查原包权限，不能用最新流 scope 替换旧排队包。安装失败、丢失 ACK 或等待超时使流不可执行。宿主丢弃一个包装（例如出口队列无空位、出口 TUN 写失败）只丢这个包：流、incarnation 与投影不变，不发布 `Retire`、不产生隔离项，下一个包照常准入。已安装流的后续包由宿主直接交给出口 TUN 的 I/O owner 批量写入，仍各自持有 `exit_scope` 至写 poll（宿主侧队列与写路径见 [Exit 设计 · 出口队列](/v2/node/exit-design#exit-queue)）。投影截止取流 idle 与所属 circuit 生命期的交集（不再有 permit 或执行／instance 截止）；投影绑定安装它的出口角色 generation，随 netmap 租约存续，租约到期、角色撤回或 generation 改变时 Retire；未 ACK 的投影 scope 还受命令等待及旧投影截止约束，不能先迟到安装再依赖 ACK 回调拒绝。续期不会因命令排队重置预算。同授权直接交接只为仍活流发出不超过旧流截止的 `Renew`，实际获准包才刷新流期限。未 ACK 的流不跨交接保留；授权 generation／路径变化结束旧流和 cipher。普通 tuple 退役以独立流 guard 取消旧输出，同 CID 的其他活 tuple 继续执行；最后一条流结束后退役该 cipher，出口端同时向来源方向发出一个逐跳认证的 `Close`（中继照常转发），来源立即结束自己一端，下一包新建 circuit，而不是在出口已不存在的 cipher 上等到 15 秒无回复。

退役先取消流 guard，再发布 `Retire`；出口端从内核精确清理成功 ACK 起隔离 tuple 至少 120 秒，源端从退役起计时。清理失败／未知结果保留隔离项，延迟清理不会缩短隔离窗口。投影失败或在 owner 之外被取消的流转为隔离项、隔离期满的项被清除，都按 100 ms 调度执行，不随每个包扫描全部流与隔离项；每次准入仍检查本流自身存活，失效流的包立即被拒，调度只使其槽位与隔离 tuple 至多多保留 100 ms。本 owner 自己的退役（空闲、投影失败回报、circuit 结束、撤回）立即产生隔离项。关闭 owner 前，宿主调用 `retire_exits` 并排空清理事件。撤回前已通过内核最终过滤的包的边界见 [内核排队](/v2/node/exit-design#生存期、撤回与内核排队)。

正常 TCP 完成进入仅排空状态：最后 ACK／RST 沿原 guard 交付，不在提交流状态时取消它自身；随后不再接纳任何包，不刷新 idle，不跨执行交接保留，最多保留至已经安装的原流截止再发 `Retire`。撤回、显式关闭和实际到期仍立即取消，不能借排空继续旧业务。

`SecureNode::with_exit_limits` 注入独立 `exit::Limits`：默认最多 1,024 出口流、128 未完成投影操作、4,096 个活流预留／退役隔离槽，ACK 等待最多 5 秒且不超过原截止，tuple 隔离 120 秒。流仍计入通用流表上限；已交宿主的超时操作在 ACK／drop 前继续占投影槽。批量交接的续期意图保存在对应流中，按空闲投影槽逐项交给宿主；尚未完成续期 ACK 的流不能输出。容量耗尽拒绝新准入。所有数量硬上限为 65,536。
## 候选受保护 circuit

逐层图与运行成本见[包流分层与加密讨论](/v2/discussions/packet-layers)，线上字段与规模约束见 [Packet 字节账本](/v2/discussions/packet)。本页定义当前行为；讨论中的候选方案不改变这些约束。

`meshway-dataplane::secure::SecureNode` 是期望内容下的 circuit 和流状态 owner。`start` 发起握手，`receive` 验证承载帧，`send` 接收本机 IP 包。业务源与终点使用相同的 `Engine`／`meshway-policy`；中继仅按已建立 circuit 执行会话级转发，不解析业务 facts、不建立业务流。

| 边界 | 当前实现 |
| --- | --- |
| 端到端身份／加密 | Snow 的 `Noise_IK_25519_ChaChaPoly_SHA256`，静态 X25519 公钥取自本机 Server 执行配置；每次握手使用新 ephemeral |
| 握手绑定 | network、明确区分 business demand／diagnostic observation 的授权引用、完整有序 path、随机 circuit ID、permit 授权摘要（排除 expiresAt）、路径上全部控制／数据面 key ID、generation 和公钥 |
| 早期数据 | 两个 Noise 握手消息的业务 payload 必须为空；握手不创建业务流 |
| 路径证明 | 握手使用有序 Ed25519 证明认证各节点独立临时 X25519 contribution；稳态使用一个方向独立的相邻 HMAC-SHA256 tag，完整原路径保留在已认证上下文 |
| 承载约束 | carrier 包含实际 dialer、listener、listener key 与协议；验证该连接在所选完整路径的候选集合内，拨号方向与业务方向分别处理 |
| 重放 | 每 circuit／方向独立的 64-bit nonce 和 1,024 包窗口；nonce 不回绕，只有通过认证与准入的包推进接收窗口 |
| 生命周期 | circuit、流、permit 与 Control 截止取交集；关闭、销毁、失效同时取消排队密文和待交付明文 |
| 承载更换 | circuit 只绑定相邻对端 boot，不绑定地址或 channel：同一 boot 的承载跟随或替换（含[漫游](/v2/node/connectivity#roaming)）保留 CID、nonce 与重放窗口；首个认证帧仍须在源端 15 秒无回复上限内到达 |

**隐私范围：**IP 包通过 Noise 在业务两端之间加密，外层不公开五元组、TCP／echo 状态。CID、方向、序号与密文长度仍公开。中继不解密业务 payload；终点解密后解析真实 IP 包并执行业务准入。

包平面密钥由 `Control::dataplane_keys` 取得（`DataPlaneKeys`），SecureNode 构造时核对其公钥与期望内容的 selfKeys。包平面签名有独立的 purpose／network／node／key generation transcript，不开放任意控制认证消息签名。该句柄是受信进程内接口；密钥不进入 Debug 或序列化，不能视为恶意插件的内存隔离。

当前 wire 标识为 `MWCIR\0\0\x06`。Noise prologue、握手证明与 key-set 使用 candidate-v6 purpose，握手绑定 path 授权摘要；会话 KDF 与相邻 MAC 沿用独立 v5 purpose；通用 `meshway/packet-proof/v2` 控制签名 transcript 保持独立。此格式仍是候选，不是稳定插件 ABI。v4／v5／v6 帧互不兼容，旧版本被拒绝，完整路径节点需要协调升级。IPC v3 和 HTTP 契约独立于该 wire 版本。

握手 UUID 使用固定 16-byte 非零 UUID，数值采用大端。最多 8 个 path 节点、128 KiB 帧、65,519-byte 内层包、255-byte listener key；默认最多 1,024 个 circuit 和 10 秒握手期限。最大握手帧容纳于 4 KiB 握手重试缓存。关闭／失败 circuit 的重放 tombstone 在该 SecureNode owner 生命周期内保留并计入 maxCircuits；permit 过期或同授权重新授予都不清除，耗尽则拒绝新 circuit。重启重新握手，不能恢复旧 cipher state。

包操作始终即时检查目标 circuit。其他 retained circuits 的维护随宿主操作按最早 UTC／单调截止调度；距离上次扫描达到 100 ms 后，下一次操作必定扫描。新增 circuit、执行交接使下次操作重新扫描，显式 `maintain` 总是扫描。该调度只减少无关状态的回收工作，期限／取消立即约束目标操作及原排队输出，关闭后的 tombstone 仍保留并占用容量。

### 会话密钥与紧凑数据帧

1. 每个节点为该 circuit 生成独立的临时 X25519 密钥，与 Noise 的静态／临时密钥分离。INIT 的每跳签名包含自己的公开 contribution、实际 carrier 和此前完整证明。节点保留已验证的上游 contribution 前缀及自己的 contribution。
2. RESPONSE 包含 INIT Noise 消息的 SHA256、完整有序 contribution 列表和 Noise response。每个反向签名绑定整个 RESPONSE、此前证明及签名者自己的 contribution。每个节点必须核对原 INIT digest、缓存前缀和自己的公开值；不能仅信任终点重述的列表。
3. 每个节点仅与路径位置 ±1 的相邻节点做 X25519 agreement；拒绝全零共享秘密。HKDF-SHA256 的 salt 绑定完整 context、两次 Noise 消息和完整 contribution 列表，info 使用 v5 purpose 与有序 signer／verifier 位置。不同方向、会话、授权、路径和 key generation 不共用 MAC key。握手完成后释放临时私钥；相邻 MAC 在派生时预构造 HMAC 0.13／SHA2 0.11 的 inner／outer 状态，每包 clone 后 update，tag 字节不变。SHA2 使用运行时硬件检测与 software fallback；启用 zeroize 清零释放的 hash 状态，原始派生 key 只在 Zeroizing 临时值中存在。HKDF 与 Snow 的 SHA2 0.10 依赖保持共用。
4. Data 固定布局为 `magic/version 8 | kind 1 | flags 1 | CID 32 | sequence 8 | body length 4 | ciphertext P+16 | hop MAC 16`。flags bit0 表示 reply，其余位必须为零；数值使用大端。每段新增 86 B，与路径长度、listener key 和内层地址族无关。未知 CID 不通过线上字段重建授权。握手保留完整 context／证明；Probe 保留自己的 metadata。
5. Data tag 为 `HMAC-SHA256(k(dir), "meshway/circuit/hop-mac/v5" || CID || flags || sequence || body length || ciphertext)[..16]`。验证者常量时间比较相邻 tag；中继验证后替换为下一跳 tag，回复使用反向密钥。实际 carrier 仍由本地 policy 和认证邻接检查，不再放入每包 MAC。相邻认证证明本跳邻居参与；它不提供 v4 的逐验证者完整前缀证明，不能抵抗任意路径节点串通跳过更早节点。端点 Noise AEAD 继续保证业务原始来源真实性。

| 角色 | 每包步骤 |
| --- | --- |
| 源 | 入口 Reading、解析真实 IP、业务流准入、选 circuit／下一跳、消耗 nonce、Noise 加密、写头与相邻 MAC、最终检查与输出 |
| 中继 | 入口 Reading、CID 查上下文、重放预检、相邻 MAC 与实际邻接验证、选原路径下一跳、替换 MAC、最终检查后提交重放 |
| 终点 | 入口 Reading、CID 查上下文、重放预检、相邻 MAC 验证、Noise 解密、真实 IP 解析、业务流准入与方向核对、最终检查后提交重放与交付 |

business 与 diagnostic 共用相邻认证实现，Probe 的 kind、measurement／deadline 使用独立 v5 域进入认证 transcript。业务流表仅由端点业务包更新。每包继续执行当前执行资格、permit、实际 carrier 与重放检查，最终 I/O 使用原 guard。86 B 不包含外层 provider／IP／UDP 开销，较小 underlay MTU 仍可能分片。

### 单缓冲所有权与就地加密

`PacketBuffer` 持有一块存储和当前有效区，提供 `payload`／`payload_mut`、`prepend`／`extend`、`trim_front`／`truncate`。默认 headroom 128 B、tailroom 64 B；owner 创建 `PacketPool` 时可调整。池限制同时在用与空闲的分配数量，耗尽返回错误，不分配溢出槽；每池最多 4096 块、存储容量总计不超过 64 MiB。包归还原池；曾暴露为有效区的全部范围在 drop 清零，包括已 trim／truncate 的明文。取消、解析失败和加密失败使用同一析构路径。

TUN 直接读入池缓冲，端点包解析借用其中切片。Noise 业务加密、Data 头写入 headroom、相邻 MAC 写入 tailroom 均不复制 payload；中继使用借用 Data 解码视图，并在原缓冲上覆盖相邻 MAC。终点就地解密后保留同一缓冲，由 `carrier::deliver` 直接借用写入 TUN。`PendingFrame` 和 `PendingAction`（通过 owned Packet）保有缓冲所有权；`PendingFrame::release` 交出 `PacketBuffer`。

Noise IK、prologue、peer static 验证及空握手 payload 仍由 Snow 完成。完成握手后，私有 crypto 模块使用 Snow 的 `risky-raw-split` 提取原 transport 方向密钥，各构造一次 ring 0.17 ChaCha20-Poly1305 `LessSafeKey`，由不可变 `Arc<Transport>` 交给 worker；每包使用 `seal_in_place_separate_tag`／`open_in_place`。transport AD 保持为空，nonce 为 4 个零字节加 little-endian u64；显式 nonce 仍由唯一 SecureNode 消耗并禁止回绕，接收重放窗口不变。原始 split key 临时值使用 Zeroizing，不进入公共接口、日志或序列化；ring 的不透明 key 随最后句柄释放，但 0.17 没有公开的可靠清零接口，不声明其内部 key drop 清零。该选择避免 Snow 分离输入／输出 API 所需的包体复制；两个方向、最大包长与 nonce 边界与 Snow stateless 输出逐字节互验，wire v5 字节不变。Meshway 业务代码继续 forbid unsafe。

Session 的 `Payload` 可保有 `PacketBuffer` 的原始存储，发送预算包括完整 capacity、headroom 和 tailroom；carrier 不再转换为 Vec。接收的 Session Vec 直接转入 PacketBuffer，Data 解码收回线上头部和 tag 所占空间。自定义 owned payload 在取回时保留同一分配与原池归属。

仍存在的复制：TUN offload 超包拆合、规范化 TUN 包转入 owner 池、UDP provider／Session 接收 scratch 转入独立报文、stream 长度 framing staging／接收 codec、握手／Probe 有界编解码与重试缓存。daemon 与 L3／L4 使用 `Tun::recv_batch_map` 在同步回调中直接复制到 PacketPool，TUN 保留分段存储的高水位槽，避免每包移交再释放临时 Vec；读取、分段、32 包／64 KiB 预算与普通 `recv_batch` 共用实现。`Packet::into_bytes`、`PacketBuffer::into_vec` 和自定义非 PacketBuffer 存储的兼容转换也会复制；不能将当前实现称为内核到内核全链路零复制。

### 发送流水线与连续性

运行循环在一次唤醒中最多接收 32 个已就绪 TUN 包或承载事件，使用正式 TUN／Session 批量接口。业务包的 owner／worker 入口如下：

| 入口 | 边界 |
| --- | --- |
| `SecureNode::prepare_send(circuit, PacketBuffer)` | 入口 Reading、解析、业务准入、路由、nonce 与状态提交；返回不透明 `SendJob` |
| `prepare_send_packet(circuit, Packet)` | 已解析输入复用相同 owner 准入 |
| `pipeline::seal(SendJob)` | worker 就地 AEAD、编码与 hop MAC；返回带原 guard 的 `PendingFrame` |
| `SecureNode::prepare_receive(carrier, PacketBuffer)` | Data CID 查找、实际邻接准入、replay 预检、固定密钥句柄与原 guard |
| `pipeline::open(RecvJob)` | worker 验证 hop MAC、端点就地解密或中继替换下一跳 MAC；返回未提交的 `Opened` |
| `SecureNode::commit_receive(Opened)` | 原 snapshot／key owner 绑定、replay 再检查、端点解析与业务准入、最终检查后提交 replay |

worker 只持有包存储、方向／序号、不可变 Arc 密钥句柄、路由元数据与原输出 guard，不读取 authority，不访问 flow 或 replay。验证失败不会提交 replay；并发验证的重复序号也只有第一次成功 owner 提交有效。关闭后的在途 crypto 可以完成，但不能提交或释放输出；密钥对象在 owner 与最后一个在途句柄释放后销毁，具体清零边界见[就地加密](#单缓冲所有权与就地加密)。

`pipeline::Pool` 默认 **0 worker（inline）**；daemon 可通过 `MESHWAY_DATAPLANE_WORKERS` 显式设置 0–256。当前硬件密码后端、32-credit 窗口下，P 的两核 worker 0／1／2／3 quick 曲线为 1859.8／1216.4／980.8／974.4 Mbps；单在途 L0 两节点 1280 B 的交接墙钟从 inline 约 9 µs 增至单 worker 24.5 µs。两核／四核实测因此支持 inline，不能按 quota 自动增加 worker。只有预期获得 **≥4 个独占核且 AEAD 占比重新升高** 时才考虑 >0，并重新验证同机吞吐、交接、RTT 和生命周期；不是所有 CPU 的最优保证，完整曲线见[有界输出队列实测](/v2/node/performance#有界输出队列实测)。

H／I 的 worker 池保留为可选路径，供高密码成本平台复用同一 crypto 实现、owner 状态边界、保序完成与 credit；不需要另写并行数据面。显式使用 worker 时，`submit`／`flush` 不等待 crypto 完成，每次通知最多八个已就绪 job；worker 逐包发布完成，完成队列从空变为非空时立即通知 owner。owner 在准备新输入前及本轮准备结束后收割 ready 前缀。`take_ready` 每轮最多释放 32 个已完成输出，不等待原输入批次的尾部，也不等待凑批。

流式入口对明文长度不超过 128 B 的包直接执行相同的无状态 crypto；接收按含 Noise tag 的 body 不超过 144 B 判断。该阈值覆盖 52-byte TCP ACK 及较小 IPv6／options 变体，也适用于同长度的其他业务包，不是未经认证的 ACK 特权。小包继续使用同一个 epoch／circuit／方向的序号、credit 和完成队列，不能越过尚未完成的大包。短包原语与交接墙钟依据见[性能诊断](/v2/node/performance#owner-路径与-tcp-拥塞诊断)。

Circuit 保留一次稳定分配；稳态 send／commit 通过拆分字段借用操作唯一 flow／circuit owner，不移出再插回会话表。worker 的完整只读 Context 由原 circuit 的 `Arc` 共享，减少路径 clone 与聚合值移动；入口、flow 提交、nonce／replay、输出及 postcheck 的真实检查保持独立。

每个 epoch／circuit／方向使用独立的 owner 接纳序号和有序完成队列，只释放连续完成前缀；crypto 失败同样占据自己的序号并释放该位置，不提交 replay。队列持有原 epoch 标识直至全部结果释放，失败结果也不能与新 epoch 混合。就绪队列按轮转次序释放，慢 circuit 不阻塞其他 circuit。

流式工作池总计 32 个全生命周期 credit，覆盖已准入、排队、计算、已完成未释放以及 TX／TUN 待完成输出；完成通知或前缀取出不归还 credit，宿主在最终 I/O 完成或明确失败／取消后归还。准入前同时检查包数、实际存储 capacity（含 headroom／tailroom）及 jumbo 数量，不以有效 payload 长度替代占用。完成队列能容纳整个窗口，worker 不因 owner 暂停轮询而阻塞；描述符微批与完成槽复用预分配存储。业务准备每轮另有 200 µs 时间预算。daemon 与 L3／L4 驱动统一使用流式入口。取消不重新绑定在途输出，pool 销毁关闭队列并 join 所有线程。

输出使用 `carrier::SendPending` 与 `TunPending` 两个独立 FIFO 驱动，和输入、crypto completion 一起按就绪状态轮询；不为每包 spawn 任务，不等待凑批。每个方向至多一个正在执行的批次。发送 future 在入队前冻结实际 channel；一次 owner `select!` 等待被取消时，队列继续保有原 future、guard 与 credit。TUN 使用同一个 `Delivery` 实现记录每次成功 syscall 的前缀，取消等待后只继续未写后缀。停止／换代显式取消旧 pending，并计入丢弃计数；不把旧输出迁到新权限。

额度耗尽时，owner 停止读取新 TUN／业务承载批次，已读取而未准入的有界前缀保留原顺序；不会把受背压的队头搬到队尾。L3 直接 UDP 入口停止 socket 读取，后续压力留在内核。生产 Manager 的业务事件消费暂停，而控制事件使用独立 32 槽；业务事件队列满后，共享 UDP socket 的接收 owner 同样停止读取该 socket，压力留在内核接收缓冲。消费者 250 ms 内仍未腾出槽位时，owner 才恢复读取并丢弃未接纳的业务事件，使同一 socket 上的 health／握手／STUN 控制至多等待这一上限，见[控制消息预算](/v2/node/connectivity#udp-控制消息预算)。

#### Pending 存储上限

| 常量／计量 | 上限 |
| --- | ---: |
| 全生命周期包数 `STREAM_CAPACITY` | 32，两个输出方向共用 |
| 普通包 storage class `MTU_STORAGE` | capacity ≤ 4,096 B |
| jumbo（含 64 KiB 大包）`JUMBO_CAPACITY` | 至多 2 包，仍占上述 32 槽 |
| 每个 jumbo 的 `JUMBO_STORAGE` | `MAX_FRAME + 128 + 64` = 131,264 B |
| 全生命周期实际 capacity 总和 `STREAM_BYTES` | `30 × 4,096 + 2 × 131,264` = **385,408 B** |

两个 pending 分别检查包／字节／jumbo 数；其元素必须携带同一个 Pool 已保留的 credit，因而不能叠加成两个 32-credit 窗口。没有因 I/O 等待额外复制整包。元数据另外有界：至多 32 个 pending future／批次、32 个 credit，Pool 预留 32 个 lane × 32 个完成槽及 32 个八项 descriptor group；具体结构字节数随 Rust ABI，以各类型 `size_of` 计。输入预取、空闲 PacketPool、Session 自身预算、TUN／UDP scratch、共享执行配置、线程栈和内核 socket 缓冲分别占用其自己的上限，不能把 385,408 B 当成整个进程的 RSS 上限。取消后的 Session 工作继续受 Session 原有帧／字节预算与取消 guard 约束。

Linux musl 的 `meshwayd` 在可执行入口选择 mimalloc；库不安装分配器，其他目标保留系统分配器。高负载对照将数百 MiB 增长定位到节点匿名驻留：仅复用 TUN 分段存储不足以消除 Manager 场景的增长，替换分配器后的 512 MiB／长窗口曲线见[匿名驻留实测](/v2/node/performance#匿名驻留与分配器对照)。这不是所有配置下的 RSS 硬上限，也不代表满载低延迟目标通过。

#### 进程与容器内存记账

进程匿名内存的预算必须包含分配器，而不只统计尚未释放的包：

```text
RssAnon ≤ H_live,max + F_allocator,max + S_resident,max + A_other,max
H_live,max ≤ PacketPool + pipeline／输入预取 + Manager事件队列
             + ΣSession预算／接收scratch + ΣUDP scratch + ΣTUN状态
             + flow／circuit／配置／任务元数据
```

`H_live,max` 按每块存储的唯一 owner 计数：PacketPool 内正在由 pipeline 持有的同一块不能重复相加。每个 UDP 批量 endpoint 的固定 scratch 为 `32 × 65,536 B`；Session 另有自己的接收 scratch。Manager 业务未准入事件默认同时限制 1,024 帧和至多 8 MiB 实际 capacity；控制事件有独立 32 槽。TUN 读侧包括一个 65,535 B 超包及 vnet header、至多 1,024 个分段；写侧包括至多 32 个超包 staging 和可复用 GROTable。元数据按配置上限和实际类型大小计，线程栈按实际驻留页计。

`F_allocator,max` 包括存活分配之外的碎片、分配器元数据和已释放但未归还 OS 的驻留页；`A_other,max` 包括其他匿名映射。**没有为这些项证明上限时，该公式不是整个进程的数值硬上限。** `VmSize` 中未触碰的虚拟预留不等于 RSS，`smaps_rollup` 的 Anonymous／Pss_Anon 用于核对实际匿名驻留。

启用对应内存记账的 Linux cgroup v2，其 `memory.current` 还计入同组其他进程、文件页和内核内存。`memory.stat` 的 `anon`、`file`、`kernel`、`sock` 分别观察这些归属；`shmem` 已包含在 `file`、`slab` 已包含在 `kernel`，不能再次相加。共享文件页的 cgroup 归属与每个进程 RSS 不同，故各进程 RSS 之和也不等于 `memory.current`。内核启动选项可以关闭 kernel／socket 记账；该环境的零计数不能证明实际无占用。

Mesh UDP socket 默认请求接收 4 MiB、发送 2 MiB（见 [UDP 批量接口](/v2/node/transport#udp-批量接口)），Linux 读回为每 socket 8 MiB／4 MiB；有 `CAP_NET_ADMIN` 时不受 `rmem_max`／`wmem_max` 截断。实际排队的 socket／skb 内存计入 cgroup，**不计入该进程的 RssAnon**。预算需按实际 socket 数（每个 UDP listener 与拨号候选每地址族一个，实际值见 `meshway status` 的 SOCKET 表）、读回上限及两方向计，并为 TUN／skb、页表、slab、应用和采样器留出空间；上限不是启动即全额分配，只在接收方跟不上时被占满。不能把请求值直接当成每 socket 总额，也不能把 `sock` 当作全部网络相关内核内存。真实占用应同时核对 `ss -m` 与 cgroup 分项，不能因 pending 有界就推导 512 MiB 容器不会 OOM。

Manager 在提交时捕获实际 link，按冻结 ChannelScope 分组；换代创建新 scope 标识。业务批次使用所有原 packet guard 与 channel guard 的交集，任何成员失效均阻止该批后续 I/O。每次 syscall／release 重新检查；同一 SecureNode 精确 snapshot 代际的 guards 可共用本次同步 Reading，但仍逐一检查 circuit 取消与各自期限，跨等待不缓存 Reading。握手、诊断、health 与 STUN 保留独立控制路径。

诊断使用独立的加密 challenge request／reply，同一 owner 验证原路径和请求关联；不产生业务 FlowTable 准入。实际发送／接收通道、计时、重放与取消边界见[路径诊断](/v2/node/diagnostics)。

保留的 circuit 继续使用其 Noise／会话 MAC keys、nonce 与重放窗口。已建立 circuit 双向 90 秒无帧时结束；源端连续发送 15 秒未收到任何认证帧时结束，下一个包重新握手。业务 circuit 的端点收到数据后 10 秒内没有发出任何帧时发送一个空 Data 帧（被动 keepalive，中继按普通 Data 转发，接收端不回应、不准入流），因此单向流的活对端总在源端的 15 秒判定内应答。

每个 circuit 绑定建立时各相邻对端的 boot 标识：连接组件在邻接 transcript 中由对端签名的 nonce 前 16 字节给出该对端本次进程状态的标识（同一 Manager 内部重启保持不变）。相邻对端的当前 channel 换成另一个 boot（对端重启）时，SecureNode 结束绑定在它上的 circuit；中继同时向另一侧相邻节点发出 `Close`（kind 8，空负载、sequence 0、相邻 MAC 鉴别，MAC 输入另含 `meshway/circuit/close/v6` 域标签），逐跳传到源端，源端下一个包立即重新握手。同一 boot 的承载更换（例如 UDP 失效改用 TCP 退路）和 channel 暂时缺失都保留 circuit。两端对“相邻节点是否换 boot”的判定对称。boot 标识的是本节点 SecureNode 的 circuit 状态：由 Manager 生成并签入每条邻接，Manager 内部重启保持它；网络 worker 在一次 apply 失败后重建 Manager（`Adjacency apply failed; restarting adjacencies`）时，只要 SecureNode 保留着 circuit，新 Manager 沿用同一 boot（对端把新通道视为同一状态的承载更换，两端都保留 circuit）；进程重启、停用或撤回后的新 Manager 才带新 boot。SecureNode 也绑定本节点 boot（`set_own_boot`，首次只绑定）：换成另一个时立即结束自己持有的全部 circuit，并向仍有承载的相邻节点各发 `Close`（中继照常逐跳传递），源端下一个包立即重新握手，而不是在对端已丢弃的 circuit 上等满 15 秒无回复上限。`Close` 一次性：收到后 CID 进入 dead 记录，不占用数据重放窗口；Data 帧格式和 86 B 开销不变。源端下一跳暂无承载时登记 `AdjacencyInterest`，包在有界首包队列中等待承载恢复或 circuit 结束。

诊断中继的 request correlation 属于已提交会话状态：保留的 circuit 保留 measurement、完成标记和原截止，不延长探测窗口。新收到的 reply 仍须通过当前完整准入、相邻认证、重放与关联检查。

内容应用与包处理由网络 owner 的独占借用串行化，不存在处理中途替换内容的状态。未提交的新流丢弃，已消耗的 nonce 不回滚；到期、取消、关闭及握手密码处理失败保持原有终止规则。

`carrier::send` 把完整 I/O lifetime 随已接纳工作交给正式 transport worker；即使完成等待者暂不被轮询，每次底层 write／flush 也检查同一许可期限和失效条件。`carrier::deliver` 在每次恢复 TUN 写入前检查所有限制。完成只表示本机写入完成，对端接收／应用回包另行证明。

握手在原总期限内按 250 ms 间隔有界重发，不另建 Noise 状态。每 CID 最多缓存两个阶段、每阶段 4 KiB 与 256 次输出；重复 INIT／RESPONSE 仍须先通过 proof 和权限检查，再匹配精确输入并返回原签名输出。缓存保留原 scope、路径、nonce 和期限，交接不重新绑定缓存；迟到调度不突发补发，Closed／过期状态不复活。

## Linux TUN

包处理器只依赖 `meshway_tun::PacketDevice` 的有界读写契约。macOS 实验 `Utun` 接受 NE 的 owned fd，平台设置与验收限制见 [Apple Host](/v2/node/apple-design#apple-host)；以下创建／offload 选项只描述 Linux `Tun`。

`meshway-tun::Tun::create(Options)` 在宿主已经选择的 network namespace 中创建内核自动命名、非持久的 L3 TUN。安全 Rust 层使用固定版本 tun-rs；不接管已有名称，不导出 FD，drop 由内核回收设备。

`Options::new(mtu)` 接受 1280–65535 bytes，可为新设备设置 IPv4／IPv6 地址及前缀。地址带来的本地／直连路由由该 namespace 的内核产生。创建需要已启用 I/O 的 Tokio runtime、既有 `/dev/net/tun` 和相应设备权限。

`recv`／`send` 保留单包接口，短写不续写为第二个报文。daemon 请求 `with_batch_io(true)`，通过 `recv_batch`／`send_batch` 使用 Linux vnet_hdr；按真实能力回退。GSO 拆分与 GRO 合并复用固定版本 tun-rs 移植的 wireguard-go 实现；Meshway 只适配缓冲、批量预算及一次写入的前缀完成语义。拆合与能力口径见[批量 TUN](/v2/node/performance#tun-批量接口与能力验证)。multiqueue 和 packet-information header 仍关闭。外部修改设备 MTU／offload 不受支持；IPv6 DAD 就绪由宿主等待。

### Linux underlay

`meshway-underlay::Host` 是一个 namespace 内本节点 capture／socket 保护的唯一 owner；daemon 驱动其 install／suspend／down／shutdown。Host 独占私有目录锁与 journal，使用新建的 Node-owned TUN、IPv4 `/32`／IPv6 `/128` 地址。`Options::default()` 的 MTU 为 IPv6 最小值 1280；`meshwayd` 装配时使用 `meshway_dataplane::secure::DEFAULT_INNER_MTU` = 1366，即 1500 字节底层链路减去 IPv6/UDP 外层 48 字节与固定 86 字节 Data 帧开销，与路径长度无关。暂无命令行覆盖。已有 TUN 不接管。

`Host::open(directory, options)` 只打开内核 owner 与受保护 `SocketFactory`，不要求 Server origin，也不打开 bootstrap resolver／HTTP 代理；同进程即使使用不同私有目录也不能打开第二个 Host。需要控制联系时，再调用 `host.attach_control(origin)` 初始化 bootstrap resolver 和固定 origin 的私有 UDS relay；同一 Host 不允许替换已有 attachment。`client_builder()` 在未挂接时返回 `ControlUnavailable`。候选 socket 可独立使用同一工厂，按需加载 resolver，并继续以候选的有效执行 scope 解析／收发；不存在未保护工厂。Host 的 Drop／shutdown 同时撤销所有工厂克隆和所属任务。

全部发布地址池都进入拒绝捕获边界；只有本机具有对应地址族的已分配地址时，才安装该族的正向 TUN 路由。缺少某族地址不会解除该族的拒绝边界；无本机地址的中转不据此获得本机发起业务资格。

capture 使用执行配置中完整的 `meshAddressPools`，其[契约](/v2/configuration-reference/node-configuration#capture-与本机-egress)不授予 Access。私有 route table 只包含所属 capture 路由；RPDB 安装 own-mark goto、各池 lookup／unreachable 和 end，受保护 socket 跳到此块末尾后继续原 RPDB，而非强制走 main table。SO_MARK 选择、保留已有 selector 语义与读回验证必须成功；同 UID 其他 socket 不获全局绕过。

安装、保留交接与运行监控均检查优先于 capture 的 local-table 路由和外部接口地址。归属核对只看路由对象：内核由所属路由派生的缓存例外（`RTM_F_CLONED`，例如 fragmentation needed 留下的 PMTU）随父路由存亡，不计入任何表的内容。只有本 owner 的精确 TUN 本机地址路由可豁免；覆盖 Mesh 池的外来 local 路由／地址会阻断安装或使其失效，外部对象保留供宿主处理。

| 生命周期 | 处理 |
| --- | --- |
| 安装／直接新鲜交接 | 验证目标与资源归属；相同 capture 可保留 TUN FD，重新绑定有效快照 |
| 期望 up 时过期／断连／shutdown | 停止正向 I/O，保留历次受管池并集的负向 capture，直到显式 down |
| 显式 down | 停止／join 所属任务，清理可证明所属的规则／路由与 TUN |
| 外部修改或恢复冲突 | 停止正向 I/O 并保留 journal／负向边界，不 flush 外部资源 |

journal 绑定 namespace／boot 身份，记录有界资源及待完成操作。过期 namespace 记录先归档，当前 namespace 的未知归属、未知／未解决操作或资源身份不匹配阻断接管；归档与冲突重试规则见[生命周期](/v2/node/runtime#生命周期)。Linux rule／route 没有 incarnation cookie 或 compare-and-delete：特权主体用完全相同对象替换的 ABA 无法证明，资源归属要求外部不并发修改这些对象。

源端出口捕获（受管外部目的）是 Host 的第二块：区间由本节点用 Policy 从 netmap 规则与持有的 artifact 计算（`Desired::capture`），装为 owned nft interval 集合与私有捕获表路由（一条 RPDB 选择器），未经活动 TUN 的受管目的立即拒绝；暂存、切换、暂停与精确删除的 journal 只记录 gate（区间与设备），表内路由由 gate 推出。机制、上限与实测见 [Routing · 执行捕获](/v2/configuration-reference/routing#执行捕获与精确许可)。

`Options` 默认上限为 64 个池、128 个受保护 socket、单次 netlink 读取 65,536 个内核对象（捕获表最多 32,768 条路由），连接超时 10 秒；一部分 socket 预算为 Server bootstrap 保留。网络安装成功不自动表示 DNS、邻接或应用包流成功。

#### Bootstrap 与普通业务隔离

Server 与获准候选的 bootstrap 只开放用途受限 socket。Server HTTP 使用固定 origin 的私有 UDS relay 接入 reqwest，HTTPS 仍执行正常系统根 TLS 校验；不会为同 IP 的普通应用流量创建全局豁免。候选 handle 绑定已发布的 peer／listener／endpoint 和 Host 已安装的期望内容。

daemon 在使用受管 resolv.conf 时先恢复真实 bootstrap 上游，再依次执行 `Host::open`、`attach_control`，随后启动网络协调器；挂接失败阻断启动并 shutdown 已打开的 Host。独立 DNS anchor 仍由同一内核 owner 管理，其 suspend／down 归属与生命周期不依赖是否挂接控制代理。

bootstrap 名称可使用[启动参数](/v2/node/runtime#启动与接入)的显式 hosts 或真实 nameserver，也读取系统 hosts／真实上游；已知 systemd-resolved stub 可从其上游文件取得真实服务器。未知保护关系的本地 DNS stub 不支持；须提供所需名称的 hosts 地址或显式真实上游。解析失败不回退公共 DNS。宿主系统 DNS、业务 resolver socket 和出口 resolver 不由这些 bootstrap 权限代替。

## DNS 查询 owner

`meshway-dns` 使用 Hickory 编解码、TCP framing 和 DNS 选择规则。`Owner` 持有一个期望内容与有限 pending 请求；其 ticket、路由与回答都是所属 scope 的子取消。

Ticket 绑定 owner 代际与本次操作；DNS request ID 相同不表示同一任务。完成时检查规范名称、网络／源节点、配置 revision 和可验证的地址绑定。`asOf` 是 Server 查询时点，不以本机与 Server 时钟相等作为条件，也不据此制造 TTL。

当前已发布／发现的 MagicDNS 记录均以 TTL 0 应答，没有正／负缓存，也不新增 TTL 来源。已知名称缺少对应地址族返回 NODATA；精确发现的 `record: null` 保持类型化 `NotVisible` 并编码为 REFUSED，无 AA／SOA，不制造可缓存 NXDOMAIN。部分 `mesh_hosts` 缺席先进行精确发现，不能被当作权威 NXDOMAIN。

`publish_scope` 发布非等价 scope 或撤回（`None`）时立即结束旧 scope 的 ticket、路由与尚未发送的回答；`stop` 与 owner 销毁同样如此。

### Service 与 resolver 路由

`Service` 消费预绑定、地址／端口相同的 loopback UDP socket 与 TCP listener，并校验客户端同为 loopback。两种传输共用一个 `OwnerDispatcher`；默认同时最多 128 查询、32 TCP 连接，查询期限 5 秒、TCP idle 30 秒、UDP payload 上限 1232 bytes。每条 TCP 连接顺序处理查询；超预算 UDP／连接丢弃，停止时取消并 join 服务任务。

`OwnerDispatcher` 从 scope 读取期望内容。普通上游仅通过 `ResolverExchange` 消费密封的 `LocalResolverRoute`：其构造在同一内容上检查[精确 localEgress](/v2/configuration-reference/node-configuration#capture-与本机-egress)，绑定源、resolver endpoint、传输、action 和请求期限。单独的 Direct／Unmanaged decision 不能构造该路由，也不是 bootstrap 豁免。

exchange 适配器须在每次 connect／write／read poll 检查路由期限与取消，验证来源、事务及 framing；UDP 许可不隐含 TCP 重试。同内容 scope 保持 owner；新内容取消旧 DNS ticket／回答，不把旧查询移到新 scope。服务对 dispatcher 失败给出原查询期限内的空 SERVFAIL；失效正向回答不能改装为负缓存。

生产 `ResolverExchange` 由 Underlay 的 `SocketFactory` 实现，复用同一 socket 保护机制；UDP／TCP 每次实际 I/O 都检查原 sealed route、请求期限和取消。发送前随机化查询事务 ID，响应经实际来源、原问题及报文／帧边界校验后恢复客户端 ID。DNS scope 由 Engine 的 DNS apply 发布；查询不借用 bootstrap 权限。

### 出口 resolver {#exit-resolver}

源节点 `DnsPlan::ViaExit` 与出口 `DnsPlan::ServeExitResolver` 的语义见 [DNS · 使用出口时](/v2/configuration-reference/dns#使用出口时)。两端都只经唯一 SecureNode 交换查询，没有 Mesh IP 上的 DNS listener。

| 段 | Owner | 行为 |
| --- | --- | --- |
| 源 owner | `meshway-dns` | 普通查询生成 UDP 与 TCP 两条独立 `MeshResolverRoute`，共用原 ticket 与输出取消；UDP 回答带 TC 时才使用 TCP 路由。失败、超时或 SERVFAIL 只结束该查询，不进入本机系统上游队列 |
| 源 → 出口 | dataplane | `query_dns` 在 `Purpose::Dns` circuit 上发送；circuit 的 grant 是 `(Dns, Mesh { exit })`，路径固定为 `DnsService.path`，握手摘要的 selection 是该 service 的摘要。出口能力事实不是 ready 时不建 circuit、不发查询，查询立即失败 |
| 中继 | dataplane | `admit_dns` 在中继自己的投影上重算 `dns_service(origin)` 并只按完整路径位置与相邻承载转发密文，不解密 |
| 出口 ingress | dataplane → `meshway-dns` | 出口收到的请求是不透明 `DnsRequest`，origin、完整路径、传输、所属内容和截止来自已认证 circuit；`OwnerDispatcher::serve_authenticated` 再用 `netmap_dns_service` 匹配 `ServeExitResolver.services` 中该源的精确 service。socket 地址不参与身份判断 |
| 出口上游 | `meshway-dns` → Underlay | `dns.overrideLocal: true` 时为配置的 default resolver（第一个，按其自己的 `LocalResolverRoute`，与本节点自身普通名称相同）；否则为 served 系统快照，每个上游各有 UDP 与独立 TCP `LocalResolverRoute`。两者都由出口自己的 egress 分割（本节点计算，`Desired::partition`）判定并经受保护 socket 发送 |
| 回答 | dataplane | 按请求自身的传输编码：UDP 不超过 1232 bytes，超出时置 TC，由源改用 TCP；无法服务时回 SERVFAIL。`answer_dns` 只沿原 circuit 反向返回，旧请求的 guard 到期后不回答 |

`dns.overrideLocal: true` 的出口不使用系统 resolver：default resolver 地址在 Mesh 池外且 egress 分割为 Direct／Unmanaged（安装时校验，不通过即 DNS 安装失败），因此不经本节点 stub、无环。两个系统后端的语义相同。

否则 served 系统快照按后端区分：

| 后端 | 上游 | 后端不可用条件 |
| --- | --- | --- |
| resolved | resolved 的代理 stub `127.0.0.54:53`，保留 resolved 按链路、按后缀选择上游；不做本地合成、LLMNR、mDNS | 锚点尚未安装或清单读取失败（`unobserved`）；resolved 的服务器清单（`DNSEx`、`FallbackDNSEx`，不含本节点锚点自己的 bind）为空（`noServers`）、含 loopback／未指定／组播或未知族地址（`localServer`）或锚点地址（`anchorServer`）；本 namespace 内 `127.0.0.54:53` 没有已绑定的 UDP socket 与监听中的 TCP socket（`proxyStubMissing`，systemd < 248 或 `DNSStubListener=no`；每 2 秒及 resolved 换进程时重读 `/proc/net/{udp,tcp}`） |
| 显式文件代理 | 安装前记录的原 resolv.conf nameserver（同时是本节点自身未受管名称的上游） | 没有记录（`unobserved`）或为空（`noServers`）；原 nameserver 为 loopback／未指定／组播／带 scope 的链路本地地址或带 `no-reload` 时文件后端本身无法安装 |

快照再按出口的 netmap 判定（`meshway_dns::exit_resolvers`）：任一上游或 resolved 可能选择的服务器属于 Mesh 地址池（`meshServer`），或不是出口 egress 分割里的 Direct／Unmanaged（`exitRoutedServer`），出口查询不可用。查询名落在本节点锚点已安装的域（MagicDNS 域、split 路由、搜索域、安装标记）之下时不交给 resolved，回 SERVFAIL，避免出口用自己的记录回答；出口模式下要用 split 后缀，在源节点把该解析器标 `useWithExit`。源节点其他链路的具体域与出口兜底共存，其下名称由本地网络解析、不经出口，见[其他链路的域](#foreign-domains)。

出口查询的可用性独立于本节点自身 DNS 安装：served 快照不可用只使出口对源回 SERVFAIL，不选公共 DNS 补救，本节点的 MagicDNS 与系统 DNS 照常安装（文件后端的原 nameserver 同时是本节点自身上游，按无出口节点同样校验）。原因由 DNS owner 唯一产生，经 daemon 报告为 IPC Status 的 `exit.dnsUnavailable` 与 State 的 [`exitDns`](/v2/state-reference)。resolved 快照随每次观察（250 ms）续期；服务器清单、代理 stub 或判定结果变化即换新快照并重新发布同内容的 scope，这会结束出口节点自身在途的旧 scope 查询（源的查询按各自截止失败后重试）。代理快照只用于出口查询，出口节点自己的查询仍按 resolved 自身路由。

内容变化（含出口 role 撤回、方案切换）和租约到期按 [scope 发布](#dns-查询-owner) 结束两端旧 ticket；新内容下的 DNS circuit 由 `SecureNode::update` 按 path 授权摘要判定保留或退役，旧回答不能换绑。

可信域名关联（X6）：served 路径回答源节点名称规则决定的名称后，产生一份关联任务完成交 Control 上报（不等待、不扩大 DNS permit，队列满即丢弃）；源节点的 client 路径持有这类回答直至 Engine 应用覆盖其地址的关联（到该查询截止前 250 ms、至多 5 秒，否则 SERVFAIL；其他查询不等它），TTL 截为关联剩余寿命。关联变化不结束 DNS scope 与已接纳查询，也不重判已建立的流；新流由 `Decisions` 以当前关联判定。见 [Exit 设计 · 可信域名关联](/v2/node/exit-design#trusted-dns-associations)。

限制：resolved 后端的出口查询需要 systemd ≥ 248 且 `DNSStubListener=yes`（提供代理 stub `127.0.0.54`），否则出口查询不可用（`proxyStubMissing`），见 [Linux 安装](/v2/install)。出口查询经出口的 IPv4 Mesh 服务；A 与 AAAA 都原样返回出口 resolver 的真实回答，不按出口能力过滤、不伪造 NXDOMAIN；IPv6 目的按 [IPv6 出口](/v2/node/exit-design#ipv6) 捕获与准入。名称规则经多个出口的显式 DNS plan 未实现（保存即拒绝）。DNS 消息上限 4096 bytes，单查询不超过 5 秒。这些范围以外或任何必需安装失败均阻断整体 applied。

## 系统 DNS 安装

宿主 DNS 按本机 bind／backend 安装选项实现，优先使用经验证的现代 systemd-resolved，否则须显式选择并证明所有权的 resolv.conf proxy。resolved 使用 Underlay 独立管理的有地址 DNS 虚拟接口作为路由域锚点，不占用 Server 分配的 Mesh 地址，保留到显式 down；资源或地址冲突明确失败。不能把锚点挂在失效时会删除的业务 TUN 上。

| 后端 | 安装与限制 |
| --- | --- |
| resolved | 使用同一 network namespace 中真实 resolve1 服务；要求系统解析使用其动态维护的 stub resolv.conf，且真实 UDP→TCP 按链路探测通过。探测要求 resolved 把查询送到锚点链路的 loopback DNS 服务器：systemd-resolved 253 起，或含 systemd commit 5f9041af 的稳定版（249.15、250.10、251.11、252.5 起）；更早的版本（如 Ubuntu 22.04 的 249.11）把该查询绑在锚点链路上发出、永远到不了 bind，探测以 `probe`（`Unsupported`）失败并写明这一要求，不回退到文件代理（见 [Linux 安装 · Ubuntu 22.04](/v2/install#ubuntu-2204)）。静态 stub 文件不发布搜索域，当前不作为已支持的系统安装模式。只修改独立锚点的 DNS／domain／安全选项，回读确认；其他链路的域按[其他链路的域](#foreign-domains)判定。 |
| 显式文件代理 | 只替换已验证所有权、祖先目录、mount identity 的普通 `/etc/resolv.conf`，不接管 symlink、硬链接或文件 bind mount。保存私有原文与安装身份，原子替换并同步；只在当前 inode／内容仍匹配时恢复，外部修改保留并报告冲突。不支持[漫游](/v2/node/connectivity#roaming)：保存的原上游属于安装时的网络，DHCP 客户端改写文件即为外来修改；会漫游的节点使用 resolved 后端。 |

#### 其他链路的域 {#foreign-domains}

resolved 按最具体匹配把名称交给路由域最长的链路。锚点安装的计划分两部分判定其他链路（含 resolved 全局设置）的搜索域与 `~domain`，唯一实现为 `SystemDnsPlan::foreign_route`，安装与每次观察（250 ms）都读 resolve1 Manager `Domains`：

| 其他链路的域 | 判定 |
| --- | --- |
| 与计划的显式域（MagicDNS 域、split 路由、搜索域）相等或更具体 | 冲突：`foreign_domain`，拒绝安装；安装后出现则撤回，外来域消失后自动恢复（未知类，见[健康检查](#dns-health)） |
| `~.`（计划自己也有兜底 `~.` 时） | 冲突：`foreign_domain` |
| 其他具体域，而计划对其下名称的覆盖只来自兜底 `~.`（出口方案 `viaExit`、`dns.overrideLocal` 或不可用方案的负向兜底） | 共存：这些后缀由该链路的 resolver 解析，其余名称仍经兜底；安装或集合变化时 INFO `Other links resolve these domains; every other name takes this Node's DNS fallback`（`domains`）一次，IPC Status `localDnsDomains` 与 `meshway status` 的 `Local DNS` 行列出（域、链路序号与名称） |
| 计划没有兜底时的其他域（含 `~.`） | 无关 |

安全说明：共存的后缀（如 OCI DHCP 下发的 `tko.oraclevcn.com`）的查询由本地网络的 resolver 看到并回答，不经出口；出口方案只保证其余名称经出口。需要这些后缀也经出口时，从该链路的 DHCP／网络配置中去掉它们。

两种后端均使用 namespace 级私有预约，避免不同数据目录竞争同一系统 DNS。MagicDNS 域加入系统搜索域；额外搜索域仍按配置执行。resolved 保留未捕获域的原有系统路由，不把原系统的分域配置压平成任意 nameserver 列表。需要这种未实现 fallback 的搜索域组合明确失败。

文件代理只安装本机 stub，原系统上游作为受监控的私有来源保存。其业务查询仍需要精确的 Server local-egress 许可；普通 UDP 系统 resolver 的 TCP 截断重试使用另一个预先获准的 TCP demand，配置为 UDP 的专用 resolver 不隐含 TCP 权限。现阶段不接受 loopback／scoped IPv6 业务 resolver；实际后端安装条件不能由配置选项或地址候选代替。

多个系统上游按原顺序在同一查询 ticket／总期限内重试，按剩余上游数量分配剩余时间。网络失败或子期限到达才尝试下一项；权限失效或取消立即停止。UDP→TCP 重试绑定当前选中的上游及其独立 TCP 许可，不借用另一上游的权限。

显式文件代理仅安装本机 stub；没有当前执行许可时，受其接管的普通查询也返回 SERVFAIL，恢复授权后继续，显式 down 恢复原配置。resolved 分域后端未接管的普通解析继续由原系统处理。保证范围覆盖 daemon、lease 失效、进程退出后保留的本机配置和整机重启（`/run` 中的归属记录随之清空并重新安装），以及 systemd-resolved 重启（Mesh 名称 5 秒内恢复）和停止后再启动（停止期间 fail closed，启动后 10 秒内恢复，无需 `up`）。resolved 的归属由 namespace、Underlay DNS 锚点和锚点链路上回读一致的标记设置共同证明，不绑定 resolved 进程身份；实测见 [Linux 安装与系统服务](/v2/install#systemd-resolved-重启与停止)。文件代理被外部改写后保持冲突，不自动覆盖。应用绕过系统 DNS 的行为不在保证范围内。

启动参数见[本机运行](/v2/node/runtime#启动与接入)。安装未完成时阻断整体 applied。

### 健康检查与恢复 {#dns-health}

DNS owner 每 250 ms 重新证明一次安装（observe），单次期限 2 秒。任一检查失败或超出期限都立即撤回查询许可，网络 worker 随即撤回整个包平面（`Required DNS component failed; suspending packet plane`），Status 为 `lastError: dns`（撤回清理完成之前即如此，见[撤回期间的原因](/v2/node/runtime#ownership-end)）。此后 owner 按后端当前状态完整重证（recover：namespace、预约、锚点、resolve1 owner、链路设置与记录）：未知的故障每秒一次，已证实的每 60 秒一次（重证拒绝而从不覆盖外来设置）；安装探测连续以 `probe`（`Unsupported`）失败时重证前的等待 1 秒起翻倍、上限 60 秒，安装成功即复位（节奏的唯一定义见[恢复语义](/v2/node/runtime#recovery)）。重证成功即以同一 netmap 重装（安装完成时记 `Required DNS component recovered; netmap reinstalled`，无论重装经由 DNS 恢复、Control 的 offer 还是重试），无需 `up` 或重启进程；失败则保持阻断，直到某次重证成功。失败与恢复都由 worker 确认后才算处理过：故障在 worker 看到之前就已重证（例如进程停顿数秒，观察超时后 7 ms 内重证成功）时，worker 仍先撤回再重装，查询许可随重装的 hold 恢复，故障随之清除。

分类与节奏的唯一定义见[本机运行 · 恢复语义](/v2/node/runtime#recovery)；下表是 DNS owner 的判定（`SystemDnsFailure::proven`）。

| 未知：每秒重证，自动恢复（Status `retrying`） | 已证实：保持阻断（Status `blocked`），每 60 秒只读重证，证据消失即恢复 |
| --- | --- |
| resolved 重启（新 bus owner 保留了标记设置；或丢失设置后链路回到原始值，此时重新安装）；`/etc/resolv.conf` 被改写后又恢复为动态 stub；系统停顿使一次检查超时或 netlink 读取失败（宿主范围的 RTNL 占用、CPU 停顿）；外部同名域路由出现后又消失；宿主内核归属的暂态结束（`anchor_ended` 为 `Backend`，由 Host 重证）；系统一侧的 hold／install 失败 | 锚点被删除或改动（Underlay monitor 判为冲突，宿主内核归属以持久冲突结束，`anchor_ended` 为 `Conflict`）；链路设置被外部写成既非已安装值、也非原始值的内容；记录处于中断的设置变更（`record_pending`）或与计划不符（`record_*`，读写失败除外）；预约属于另一个 owner；namespace 改变 |

一次读取被中断或失败，不证明锚点归属有任何变化：锚点读取 socket 随之关闭，下一次读取另开新 socket，锚点本身只在 owner 删除它或宿主内核归属结束时结束。归属结束（见[宿主内核归属的结束与重证](/v2/node/runtime#ownership-end)）后 Host 重证成功时，网络 worker 停止 DNS owner，DNS 在新一期上重新准备并校验同一锚点，随同一 netmap 重装；持久冲突保持阻断，直到 Host 的 60 秒重证读到证据消失。成功的 hold 或 install 清除此前的故障（install 以一次自己的观察结束）。

每次失败记录唯一一项检查，WARN 日志带 `operation`（`observe`／`recover`／`hold`／`install`／`prepare`）、`error`、`check`、`expected`、`actual`、`source` 字段，Status 的 `lastErrorDetail`（[本机运行](/v2/node/runtime)）给出同一内容的一行文本。同一失败（四个字段都相同）只记录一次，重证持续失败时不会每秒重复。`check` 取值：

| `check` | 检查项 | `source` |
| --- | --- | --- |
| `observe_deadline` | 一次 observe 在 2 秒内完成；`actual` 为放弃时的实际耗时，远大于 2 秒说明进程或内核曾停顿 | — |
| `namespace` | 进程仍在预约记录的 network namespace（boot、device／inode、cookie） | `/proc/self/ns/net` |
| `owner_record`／`owner_fenced` | 预约仍属于本 owner；此前的失败已隔离本 owner，须先重证 | 预约文件 |
| `anchor_ended` | Underlay 锚点仍有效：owner 删除了锚点，或宿主内核归属已结束（`actual` 区分暂态阻断与冲突；具体原因见此前那条 WARN） | Kernel DNS anchor |
| `anchor_netlink` | 锚点读取所需的 netlink dump 完成（失败、5 秒超时或超出对象上限） | `rtnetlink RTM_GETLINK`／`RTM_GETADDR`／`RTM_GETROUTE` |
| `anchor_link`／`anchor_address`／`anchor_collision`／`anchor_namespace`／`anchor_identity` | 锚点链路、唯一地址、无其他持有该地址的地址或路由、所在 namespace、身份与记录一致 | 对应 dump |
| `resolved_owner`／`resolved_process`／`resolved_namespace` | `org.freedesktop.resolve1` 有 bus owner，且在本 namespace 中运行 | D-Bus `GetNameOwner`／`GetConnectionUnixProcessID`、`/proc/<pid>/ns/net` |
| `resolv_conf_mode`／`stub_listener` | resolved 报告 `ResolvConfMode=stub`、`DNSStubListener=yes`（后端不支持，error 为 `Unsupported`） | resolve1 Manager 属性 |
| `resolved_link` | resolved 识别锚点链路 | resolve1 `GetLink` |
| `link_settings` | 锚点链路的设置与记录的已安装值（未安装时为原始值）完全一致；`expected`／`actual` 只列出不同字段 | resolve1 Link 属性 |
| `link_settings_applied` | 安装或恢复中的每一步设置回读一致 | resolve1 Link 属性 |
| `foreign_domain` | 没有其他链路的域覆盖本计划的显式域，也没有第二个 `~.`（[其他链路的域](#foreign-domains)） | resolve1 Manager `Domains` |
| `record_*` | resolved 私有记录可读、版本、计划、链路、bind 一致且没有中断的变更 | 记录文件 |
| `probe` | 首次安装的 UDP→TCP 探测通过；没有查询到达锚点的 loopback 服务器时 `expected` 写明所需的 systemd-resolved 版本、`actual` 给出 systemd 版本（读不到为 `systemd version unknown`） | — |
| `resolv_conf` | 显式文件代理的 resolv.conf 检查 | 文件路径 |
| `query_service` | DNS 查询服务仍在服务 bind | — |

## 验收范围 {#daemon-与内核资源-conformance}

本页组件的 TUN、受保护包流与 daemon 整组验收状态统一见[验收范围](/v2/node/validation)。
