---
title: Packet：字节账本与固定开销 · 设计讨论
description: 逐字段解释 packet 多带了什么、谁使用它，以及网络规模与路径长度对线上开销的影响。
---

# Packet：字节账本与固定开销

**设计要求：网络变大，不应让每个业务 packet 携带更多网络信息。** 本页专门讨论线上字节：多塞了什么、为什么需要、谁验证、能否移到本地上下文，以及每包开销能否保持固定。

::: info 讨论中
当前账本对应 Meshway `97dbb138` 的 candidate-v4 Data 编码。下文的固定开销方案是**条件化预算，不是已决定或已实现的新 wire**。现行行为定义仍见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)；owner、队列、复制与运行成本见[包流分层](/v2/discussions/packet-layers)。
:::

## 先定义“不随网络变大”

| 变量 | 含义 | 我们关心的约束 |
| --- | --- | --- |
| N | 整个网络的成员数 | **已明确的设计要求：**同一业务包不因新增无关节点而变大；不携带全网成员／路由／授权列表 |
| H | 本次会话的单向路径边数；A→R→B 为 H=2，共 H+1 个节点 | **建议采用的更强目标：**增加必经中继也不增加每段 Data 的封装字节；完整路径留在上下文，避免逐跳追加列表 |
| S | 节点保留的活动会话／流数量 | 标识字段保持固定宽度；增加 S 的代价体现在有界状态、查找和建立过程，不通过拉长 packet 承担 |
| P | 完整内层 IP 包的字节数，包含 IP 和 TCP／UDP／ICMP 头 | 业务包本身，不算 Mesh 新增开销 |

记第 k 段的额外字节为 `Oₖ = DataBytesₖ − P`。目标是在固定协议模式／地址族下，`Oₖ` 与 N 无关；进一步讨论能否对所有 H、k 都使用同一预算 C。若保留与内层协议有关的 metadata，可有固定的模式上限，而不是随规模增长的列表。

**当前 v4 已经不随 N 增长，但仍随 H、所在段 k 和 listener key 长度变化。** 虽然实现只接受 2–8 个路径节点，有上限不等于开销与路径长度无关。

还要分清三本账：

- **单段大小：**`P+C`，决定这一段的 MTU／封装效率。
- **沿途总传输量：**即使每段固定，走 H 段仍至少发送 `H×(P+C)`；固定头不意味着多跳免费。
- **建立与本地状态：**完整路径、密钥、授权、续租和恢复仍要有归属与容量预算。移出 Data 是转移成本，不是消灭成本；短流还要计算 setup 摊销。

## 当前 packet 长什么样

主例：`A→R→B`，Native UDP，完整内层 IPv4/TCP 包 P=1280，每条边 listener key 为 `lab-udp`（7 个 UTF-8 bytes）。

<figure style="margin:24px 0">
  <div style="overflow-x:auto;border:1px solid var(--vp-c-divider);border-radius:12px">
    <img src="/diagrams/packet-layout-v4.svg" alt="R 到 B 的 Data：头84 bytes，Noise ciphertext1296 bytes，carrier 与 MAC 后缀39 bytes；UDP payload共1419 bytes。" style="min-width:960px;width:100%;max-width:none" />
  </div>
  <figcaption>当前 v4 字节布局。块宽不按字节比例。<a href="/diagrams/packet-layout-v4.svg" target="_blank" rel="noopener">打开完整 SVG</a></figcaption>
</figure>

### 每个字段的钱花在哪里

以下按线上顺序列出。字段可解析不代表已认证；CID 查表也不是授权成功。

