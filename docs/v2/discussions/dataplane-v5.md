---
title: 数据面 v5 · 已决定的设计与实现契约
description: 会话级中继、定长相邻认证、单缓冲包流水线与批量 I/O 的目标设计；实现完成前不是可用契约。
---

# 数据面 v5：已决定的设计与实现契约

::: warning 已决定，实现中
本页记录 2026-10-06 基于[分层测量](/v2/node/performance)与[字节账本讨论](/v2/discussions/packet)做出的设计决定，作为并行实现的共享契约。现行可用行为仍以 [数据面组件](/v2/node/dataplane) 为准；实现与验收完成后，本页内容迁入规范，本页改为引用。
:::

## 决定

| 编号 | 决定 | 取代 |
| --- | --- | --- |
| D1 | 中继按**会话级**执行：验证包属于已授权且仍有效的 circuit（CID、方向、序号／重放、相邻认证），按本地上下文转发；业务 IP／端口／TCP 状态准入只在源与终点 | 中继逐包读取公开 PacketFacts 并维护业务流表 |
| D2 | 稳态包采用**定长相邻认证**：每跳一个 16-byte MAC，由 circuit 握手中相邻两节点的临时贡献派生，绑定 CID、方向、序号与密文；中继验证后替换为下一跳 MAC。端到端机密性与来源真实性由 Noise AEAD 保证 | 逐验证者 MAC 链与随路径增长的 carrier 前缀 |
| D3 | 包在整个处理过程中使用**同一块带 headroom／tailroom 的缓冲**：TUN 读入、就地 AEAD、头部写入 headroom、UDP 发送；接收反向同理。codec／session／crypto 以借用视图工作，不再逐层 `Vec` 复制 | TUN→Vec、parse、encrypt 新分配、`frame.base()` 重编码 ×3、decode body 复制、decrypt 临时明文再复制 |
| D4 | TUN 与 UDP 使用**批量 I/O**：Linux TUN `IFF_VNET_HDR` 的 GSO／GRO 超包拆合，UDP `UDP_SEGMENT`／`UDP_GRO`，并以 `sendmmsg`／`recvmmsg` 作为无 GSO 时的批量路径；Session 提供批量提交与单次完成 | 每包一次 TUN syscall、一次 UDP syscall、一个 completion |
| D5 | 每包工作拆为**owner 段**与**worker 段**：唯一 SecureNode 仍拥有 circuit、flow、nonce、replay 的状态变更；AEAD、HMAC、编码／解码在工作线程池并行，输出按 circuit 保序。guard 契约不变 | 单线程串行完成全部工作 |

握手保持 Noise IK 与有序 Ed25519 逐跳证明；完整路径、身份、permit 摘要仍只在 setup 携带并保存在本地上下文。承载切换、合法连续性交接、lease／撤销、原路径回复的语义不变。

## 稳态 Data 帧（wire v5）

```text
magic/version 8 | kind 1 | flags 1 | CID 32 | sequence 8 | body length 4
| Noise ciphertext P+16 | hop MAC 16
```

- `flags`：bit0 reply 方向；其余保留为 0，接收方拒绝非零保留位。
- 开销 **86 B**，与路径长度、listener key、内层地址族无关；P=1280 时 UDP payload 1366 B，外层 IPv4 包 1394 B。`meshwayd` 的默认内层 MTU 由此导出：`DEFAULT_INNER_MTU` = 1500 − 48（IPv6/UDP）− 86 = 1366，任何 1500 字节底层链路上的任何路径都不分片。
- 中继不解析 ciphertext；终点解密后 `Packet::parse` 并做业务准入，源在加密前做业务准入。
- Probe 帧保留自己的 metadata 字段；握手帧保留完整 context 与证明。

### 相邻 MAC

