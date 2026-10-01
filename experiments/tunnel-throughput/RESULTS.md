# 托管隧道吞吐定位与有界 TUN 队列优化（历史：e69cb3d 旧 pump）

**版本边界：本文所有测量来自 e69cb3d 的旧 Agent packet pump。** main 后续
5029c0a 已将它替换为共享 `meshway_dataplane::packet_io::run_with_activation` 和
PacketDevice batch 接口；这些数值不能当成共享 pump 的当前性能。共享 pump 的
独立集成复测见 [SHARED-PUMP.md](SHARED-PUMP.md)，原始旧 JSON 保持不变。

## 可证实的瓶颈

托管 TUN 的 `tx_queue_len=32` 是一个重要的源端突发丢包点。内核 TCP 向 TUN
投递数据时，32 包队列在 Agent 读出 / 加密前溢出；丢包发生在进入 Runtime 之前。
`managed20-base.json` 首个直连 P1 forward：TUN **TX dropped 14412**、TCP measured
retransmits **13127**、receiver **436.72 Mbit/s**。这不是认证拒绝、IP fragmentation，
也不是把仅有注册 / 心跳的状态当成隧道。

同一个 frozen 基线二进制、相同 managed generation / 密钥生成方法 / CPU / 内存 /
路径 / 加密 / MTU，仅在隔离 namespace 将 TUN queue 改为 128：同一类 case 的
TUN TX dropped **61**、TCP retransmits **5793**、receiver **518.03 Mbit/s**。
TUN 本地丢弃下降约 99.6%，TCP 重传下降约 55.9%。剩余重传说明还有其它损失点，
不能把扩大一个队列当成整个数据面的性能问题已解决。
同 case 总 cgroup CPU / measured receiver byte 的上界估计从 **32.57 降至 29.53 ns/B**；
源 / 接收 CPU 从 73.42% / 81.34% 到 79.72% / 86.13%，提高了有用工作占比。
queue128 case 的 source TUN TX packets 1504346、destination TUN RX packets 1498012，
仍有约 6334 包差值，receiver kernel UDP RcvbufErrors 54。跨节点采样不是完美对齐，
不能把此差值直接指定给某一个 userspace queue；最终默认 image 用实验 sidecar
补齐 Runtime 独占 drop 原因，保留原始窗口与计数。

## 同二进制、同资源 A/B

- 基线与 treatment 都使用 `meshway-throughput:managed-baseline-e69cb3d`，实际 image
  ID 在两个 JSON 内；不是以重新构建 / 不同 Rust flags 制造差异。
- Native ARM64，VM 12 logical CPU / 9435258880 bytes RAM；每节点 1 CPU quota /
  256 MiB、两个 Tokio workers；release 测试 executable 执行真实 managed Manager。
- NativeUDP，`Noise_IK_25519_ChaChaPoly_BLAKE2s` 逐跳 + E2E；64 字节节点 ID，
  全部有序路径授权保留；MTU1000 / MSS948；TUN offload 关闭。
- `iperf3 -P1/-P4`，forward/reverse 分开，`-O2 -t20`，每种条件两次。
- 原始文件：`managed20-base.json`、`managed20-tun128.json`。数值为 receiver Mbit/s：

| 路径 / 流数 / 方向 | queue32 两次 | queue128 两次 |
| --- | --- | --- |
| 直连 P1 forward | 436.72, 369.84 | 518.03, 310.18 |
| 直连 P1 reverse | 422.78, 381.23 | 505.74, 502.94 |
| 直连 P4 forward | 434.13, 447.57 | 519.01, 511.34 |
| 直连 P4 reverse | 435.05, 435.84 | 523.69, 523.75 |
| 中继 P1 forward | 131.53, 201.28 | 290.41, 246.40 |
| 中继 P1 reverse | 141.50, 223.77 | 267.23, 321.21 |
| 中继 P4 forward | 297.92, 300.28 | 357.00, 402.11 |
| 中继 P4 reverse | 315.19, 314.83 | 392.50, 392.36 |

