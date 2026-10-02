# PPS、CPU 与组件基线：根因调查记录

## 已证实的边界

本轮安全基线为 `eb127b5`，包含 HOL / R3 / R5；对应原始四个提交
`5814f56 / f0a0262 / 2085439 / eb127b5` 已由协调者独立复核并整合到 main
`9c7d922`。所有原始 frozen image / JSON 保留，未因主线后续提交重测基线。

### 1. 当前两节点 / 三节点实验没有 IP fragmentation

MTU1000、64-byte IDs，权威 `native_udp_datagram_len()` 给出外层 IPv4/UDP 总长：
直连 **1259 B**，三节点 **1324 B**。底层 MTU1500；安全基线所有样本的
`FragCreates / FragFails / ReasmFails` 均为0。此处没有证据把瓶颈归于 fragmentation。

当前 compiler 还预留 IPv6/UDP 48B，三节点同路径静态安全内层上限为1156B。
在 PPS 不变时从1000提升至1156只增加约16%字节吞吐，不能解释数量级差距。
真实 WAN 的 PMTU 是后续必须处理的可达性边界，不能用未测 WAN 解释本地结果。

### 2. 按包看，已在支付显著的 CPU / syscall / 调度成本

`safe-r1.json` P4 forward 的 kernel sampling window（包含边界开销）：

| 路径 / 节点 | UDP 入 / 出千包每秒 | 容器 CPU（单核%） | 容器 CPU µs / UDP 收发操作 | system CPU 比例 |
| --- | ---: | ---: | ---: | ---: |
| 直连 source | 20.3 / 60.1 | 83.9 | 10.66 | 29.6% |
| 直连 receiver | 60.0 / 20.3 | 94.1 | 11.95 | 34.1% |
| 中继 source | 12.1 / 42.8 | 67.9 | 12.76 | 31.7% |
| 中继 relay | 54.9 / 54.8 | 64.8 | 6.09 | 29.5% |
| 中继 receiver | 42.7 / 12.1 | 74.4 | 13.98 | 34.0% |

relay 每个转发包需要一次收、一次发，因此其每转发包约12µs，不能把6µs与端点
成本直接相减。CPU 是容器聚合，含 iperf / 辅助进程；窗口并不完全同起止，作为
上界估计。单独 `/proc/PID/stat`（Linux USER_HZ=100）的 Agent 窗口：直连两端约
83% CPU，中继 relay 约66%；iperf receiver 自己还使用约10–13% CPU。

Tokio workers 的 OS context switch 也已保存：直连 source 每秒约20k voluntary /
15k involuntary，receiver 约16k / 9k。中继未用满 quota 仍丢包，说明 CPU 单一
饱和不能解释全部；突发、消费调度、TCP 丢包反馈必须分开看。

### 3. 真实 CPU 栈确认 scalar crypto 是主要用户态热点之一

独立诊断 image 使用同一安全 Runtime，加 debug symbols / frame pointers；
`perf record -F99 -e cpu-clock:u -g` 仅采本 namespace 的 Agent PID，不采整台 VM。
该诊断的 throughput 不用作优化 A/B。每节点136–329个用户态样本，精度有限：
报告由 `perf report --no-children` 生成，以下百分比是 exclusive/self 样本；不把
父栈与子栈百分比相加，也不把用户态百分比当作全部 CPU 或全隧道成本。

- 直连：ChaCha20 主函数及相关分支约32%，Poly1305 scalar block约14%。
- relay：ChaCha20约28%，Poly1305 scalar block约19%。
- wire encode/decode、分配、Tokio/锁/通知等余项分散存在；不能仅凭函数名猜测其
  占据全部成本。内核 CPU 不在 `cpu-clock:u` 中，另由 cgroup user/system 计数记录。

核对锁定依赖源码：`chacha20 0.9.1` 在 aarch64 默认选择 `soft` backend；NEON
需要显式 `chacha20_force_neon`。`poly1305 0.8.0` 的 ARM 路径也是 scalar。这里
没有开启新的算法、取消 E2E 或改 wire；优化 backend 须另做单因素和正确性验收。

同 native ARM64 / release / 1 CPU 的真实 Cipher（含 Vec / replay）原语诊断：

| payload bytes | seal + open ns | 2-ID encode + decode ns | 3-ID encode + decode ns |
| ---: | ---: | ---: | ---: |
| 64 | 610.71 | 306.96 | 400.41 |
| 256 | 1431.76 | 370.76 | 430.18 |
| 1000 | 4489.19 | 350.97 | 481.09 |
| 1156 | 5228.83 | 466.12 | 698.94 |
| 1280 | 6283.98 | 385.82 | 552.72 |
| 8192 | 38489.02 | 777.56 | 936.67 |

