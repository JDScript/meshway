# 共享 packet pump 的 Linux 集成与 queue32/128 复测

## 结论与版本边界

当前完整源 **4edb70e7eaeccca0fe461c77a2d0c4479b19ed4d**，包含 5029c0a 引入的
`meshway_dataplane::packet_io::run_with_activation`、PacketDevice enum / batch 接口，
以及 main 合入的 queue128 与 cfg(test, Linux) Weak 统计 hook。

**共享 pump 的 Linux 正确性校验通过；queue32 仍有大量 TUN TX 丢包，queue128
在本轮所有 P4 路径 / 方向的双样本均值均有吞吐收益。** 同时观察到加载下 RTT
变长、用户态队列与 socket 丢包增加的代价，不能把扩大队列称为全部问题已解决。
生产 packet_io / tun 未做任何修改；fixture 无需 Rust 兼容修复。

旧 [RESULTS.md](RESULTS.md) 的 e69cb3d 旧 pump 数字保留为历史证据。本轮没有做
新旧 pump 的匹配 A/B，不拿两份不同历史 / 采样条件的表直接推断重构损耗。

## 方法与可复现命令

- 一次 native ARM64 release build，固定同一 resolved image ID：
  `sha256:6e94eb5955d3bfecb294db11e2355cae28044750442eb5f4a2b34f4ce5623a7d`。
- `meshway-managed-measure` ignored fixture 执行真实 managed Manager、coherent plan
  编译、Linux TUN、共享 pump、300 秒有界 lease watchdog；不模拟平台注册 / HTTP。
- NativeUDP + 逐跳 / E2E `Noise_IK_25519_ChaChaPoly_BLAKE2s`，64 字节节点 ID；
  完整授权直连 / 三节点有序中继，MTU1000、MSS948、TUN offload 关闭。
- 每容器 1 CPU quota / 256 MiB、两个 Tokio workers，P4，forward / reverse 分开；
  `iperf3 -O2 -t20`。Host Apple M3 Pro、12 CPU / 18 GiB；VM 12 CPU / 9435258880 bytes。
- ABBA 顺序 **q32-r1 → q128-r1 → q128-r2 → q32-r2**；每个 block 都是独立网络、
  容器和新密钥，包含直连 / 中继、两方向。仅通过 namespace `ip link set
  mwlab0 txqueuelen 32/128` 改队列，校验实际值，所有其它参数保持一致。
- 每个 fixture 仅两种方向 / 两个 case，避免不必要重跑；不重测裸 TCP / 无 TUN。
- 每个 case 同条件采样加载下 RTT：client 发起 3 秒后，从 NYC 经已授权 Mesh
  路径向 SHA 发起 ICMP echo，0.5 秒间隔、`-c32`、`-w19`。保存实际 transmitted /
  received（部分 case 发送数大于 32）、逐次成功 RTT、采样时间、median/p95/max。
  `<` 上界与未响应不会填成零。RTT 反映同一往返路径，不冒充单向延迟。

```sh
# 从当前 worktree 根运行；ART 必须是新建的批准目录，不能覆盖旧 JSON。
export MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
export ART=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/shared-pump-4edb70e-20261001
sh experiments/tunnel-throughput/revalidate-shared.sh
```

脚本自身对 build / checks 加锁，各 runner 自己持锁，不嵌套。Dockerfile 只读复制
Apple core workspace member以满足 Cargo 解析，没有更改 Apple 源码或 manifests。

## 吞吐、CPU 与 TUN drop

receiver Mbit/s，两次原值全部保留；括号是相应两次均值，不删波动较大的样本：

| P4 场景 | queue32 r1, r2（均值） | queue128 r1, r2（均值） | 均值变化 |
| --- | --- | --- | --- |
| 直连 forward | 420.56, 462.54（441.55） | 524.55, 532.41（528.48） | +19.7% |
| 直连 reverse | 397.03, 500.72（448.88） | 501.11, 528.02（514.57） | +14.6% |
| 中继 forward | 312.83, 341.44（327.13） | 370.94, 411.62（391.28） | +19.6% |
| 中继 reverse | 297.20, 340.44（318.82） | 387.25, 413.76（400.51） | +25.6% |

直连反向 queue32 第二次 500.72 与 queue128 第一次 501.11 几乎相同，样本分布
存在明显重叠；N=2 不给置信区间、不宣称每种环境都有固定百分比收益。

| P4 场景 | source TUN TX dropped q32 两次 | q128 两次 | 总 CPU ns/B 上界均值 q32→q128 |
| --- | --- | --- | --- |
| 直连 forward | 25800, 28285 | 442, 680 | 35.95 → 31.47 |
| 直连 reverse | 25143, 29667 | 520, 546 | 35.29 → 32.39 |
| 中继 forward | 19859, 20906 | 6, 64 | 51.25 → 48.40 |
| 中继 reverse | 19301, 21071 | 29, 37 | 52.94 → 47.29 |