双样本均值：直连 P4 forward / reverse 约 **+16.9% / +20.3%**；中继 P4 约
**+26.9% / +24.6%**，中继 P1 两方向约 **+61%**。直连 P1 forward 的第二个
样本只有 310.18 Mbit/s，仍存在明显异常 / 波动，不宣称每个 case 都提高。
单流的拥塞反馈、丢包和 E2E session rollover 需要进一步分离；本轮没有把它的
具体异常原因定位到独占计数，因此不把 rollover 猜测当成结论。

## 保留的实现

`apps/agent/src/managed_dataplane/tun.rs` 的默认 TUN queue 从 32 改为 **128**。
仍是有限内核队列；队列上限低于既有 256 次 nonblocking drain 预算，留出代际
撤回 / 清空的余量。IPv4 检查、源 / 目的授权、逐跳与 E2E、重放保护、lease watchdog、
route sink、有界 Runtime 队列和所有权规则均保留。

## 已测后否决的候选

曾将 managed packet 收发的每包 `yield_now()` 改成每 16 包一次。完整 `-O2 -t20`
双重复 `managed20-budget16.json`：直连 P4 有提高，但中继 P4 reverse 从
315.19 / 314.83 降至 297.72 / 274.58 Mbit/s，均值约 **-9.2%**。
因此没有保留此改动；生产仍每包 yield。不是预设更多 batching 一定更快。

## 下一个量化切片

1. 对仍有的 UDP socket / ingress / endpoint-delivery / outbound 队列损失建立同一
   active-load 窗口的计数。托管 observation 尚不暴露 Runtime drop 明细，优先用
   独立实验 instrumentation（本轮已增加 ignored fixture sidecar），不先扩产品 API。区分 TUN TX drop、`RcvbufErrors`
   和 userspace try_send 拒绝，防止只把损失移动到下一个队列。
2. 固定 20k / 40k / 64k packet/s，给 memory-device 与真实 TUN 两臂使用完全相同的
   64 字节节点 ID、有效 IPv4 / UDP packet、路径、加密、payload size 与到达节奏。
   本轮 synthetic Runtime 已验证同协议 / 加密下无需 TUN 也会丢包，但其 payload
   与 TCP workload 不相同，不能直接相减当作严格的 TUN 增量成本。
3. 分段采样 CPU ns/packet：TUN read/write、IPv4 校验、缓存锁、encode / decode、
   E2E seal/open、hop seal/open、UDP send/recv、队列入 / 出。对比有用 byte 的
   CPU 成本、drops、加载下 RTT；确认 CPU 饱和段后再做 buffer reuse / batch syscall。
4. 对 P1 异常做一次受限的 session rollover 前 / 后窗口对比；使用已有真密钥、
   不缩短路径、不关闭认证、不放宽 replay window，不无止境重跑找最高数。

本轮没有 perf 硬件 profile、TLS-VLESS 托管 TUN 或真实 WAN 联测。裸 Docker 约
100 Gbit/s 只说明此 VM / bridge 的环境容量；保留 TCP offload 的 L4 与逐包加密的
L3 完整隧道并非同语义，不能把其差值全部称为 Meshway 包装损耗。

## 最终默认值验证与剩余损失归因

`managed10-default128.json` 是重新构建后的默认 queue128，**没有 namespace queue
override**、仍每包 yield。`-O2 -t10`、P1/P4、两方向完整通过；relay stop 两方向
不可达；每个节点的真实 Runtime sidecar before/after 均已记录：

| 路径 | P1 forward / reverse | P4 forward / reverse |
| --- | --- | --- |
| 直连 | 526.96 / 517.02 | 536.00 / 493.15 Mbit/s |
| 三节点中继 | 326.38 / 330.92 | 370.59 / 381.09 Mbit/s |

其直连 P1 forward：source TUN TX drop **105**、outbound queue full **641**；
receiver ingress queue full **206**、endpoint delivery full **2174**、UDP RcvbufErrors
**29**。CPU source / receiver **78.6% / 85.3%**；该 10 秒含 warmup / 采样的总
cgroup CPU 成本上界约 **32.66 ns/B**，不与 20 秒不同窗口作严格增量相减。
直连 P4 forward receiver endpoint delivery full **9105**，CPU **89.7%**；
中继 P4 forward LAX ingress **733**、outbound **970**，最终 SHA endpoint delivery
**5049**。因此 TUN queue 修复后的主要损失确实仍包括有界 userspace 队列拥塞，
不是把少量内核 UDP 错误当成所有剩余损失。