这些是同步 wall-time ceilings，不是完整隧道提升比例；大包行仅用于观察字节成本，
不代表能把8192B放进本轮MTU1500链路。每个端点的数据路径有 E2E + hop 两层，
而 relay 做 hop decrypt + encrypt；因而不能直接拿一层 crypto 的数字解释整个节点。

### 4. 固定 UDP PPS 暴露突发损失，不能冒充无丢包容量

同 image、100µs iperf pacing、10秒，输入 full IPv4 size 1000：
requested 60k PPS 的 managed 直连收到约32k PPS、中继约34k PPS。直连 source
TUN TX drop **277,915**（含warmup/边界）、receiver delivery drop34,649；source
CPU仅74%。这首先说明源端突发溢出，不能把32k称为 crypto 或 Runtime 处理上限。

相同 UDP datagram 大小的 raw underlay 在 requested 400k PPS 时交付约256k PPS /
**2.52 Gbit/s**，同时丢失36.7%；receiver kernel RcvbufErrors 1,791,158
（含warmup/边界），CPU仅19%。这是实际有损交付，不是“无丢包原生容量”。连 raw
UDP 都可因缓冲/调度突发在低平均 CPU 下丢包，进一步说明只报 Mbps 会误导。

### 5. 有些低吞吐样本包含真实 blackout，不能全归为稳态每包成本

按实际 **receiver** JSON、排除 omitted 窗口：`inbound-r2` relay P4 forward 有
6.002秒零字节区间，占实际receiver窗口30.01%；整窗252.68Mbps，非零窗口等价
361.00Mbps、有效非零1秒区间中位372.85Mbps。并出现 authentication/stale-channel
计数。`safe-after-r2` direct P4 forward 有5.000秒零字节，占其实际21.64秒receiver
窗口23.10%；整窗248.08Mbps，非零窗口等价322.62Mbps、中位350.36Mbps。这些
原值没有删掉。它们与另一类“整个窗口持续较慢”的样本应分开诊断。
“非零窗口/中位”只作稳态参考，仍包含ramp与拥塞变化，不替代整窗A/B。已发现
iperf首个post-omit interval的start/end与seconds不一致；工具单独报告该项，不
把该异常半速区间当稳态或用它虚构时长。统计只用receiver，避免误把sender暂停
等同于精确网络断流；1秒采样分辨率也不是逐包的精确断流时刻。
当前 Runtime 邻接探测一次 timeout 会注销 channel，而 probe 和业务共用有界队列；
这是需要时序证据 / 丢控制包回归的具体候选，尚不把全部 blackout 指定给它。
后续优先用有界时序关联 probe admission/timeout、channel退休/重连、错误计数和
lease/config/fence，区分正常timeout failover与nonce/retry缺陷；稳定性修复独立
提交，不通过拉长认证期限或取消探测来提高速度。

## 尚在完成的组件 / 单因素对照

- 同资源 wireguard-go userspace TUN：真实 peer、TUN 与 UAPI；正常 offload 和
  无 virtio header 两臂，明确保留其 UDP batching / crypto / wire 的差异。
  默认 Go heap 首轮在256MiB receiver触发OOM，事件与失败JSON保留；显式
  `GOMEMLIMIT=128MiB` 的复测继续使用同一容器内存上限。
- 同机 tun-rs 2.8.11 async + UDP 无加密组件：offload关闭、MTU1000、queue128、
  reusable buffers、无中间队列；不是完整 VPN，不用作包装损耗分母。
- 同一 managed binary 的1/2 Tokio workers对照，检查1 CPU quota下的跨线程代价。

全部原始数据位于 [NEXT.md](NEXT.md) 的 ART；`cpu-profile.json`、五个
`*.perf.data`、`profile-<mode>-<node>.txt`、`primitives-soft.log`、
`pps-size-100us.json`、`raw-udp-ceiling.json`、`pps-burst-1ms.json` 可独立审计。
`receiver-window-diagnostics.json` 保存48个已完成A/B case的整窗/零字节/非零
窗口和interval不一致明细；`intervals.py` 及其测试固定计算口径。

## 环境与解释限制

共享 performance.lock 串行化 Docker build / workloads，但每节点1 CPU quota不是
专用物理核，也不能隔离并行 host 构建和本机服务对 VM 宿主的影响。前后控制已显示
漂移与 blackout；不能在明显受扰动样本上归因固定提升。最终同时保留整个测量窗口
和另行明确计算的稳态窗口、断流秒数/占比，不能只裁去0吞吐区间后报最好成绩。
WireGuard / tun-rs / worker对照在本次文档提交时仍 pending，结果不提前写为通过。