CPU 窗口包含 omitted warmup / 容器内辅助进程 / 采样边界延长，除以 measured
receiver bytes 是上界估计，不是硬件 profile 或严格对齐的纯热路径 CPU 成本。
queue128 接收端 CPU：直连约 **92.7–94.6%**，中继约 **71.1–72.4%**；
queue32 直连约 **87.9–90.8%**，中继约 **63.8–64.7%**。

## 剩余损失：不能只看 TUN 队列

两个代表性 forward r1 case 的独占 drop 计数（所有 before/after 原值也已保存）：

| 计数 | 直连 q32→q128 | 中继 q32→q128 |
| --- | --- | --- |
| source outbound_queue_full | 4 → 4192 | 0 → 1450 |
| 最终 receiver ingress_queue_full | 0 → 990 | 2 → 827 |
| 最终 receiver endpoint_delivery_full | 79 → 15883 | 47 → 9975 |
| 最终 receiver UDP RcvbufErrors | 0 → 294 | 0 → 104 |
| 中继 ingress / outbound queue full | — | 24/1 → 1310/1605 |
| source TUN TX dropped | 25800 → 442 | 19859 → 6 |
| TCP measured retransmits | 23239 → 20466 | 18042 → 13874 |

其它方向 / 第二次显示同类迁移：queue128 的最终 delivery queue drop 约
**9975–16319**，中继 ingress **1310–2150**、outbound **1605–1983**，
直连 kernel UDP receive error **199–294**。TUN 丢包被显著减少后，更多真实负载
进入了 Runtime / 接收器，瓶颈迁移到这些有界队列；总体 TCP 重传下降但远未消失。
队列 high-water、occupancy、RSS、CPU user/system/throttling、IO、IP fragmentation
和其它 drop 原因均在原始 JSON / `summary.txt` 中，计数边界存在每秒 sidecar 滞后。

## 加载下 RTT 的代价

每项是 r1 / r2 的 **median；p95**（ms），只由成功 echo 样本计算：

| P4 场景 | queue32 median；p95 | queue128 median；p95 |
| --- | --- | --- |
| 直连 forward | 0.485/0.506；1.40/1.13 | 0.799/0.744；1.97/1.59 |
| 直连 reverse | 0.543/0.394；1.50/1.13 | 0.846/0.897；1.51/1.90 |
| 中继 forward | 0.811/0.803；1.55/1.42 | 1.145/1.170；2.36/2.41 |
| 中继 reverse | 1.135/0.727；1.87/2.00 | 1.345/1.150；2.29/2.47 |

queue32 echo 总计 **258 transmitted / 256 received**；queue128 **263 / 256**。
较大队列提高吞吐，但成功样本 median / p95 多数变长，且有更多未返回 echo；
不能声称延迟也优化。采样量少且有幸存样本偏差，不用 RTT p95 推断业务 SLO。

## 正确性、隔离和 handoff

- native Linux `cargo fmt --all --check`、Clippy `--all-targets -D warnings` 通过。
- dataplane **62** 个 release 行为测试通过（含共享 callback batch 真加密多跳、
  activation、lease expiry / closed watch 取消 blocked receive、withdrawal 取消
  full callback write 无延后注入等）；managed **4** 个通过，packet / lease /
  withdrawal **7** 个通过。2 个 ignored fixture tests 通过实际 Docker命令运行。
- Python 统计 / RTT 校验 **4** 个通过。
- 16 个实际 P4 workloads 全部成功，4 个 artifact 均 `status=ok`、
  `cleanup_errors=[]`，各 block 的 relay stop 两方向不可达证明均成立。
- 所有 Docker build / workload 使用同一 performance lock 串行、native ARM64；
  无 host network / privileged、无宿主机 route/DNS/firewall/daemon 改动，未操作 live
  demo 或根前后端。密钥 / 数据独立，精确 run ownership 清理，无 prune。
- 已完整结束旧路径的构建 / 测量，关闭本任务 idle tmux 后，使用 `git worktree move`
  安全迁移到 `/Users/zeyu/Developer/meshway/.worktree/perf-shared-pump`。
- 原始新数据：
  `/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/shared-pump-4edb70e-20261001/`。
  `q32-r1.json` / `q128-r1.json` / `q128-r2.json` / `q32-r2.json`、`comparison.json`、
  `aggregate.json`、`summary.txt`、`run.log`、`image-id.txt`；旧目录的 JSON 未覆写。

本轮支持保留当前 queue128 的正确性及吞吐方向，但需连同 RTT / 剩余 drop 代价
交给 main 决定部署。下一轮只建议分别测试 userspace queue64→256、UDP buffer
单因素（另一个保持固定），联合评估内存上界、drop 迁移、加载下 RTT与 CPU。
未执行 queue256 / buffer tuning，未新增其它优化，P1 / VLESS / WAN 未在本轮复测。
