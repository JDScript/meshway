---
title: 包流分层、封装与加密 · 设计讨论
description: 当前 Meshway 包流的逐层图、字节账本、运行成本及待讨论设计问题。
---

# 包流分层、封装与加密

::: info 讨论中 · 事实基线 97dbb138
本页以 2026-10-06 的源码与已完成测量为起点。**49 Mbps 左右是一个受限开发环境中的结果，不是“已经足够快”的结论。**我们要从每包的真实工作量重新检查设计。现行协议定义见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)，本页只做拆解、推导和候选设计讨论。
:::

## 先对齐三个问题

1. **语义：**我们要求中继证明什么、检查什么、看见什么？“加密隧道”“成员认证”“逐流授权”“强制有序必经点”是不同要求。
2. **字节：**一个 IP 包实际被包成多大？header、AEAD tag、路径 MAC、transport framing、TLS / WS 分别在哪里？
3. **执行：**同一份数据被解析、复制、哈希几次；经过多少队列、锁、future 和有效性检查？调用多一层不等于一定慢，也不等于多做一次加密。

讨论主例是 **A → R → B、Native UDP、无出口 NAT 的业务路径**。此处 R 是 overlay 必经中继；底层承载可能另经过普通 IP 路由器或用于连通性测试的 SNAT。

## 一张图看明白加密范围

<figure style="margin:24px 0">
  <div style="overflow-x:auto;border:1px solid var(--vp-c-divider);border-radius:12px">
    <img src="/diagrams/packet-path-v4.svg" alt="A 与 B 端到端 Noise，中继 R 验证 PacketFacts 和自己的路径 MAC 前缀，并原样转发 ciphertext；回复经 B、R、A。" style="min-width:960px;width:100%;max-width:none" />
  </div>
  <figcaption>图 1 · 逻辑路径与密码边界。<a href="/diagrams/packet-path-v4.svg" target="_blank" rel="noopener">打开完整 SVG</a></figcaption>
</figure>

### 三种认证／保护，各自解决什么

| 机制 | 发生在哪个时点 | 证明或保护什么 | 不能直接替代什么 |
| --- | --- | --- | --- |
| 相邻通道 `MWADJ` | 物理邻接建立；UDP 另有周期性签名 Ping / Pong | 当前登记身份与实际 carrier、来源地址和新鲜 challenge；通道健康 | 原业务源身份、完整业务路径、该业务流的权限 |
| circuit 握手：Noise IK ＋签名临时贡献 | 新 circuit 建立 | A 与 B 的端到端加密状态；完整 context 和各节点临时贡献的认证；派生逐验证者 MAC keys | 每次未来 I/O 的当前资格与实际写入结果 |
| 稳态 packet：Noise transport ＋路径 MAC | 每个 Data / Probe 帧 | 整个 IP payload 的端到端机密性；公开 facts、方向、序号、ciphertext 和 carrier 前缀的认证 | policy / 流状态、吊销和到期检查；实际对端交付 |

**Native UDP 稳态数据不再逐包做 Ed25519。**UDP 邻接的健康消息仍会签名，但它们不是每个业务 Data 帧的额外签名层。R 不解密 IP payload 再加密；它更新路径认证信息，转发同一份 Noise ciphertext。选用 TLS 等 provider 时，其外层保护另算。

### 为什么“只给邻居一个 MAC”不是等价替换

在 A→R→B 中，B 需要验证面向自己的 A 和 R 的贡献。仅有 R→B 的相邻认证，不能让诚实 B 区分“真实 A 发起并经 R 处理”与“R 自己编造来源”。当前实现给每个未来验证者一条独立 MAC 链：R 验自己的列，然后更新 B 的列；B 从头重算自己的完整前缀。

