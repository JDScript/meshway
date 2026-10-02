# 隧道吞吐专项（隔离实验）

## 版本边界

旧 `e69cb3d` Agent pump 的测量见 [RESULTS.md](RESULTS.md)，只作为历史证据。
`5029c0a` 起 Agent 已接入共享 `packet_io::run_with_activation` / PacketDevice；
历史集成源 `4edb70e` 的 native Linux queue32/128 复测见
[SHARED-PUMP.md](SHARED-PUMP.md)，两者不得混作当前版本性能。
P1c 后的 `564780e` 专项、安全修复基线与入站单因素复测见
[NEXT.md](NEXT.md)；R3/R5 retirement / setup retry 保证见
[CORRECTNESS.md](CORRECTNESS.md)。

用户反馈的 Docker 原生约 100 Gbit/s、Meshway 约 300 Mbit/s 是待复现测量；其
iperf 参数、资源、方向和运行版本未知。本目录提供独立复测，不补造这些参数。

## 三种测量边界

- `run.py`：同资源裸 Docker TCP，以及真实 transport-lab IPv4 TUN 的 NativeUDP
  直连 / 有序三节点中继。真实 hop Noise 和 E2E Noise、重放保护及完整路径授权。
  Lab 节点 ID 是三个字母，TUN MTU 1280；不是托管配置流水线。
- `managed.py`：真实 managed Manager 的 coherent `NodeDataplane` 编译、邻接启动、
  托管 TUN、IPv4 源 / 目的检查和 lease watchdog。使用现有 Rust 行为测试 fixture
  生成新密钥和授权，两节点 / 三节点，64 字节节点 ID，TUN MTU 1000。
  不含控制面注册、HTTP 配置拉取 / 刷新；测试 generation 的 lease 为 300 秒，
  测量结束或 SIGINT 后撤回。300 秒上限须覆盖每个 fixture 的所有 case。
  `apps/agent/src/managed_dataplane/throughput.rs` 仅编译进 ignored Linux 测试。
- `runtime.py`：相同 NativeUDP Runtime、hop / E2E 加密和路径授权，去除 TUN，
  以 1280 字节合成 payload 在受控 20k / 40k / 64k packet/s 负载下观察接收率、
  队列丢包和 CPU。每毫秒拆分成最多 16 包的有界 batch，调度落后时跳过 tick，实际发送数单独记录。
  **不是 TCP、没有 TCP 拥塞反馈，不能用其吞吐与 TUN TCP 的差值声称 TUN 增量成本。**

所有容器 native ARM64、每节点 1 CPU quota / 256 MiB、两个 Tokio worker。
CPU quota 不等于专用物理核；裸 Docker 保留内核 offload，TUN offload 关闭。
裸 TCP 是环境容量参考，不是同协议 / 同加密的包装损耗基线。

`managed.py --parallel 4 --directions forward reverse` 可仅复测所需 case。
`--loaded-rtt` 在 iperf client 发起后延迟 3 秒，于负载期间从 NYC 经授权 Mesh
路径向 SHA 发起 32 个 ICMP probes（0.5 秒间隔）；保存逐样本、丢失数、median /
p95 / max 及采样时刻，两臂同条件。非响应与 `<` 上界不会被填成 0 RTT。
`revalidate-shared.sh` 使用 queue32/128/128/32（ABBA）四个独立 fixture block，
每个路径 / 方向每臂两个 20 秒样本，固定同一 resolved image ID。Docker build
只读复制 Apple core workspace member以满足 Cargo workspace 解析，不改其代码。

## 运行

从 workspace 根运行。artifact 父目录必须位于已批准的 opencode TMP；新输出不覆盖旧文件。
使用自己的 `meshway-perf-throughput-*` tmux session。各 Python runner 自己取同一把锁，
不要在外层再锁一次；Docker build 和验证命令通过现有 lock wrapper。