| 字段 | 当前 bytes | 谁使用／解决什么 | 能否移入上下文或缩减 |
| --- | ---: | --- | --- |
| magic／wire version | 8 | 收包端识别 MWCIR 与版本，和其他共享 socket 消息区分 | 可讨论更紧凑 discriminator；仍要能无歧义分流／拒绝旧版本 |
| kind | 1 | 区分 Data、握手、Probe 等解析方式 | 可与 flags 紧凑编码；信息本身仍需要 |
| CID | 32 | 所有路径节点定位已建立 circuit，恢复源、目标、路径、权限和 keys | 完整上下文已经移出 packet；短 selector 可另研，但不能直接截断现有 CID，也不能只用发送者替代 |
| reply | 1 | 确定原路径方向、对应密钥／序号／流处理 | 可用 flags 或分方向上下文表达；不是 B 主动新流的授权 |
| sequence | 8 | 端点 Noise nonce、各节点重放处理与 MAC 绑定 | UDP 可乱序／丢失，不能靠本地“已收到几个包”推导；必须避免同 key 下复用 |
| PacketFacts | IPv4/TCP 30；IPv6/TCP 54 | 中继不解密也能检查地址、端口与 TCP／echo 状态；端点解密后精确比对真实包 | 静态 tuple 可讨论放入 flow context；动态事实若仍由中继检查，就需要可信的逐包输入 |
| body length | 4 | codec 定界 ciphertext 与后缀，检查长度／尾随 | UDP 总长不能单独确定可变后缀边界；只有布局固定／可推导后才可讨论省略。stream 还有自己的 framing |
| ciphertext | P | 整个原 IP 包的端到端机密性；R 转发，端点解密 | 业务数据本身，不能算“多塞的头”；IP 内已有的地址／端口在这里也是加密的 |
| Noise AEAD tag | 16 | 端点验证 ciphertext；R 没有端点解密 key | 必须计入预算；不能拿这个 tag 代替 R 可验证的逐包认证 |
| carrier count | 1 | 定界已走过多少条边，以及还剩多少 verifier tags | 若不再传前缀列表，可由本地路径位置／方向代替；取决于路径证明设计 |
| 每条 carrier 的 dialer／listener index | 1＋1 | 结合 retained path 恢复该边的物理拨号方向 | 静态绑定可入上下文；当前携带的是已走各段的实际选择，不只是预期拓扑 |
| 每条 carrier 的 protocol | 1 | 认证实际 Native UDP／TCP／VLESS／Trojan 选择 | 可讨论绑定到已认证的通道／会话上下文，切换时明确更新边界 |
| 每条 carrier 的 key length／key | 1＋Lᵢ | 绑定所选 listener；Lᵢ 是 UTF-8 bytes，当前允许1–255 | 字符串标识是显著可变项；可讨论 setup 中绑定与固定索引，但不能丢失实际 carrier 校验 |
| 剩余 verifier tags | `16×(H+1−k)` | 每个后续验证者有一列路径 MAC；当前接收者重算自己的前缀，后续列留给对应验证者 | 当前设计直接依赖 H。换成单个相邻 MAC 会改变证明范围；尚无已验证的同语义定长替代 |

**额外发送者 NodeID：0 bytes。** 稳态 Data 已从 CID 恢复原始发送者。完整路径、network／demand、permit／key 摘要、Ed25519 签名和临时公钥贡献在 setup／本地 context 中，不在每个 Data 里。握手重传仍可能再次发送 setup 信息。

Carrier index 表示 **dialer／listener 在原 path 中的位置**，不是当前业务包的发送者／接收者。回复改变遍历方向，物理通道的拨号方向不因此反转。

### PacketFacts 为什么与内层 IP 重复

| facts 内容 | IPv4 | IPv6 | 用途 |
| --- | ---: | ---: | --- |
| 原 packet length | 4 | 4 | 对外声明包长，端点与真实解密包核对 |
| source／destination IP（各带1B family） | 10 | 34 | 中继业务源／目标检查 |
| source／destination port | 4 | 4 | 中继端口规则／流关联；echo 使用占位0 |
| transport kind | 1 | 1 | TCP／UDP／echo 分支 |
| TCP seq、ack、flags、payload length | 11 | 11 | 当前 TCP 有界状态关联 |
| **TCP facts 合计** | **30** | **54** | 与 ciphertext 中的真实 IP/TCP 信息重复，但明文可供 R 验证后使用 |

