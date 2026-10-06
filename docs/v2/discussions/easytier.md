---
title: EasyTier 源码对照 · 设计讨论
description: 固定版本的 EasyTier packet、密钥、relay 与 buffer 实现，及其与 Meshway 的语义差异。
---

# EasyTier 源码对照

::: info 固定提交研究，未运行性能对照
上游：[EasyTier/Easytier](https://github.com/EasyTier/Easytier)；本地独立 checkout：`references/easytier`。研究基线为 `main` 的 **`728ba94b5029240add6204eed591f22e4e039e6b`**，研究日期 2026-10-06，根 [LICENSE][license] 为 GNU LGPL v3。

本页整合 **EasyTier 子代理的源码研究**，关键默认值、派生代码、AEAD 覆盖与可选 QUIC 实现已另行核对。结论只覆盖链接所指源码，不代表项目所有版本、build features 或运行配置；没有由此得到吞吐倍数、性能平价或完整安全审计结论。
:::

## 先固定比较模式

本次提交的默认配置是 `enable_encryption=true`、**AES-128-GCM**、secure mode 未开启、compression=None、`default_protocol=tcp`。默认编译带有 WG / QUIC 不等于运行时默认选它们。依据：[默认 flags][defaults]、[默认算法][default-aes]、[连接包装中的 secure-mode 选择][conn-wrap]。

因此，下面把普通同网的传统模式、显式 secure mode、可选 provider 分开。Meshway 对照对象是[当前 candidate-v4](/v2/discussions/packet-layers)，不要把两边名字里相同的 Noise、UDP 或“relay”当作相同的授权和攻击边界。

## 实际包流：有队列，也有不同的中继职责

```text
TUN AsyncDevice / TunStream（BytesMut）
  → NicCtx 的 NIC→peers task
  → owned HostPacket → CorePacketPlane
  → PeerOutboundPacketRouter
     Outbound ACL / pipeline / IP→peer route / 可选 compression
  → payload encryption（传统或 secure session）
  → PeerConn / RelayPeerMap
  → MpscTunnel（容量32）→ provider → socket

socket / provider Stream
  → PeerConn receive task → PeerPacketEnvelope / mpsc（容量128）
  → PeerInboundPacketRouter
     ├─ transit：forward_counter + next-hop 路由 → 转发 ciphertext
     └─ local：decrypt / decompress / ACL / pipeline
  → HostPacket channel（容量128，满则丢）→ TUN writer task
```

入口：[native NIC task][nic]；[core packet seam][plane]；[出站处理][outbound]；[接收与转发][inbound]；[接收 envelope / queue][peer-queue]。

- 普通 transit relay 读取公开 overlay 目的 peer 并重新选择 next hop，增加 `forward_counter`；这个计数**不是 replay sequence，也不是经过哪些节点的证明**。
- secure transit packet 保留原始端点 session ciphertext。`RelayPeerMap::send_msg` 在 `from != self` 时直接转发，不建立另一层 payload pair encryption。[relay send][relay-send]、[session filter][session-filter]。
- B→A 的业务回复从 B 的 TUN 重新进入路由，使用当时的 next hop。所查实现没有从正向包恢复 `[B,R,A]` 完整逆序列表；同一 PeerConn 的 Ping / Pong 回应不能扩大成业务原路径回复保证。
- foreign-network relay 另有 wrapper，可能复制并嵌套另一份 overlay packet。[foreign wrapper][foreign]。其大小和密钥边界不能套用下面普通同网表格。

## 数据帧：固定路由头，不含完整必经路径

Overlay header 精确为 **16 bytes**；整数在该 header 中为 little-endian。依据：[packet header][headers]。

| Offset | 长度 | 字段 |
| --- | ---: | --- |
| 0 | 4 | `from_peer_id`，u32 |
| 4 | 4 | `to_peer_id`，u32 |
| 8 | 1 | `packet_type` |
| 9 | 1 | `flags`，含 encrypted / compressed 等 |
| 10 | 1 | `forward_counter` |
| 11 | 1 | reserved；liveness token 可能使用此 byte |
| 12 | 4 | `len`，填 header 时的 payload 长度；不应等同后续 AEAD 后 wire 长度 |

普通 AEAD 数据为：`overlay16 || ciphertext(P) || tag16 || nonce12`。尾部是 **28 bytes**，不是文件旁旧注释中的 “Encrypted 20B”。[StandardAeadTail][tail] 明确给出 tag／nonce 字段；[Ring encryptor][aead] 给出实际追加和移除方式。

### 只比较字段预算

`P` 是**完整内层 IP 包**，不是 application payload。假设无压缩、无 foreign wrapper；下表不计 L2 或 IP options／扩展头。

| 模式 | provider 中的数据长度 | 加外层 IPv4 基础头后 | 加外层 IPv6 基础头后 |
| --- | ---: | ---: | ---: |
| EasyTier UDP ＋ overlay AEAD | P＋8（UDP provider）＋16＋28＝**P＋52** | P＋80 | P＋100 |
| EasyTier TCP ＋ overlay AEAD | P＋4（length）＋16＋28＝**P＋48** 的 stream record | 单 segment 最小预算 P＋88 | 单 segment 最小预算 P＋108 |
| Meshway v4 Native UDP，3节点最后一段，key=`lab-udp` | [P＋139](/v2/discussions/packet#用一条公式覆盖路径的每一段) | P＋167 | P＋187 |

TCP 的 options、ACK、重传、合并和分段另算，不能将其一条 stream record 当作抓包中固定大小的 TCP packet。EasyTier UDP provider 的 8 bytes 为 conn_id4、msg_type1、padding1、len2；TCP provider 的长度为 LE u32。[UDP codec][udp-codec]、[TCP framing][framing]。

`P=1280` 时，EasyTier 普通 AEAD UDP payload 推导为 **1332 bytes**；Meshway 所选最后一段为 **1419 bytes**。差额 87 bytes 可拆为：

| 字节项 | EasyTier 普通 AEAD UDP | Meshway v4 本例 |
| --- | ---: | ---: |
| overlay header（含各自的长度／序号等字段） | 16 | 84 |
| 加密的完整 IP 包 | 1280 | 1280 |
| AEAD tag／附带 nonce | 28 | 16（sequence 已在前一行） |
| carrier 前缀＋逐验证者 MAC | 未见同类字段 | 39 |
| UDP provider framing | 8 | 0 |
| 合计 | **1332** | **1419** |

这只解释字段账本。EasyTier 没有在该头中表达 Meshway 的 PacketFacts、完整路径上下文与 carrier 证明；更小的头不是同等安全语义下性能差距的证据。

## 密钥模型：传统与 secure 要分别看

| 问题 | 传统 network-secret 模式 | 显式 secure mode |
| --- | --- | --- |
| 正常 payload key 来自哪里 | manager 从 network_secret 派生共享 payload key | PeerSessionStore 保存端点 pair session；responder 随机生成 root key，经 Noise 消息运送 |
| 握手 | network identity / feature / secret digest 等 legacy Handshake | 直接 PeerConn 用 Noise XX；经中继的端点握手用 Noise IK |
| 稳态加密 | 所选 AES-GCM 等 encryptor | 独立 SecureDatagramSession；HKDF 从 root、epoch、方向派生 traffic keys，**不是直接使用 Noise split transport 格式** |
| nonce | AEAD 路径随机 12 bytes | 12 bytes＝epoch BE u32＋sequence BE u64 |
| replay | 所查传统 Data 路径未发现 sequence/replay window | current / previous epoch、每方向 256-bit window，解密前 precheck、成功后 commit；有 idle 清理／SYNC grace 等边界 |
| 正常中继做不做解密 | 转发分支不调用 decrypt | transit 保留端点 ciphertext，不再为它叠加邻接 pair encryption |
| 中继能否导出同一 payload key | 持有同网 secret 的节点可派生；不能称为 pair-key 机密性隔离 | 普通 transit relay 不因此拥有 A↔B 的 pair root key；仍需检查实际握手、static-key 信任和成员认证等级 |

### 不宜从名字推断的细节

1. 传统 `derive_key_128/256` 实际使用 `DefaultHasher`，不是口令 KDF 或 DH。[派生函数][derive]、[manager key 初始化][network-key]。这里只如实记录研究版本，不将该派生方式作为 Meshway 候选建议。
2. secure 的直接握手与 relay 握手分别为 [Noise XX][noise-direct]、[Noise IK][noise-relay]；同一 PeerSessionStore 可关联多条连接。[session store][sessions]。peer ID 仍是 lookup 标识，不是可信公钥本身。
3. Noise 握手后的认证等级还区分 network-secret confirmation、peer verification 和特定路径的 encrypted-unauthenticated 状态；“用了 Noise”本身不保证它们等价。[认证判定][noise-auth]。同网最终准入拒绝 secure-mode 不一致，不能把 server 能解析 legacy 理解为同网 secure 静默降级。[同网准入][admission]。
4. secure traffic epoch 每百万包或 10 分钟轮换，不是每个 epoch 重新 DH。[traffic keys / nonce][secure-keys]。replay window 的 idle 清理、前一 epoch、SYNC grace 等仍需攻击与恢复测试，不能概括为无生命周期限制的永久抗重放。[replay][replay]。
5. 所查 AEAD 调用的 **AAD 为空**。nonce 作为 AEAD 输入，peer / session 的选择也带来其他约束；不能因此简单宣称整个公开 header 都可任意修改。但也不能声称它像 Meshway 的 MAC transcript 那样覆盖全部公开 facts／有序路径／实际 carrier。[AEAD 调用][aead]、[secure data][secure-data]。

## 有 ACL，但所查 relay 路径没有相同的路径契约

EasyTier 有源端 Outbound ACL、目的端 Inbound／网关 Forward ACL，处理 IP、协议、端口和 peer-group；也有 credential / group proof、信任与撤销相关代码。源码入口为 [ACL filter][acl] 与 peer route 模块，不能概括成“没有业务权限”。

这里的 Forward ACL 是目的 overlay 节点向内层目标转发时的检查，**不是透明 ciphertext relay 对原业务 IP facts 的同等逐包准入**。反向五元组允许记录也不包含原完整有序路径／Meshway 执行 permit。所查 header、session、relay、route 和 ACL 数据路径没有发现 Meshway 式逐未来验证者 MAC 链、实际 carrier 前缀绑定或完整逆序业务回复许可。

这正对应我们要讨论的 [Q1／Q3](/v2/discussions/packet-layers#待讨论问题)：是保留每个诚实中继能验证的路径参与与业务检查，还是采用不同的中继职责？这是语义决定，不能只由头长或一次吞吐数字代替。

## 最值得直接阅读的 buffer 与调度组织

| 源码事实 | 对 Meshway 的讨论价值 | 不能扩大成的结论 |
| --- | --- | --- |
| Native TunStream 使用 BytesMut、预留 headroom / tailroom，再 split / move 给 owned HostPacket | 对照我们当前 TUN buffer→Vec、decode body→Vec 等所有权交接点 | 不是 kernel/user 零拷贝，也不代表所有入口相同 |
| headroom=max(TCP4,UDP8,WG20)+overlay16＝36 B；AEAD tailroom=28 B | buffer 可有预留空间而 wire 只发当前 provider 所需前缀 | 不能把 36 B 全部加到每个 UDP packet 的 wire 账本 |
| Ring / RustCrypto AEAD 就地处理 mut payload、追加／移除 tag 和 nonce | 值得研究预留空间、就地加密与原 packet guard 共同的 ownership 设计 | 没有由源码得出性能倍数；明文清理／取消后的 buffer 寿命还需约束 |
| MpscTunnel 容量32，收到一项后 drain＋feed，再 flush；TCP FramedWriter 有 vectored 聚合路径 | 对照固定调度成本与真实 writev 合并率 | 队列容量32不是每轮最多32的显式计数预算；合并率需要测 |
| Native UDP 仍逐 datagram send_to / recvmsg；native TUN 研究入口是一份 AsyncDevice、单读／写 task | 比较单包 syscall、session ring、锁与 actor 调度 | 未发现显式 sendmmsg / recvmmsg / UDP_SEGMENT / UDP_GRO 或 native TUN multiqueue；不能把多线程等同多队列 |
| foreign wrapper、压缩、headroom 不足的类型转换、WG／QUIC adapter 中仍有复制 | 要沿具体 provider / feature 路径逐项数 copy | BytesMut / “ZC” 命名不证明全栈无复制或无锁 |

源码：[NIC buffer][nic-buffer]、[packet buffer 转换][buffer]、[AEAD][aead]、[MpscTunnel][mpsc]、[TCP writer][tcp-write]、[UDP session][udp-session]、[native UDP][native-udp]。

## 可选 WG / QUIC：另算协议与信任边界

- **WG provider：**当前内部实现把 overlay 包放入合成 IPv4 20-byte 包，再交 boringtun 封装。锁定依赖 `boringtun-easytier 0.6.1` 的数据 header16＋tag16，所查代码明确尚未补 16-byte 对齐 padding。普通 Classified UDP 路径、overlay AEAD 开启时，可推导 UDP payload 为 `P+16+28+20+32=P+96`；通过 EasyTierMux session 的其他取得路径须另外判断是否加8B。[WG 包装][wg]、[锁定依赖][wg-lock]、[依赖 session 实现][wg-session]。
- **QUIC provider：**此固定提交的 `quic://` 使用自定义 HeaderKey／PacketKey：header encrypt/decrypt 为空，packet 只写／验证非秘密 SeaHash 8-byte checksum。不能按通常 TLS 1.3 QUIC 的 payload AEAD／证书认证解释，业务机密性依赖上层。它用 bidirectional stream＋4-byte framing，不是 QUIC DATAGRAM；QUIC header、frame、分段／重传等开销可变。[QUIC 源码][quic]。
- QUIC 配置虽启用 segmentation offload，所查 session adapter 仍把 segment 切成 Vec 后逐段发送；不能据配置 flag 宣称真实 native UDP GSO 已执行。[session socket adapter][quic-socket]。

这些是该研究提交的具体代码性质，不是对所有版本／部署的泛化评价；我们没有做上游攻击实验，也没有把这些可选层当作默认数据路径。

## 带回共同讨论的顺序

1. **先比较语义：**普通路由 relay 与强制有序路径参与证明是否都需要？谁拥有 payload key，谁能判定源业务权限？
2. **再比较所有权：**能否把 Meshway 的已验证 packet facts、编码前缀、ciphertext 视图和 buffer 共用起来，同时保持原 guard？
3. **再比较系统调用：**每 packet 的 read/write、SO_MARK、锁和 allocator 次数各是多少？batch filling／group wait／多 peer 干扰如何？
4. **最后做可复现对照：**明确 EasyTier 的传统或 secure mode、provider、压缩、TUN、资源配额与 route；不要把不同的功能成本当作相同协议的包装损耗。

待补研究包括：secure replay idle/SYNC 的边界、公开 header 与 encrypted-flag 例外、route static-key 的完整信任传播、显式算法组合的准入条件，以及真实 I/O profile。本页的源码观察不能替代这些行为验证。

[license]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/LICENSE
[defaults]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/config/toml.rs#L29-L64
[default-aes]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/config/encryption.rs#L5-L21
[conn-wrap]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/conn/peer_conn.rs#L343-L366
[nic]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier/src/instance/virtual_nic.rs#L867-L923
[plane]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/instance/packet_plane.rs#L44-L61
[outbound]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/peer_manager.rs#L2505-L2858
[inbound]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/peer_manager.rs#L3010-L3196
[peer-queue]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/mod.rs#L41-L151
[relay-send]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/relay_peer_map.rs#L226-L379
[session-filter]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/conn/peer_conn.rs#L125-L232
[foreign]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/packet/mod.rs#L585-L610
[headers]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/packet/mod.rs#L18-L136
[tail]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/packet/mod.rs#L331-L346
[aead]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/encrypt/ring.rs#L78-L146
[udp-codec]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/socket/udp/session.rs#L286-L348
[framing]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/framed.rs#L61-L149
[derive]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/encrypt/mod.rs#L60-L84
[network-key]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/peer_manager.rs#L897-L911
[noise-direct]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/conn/peer_conn.rs#L785-L1175
[noise-relay]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/relay_peer_map.rs#L382-L688
[sessions]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/conn/peer_session.rs#L115-L340
[noise-auth]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/conn/peer_conn.rs#L689-L755
[admission]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/peer_manager.rs#L379-L415
[secure-keys]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/secure_datagram.rs#L250-L481
[replay]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/secure_datagram.rs#L489-L744
[secure-data]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/secure_datagram.rs#L690-L744
[acl]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/peers/acl/filter.rs#L355-L552
[nic-buffer]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier/src/instance/virtual_nic.rs#L46-L240
[buffer]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/packet/mod.rs#L421-L795
[mpsc]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/mpsc.rs#L45-L108
[tcp-write]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/tunnel/framed.rs#L153-L331
[udp-session]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier-core/src/socket/udp/session.rs#L442-L764
[native-udp]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier/src/socket/udp.rs#L80-L132
[wg]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier/src/tunnel/wireguard.rs#L174-L376
[wg-lock]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/Cargo.lock#L1002-L1006
[wg-session]: https://github.com/EasyTier/boringtun/blob/431e8c49567c7c870e96d2d07770cff078e010cb/boringtun/src/noise/session.rs#L193-L267
[quic]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier/src/tunnel/quic.rs#L93-L484
[quic-socket]: https://github.com/EasyTier/Easytier/blob/728ba94b5029240add6204eed591f22e4e039e6b/easytier/src/tunnel/quic/session_socket.rs#L65-L264