```sh
export MESHWAY_PERF_LOCK=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
export ART=/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/throughput-e69cb3d-20261001

python3 experiments/transport/performance_lock.py docker build --platform linux/arm64 \
  -f experiments/tunnel-throughput/Dockerfile -t meshway-throughput:runtime-e69cb3d .
python3 experiments/transport/performance_lock.py docker build --platform linux/arm64 \
  -f experiments/tunnel-throughput/Dockerfile.managed -t meshway-throughput:managed-tun128-e69cb3d .

python3 experiments/tunnel-throughput/run.py --image meshway-throughput:runtime-e69cb3d \
  --output "$ART/matrix-new.json" --temp-dir "$ART" --seconds 10
python3 experiments/tunnel-throughput/managed.py --image meshway-throughput:managed-tun128-e69cb3d \
  --output "$ART/managed-control32-new.json" --temp-dir "$ART" --seconds 20 --iterations 2 --tun-queue-length 32
python3 experiments/tunnel-throughput/managed.py --image meshway-throughput:managed-tun128-e69cb3d \
  --output "$ART/managed-treatment128-new.json" --temp-dir "$ART" --seconds 20 --iterations 2 --tun-queue-length 128
python3 experiments/tunnel-throughput/runtime.py --image meshway-throughput:runtime-e69cb3d \
  --output "$ART/runtime-new.json" --temp-dir "$ART"
python3 experiments/transport/performance_lock.py python3 experiments/tunnel-throughput/summarize.py "$ART/managed-treatment128-new.json"
python3 experiments/transport/performance_lock.py python3 -m unittest discover -s experiments/tunnel-throughput -p 'test_*.py'
```

本目录不操作宿主机 route / DNS / firewall、Docker daemon、现有 demo 或 SSH。
新 Docker internal bridge、容器网络命名空间内 NET_ADMIN/TUN、独立密钥和数据。
精确名称加 run ownership label 校验清理；不 prune、不删除用户 image。

## 证据与限制

每个 TCP case 保存完整 iperf3 双端 JSON / 实际 argv（`-P1/-P4`、forward/reverse
分开、`-O2` warmup、`-t10` 或稳定 `-t20`）、MSS / TCP 窗口 / 重传信息。
记录 image ID / release build log、host / VM CPU 与内存、container quota / memory、
接口 MTU / route / offload、cgroup CPU user / system / throttling、进程 RSS / IO、
kernel UDP / TCP / fragmentation counters、可用时 Runtime 专属 drop 原因。
中继停止后检查两方向不可达，确认没有绕过必经节点。

CPU 窗口含 warmup、容器内 Docker exec 采样辅助进程和测量边界开销，除以 measured
receiver bytes 得到的是 **CPU ns/B 上界估计**，不是严格对齐的热路径成本。
Runtime 无 TUN 接收器另有周期性对齐 CPU 采样。`/proc/PID/io` 的 syscr/syscw
仅覆盖 read/write 等 IO，不能冒充所有 UDP sendto/recvfrom 系统调用。
Runtime status 每秒发布，差值存在边界采样滞后；保留原始快照而不是只取表格。
托管 Manager observation 未暴露 Runtime drop 计数。本专项的 ignored Linux fixture
通过一次性 Weak 注册及每秒 sidecar 采样取得 Runtime 计数，生产无此注册 / sampler，
不新增产品 API、不延长运行代际或保留密钥。

未测项包括 TLS-VLESS 托管 TUN、完整 HTTP enrollment/refresh 成本、真实 WAN、
perf 硬件采样和 offloaded TUN；不依据它们下结论。已知代码路径：每 UDP 包独立
sendto/recvfrom；TUN 非 offload 逐包读写；端点 E2E + 每跳 Noise；中继只开 hop
加密，不开端点 E2E 密文。存在多次 Vec 分配 / 拷贝、sending/endpoint cache 锁；
没有逐包 spawn。未经 matched A/B 不把这些代码观察当作已证明的主要瓶颈。

## 第一轮独立复测

`matrix10-fixed.json`，base `e69cb3d` 的 release image，NativeUDP 默认 queue64 /
work budget16 / OS UDP receive buffer；单次 10 秒，warmup 2 秒：