UDP 没有最后11B TCP 字段，facts 为 **19／43B**；echo 改为 reply1＋identifier2＋sequence2，facts 为 **24／48B**。这些是固定的协议差异，与 N、H 无关。

如果 R 只执行会话级转发、端点负责真实业务包检查，就可以讨论去掉公开 facts。如果 R 仍须逐包关联 TCP 状态，仅把地址／端口放入 flow context 并不能移除动态 seq／ack。**删字段的前提是明确谁还负责它原来支持的功能。**

## 用一条公式覆盖路径的每一段

令 F 为 facts 长度，k 为该方向已经走过的边数（1…H），Lᵢ 为第 i 条已走边的 listener key 字节数：

```text
DataBytesₖ = P + Oₖ
Oₖ = 71 + F + Σ(i=1…k)(4 + Lᵢ) + 16×(H+1−k)

71 = magic8 + kind1 + CID32 + reply1 + sequence8
   + bodyLength4 + NoiseTag16 + carrierCount1
```

这里没有 N 或 S。H 对 carrier 前缀和未来 verifier 列都有影响。经过下一个中继，差值是 `L_next−12`：7-byte key 时每段减5B，长于12-byte 的 key 则会增加。因此要算所有段、两个方向，不能只看最后一段。

P=1280、IPv4/TCP、全部 Lᵢ=7 时，`Oₖ=101+11k+16(H+1−k)`：

| 路径边数 H（节点数） | 首段新增 O₁ | 最后一段新增 O_H | 首段／末段 UDP payload | 首段／末段外层 IPv4 包 |
| --- | ---: | ---: | --- | --- |
| 1（2） | 128 | 128 | 1408／1408 | 1436／1436 |
| 2（3） | 144 | 139 | 1424／1419 | 1452／1447 |
| 4（5） | 176 | 161 | 1456／1441 | 1484／1469 |
| 7（8） | 224 | 194 | 1504／1474 | 1532／1502 |

固定整个 P=1280，内层改 IPv6/TCP 时每格加24B。外层从 IPv4/UDP 换成 IPv6/UDP 再加20B；例如 H=2 的 IPv6 内层＋IPv6 外层为1496／1491B。以上外层均按无 IP options／扩展头计算，不含 Ethernet 等 L2 成本。H=7 示例的两端都超过1500；不是只有中间某段可能分片。

**小包比例更高：**52-byte 内层 IPv4/TCP ACK，H=2 的最后一段仍新增139B，变为191B UDP payload、219B 外层 IPv4 包。139/1280≈10.9%，139/52≈267.3%；这里分母为原 IP 包，不是 TCP application goodput。减少字节不等于同比提高吞吐，仍要看 pps 与每包 CPU。

<figure style="margin:24px 0">
  <div style="overflow-x:auto;border:1px solid var(--vp-c-divider);border-radius:12px">
    <img src="/diagrams/packet-budget-scaling.svg" alt="当前 v4 头不随全网节点数增长，但随路径边数增长；定长目标将路径与权限留在本地上下文，每段只携带固定大小的选择、序号和认证信息。" style="min-width:960px;width:100%;max-width:none" />
  </div>
  <figcaption>规模约束：固定每段预算，另算沿途总流量与建立／状态成本。<a href="/diagrams/packet-budget-scaling.svg" target="_blank" rel="noopener">打开完整 SVG</a></figcaption>
</figure>

对当前等长 key 的格式，把同一个包的 carrier 描述与 verifier tags 在 H 段上全部相加，为 `(L+20)×H×(H+1)/2` bytes。它描述累计传输量，**不是某一个 packet 的头长**；不包含各段的 count1、P 和其余固定部分。当前最多7条边，不能把公式外推为已支持的无限长路径。

## 本地上下文可以替代什么