## 独立 Runtime 与实验队列因果控制

`runtime10-final.json` 完整 12 个 case，无 TUN、同 NativeUDP / hop Noise / E2E /
有序路径授权、1280 字节 synthetic payload。requested packet/s 与 accepted local
enqueue 数分开记录（`MissedTickBehavior::Skip` 会降低实际 offered rate）；以下是
实际 receiver / accepted sender 比例，不是 TCP 吞吐保留率：

| 请求负载 | 直连 forward / reverse | 中继 forward / reverse |
| --- | --- | --- |
| 20k packet/s | 99.31% / 98.93% | 99.11% / 99.17% |
| 40k packet/s | 90.37% / 90.37% | 81.14% / 86.60% |
| 64k packet/s | 88.36% / 80.74% | 68.20% / 62.02% |

高负载在进入真实 TUN 前就会丢包，排除“全部损失只是 TUN”这一解释；由于
synthetic payload / 发送节奏 / TCP 拥塞反馈不同，也不把此表直接相减为设备成本。
周期性 receiver CPU samples 限定保留最近 32 个窗口，可用于 active-load 对齐。

同一个 lab release image、同路径 / 加密 / MTU1280、1 CPU / 256 MiB、P1 forward、
`-O2 -t20` 双重复，两组都确认无 kernel UDP receive loss：

| 配置 | receiver Mbit/s | 总 cgroup CPU ns/B 上界 | LAX ingress / outbound drops |
| --- | --- | --- | --- |
| queue64 / OS UDP buffer | 417.07, 419.14 | 32.36, 32.45 | 404/711, 354/854 |
| queue256 / requested UDP buffer1MiB | 686.43, 662.45 | 27.85, 29.76 | 34/246, 84/116 |

receiver SHA delivery / ingress drop 也从 **270/368、247/390** 降为
**138/141、148/146**；平均吞吐约 **+61.3%**，重传 1554/1671 降为 465/467。
这是**两项设置的联合控制**，不声称单独 UDP buffer 的收益；基线 RcvbufErrors
就是 0，主要观测改善在 userspace 队列。没有直接把 queue256 改成所有生产
Runtime 的默认值：需要先审计最大连接 / socket 数下的总内存预算，再做单因素
与加载下延迟验证，不能只依据三个节点的峰值。

补充同资源裸 TCP **MTU1000 / MSS948**：P1 forward / reverse **93.97 / 125.38
Gbit/s**，P4 **57.81 / 57.99 Gbit/s**（`bare1000.json`，10 秒）。所以环境量级差距
并非由“无限 CPU 裸 TCP对 1 CPU / 小 MTU 隧道”的不公平资源对比制造。
host 独立采样：Apple M3 Pro、12 physical / logical CPU、19327352832 bytes RAM；
Docker VM 资源和全部 container limits / RSS / counters 保留于 JSON。

## 验证与产物

- 共享 performance lock 全程串行化实际 build / checks / workload，owned tmux
  session `meshway-perf-throughput-e69cb3d`。
- Rust formatter、Clippy `--all-targets -D warnings`；native Linux release dataplane
  56 个行为测试、managed-dataplane 4 个行为测试（2 个 fixture tests ignored，
  通过 Docker 命令显式运行）、managed packet / lease / withdrawal 7 个行为测试。
- 统计校验测试 3 个；所有完整的最终测量 artifact `status=ok`、`cleanup_errors=[]`。
- 原始和失败 / 被否决候选的数据都保留在：
  `/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/throughput-e69cb3d-20261001/`。
  `summary-final.txt` 提供全部计数及 CPU 摘要；`managed-comparison.json` 是稳定托管 A/B 的缩略副本。
- Linux flood discovery test 原先并行 / 串行均在 post-flood 单次 UDP reply 的恢复
  检查超时；仅该恢复 probe 改为最多 4 个有效回复、5ms 间隔，保留 source /
  transaction / codec、容量和 flood deadline 的全部断言。没有改变生产发现逻辑。