这项保证针对**诚实验证者**。被攻陷节点可以泄露自己的 keys；MAC 也不是供任意第三方公开验签的数字签名。每个接收者只验证自己的列，后续列由对应接收者验证；“R 已转发”不代表“B 已接受”。这些限制见[当前会话定义](/v2/node/dataplane#会话密钥与紧凑数据帧)。

以单个 A→R→B Data 包计，A 做一次 Noise encrypt；B 做一次 Noise decrypt；R 不做 Noise payload 加解密。MAC 则为 A 生成两列、R 验一列并更新一列、B 重算两段，共六次 HMAC 调用。这里数的是该包的逻辑运算，不包含建连、周期健康消息或 TCP 后续 ACK；不是六层 AEAD。

## Layer 不只是包装层

| 层／owner | 数据路径中做的工作 | 是否改业务帧的线上字节 | 需要关注的成本 |
| --- | --- | --- | --- |
| Kernel / `meshway-tun` | 端点 OS 协议栈、TUN read / write；接收的是完整 IP 包 | 给出／接收原 IP 包 | 系统调用、内核 TCP / ACK、读写 buffer；当前关闭 TUN offload / multiqueue |
| daemon `Packets` / worker | 本机队列、取得需求、选择 circuit、有界接收和独立发送调度 | 不定义另一份协议 | 取包复制、队列滞留、future、调度次数；当前每轮最多 32 |
| `SecureNode` / 唯一 `Engine` / policy | 准入、五元组与 TCP / echo 状态、Noise、MAC、重放、frame codec | 生成／解析 `MWCIR` | 每包查表、结构检查、哈希、编码、分配；端点还做加解密与真实包核对 |
| connectivity `Manager` | 认证邻接、真实通道选择、健康、连通性和承载切换 | 握手／health 有自己的控制消息；稳态 Native UDP 不因此加一个数据加密头 | link lock、scope 校验、事件队列、选择／重连；不拥有第二份业务流表 |
| dataplane `carrier` adapter | 把原 packet guard 与所选 channel scope 交给正式发送器；交付 TUN | 不再增加 `MWCIR` 字段 | 生命周期交集、等待／完成、引用与 buffer 所有权 |
| transport `Session` | 完整帧、预算、接纳与逐帧完成；必要时交原 worker | stream 加 u32 长度；datagram 保留边界 | sender mutex、预算、framing／flush、backpressure；pipeline 不等于一次内核 batch |
| provider ＋ Underlay | UDP / TCP / VLESS / Trojan 协议 I/O，受保护 socket 与真实资源生命周期 | 按 provider 增加外层 | TLS / WS（若选择）、复制、poll 和系统调用；Underlay 不定义另一层 payload 加密 |
| Control / Server | 下发／续租／撤销资格，处理需求与 State | 正常业务包不经过 Server HTTP 转发 | 业务路径需要检查已安装资格；HTTP 等待属于准备／恢复，不应被计入 probe RTT |

库的分界不等于线程／task 分界。共享逻辑的调用成本要用实测确认；有些层主要交接所有权和许可，有些层确实触及全部字节。职责定义仍以[架构](/v2/architecture)和[transport 分层](/v2/node/transport#分层)为准。

**Native UDP 的真实写入嵌套：**`Session::poll_datagram → connectivity physical gate → Underlay ProtectedDatagram → UdpTransport → Tokio UDP socket`。表中的 provider／Underlay 是职责关系，不能误画成“provider 在 Underlay 外又加密一次”。每轮的 32 限制分别约束就绪读取、event drain 和发送组；`Packets::drive` 还可能遍历此前保留的有界队列，并非所有工作合计只处理 32 项。

<figure style="margin:24px 0">
  <div style="overflow-x:auto;border:1px solid var(--vp-c-divider);border-radius:12px">
    <img src="/diagrams/packet-runtime.svg" alt="中继路径经过单一 UDP receiver、Manager 队列、daemon、SecureNode、独立发送 future、正式 Sender 和受保护 socket；原始 guard 始终约束最终 I/O。" style="min-width:960px;width:100%;max-width:none" />
  </div>
  <figcaption>图 2 · 稳态转发与等待边界。<a href="/diagrams/packet-runtime.svg" target="_blank" rel="noopener">打开完整 SVG</a></figcaption>
</figure>

### 源码能确认的重复工作

这些是源码操作点，不是已测出的耗时排序或精确 allocator 调用计数。

| 操作点 | 当前工作量 | 源码入口（仓库根相对路径，97dbb138） |
| --- | --- | --- |
| TUN → owned packet | 复用读 buffer，每包 `to_vec()` 后 `Packet::parse`；`Packet::facts()` 取已存 metadata，不重 parse | `apps/meshway-daemon/src/runtime/network/worker.rs:524–541`；`crates/meshway-dataplane/src/packet/mod.rs:162–203,252–282` |
| UDP 收取 → codec | Session 收取后先复制成 Frame payload Vec；Data codec 再复制 body、拥有 carrier key 字符串并 clone retained Context | `crates/meshway-transport/src/session/io.rs:372–388`；`crates/meshway-dataplane/src/secure/codec/mod.rs:300–359` |
| 端点 Noise | encrypt 新建 P+16 输出；decrypt 新建 P 临时明文，再复制给 Packet parser，与 facts 比较 | `crates/meshway-dataplane/src/secure/crypto/mod.rs:148–201` |
| 中继 MAC + encode | verify、append 各自调用 `frame.base()` 并 hash；encode 又生成 base。每个 base 重新序列化并复制 body | `crates/meshway-dataplane/src/secure/session/mod.rs:185–196,227–275`；`crates/meshway-dataplane/src/secure/codec/mod.rs:180–244` |
| MAC 的 carrier 输入 | hash 扩展使用 full carrier 编码，wire 使用 compact carrier；当前每个扩展另建 Vec | `crates/meshway-dataplane/src/secure/session/mod.rs:260–265` |
| 本地流查找 | `Packets` 查 `local_flow`，`local_circuit` 内再次查本地 flow，然后 send 做真正准入 | `apps/meshway-daemon/src/runtime/network/packets.rs:536,614,631`；`crates/meshway-dataplane/src/secure/mod.rs:1429–1431` |
| circuit 维护 | send / receive 开头扫描 retained circuits；目标状态临时 move 出树表再插回。Rust move 不等于 clone 整份 Noise/key，也不证明它就是主要瓶颈 | `crates/meshway-dataplane/src/secure/mod.rs:711–725,1055–1068,1319–1349` |
| 每包发送组织 | 独立 boxed future、OutputLifetime / channel scope Arc、child token、单帧 transport operation 与 completion；就绪路径不一定发生 task 切换 | `apps/meshway-daemon/src/runtime/network/packets.rs:30–49,733–746`；`crates/meshway-dataplane/src/carrier.rs:166–289` |
| **socket poll 的保护工作** | `ProtectedDatagram::with` 在进入 provider 前做 `scope.prepare`；其中读 SO_MARK、transform、set、再读回验证。实际 I/O 之外还有这些工作 | `crates/meshway-underlay/src/linux/datagram.rs:79–126`；`crates/meshway-underlay/src/linux/protection.rs:102–110,245–252` |

典型 Data 的 base 构造次数为 **A 两次、R 三次、B 一次**；这并不等于 AEAD 次数。SO_MARK 路径则应按**实际 poll 次数**统计：一次成功 packet 可能经历 Pending／重试，不能直接以固定“每包三次 socket option 调用”推算整机 CPU。减少重复工作之前，先明确 immutable 结构校验与动态权限／资源／取消检查的边界。

## Packet 字节账本

<figure style="margin:24px 0">
  <div style="overflow-x:auto;border:1px solid var(--vp-c-divider);border-radius:12px">
    <img src="/diagrams/packet-layout-v4.svg" alt="1280-byte IPv4 TCP 包在 R 到 B 封装成1419-byte UDP payload，再加外层IPv4与UDP共1447 bytes；头84、Noise body1296、路径后缀39。" style="min-width:960px;width:100%;max-width:none" />
  </div>
  <figcaption>图 3 · 字节数为精确推导，矩形宽度不是字节比例。<a href="/diagrams/packet-layout-v4.svg" target="_blank" rel="noopener">打开完整 SVG</a></figcaption>
</figure>

### 用一条公式覆盖路径的每一段

设 `P` 为完整内层 IP 包长，`F` 为 PacketFacts 长度，`n` 为 path 节点数，`k` 为该方向已经走过的边数（1 到 n−1），每条已走 carrier 的 listener key UTF-8 长度为 `Lᵢ`：

```text
MWCIR Data bytes
  = P + 71 + F + Σ(4 + Lᵢ) + 16 × (n − k)

71 = magic8 + kind1 + CID32 + reply1 + sequence8
   + bodyLength4 + NoiseTag16 + carrierCount1

IPv4 TCP facts: F = 30          IPv6 TCP facts: F = 54
outer IPv4 + UDP: +28           outer IPv6 + UDP: +48
```

这是当前 Data 编码的推导，不适用于带完整 context／贡献列表的握手帧，也不适用于 Probe metadata。IPv4 / IPv6 外层按无 options / 扩展头计算；不含 Ethernet、VLAN、preamble 等链路开销。

`P=1280`、每个 listener key 均为 `lab-udp`（7 bytes）时：

| 内层／path | 首段 UDP payload | 最后一段 UDP payload | 首段＋外层 IPv4/UDP | 首段＋外层 IPv6/UDP |
| --- | ---: | ---: | ---: | ---: |
| IPv4/TCP，2 节点 | 1408 | 1408 | 1436 | 1456 |
| IPv4/TCP，3 节点 | 1424 | 1419 | 1452 | 1472 |
| IPv6/TCP，3 节点 | 1448 | 1443 | 1476 | 1496 |
| IPv4/TCP，8 节点 | 1504 | 1474 | 1532 | 1552 |

每经过一个中继，去掉一个 16-byte verifier tag，但新增一个 carrier 描述；因此后缀不一定只增不减，也不能仅以最后一段估计整条路的 MTU。8 节点示例在 MTU1500 下仍可能分片。

**小包也要算：**以 52-byte 内层 IPv4/TCP ACK 为例，3 节点最后一段仍有同样的 header / tag 开销，变为 **191-byte UDP payload**。大包 Mbps 无法单独描述 ACK、小包和高 pps 的 CPU 成本。

### 换 transport 后，外面还有什么

| 承载选择 | 概念上的线上嵌套（外 → 内） | 关键区别 |
| --- | --- | --- |
| Native UDP | outer IP / UDP / MWCIR / Noise-encrypted IP | 一个 datagram 一帧；正常 Data 没有额外 channel AEAD 或公共 u32 frame 长度 |
| Native TCP | outer IP / TCP / u32 frame length / MWCIR / Noise-encrypted IP | 字节流分段边界不等于 Mesh frame；内层 TCP 和外层 TCP 各有自己的重传／有序语义 |
| VLESS TCP/TLS | outer IP / TCP / TLS / VLESS stream / u32 frame length / MWCIR / Noise-encrypted IP | VLESS CONNECT 是建流协议，不是每个 Mesh 包重复一个完整 CONNECT 头；TLS 为额外外层保护 |
| VLESS WS/TLS | outer IP / TCP / TLS / WS binary stream / VLESS stream / u32 frame length / MWCIR / Noise-encrypted IP | WS 先适配成连续字节流；WS / TLS record 与 Mesh frame 不必一一对应 |
| Trojan TCP/TLS | outer IP / TCP / TLS / Trojan stream / u32 frame length / MWCIR / Noise-encrypted IP | 连接握手与稳态 stream 数据分开计费 |

上述嵌套表达层次，不声称每个包都会触发每层新握手、每层都复制一次，或所有 provider 已完成当前 v4 的整组 Linux 验收。协议选项和验证边界见[承载会话](/v2/node/transport)、[支持状态](/v2/configuration-reference/transports)与[当前验收范围](/v2/node/dataplane#daemon-与内核资源-conformance)。

## 实测支持了什么，仍没说明什么

现有同路径比较：每 Node 0.5 CPU / 384 MiB、内层 MTU1280、单 TCP 流、关闭插桩、2 秒省略 warmup＋20 秒测量、两次重复；路径 `sha→lax→nyc`。这是开发 VM 数据，未实跑同环境 EasyTier / WireGuard 对照。

| 比较点 | 接收 Mbps | 能支持的结论 |
| --- | --- | --- |
| 先前 wire / 调度基线 | 23.47 / 23.74 | 完整受保护转发的比较基线 |
| 会话化认证＋紧凑帧 | 25.12 / 27.51 | 消除了本例最后一段外层分片，但整机吞吐只小幅增加 |
| 加上就绪发送、承载视图复用、有界包处理／独立发送 pipeline | 48.20 / 49.50 | 这一组合约为原均值的 2.07 倍；不能把收益全部归于加密算法 |

粗采样曾显示 carrier 发送组织自身约 14–18 μs／次，承载视图重复查询累计开销高于 MAC；这支持检查等待器、队列和调用频率。它没有把所有 guard／锁／分配进一步准确拆开，也不能用“单次验签 40 μs”直接预测删除后的吞吐倍数。Box 化整个 Circuit 的独立实验未测出收益，已撤回。

当前还缺少的分层证据包括：实际分配／复制次数、每级队列年龄与丢弃占比、最终 I/O 检查的自耗时、不同批次填充度、小包 pps／ACK 比例，以及同环境外部实现对照。底层 syscall batch / GSO 的收益也尚未测出。验证入口与报告口径见[性能对照](/v2/node/transport#验证与性能对照)；当前版本的恢复缺口见[验收边界](/v2/node/dataplane#daemon-与内核资源-conformance)。

## 待讨论问题

以下都为**候选问题，未决定、未实现为新契约**。

| 编号 | 问题 | 当前代价／可以讨论的方向 | 作出决定前要证明什么 |
| --- | --- | --- | --- |
| Q1 | R 必须做完整的 L4 流状态检查吗？ | 当前 R 读取每包 facts，并跟踪 TCP / echo / 反向流。可以审视“中继逐流执行授权”与“只验证路由绑定资格及发送者”的产品差异 | 两者能阻止的攻击、保护的资源和拒绝时点不同；不能把删除 FlowTable 当纯内部优化 |
| Q2 | 哪些 header 常量应移到 flow context？ | CID 已移除完整 path / key 列表；地址、端口等仍逐包重复，TCP 动态事实也重复出现在加密 IP 内 | 新上下文的创建／确认、容量、过期／吊销、重连和错误复用；flow ID 不能自授权限 |
| Q3 | 我们需要逐诚实中继的可验证路径参与，还是一般路由选择？ | 当前路径 MAC 对每个未来验证者独立派生 keys。仅验证相邻通道更简单，但不是同一要求 | 恶意中继／串通节点能否伪造源、跳过必经点；“正常路由经过 R”不能替代密码学保证 |
| Q4 | 同一包能否共用 buffer 与一次解析／hash？ | 当前 owned Vec、frame base 编码、验证后再次编码和发送 coroutine 都值得单独计数。可评估预留 headroom、借用视图、in-place AEAD、认证后缓存解析结果 | buffer 唯一 owner、异步持有期限、敏感明文清理、被改字节与缓存失效、预算按 capacity 计费；不能缓存未验证解析结果当权限 |
| Q5 | 动态 guard 能否保留而减少重复结构检查？ | 不变配置绑定与动态取消／UTC／单调期限／health 是不同工作，可研究按精确 snapshot 缓存前者 | 缓存键必须覆盖真实主体、路径、carrier、材料与版本；读缓存不延长 lease，最终 I/O 的动态检查仍可触发 |
| Q6 | 下一步先做 kernel batch，还是先减少 task / lock / queue 交接？ | 当前 32 包是就绪读取和独立 future pipeline，底层仍逐包 syscall；真正 batch 可能摊薄固定成本，也会增加等待和突发 | 每帧期限与取消、部分成功／不确定回执、批次填充延迟、控制工作公平性、内存上限；不能把一份宽松 batch guard 套到全部包 |
| Q7 | MTU 按哪个最坏段来定？ | 路径长度、地址族、listener key、provider 都影响最大线上长度；握手与稳态应分开 | 所有 cut 的上界、路径变化、IPv6 最小 MTU、PMTUD / 分片行为；不能只把某次 iperf MSS 调小就宣布解决 |
| Q8 | SO_MARK 与资源检查需要在每次 poll 做到什么粒度？ | 当前每次进入受保护 socket poll 都有 mark 读取／设置／读回，以及多层 scope 检查；应独立计量系统调用数与自耗时 | 实际 descriptor 是否独占、provider／外部变更如何受控、selector 和资源 handoff 如何失效；不能以“socket 私有”为由直接删除冲突与最终 I/O 防护 |

### 建议先对齐 Q1，同时测量 Q4 / Q5 / Q8

Q1 决定我们**究竟要付哪些必要成本**。如果保留当前每跳业务准入与强制有序路径语义，下一步应先用分配／自耗时／pps 证据审视 buffer、hash、重复结构检查与 socket 保护操作；如果要降低中继职责，则先明确新的授权与攻击边界，再估算可能减少的字段和工作量。SO_MARK 是当前源码已经确认的重复操作，**还不是已经实测确认的最大热点**。

[EasyTier 对照](/v2/discussions/easytier)提供不同组织方式的源码依据。它是否更快、差多少，需要同语义或明确标注语义差异的实际对照；代码层次少、header 小、宣称 zero-copy 都不能直接充当结果。

## 讨论记录方式

| 条目 | 当前内容 |
| --- | --- |
| 事实基线 | Meshway `97dbb138`；图、公式对应 candidate-v4 的 Native UDP Data 路径 |
| 已有约束 | [端到端／路径／生命周期](/v2/node/dataplane#候选受保护-circuit)、[本机接纳与完成](/v2/node/transport#本机接纳与完成)、[访问控制](/v2/configuration-reference/access) |
| 外部研究 | [EasyTier 固定提交源码对照](/v2/discussions/easytier)，具体 checkout 信息同时记在仓库 `references/README.md` |
| 未决定 | Q1–Q8；当前没有由本页批准的语义削减、替代密码方案或新 wire |
| 下一份论据 | 每层 allocations / copied bytes / pps / CPU / queue age；相同主机、资源、路径、包长与 transport 条件的对照 |

新增结论时写明“决定、理由、语义影响、验证结果”，并将确定的行为更新到对应规范。本页保持可继续讨论的问题清单，源码追溯使用上述版本，原始实验保存在私有报告中。