“知道发送者，再查当前本地状态”是方向，但**发送者不唯一决定一次会话**：A 可以同时沿 A→R→B 和 A→D→B 通信，两者的业务资格、期限与回复路径也可能不同。

```text
实际认证通道 ＋ packet 中的固定 selector
  → 查找候选上下文
  → 验证逐包认证、方向与序号
  → 检查当前授权、实际邻居、期限／取消
  → 用上下文绑定的下一跳转发；端点解密并检查真实业务包

本地上下文：
  原发送者／目标、完整原路径、角色／前后邻居、业务绑定、密钥／replay、
  当前执行资格与取消、carrier 绑定、原路径回复关系
```

这是候选职责划分，不表示当前实现已去掉 R 的业务流检查。本地依据必须来自 **Control 有效执行快照／permit、认证握手和真实流准入**，不能把观测 State、在线状态或一条存在的 socket 当权限。CID／selector 只是 lookup，验 MAC 前读到的内容不能直接触发交付或获得新的执行资格。

定长 selector 需要有明确命名空间、碰撞处理、方向／key epoch、释放和重用规则。未知 selector 不能按旧发送者猜一个新上下文；重启或路径更新后的迟到包不能绑定新权限。合法连续性交接仍必须保留原 queued output 的 guard。

### 区分验证目标

| 目标 | 本地上下文能否给出依据 | 额外需要什么 |
| --- | --- | --- |
| 这条路径现在是否允许、我应该从谁收／往哪发 | 可以：有效 permit、角色、实际 channel 与期限 | 包自身必须经过适合该接收者的认证；持续检查当前有效性 |
| 原始 A 是否产生了这份业务密文 | 端点可以结合 A↔B 加密上下文验证 | R 不能验证端点 Noise tag；若要求 R 也独立验证原始来源，需专门认证设计 |
| 这个包此前是否具有所有指定节点的认证贡献 | 静态的合法路径列表不能证明实际 packet 的过去 | 当前使用逐验证者 MAC 链；定长替代尚未确定 |

把相邻 MAC 每跳替换而不累加，可获得定长的**相邻认证**，但其验证者只能确认获密钥的邻居贡献。恶意邻居可以在该范围内造包；端点 AEAD 拒绝伪造业务密文，不等于中间节点已经证明原始来源或完整路径参与。反过来，也不能未经论证就断言“更强证明必然要求线性头长”；这里只确认当前构造的增长性，没有证明同语义定长方案不可能。

## 固定开销的预算草案

先固定功能，再选位宽。以下保留当前 discriminator／CID 等尺寸，**只用来展示哪一项改变减少哪些 bytes**，不代表推荐了某个新 tag 协议。

共同基座：`magic8 + kind1 + CID32 + reply1 + sequence8 + bodyLength4 = 54B`。Noise tag 另16B，基础为70B。

| 模式／前提 | 每段新增预算（IPv4/TCP） | 随 H 增长？ | 语义状态 |
| --- | ---: | --- | --- |
| 当前 v4，H=2、L=7、最后一段 | 70＋facts30＋count1＋carriers22＋MAC16＝**139B** | 是，按上文公式 | 已实现 |
| 条件草案 A：保留 facts，路径／carrier 绑定入 context，以单个16B相邻 MAC 替换整个后缀 | 70＋30＋16＝**116B** | 预算不增长 | 中继仍可读 facts，但不再拥有同等的完整前缀认证；不能标为等价优化 |
| 条件草案 B：在 A 基础上由端点做真实业务包检查，Data 不公开 facts | 70＋16＝**86B** | 预算不增长 | 进一步改变中继职责；流级准入／回复和资源滥用边界待决定 |
| 保留当前逐诚实验证者的来源／路径证明，同时采用定长封装 | **待定** | 目标不增长 | 需要独立构造与验证，不能填入116B或86B宣称已解决 |

A 的 IPv6/TCP 为140B；B 因为不带 facts，内层地址族不改变该预算。P=1280 时 B 的条件账本是1366B UDP payload、1394B 外层 IPv4 包；它**没有实装／性能结果**。若还需要独立 channel ID、epoch、hop counter 或其他控制字段，要逐项加回，不能藏在“认证信息”里漏算。

