# 数据面、TUN 与 DNS 组件

本页定义策略准入、受保护 circuit、Linux TUN、underlay 与 DNS 组件的行为。安装门槛见[网络模式](/v2/node/runtime#安装范围与状态)，验收状态见[验收范围](/v2/node/validation)。

## 策略与包准入

`meshway-policy` 是 Access、主体匹配、routing、topology 和编译规则的实现归属。Server 对可信完整输入编译；Node 使用自己的 `NodeConfiguration`／`PathPermit`，通过 `admission::admit_forward`／`admit_reply` 检查业务元数据与完整路径。

执行发布中的 `mesh_hosts` 也按该节点作为请求源的 Access 裁剪。拥有中继／回复资格、获知路径 peer 的地址，不自动获得其 MagicDNS 名称可见性；预需求发现仍通过精确名称接口完成。

判定检查当前 permit、业务源／目的地址、协议／端口、完整路径、实际相邻 peer 和出口资格。回复必须对应数据面已有的正向流，并沿原完整路径反向运行。规则语义分别见 [Access](/v2/configuration-reference/access) 和 [Routing](/v2/configuration-reference/routing)；NodeID 或 pure predicate 的调用不证明身份认证。

`meshway-dataplane::Engine` 持有一个[执行快照](/v2/node/control#执行快照)和唯一有界流表。`Packet::parse(PacketBuffer)` 验证包并保留原始 buffer；`Packet::parse(&[u8])` 返回借用视图；兼容的 Vec 输入在边界转入带预留空间的存储。`process` 产生 `PendingAction`。宿主必须在最终同步发送／交付边界调用其 `release`，等待期间保留 wrapper 并监听 `invalidated()`。

同步操作在入口取得一次 `ExecutionSnapshot::reading()`，本次 policy 判定、流时间和执行／permit／circuit 截止计算共用其中的配置、UTC 高水位与单调时间。流提交、nonce／replay 消耗、circuit 状态提交及产出前仍执行完整动态最终检查；用于最终截止检查的新读数不重新准入或延长入口确定的期限。`Reading` 只由同步内部调用借用，排队输出保存原快照和截止，跨等待的 `release`／carrier I/O 边界继续独立检查原 guard。本地重试 pacing 与 probe 令牌预算仍以实际扣除预算时的单调时间计时，避免慢操作提前补充预算；它们不延长任何授权截止。

流表内部按双向 tuple 索引查询与冲突检查，按单调／UTC 截止索引回收；刷新和删除同步移除旧截止项，索引大小受同一流容量约束。索引只定位候选，不替代当前 permit、完整路径、协议状态和最终输出 guard 检查。读取不刷新 idle／lease，也不创建反向流。

`Ingress::Peer` 的 original origin、packet producer、previous peer 必须由受信身份层提供。网络入口使用下节的 `SecureNode::receive`，端点验证解密后进入同一流表，中继使用会话级转发；不能把未经认证的 transport payload 直接包装成明文入口输入。

### 包与流边界

| 项目 | 当前范围 |
| --- | --- |
| IPv4 | 精确长度与 checksum；暂不支持 options 或分片 |
| IPv6 | 精确长度；最多两个顺序合法、仅含 padding 的扩展头；拒绝 routing header、全部 fragment header 和 jumbogram |
| 业务协议 | TCP、UDP、ICMP echo；检查端口、协议头及适用 checksum |
| UDP／ICMP | 回复匹配已建立反向 tuple；echo 另绑定 identifier／sequence |
| TCP | 三次握手、每方向 16 MiB 序号历史内的 ACK／重分段重传、前方 16 MiB 内最多 64 个已见乱序区间、FIN 与已建立 RST；支持序号回绕，状态有界。由端点 TCP 处理实际重排／SACK；不缓存包体，不支持 simultaneous-open、TFO 或握手 RST |
| 资源 | 默认 4,096 流、60 秒空闲；最大 65,536 流、24 小时空闲设置；实际寿命仍受 permit 和执行快照约束 |

同一流固定原 demand ID 与完整路径；同一节点／角色范围的活 tuple 在多个 demand ID 之间也不能被另一条路径接管。`LocalFlow` 只读查询同一 FlowTable，供本机源包／原路径回复使用，不刷新流、推进协议状态或形成第二份路由表。SecureNode 中继不创建业务流，也不因此获得本机发起资格；歧义拒绝。

SecureNode 的实际 forward 包通过认证与流准入后，CID 与协议状态在同一流表事务中提交；handshake 完成、读取状态、失败准入和重放均不能建立或改写绑定。`SecureNode::local_circuit` 据此选择本机回包的 cipher，reply 准入再次要求相同 CID，避免从相同 demand／path 的多个本地 Established circuit 中误选未完成全路径交付的旧响应。普通 Engine 流的 cipher 引用为不适用；只有业务源可在旧 cipher 结束后为同一获准路径取得新 cipher。

TCP 乱序区间只记录已准入字节的序号，不制造接收确认：缺口补齐前不推进连续 ACK 边界，不能用“最高见过的序号”接受越过缺口的 ACK。空 ACK 不创建区间或推进边界；重叠重分段可以补充新后缀。FIN 的字节位置一经准入不能改变，也不能重新解释已准入的数据；RST 仍要求精确连续前沿。区间容量、窗口边界或协议检查失败不提交状态。此前丢失的包不会因此自动重发，重传和实际接收窗口仍由两端 TCP 拥有。

`Engine::reconcile` 使用 Control 的[直接连续性交接](/v2/node/control#执行快照)，仅保留原授权与完整路径仍相同的活流。比较使用排除 `expiresAt` 的 `permit_authorization_digest`，同时检查路径相关身份、地址与 key 绑定。换 instance、撤回、取消、跳过 publication 或旧期限已到都不迁移状态。

交接本身保守保留流的旧 idle／permit／快照截止，不凭读状态延长流。只有旧流仍有效时，实际获准包通过同一协议状态机才能刷新其期限；已排队输出继续受原截止与 guard 约束。拒绝包不建立或推进流状态。

## 候选受保护 circuit

逐层图与运行成本见[包流分层与加密讨论](/v2/discussions/packet-layers)，线上字段与规模约束见 [Packet 字节账本](/v2/discussions/packet)。本页定义当前行为；讨论中的候选方案不改变这些约束。

`meshway-dataplane::secure::SecureNode` 是同一执行快照下的 circuit 和流状态 owner。`start` 发起握手，`receive` 验证承载帧，`send` 接收本机 IP 包。业务源与终点使用相同的 `Engine`／`meshway-policy`；中继仅按已建立 circuit 执行会话级转发，不解析业务 facts、不建立业务流。

| 边界 | 当前实现 |
| --- | --- |
| 端到端身份／加密 | Snow 的 `Noise_IK_25519_ChaChaPoly_SHA256`，静态 X25519 公钥取自本机 Server 执行配置；每次握手使用新 ephemeral |
| 握手绑定 | network、明确区分 business demand／diagnostic observation 的授权引用、完整有序 path、随机 circuit ID、permit 授权摘要（排除 expiresAt）、路径上全部控制／数据面 key ID、generation 和公钥 |
| 早期数据 | 两个 Noise 握手消息的业务 payload 必须为空；握手不创建业务流 |
| 路径证明 | 握手使用有序 Ed25519 证明认证各节点独立临时 X25519 contribution；稳态使用一个方向独立的相邻 HMAC-SHA256 tag，完整原路径保留在已认证上下文 |
| 承载约束 | carrier 包含实际 dialer、listener、listener key 与协议；验证该连接在所选完整路径的候选集合内，拨号方向与业务方向分别处理 |
| 重放 | 每 circuit／方向独立的 64-bit nonce 和 1,024 包窗口；nonce 不回绕，只有通过认证与准入的包推进接收窗口 |
| 生命周期 | circuit、流、permit 与 Control 截止取交集；关闭、销毁、失效同时取消排队密文和待交付明文 |

**隐私范围：**IP 包通过 Noise 在业务两端之间加密，外层不公开五元组、TCP／echo 状态。CID、方向、序号与密文长度仍公开。中继不解密业务 payload；终点解密后解析真实 IP 包并执行业务准入。

密钥通过 `Control::dataplane_identity` 从有效执行快照取得。包平面签名有独立的 purpose／network／node／key generation transcript，不开放任意控制认证消息签名。该句柄是受信进程内接口；密钥不进入 Debug 或序列化，不能视为恶意插件的内存隔离。

当前 wire 标识为 `MWCIR\0\0\x05`。Noise prologue 与握手证明保留原 purpose（prologue 同时绑定新的 magic），会话 KDF 与相邻 MAC 使用独立 v5 purpose；通用 `meshway/packet-proof/v2` 控制签名 transcript 保持独立。此格式仍是候选，不是稳定插件 ABI。v4／v5 帧不兼容，旧版本被拒绝，完整路径节点需要协调升级。IPC v3 和 HTTP 契约独立于该 wire 版本。

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

仍存在的复制：TUN offload 超包拆合、规范化 TUN 包转入 owner 池、UDP provider／Session 接收 scratch 转入独立报文、stream 长度 framing staging／接收 codec、握手／Probe 有界编解码与重试缓存。`Packet::into_bytes`、`PacketBuffer::into_vec` 和自定义非 PacketBuffer 存储的兼容转换也会复制；不能将当前实现称为内核到内核全链路零复制。

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

`pipeline::Pool` 默认 worker 数为可用核数减一、最少一；daemon 可通过 `MESHWAY_DATAPLANE_WORKERS` 设置 0–256，0 为同步 inline。`submit`／`flush` 不等待 crypto 完成，每次通知最多八个已就绪 job；worker 逐包发布完成，完成队列从空变为非空时立即通知 owner。owner 在准备新输入前及本轮准备结束后收割 ready 前缀。`take_ready` 每轮最多释放 32 个已完成输出，不等待原输入批次的尾部，也不等待凑批。

流式入口对明文长度不超过 128 B 的包直接执行相同的无状态 crypto；接收按含 Noise tag 的 body 不超过 144 B 判断。该阈值覆盖 52-byte TCP ACK 及较小 IPv6／options 变体，也适用于同长度的其他业务包，不是未经认证的 ACK 特权。小包继续使用同一个 epoch／circuit／方向的序号、credit 和完成队列，不能越过尚未完成的大包。短包原语与交接墙钟依据见[性能诊断](/v2/node/performance#owner-路径与-tcp-拥塞诊断)。

Circuit 保留一次稳定分配；稳态 send／commit 通过拆分字段借用操作唯一 flow／circuit owner，不移出再插回会话表。worker 的完整只读 Context 由原 circuit 的 `Arc` 共享，减少路径 clone 与聚合值移动；入口、flow 提交、nonce／replay、输出及 postcheck 的真实检查保持独立。

每个 epoch／circuit／方向使用独立的 owner 接纳序号和有序完成队列，只释放连续完成前缀；crypto 失败同样占据自己的序号并释放该位置，不提交 replay。队列持有原 epoch 标识直至全部结果释放，失败结果也不能与新 epoch 混合。就绪队列按轮转次序释放，慢 circuit 不阻塞其他 circuit。

工作池总计 32 个全生命周期 credit，覆盖已准入、排队、计算、已完成未释放以及 TX／TUN 待完成输出；完成通知或前缀取出不归还 credit，宿主在最终 I/O 完成或明确失败后归还。owner 在业务准入前检查额度，未准入输入留在独立有界队列。完成队列能容纳整个窗口，worker 不因 owner 暂停轮询而阻塞；描述符微批与完成槽复用预分配存储。业务准备每轮另有 200 µs 时间预算。`run`／`run_prepared` 保留为同步批次调用方的便利入口；daemon 与 L3／L4 驱动使用流式入口。取消不重新绑定在途输出，pool 销毁关闭队列并 join 所有线程。

当前限制：crypto 完成已与下一批输入解耦，但宿主仍等待本次已释放前缀的 carrier／TUN I/O；该 I/O 真正挂起时，业务 owner 的下一轮输入仍会等待。Manager 的健康接收／计时任务独立推进，原 guard 继续约束 I/O。独立的有界 TX pending 驱动尚未实现；现有墙钟观察、实际吞吐与满载排队边界见[性能诊断](/v2/node/performance#owner-路径与-tcp-拥塞诊断)。

Manager 在提交时捕获实际 link，按冻结 ChannelScope 分组；换代创建新 scope 标识。业务批次使用所有原 packet guard 与 channel guard 的交集，任何成员失效均阻止该批后续 I/O。每次 syscall／release 重新检查；同一 SecureNode 精确 snapshot 代际的 guards 可共用本次同步 Reading，但仍逐一检查 circuit 取消与各自期限，跨等待不缓存 Reading。握手、诊断、health 与 STUN 保留独立控制路径。

诊断使用独立的加密 challenge request／reply，同一 owner 验证原路径和请求关联；不产生业务 FlowTable 准入。实际发送／接收通道、计时、重放与取消边界见[路径诊断](/v2/node/diagnostics)。

`SecureNode::reconcile` 在相同授权、原路径和有效直接交接下保留 Noise／会话 MAC keys、nonce、重放窗口与同一流表。显式授权续租可在旧 UTC／单调期限到达前延长活 circuit 的期限；已关闭／过期 circuit 不复活，握手期限也不由续租重置。已签发输出保留旧 guard；流截止按上节处理。

执行快照在同步包处理期间被替换时，该操作及其旧输出立即拒绝；快照失效本身不等于 circuit 的永久关闭。尚在原期限内的密码状态、已提交流状态和 nonce／重放窗口仅作不可执行的待交接状态保留，仍须由上述 Control 连续性检查决定能否接续。未提交的新流丢弃，已消耗的 nonce 不回滚；实际到期、取消、关闭及握手密码处理失败保持原有终止规则。

`carrier::send` 把完整 I/O lifetime 随已接纳工作交给正式 transport worker；即使完成等待者暂不被轮询，每次底层 write／flush 也检查同一许可期限和失效条件。`carrier::deliver` 在每次恢复 TUN 写入前检查所有限制。完成只表示本机写入完成，对端接收／应用回包另行证明。

握手在原总期限内按 250 ms 间隔有界重发，不另建 Noise 状态。每 CID 最多缓存两个阶段、每阶段 4 KiB 与 256 次输出；重复 INIT／RESPONSE 仍须先通过 proof 和权限检查，再匹配精确输入并返回原签名输出。缓存保留原 scope、路径、nonce 和期限，交接不重新绑定缓存；迟到调度不突发补发，Closed／过期状态不复活。

## Linux TUN

`meshway-tun::Tun::create(Options)` 在宿主已经选择的 network namespace 中创建内核自动命名、非持久的 L3 TUN。安全 Rust 层使用固定版本 tun-rs；不接管已有名称，不导出 FD，drop 由内核回收设备。

`Options::new(mtu)` 接受 1280–65535 bytes，可为新设备设置 IPv4／IPv6 地址及前缀。地址带来的本地／直连路由由该 namespace 的内核产生。创建需要已启用 I/O 的 Tokio runtime、既有 `/dev/net/tun` 和相应设备权限。

`recv`／`send` 保留单包接口，短写不续写为第二个报文。daemon 请求 `with_batch_io(true)`，通过 `recv_batch`／`send_batch` 使用 Linux vnet_hdr；按真实能力回退。GSO 拆分与 GRO 合并复用固定版本 tun-rs 移植的 wireguard-go 实现；Meshway 只适配缓冲、批量预算及一次写入的前缀完成语义。拆合与能力口径见[批量 TUN](/v2/node/performance#tun-批量接口与能力验证)。multiqueue 和 packet-information header 仍关闭。外部修改设备 MTU／offload 不受支持；IPv6 DAD 就绪由宿主等待。

### Linux underlay

`meshway-underlay::Host` 是一个 namespace 内本节点 capture／socket 保护的唯一 owner；daemon 驱动其 install／suspend／down／shutdown。Host 独占私有目录锁与 journal，使用新建的 Node-owned TUN、IPv4 `/32`／IPv6 `/128` 地址。`Options::default()` 的 MTU 为 IPv6 最小值 1280；`meshwayd` 装配时使用 `meshway_dataplane::secure::DEFAULT_INNER_MTU` = 1366，即 1500 字节底层链路减去 IPv6/UDP 外层 48 字节与固定 86 字节 Data 帧开销，与路径长度无关。暂无命令行覆盖。已有 TUN 不接管。

`Host::open(directory, options)` 只打开内核 owner 与受保护 `SocketFactory`，不要求 Server origin，也不打开 bootstrap resolver／HTTP 代理；同进程即使使用不同私有目录也不能打开第二个 Host。需要控制联系时，再调用 `host.attach_control(origin)` 初始化 bootstrap resolver 和固定 origin 的私有 UDS relay；同一 Host 不允许替换已有 attachment。`client_builder()` 在未挂接时返回 `ControlUnavailable`。候选 socket 可独立使用同一工厂，按需加载 resolver，并继续以候选的有效执行 scope 解析／收发；不存在未保护工厂。Host 的 Drop／shutdown 同时撤销所有工厂克隆和所属任务。

全部发布地址池都进入拒绝捕获边界；只有本机具有对应地址族的已分配地址时，才安装该族的正向 TUN 路由。缺少某族地址不会解除该族的拒绝边界；无本机地址的中转不据此获得本机发起业务资格。

capture 使用执行配置中完整的 `meshAddressPools`，其[契约](/v2/configuration-reference/node-configuration#capture-与本机-egress)不授予 Access。私有 route table 只包含所属 capture 路由；RPDB 安装 own-mark goto、各池 lookup／unreachable 和 end，受保护 socket 跳到此块末尾后继续原 RPDB，而非强制走 main table。SO_MARK 选择、保留已有 selector 语义与读回验证必须成功；同 UID 其他 socket 不获全局绕过。

安装、保留交接与运行监控均检查优先于 capture 的 local-table 路由和外部接口地址。只有本 owner 的精确 TUN 本机地址路由可豁免；覆盖 Mesh 池的外来 local 路由／地址会阻断安装或使其失效，外部对象保留供宿主处理。

| 生命周期 | 处理 |
| --- | --- |
| 安装／直接新鲜交接 | 验证目标与资源归属；相同 capture 可保留 TUN FD，重新绑定有效快照 |
| 期望 up 时过期／断连／shutdown | 停止正向 I/O，保留历次受管池并集的负向 capture，直到显式 down |
| 显式 down | 停止／join 所属任务，清理可证明所属的规则／路由与 TUN |
| 外部修改或恢复冲突 | 停止正向 I/O 并保留 journal／负向边界，不 flush 外部资源 |

journal 绑定 namespace／boot 身份，记录有界资源及待完成操作。未知归属、未知／未解决操作、身份不匹配阻断恢复。Linux rule／route 没有 incarnation cookie 或 compare-and-delete：特权主体用完全相同对象替换的 ABA 无法证明，资源归属要求外部不并发修改这些对象。

`Options` 默认上限为 64 个池、128 个受保护 socket、16,384 个内核对象，连接超时 10 秒；一部分 socket 预算为 Server bootstrap 保留。网络安装成功不自动表示 DNS、邻接或应用包流成功。

#### Bootstrap 与普通业务隔离

Server 与获准候选的 bootstrap 只开放用途受限 socket。Server HTTP 使用固定 origin 的私有 UDS relay 接入 reqwest，HTTPS 仍执行正常系统根 TLS 校验；不会为同 IP 的普通应用流量创建全局豁免。候选 handle 绑定已发布的 peer／listener／endpoint 和有效执行快照。

daemon 在使用受管 resolv.conf 时先恢复真实 bootstrap 上游，再依次执行 `Host::open`、`attach_control`，随后启动网络协调器；挂接失败阻断启动并 shutdown 已打开的 Host。独立 DNS anchor 仍由同一内核 owner 管理，其 suspend／down 归属与生命周期不依赖是否挂接控制代理。

bootstrap 名称可使用[启动参数](/v2/node/runtime#启动与接入)的显式 hosts 或真实 nameserver，也读取系统 hosts／真实上游；已知 systemd-resolved stub 可从其上游文件取得真实服务器。未知保护关系的本地 DNS stub 不支持；须提供所需名称的 hosts 地址或显式真实上游。解析失败不回退公共 DNS。宿主系统 DNS、业务 resolver socket 和出口 resolver 不由这些 bootstrap 权限代替。

## DNS 查询 owner

`meshway-dns` 使用 Hickory 编解码、TCP framing 和 DNS 选择规则。`Owner` 持有同一个执行快照与有限 pending 请求；宿主用唯一 Control driver 执行 `Control::query_dns`，将类型化响应交回 `complete_discovery`。

Ticket 绑定 owner 代际与本次操作；DNS request ID 相同不表示同一任务。完成时检查规范名称、网络／源节点、配置 revision 和可验证的地址绑定。`asOf` 是 Server 查询时点，不以本机与 Server 时钟相等作为条件，也不据此制造 TTL。

当前已发布／发现的 MagicDNS 记录均以 TTL 0 应答，没有正／负缓存，也不新增 TTL 来源。已知名称缺少对应地址族返回 NODATA；精确发现的 `record: null` 保持类型化 `NotVisible` 并编码为 REFUSED，无 AA／SOA，不制造可缓存 NXDOMAIN。部分 `mesh_hosts` 缺席先进行精确发现，不能被当作权威 NXDOMAIN。

`refresh`、`replace`、`stop` 或 owner 销毁取消旧 ticket 与尚未发送的回答；失效 Control 快照同样阻止回答交付。

### Service 与 resolver 路由

`Service` 消费预绑定、地址／端口相同的 loopback UDP socket 与 TCP listener，并校验客户端同为 loopback。两种传输共用一个 `OwnerDispatcher`；默认同时最多 128 查询、32 TCP 连接，查询期限 5 秒、TCP idle 30 秒、UDP payload 上限 1232 bytes。每条 TCP 连接顺序处理查询；超预算 UDP／连接丢弃，停止时取消并 join 服务任务。

`OwnerDispatcher` 接受执行快照与 resolver key → demand ID 的有界映射，使用 `DiscoveryGateway` 调用宿主唯一 Control driver。普通上游仅通过 `ResolverExchange` 消费密封的 `LocalResolverRoute`：其构造重新检查同一 publication 中[精确 localEgress](/v2/configuration-reference/node-configuration#capture-与本机-egress)，绑定源、resolver endpoint、传输、profile、action 和期限。单独的 Direct／Unmanaged decision 不能构造该路由，也不是 bootstrap 豁免。

exchange 适配器须在每次 connect／write／read poll 检查路由期限与取消，验证来源、事务及 framing；UDP 许可不隐含 TCP 重试。scope 更新（包括同目标续租）取消旧 DNS ticket／回答，不把旧查询移到延长后的 scope。服务对 dispatcher 失败给出原查询期限内的空 SERVFAIL；失效正向回答不能改装为负缓存。

生产 `ResolverExchange` 由 Underlay 的 `SocketFactory` 实现，复用同一 socket 保护机制；UDP／TCP 每次实际 I/O 都检查原 sealed route、执行期限和取消。发送前随机化查询事务 ID，响应经实际来源、原问题及报文／帧边界校验后恢复客户端 ID。daemon 经唯一 Control owner 预先建立、续租 resolver 需求，再以原子刷新所得的快照安装 DNS scope；查询不借用 bootstrap 权限。

出口 resolver、显式出口 DNS plan 和可信域名关联入口仍未接线，见 [DNS](/v2/configuration-reference/dns#使用出口时)。这些范围或任何必需安装失败均阻断整体 applied。

## 系统 DNS 安装

宿主 DNS 按本机 bind／backend 安装选项实现，优先使用经验证的现代 systemd-resolved，否则须显式选择并证明所有权的 resolv.conf proxy。resolved 使用 Underlay 独立管理的有地址 DNS 虚拟接口作为路由域锚点，不占用 Server 分配的 Mesh 地址，保留到显式 down；资源或地址冲突明确失败。不能把锚点挂在失效时会删除的业务 TUN 上。

| 后端 | 安装与限制 |
| --- | --- |
| resolved | 使用同一 network namespace 中真实 resolve1 服务；要求系统解析使用其动态维护的 stub resolv.conf，且真实 UDP→TCP 按链路探测通过。静态 stub 文件不发布搜索域，当前不作为已支持的系统安装模式。只修改独立锚点的 DNS／domain／安全选项，回读确认；外部相同或更精确的域路由冲突时拒绝安装。 |
| 显式文件代理 | 只替换已验证所有权、祖先目录、mount identity 的普通 `/etc/resolv.conf`，不接管 symlink、硬链接或文件 bind mount。保存私有原文与安装身份，原子替换并同步；只在当前 inode／内容仍匹配时恢复，外部修改保留并报告冲突。 |

两种后端均使用 namespace 级私有预约，避免不同数据目录竞争同一系统 DNS。MagicDNS 域加入系统搜索域；额外搜索域仍按配置执行。resolved 保留未捕获域的原有系统路由，不把原系统的分域配置压平成任意 nameserver 列表。需要这种未实现 fallback 的搜索域组合明确失败。

文件代理只安装本机 stub，原系统上游作为受监控的私有来源保存。其业务查询仍需要精确的 Server local-egress 许可；普通 UDP 系统 resolver 的 TCP 截断重试使用另一个预先获准的 TCP demand，配置为 UDP 的专用 resolver 不隐含 TCP 权限。现阶段不接受 loopback／scoped IPv6 业务 resolver；实际后端安装条件不能由配置选项或地址候选代替。

多个系统上游按原顺序在同一查询 ticket／总期限内重试，按剩余上游数量分配剩余时间。网络失败或子期限到达才尝试下一项；权限失效或取消立即停止。UDP→TCP 重试绑定当前选中的上游及其独立 TCP 许可，不借用另一上游的权限。

显式文件代理仅安装本机 stub；没有当前执行许可时，受其接管的普通查询也返回 SERVFAIL，恢复授权后继续，显式 down 恢复原配置。resolved 分域后端未接管的普通解析继续由原系统处理。保证范围覆盖 daemon、lease 失效和进程退出后保留的本机配置；整机重启、系统 resolver 服务停机及应用绕过系统 DNS 的行为不在当前保证范围内。

启动参数见[本机运行](/v2/node/runtime#启动与接入)。安装未完成时阻断整体 applied。

## 验收范围 {#daemon-与内核资源-conformance}

本页组件的 TUN、受保护包流与 daemon 整组验收状态统一见[验收范围](/v2/node/validation)。