对路径位置 `i`（发送）与 `i+1`（接收）的相邻节点，使用双方握手贡献 `DH(eᵢ, eᵢ₊₁)` 经 HKDF-SHA256 派生两把方向密钥 `k(i→i+1)`、`k(i+1→i)`，salt 为握手 transcript 摘要（与当前 v4 派生相同，仅保留相邻对）。

```text
tag = HMAC-SHA256(k(dir), "meshway/circuit/hop-mac/v5" ‖ CID ‖ flags ‖ sequence ‖ body length ‖ ciphertext)[..16]
```

验证者只需本跳密钥；验证通过后用下一跳密钥重算 tag 覆盖原位置。回复方向使用反向密钥。实际 carrier 的绑定由相邻通道的 MWADJ 认证与 Control 下发的邻接资格承担，不再进入每包 MAC。

实现后端为 HMAC 0.13／SHA2 0.11，派生时预 key、每包 clone，运行时检测硬件 SHA2；端到端稳态 AEAD 为 ring 0.17 ChaCha20-Poly1305，方向 key 在 Snow 握手 split 时一次构造并由不可变 Arc 共享。Noise 握手继续使用 Snow，nonce／AAD／HMAC transcript 与 wire 字节不变；清零边界和一致性验证见[就地加密](/v2/node/dataplane#单缓冲所有权与就地加密)。

### 每跳步骤

| 角色 | owner 段（串行） | worker 段（并行） |
| --- | --- | --- |
| 源 | 入口 `Reading`；`Packet::parse`；业务流准入；取 nonce；选 circuit／下一跳；提交 | AEAD 就地加密；写头；计算 hop MAC |
| 中继 | 入口 `Reading`；CID→上下文；重放预检；下一跳；提交重放 | 验证 hop MAC；重算下一跳 MAC（就地覆盖） |
| 终点 | 入口 `Reading`；CID→上下文；重放预检；（解密后）`Packet::parse`、业务流准入、方向核对；提交重放 | 验证 hop MAC；AEAD 就地解密 |

每个公开操作保持：入口一次 `Reading`、提交／产出前一次最终检查；输出在 release／I/O 边界再检查原 guard。worker 段不读取 authority，只使用 owner 段交付的密钥句柄与序号。

## 缓冲契约

```rust
/// 一块可复用缓冲：headroom 容纳 v5 头与外层 provider 前缀，tailroom 容纳 AEAD tag 与 hop MAC。
pub struct PacketBuffer { /* pooled Box<[u8]>, head, len */ }
impl PacketBuffer {
    pub fn payload(&self) -> &[u8];              // 当前有效区
    pub fn payload_mut(&mut self) -> &mut [u8];
    pub fn prepend(&mut self, n: usize) -> Result<&mut [u8]>;  // 使用 headroom
    pub fn extend(&mut self, n: usize) -> Result<&mut [u8]>;   // 使用 tailroom
    pub fn trim_front(&mut self, n: usize); pub fn truncate(&mut self, len: usize);
}
```

- 默认 headroom 128 B、tailroom 64 B；单包最大为当前 `MAX_PLAINTEXT` 加开销。池有界、按 owner 归还；取消或失败时缓冲随 guard 释放并清零敏感明文区域。
- `PendingFrame`／`PendingAction` 持有 `PacketBuffer` 而非 `Vec<u8>`；`release` 交出缓冲所有权；Session 接受 `PacketBuffer` 或其借用切片，不再复制。
- `Frame` 解码为借用视图（头字段 + `&[u8]` ciphertext），验证与解密就地进行；`Packet::parse` 接受借用切片，终点交付直接写 TUN。

## 批量 I/O 契约

- **TUN**：Linux 开启 `IFF_VNET_HDR`，启用 `TUN_F_CSUM | TUN_F_TSO4 | TUN_F_TSO6 | TUN_F_USO4/6`（按内核支持探测）。读取得到带 `virtio_net_hdr` 的超包，按 `gso_size` 拆成多个 IP 包交给 owner 段；写入方向把同流连续包合并为一个超包写入。无 vnet_hdr 时回退为逐包读写，报告中必须标明实际模式。
- **UDP**：发送同一目的地址、等长（最后一段可短）的多帧时用 `UDP_SEGMENT` 一次 `sendmsg`；接收开启 `UDP_GRO` 并按 cmsg `gso_size` 拆分。不支持时使用 `sendmmsg`／`recvmmsg`；再不支持时逐包。
- **Session**：`submit_batch(frames, context) -> Admission`，一个批次一次完成通知；批次内所有帧必须属于同一 channel scope，`SendContext` 的截止与取消取批次内各帧 guard 的**交集**（最早截止），不得比任何单帧更宽松。完成为批次级 `LocalComplete`；部分失败标为整批 `Uncertain`。
- 批次大小由就绪数据与上限（默认 32 帧／64 KiB）决定，不等待凑批；控制消息（握手、health、STUN）不与业务批次合并。

## 并行流水线契约

```text
TUN/UDP 入站（批量） → owner 段（SecureNode，单线程，状态变更） → 工作池（AEAD/HMAC/编码，无状态） → 按 circuit 保序输出 → 批量发送 / TUN 写入
```

- 工作池大小默认为可用核数减一，有界队列；队列满时对该 circuit 施加背压，不丢弃已准入的包。
- owner 段交付给 worker 的是：缓冲、方向、序号、`Arc` 的密钥句柄（AEAD key、hop MAC key）、输出 guard；worker 不访问 flow 表、replay 窗口或 authority。
- 重放提交在 owner 段，位于 worker 验证成功之后；验证失败的包不提交。
- 顺序：同一 circuit 的输出按 owner 段接收顺序交付；不同 circuit 之间无序。
- daemon 的 `Packets`／Manager／carrier 适配器改为批次提交：一次批次一个 `OutputLifetime`，一个 completion；channel scope 检查在批次提交与最终 poll 进行。

## 实现分工与验证

| 工作包 | 范围 | 验证 |
| --- | --- | --- |
| F | wire v5、相邻 MAC、D1 中继职责、`PacketBuffer` 与就地 crypto／借用 codec（dataplane、policy 中继准入、static-authority fixture） | dataplane 单元／runtime 测试、L0、L3 `secure` 对照 v4 |
| G | TUN vnet_hdr GSO／GRO、UDP GSO／GRO／mmsg、Session 批量提交（tun、transport、transport-udp） | L2 `plain` 的逐包 vs 批量对照、既有 transport 测试 |
| H | owner／worker 流水线、daemon 批次提交、carrier 适配器精简（dataplane 流水线模块、daemon network） | L3 `secure`→L4 `manager`、L5 lab 同路径吞吐 |

F 与 G 并行；H 依赖两者。每个包交付都必须给出同口径的前后对照与关闭插桩的正式数字，并保持端到端保护、源授权、必经路径、原路径回复、lease／撤销与 release 边界检查的既有回归通过。验收矩阵中依赖 v4 的行在 v5 落地后标为“历史通过未重验”，直至按 v5 重跑。

H 的实现入口：D5 owner／worker API 与有界工作池为 `3e9d5e86`，原存储贯通 Session 与批次 guard 交集为 `cc1e0200`，daemon／Manager／TUN 批次装配为 `fcd38a65`，owner 准备与 worker 小组重叠为 `7976b109`，共用 L3／L4 驱动为 `5ca93df6`，L4 正式 Host 执行安装为 `2d0b7584`。现行边界见[数据面](/v2/node/dataplane#发送流水线与连续性)与[承载会话](/v2/node/transport#本机接纳与完成)，实际验收以[矩阵](/v2/node/validation)为准。D3 的内核到内核全链路无复制尚未完成，剩余复制位置见[单缓冲所有权](/v2/node/dataplane#单缓冲所有权与就地加密)；L4 高负载失败样本与尚未达到的 TUN 交付目标见[实测](/v2/node/performance#l3-流水线与-l4-实测)。