这两个草案假设固定 selector 能在实际 channel 上恢复每方向 key／上下文；MAC 必须覆盖相应 selector、方向、序号、ciphertext 与绑定 context。carrier 切换、重连和 key epoch 如何更新仍要定义。若使用逐跳重写的 label，端到端认证还必须区分不变逻辑绑定与可变 hop 字段；不能直接改掉端到端 tag 所绑定的 header。

**单独缩短 CID、合并 flags、推导长度只减少常数项，不能解决 H 项。** 同样，移除 facts 可以减少30／54B，却不消除 carrier／verifier 列的增长。要满足更强的定长目标，必须正面决定路径认证格式与中继职责。

## 换 transport 后，外面还有什么

| 承载选择 | 概念上的线上嵌套（外 → 内） | 另算的预算 |
| --- | --- | --- |
| Native UDP | outer IP / UDP / MWCIR / Noise-encrypted IP | 无 IP options／扩展头时 IPv4＋UDP为28B，IPv6＋UDP为48B；当前没有额外公共 u32 长度或 channel AEAD 头 |
| Native TCP | outer IP / TCP / u32 frame length / MWCIR / Noise-encrypted IP | 每 Mesh frame 加4B长度；TCP 分段、options、ACK、重传另算，不能视为每包固定加一个 TCP 头 |
| VLESS TCP/TLS | outer IP / TCP / TLS / VLESS stream / u32 frame length / MWCIR / Noise-encrypted IP | CONNECT 为 setup；TLS 为额外外层保护，不保证一个 frame 对应一个 record |
| VLESS WS/TLS | outer IP / TCP / TLS / WS binary stream / VLESS stream / u32 frame length / MWCIR / Noise-encrypted IP | WS / TLS 按真实消息与 record 分摊；握手不是每个 Data 重复 |
| Trojan TCP/TLS | outer IP / TCP / TLS / Trojan stream / u32 frame length / MWCIR / Noise-encrypted IP | Trojan CONNECT 为 setup；稳态 stream framing 与 TLS 另算 |

Data 的公式不用于握手或 Probe。Probe 有 measurement／deadline 等自己的字段；MWADJ／health／STUN 也是独立消息，按频率及 setup 成本统计。MTU 要同时覆盖实际 provider 和全路径所有段；短流的 setup 不能因移出 Data 就不计费。当前验证范围见[数据面验收](/v2/node/dataplane#daemon-与内核资源-conformance)，本页不扩大承载组合的验收声明。

## 讨论收敛条件

| 问题 | 当前结论／下一步依据 |
| --- | --- |
| 不随 N 增长 | 明确要求；当前 v4 已满足该字节性质 |
| 不随 H 增长 | 本页建议的更强目标；当前 v4 不满足，先确认路径认证与中继职责 |
| 原始发送者字段 | 当前 Data 已无独立 NodeID；通过 CID 恢复，继续优先讨论固定上下文引用 |
| 中继必须逐包检查什么 | 待决定：仅会话／邻接／有效性，还是仍要原始来源独立证明及完整业务流事实 |
| 确切 C | 待定；116B／86B 是上述条件下的算术，不是协议定案 |
| 结果应如何验证 | 对每种 Data 模式给出逐字段表；同一 P、地址族与 provider 下改变 N／H／S，核对每段抓包长度、setup、内存与 pps／CPU；另验旧包、撤销、方向及原路径回复 |

源码定位：`crates/meshway-dataplane/src/secure/codec/mod.rs:180–244,247–359,396–522`（编码、字段、carrier／facts）；`secure/session/mod.rs:166–275`（各验证列与 transcript）；`secure/crypto/mod.rs:148–201`（Noise body/tag）。外部对照见 [EasyTier 字段预算](/v2/discussions/easytier#只比较字段预算)。