| 模式 | P1 forward / reverse | P4 forward / reverse |
| --- | --- | --- |
| 裸 Docker MTU1500 | 114.16 / 112.25 Gbit/s | 72.75 / 71.16 Gbit/s |
| 裸 Docker MTU1280 | 125.44 / 99.15 Gbit/s | 71.46 / 64.45 Gbit/s |
| lab TUN 直连 MTU1280 | 483.94 / 558.61 Mbit/s | 758.93 / 754.03 Mbit/s |
| lab TUN 三节点中继 MTU1280 | 422.41 / 414.72 Mbit/s | 528.47 / 526.43 Mbit/s |

这复现了数量级差距和中继折损，尚不是稳定性能声明。原 `matrix10.json` 在第二个
裸 MTU case 因 iperf logfile 同名追加而失败；已保留并标记 failed，不用它做结论。
Harness 现逐 case 清理精确的 `/run/tcp-*.json` logfile，修正后完整矩阵通过。

`managed10.json` 首轮直连约 442–448 Mbit/s；其中继 fixture 把 SHA 的下一跳
写成了 LAX 在另一网段的地址，无法建连，故整个 artifact 标记 failed。这是实验
fixture 错误，不是产品中继性能结果；已修正为 LAX 在 SHA 同一隔离网络的 endpoint。
`runtime10.json` 首轮 synthetic sender 超过公共 `MAX_BATCH_FRAMES=16`，全部被
合法拒绝（accepted=0）；该负载无效，不用于吞吐 / 丢包结论。修正后拆 batch，
并要求 sender exit=0 且 accepted>0，否则 runner 失败。失败原始数据保留以审计。
第二轮 `runtime10-fixed.json` 的直连六个 case 和首个中继 case 已完成，后续因
恢复检查错误地要求两端直接成为 authenticated peer 而失败。已修正为检查真实
下一跳并等待新的进程代际；最终完整运行另存 `runtime10-final.json`，不覆盖失败数据。

## 实际托管 generation 的稳定基线

`managed20-base.json` 完整通过，NativeUDP + `Noise_IK_25519_ChaChaPoly_BLAKE2s`
逐跳和 E2E，MTU1000 / MSS948，`-O2 -t20`，每个 case 两次，1 CPU / 256 MiB。
表内为两次独立复测的 receiver Mbit/s，均保留原值而非只报平均：

| 路径 | P1 forward | P1 reverse | P4 forward | P4 reverse |
| --- | --- | --- | --- | --- |
| 直连 | 436.72, 369.84 | 422.78, 381.23 | 434.13, 447.57 | 435.05, 435.84 |
| 三节点中继 | 131.53, 201.28 | 141.50, 223.77 | 297.92, 300.28 | 315.19, 314.83 |

原始 image 与环境资源保存在 artifact：Docker 29.4.0，ARM64 OrbStack Linux
`7.0.14-orbstack-00380-ga7e0a2dc9535`，VM 12 logical CPU / 9435258880 bytes RAM。
中继停止后两方向 ICMP 均失败，确认真实逻辑路径未绕过中继。
P1 中继波动明显，两个样本不能当成精确的长期平均或置信区间。

独立 lab 的损失证据（不是冒充托管的未暴露计数）：直连 P1 forward receiver
ingress queue 丢弃 1352 包、endpoint delivery queue 504 包、kernel UDP RcvbufErrors
31，receiver CPU 68.30% / quota throttling 0；中继 P1 forward LAX ingress 266、
outbound 401，CPU 47.93% / throttling 0。相关场景 IP fragment / reassembly 和
authentication / replay / unauthorized-path 错误均为 0。队列损失和 TCP 反馈是
可确认的性能限制；仅从 CPU 平均低于 quota 不能推断它没有热路径 CPU 成本。

主要托管 TUN 丢包定位、同二进制 queue32/128 A/B、保留 / 否决的优化及后续计划见
[RESULTS.md](RESULTS.md)。
