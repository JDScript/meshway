# P1c 后的安全基线与入站调度实验

## 版本与测量边界

本轮从 `564780e807a6bf7ad83901991f114dc9319c2bfd` 开始；旧
[SHARED-PUMP.md](SHARED-PUMP.md) 的数值不是当前源的结果。

- 初始生产源 `564780e`，image
  `sha256:c1f29ef1c89f82292b81e187175114d2605de6d9c0f2035451f08041243c9e69`。
  `baseline-r1.json` / `baseline-r2.json` 保留修复前完整双重复。
- `f0a0262` 修复跨路径 setup HOL：真实加密 regression 在旧源下失败，新源下通过。
  这是慢 setup 隔离的行为证据，不声称仅靠此锁改造提高稳态单路径吞吐。
- `2085439`（R3）和 `eb127b5`（R5）修复队列跨 admission retirement 与 Ready
  丢失重试黑洞；详细接口保证、内存上界和剩余协议限制见
  [CORRECTNESS.md](CORRECTNESS.md)。后续优化必须使用包含两项修复的安全基线。
- 安全基线源 `eb127b5`，image
  `sha256:1bef0afe6e65f66f8fe9f0a7ca8803b4669b5e38a1a9e0bb8f6166f8b392aef5`。
  `safe-source.txt` / `safe-dirty.txt` 固定实际构建源，后者为空。

JSON 的 `source_revision` / `source_dirty` 是启动 runner 时工作树的状态，并非
image 的构建标签；开发候选期间可能与被测 frozen image 不同。构建归属以各臂
`*-source.txt`、保存的 source patch、resolved image SHA 与 fixture 内计算的
`binary_sha256` 为准；不同臂不会因工作树继续编辑而替换正在运行的二进制。

每个 arm 固定一个 resolved image / 同一 release binary，真实 managed Manager、
coherent plan、Linux TUN、NativeUDP + hop/E2E Noise、64-byte identity、有序路径授权。
每节点 1 CPU quota / 256MiB、两个 Tokio workers、MTU1000 / MSS948、TUN offload
关闭。TUN queue128、Runtime queue64、OS UDP buffer 默认、Runtime work budget16。
P1/P4，forward/reverse 分开；每 case `iperf3 -O2 -t20`，两个独立 fixture block。
每 fixture 四个 case，允许覆盖真实 E2E session rollover，不人为延长 key lifetime。

每 case 保存 iperf 双端原始 JSON、CPU/user/system/throttling、RSS、TUN drops、
UDP RcvbufErrors 与 Runtime 独占 drop；加载下 RTT 在 launch 后3秒开始、0.5秒间隔
32个 ICMP。RTT 是成功回应的样本分位，不将未响应填0；CPU ns/B 是含 warmup /
采样辅助进程的上界估计。全部样本保留，不选最好一轮。

## 已撤回的单因素候选

共享 packet pump 入站从每包 yield / 单包 callback write，改为每轮最多消费8个
**已到达**包后 yield，逐包做同一 admission 检查，再移动 payload 所有权到一次
device batch。没有等待凑批，没有扩大 channel/socket/TUN queue，没有改出站预算。
Linux TUN 仍逐包 syscall；不声称 sendmmsg/recvmmsg 或零拷贝收益。

本轮 Linux direct-write adapter 新增内存只是一个最多8包 / 64KiB的在途组，包含
原有单包份额；MTU1000 负载下最多8KiB。通用 `ChannelDevice` 的 queue 单位本来
就是 batch（公开上界 capacity × 64KiB）；现在每个 egress entry 可能包含多包，
实际积压内存可高于旧单包 writer，但不超过既有公开上界。拒绝包也计入8次工作
预算，watch/lease 仍包围 blocked device I/O，R3 retirement 与逐包目的授权保留。

候选 `2e17aba` / image
`sha256:b6e948445a9a8a3eedec6ddd4cf2f11e8bf7d6ae5040d168138c6b02630165d0`。
`654f6bf` 已撤回；没有把较低 delivery drop 宣称为最终性能收益。

| 场景 | 安全基线两次 Mbps | batch8 两次 Mbps | 原安全 image 回跑两次 Mbps |
| --- | --- | --- | --- |
| 直连 P1 forward | 407.62,343.56 | 478.21,497.26 | 440.31,383.48 |
| 直连 P1 reverse | 487.54,396.49 | 398.33,492.06 | 314.54,435.36 |
| 直连 P4 forward | 467.87,456.79 | 410.20,499.67 | 317.08,248.08 |
| 直连 P4 reverse | 474.52,420.93 | 411.98,495.24 | 471.13,443.76 |
| 中继 P1 forward | 302.20,302.79 | 271.91,308.69 | 169.62,279.71 |
| 中继 P1 reverse | 295.40,291.79 | 291.82,308.73 | 281.83,311.26 |
| 中继 P4 forward | 334.88,330.57 | 319.72,252.68 | 331.19,336.79 |
| 中继 P4 reverse | 353.94,335.54 | 331.23,356.03 | 346.41,196.62 |

中继 P4 forward 的前后控制都约331–337Mbps，候选均值低约14%；其中候选 r2
有6秒左右的 blackout，不把整个差值当作纯稳态 CPU 成本。其它控制也有明显波动，
不能通过选一组较低控制来宣布候选全面提高。

首轮直连 P4 forward：delivery drop **12167→2998**，source outbound drop
**5966→12742**，source TUN TX drop **691→1363**，吞吐 **467.87→410.20Mbps**。
这证明损失移动到了上游，而不是整体问题解决。加载下 p95 RTT（ms）也并未一致
改善：直连 P4 reverse 基线 **1.72/2.86**，候选 **5.37/2.32**；中继 P4 reverse
基线 **2.78/2.77**，候选 **4.68/2.54**。

六个完整 artifact 共48个 workload 均 `status=ok`、`cleanup_errors=[]`；每个
relay fixture 的停止后双向不可达检查通过。原始逐包统计、RSS、CPU、p50/p95/max
与丢失 echo 均在 JSON；`batch8-comparison.json` 是不删样本的汇总。

根因调查已转向 PPS / CPU profile / 组件基线，当前证据见
[ROOT-CAUSE.md](ROOT-CAUSE.md)。正确性四个提交已由协调者独立复核并合入主线
`9c7d922`；这里继续保留原始 SHA / images，不因主线变化重建原基线。

## 原始产物与复现

目录：
`/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/perf-tunnel-next-564780e-20261001/`。

从本 worktree 根运行：

```sh
export MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
# ART 必须是新的私有批准目录；IMAGE 为对应构建记录中的 resolved SHA。
ART="$ART" IMAGE="$IMAGE" ARM="$ARM" sh experiments/tunnel-throughput/measure-next.sh
```

Docker build/check 通过同一 `performance_lock.py`；本轮 Cargo Docker cache 使用
专属 `meshway-perf-tunnel-next-arm64`，本机 target 为 `target/perf-next`。
所有容器与网络由 runner 的精确名称 / ownership 标签清理。

失败记录也保留：`base-r1.json` 在容器创建前因锁等待提示混入捕获的 image ID 而
失败，随后用显式 SHA 重跑为 `baseline-*`；`hol-verify.log` 是 launcher 文件被
运行期间编辑导致的 shell 失败，未启动 Docker / workload，后续脚本冻结后运行。
早期 `tmux wait-for` 前台工具等待曾返回 `Shell.NotFoundError`，未用它冒充完成；
后续用专属进程的有界 kqueue exit 事件与真实 exit/JSON判断结果。
