# 分层性能测量

性能脚本显式运行，不进入 `mise run check` 或普通行为测试。每层只回答自身处理边界的成本；本机接纳、对端接收、真实业务交付分别计量。

## 层级

| 层级 | 处理步骤 | 状态与入口 |
| --- | --- | --- |
| L0 | 内存中的包编解码、密码与证明原语；没有 socket、TUN 或调度链 | 定义；待实现／由其他工作包提供 |
| L1 | 正式协议 provider 的原生 socket I/O，对照公共 datagram Session；代理协议保留自身 TLS／WS | 已实现；[承载性能对照](/v2/node/transport#验证与性能对照)，`perf:transport` |
| L2 | 两个 Linux namespace 中的真实 TUN、Tokio、正式 UDP provider 与 Session，原样转发明文 IP 包 | 已实现；`perf:dataplane` 的 `plain` |
| L3 | L2 I/O 加正式 SecureNode、真实认证／加密、路径证明与重放处理 | 定义；`secure` 待实现／由其他工作包提供 |
| L4 | L3 加 connectivity Manager 的真实邻接获取、承载调度与生命周期 | 定义；`manager` 待实现／由其他工作包提供 |
| L5 | 完整 meshwayd、Control／Policy、真实执行资格、Manager、SecureNode、TUN 与系统业务应用 | [常驻三节点 lab](/v2/node/docker-lab)；验收范围见[数据面](/v2/node/dataplane) |

L2 的 `raw` 是同一隔离网络中直接运行 iperf3 的内核参照，不经过 Meshway 代码；它不是 L0。L2 `plain` 不启用认证、加密、策略或控制面，也不代表 Meshway 业务数据面的吞吐。L3／L4 枚举预留仅为工具扩展接口，当前请求这些阶段会在创建资源前失败。

## L2 实现

`tools/meshway-dataplane-perf` 的两个节点分别使用 `meshway-tun::Tun` 打开独占、非持久化 L3 TUN，地址为 `100.127.0.1/30` 与 `100.127.0.2/30`。每个节点通过 `meshway-transport-udp::UdpTransport` 和 `meshway-transport::Session` 收发明文 IP 包，收到的完整 UDP payload 写入对端 TUN。UDP source 必须等于显式配置的对端，截断与发送完成错误阻断该样本；这项 source 检查不是身份认证。

| 参数／资源 | 行为 |
| --- | --- |
| `--stage plain` | 本页实现的 L2 明文阶段 |
| `--role a\|b` | 节点角色与 TUN 地址；公共 `NodeOptions` 是后续阶段的节点边界 |
| `--bind`、`--peer` | 当前 namespace 中的真实 UDP 本地／对端 socket 地址 |
| `--mtu` | 节点允许 1280–65000；harness 限制 1280–1472，避免默认 1500 字节外层 MTU 分片 |
| `--batch` | 每方向一轮最多处理 1–1024 个立即就绪的包，然后显式 yield；默认比较 1 与 32 |
| 发送预算 | 所有 batch 变体保持 1024 帧、`MTU × 1024` 字节、1 个在途操作 |
| runtime | 每个节点一个 Tokio current-thread runtime；两个独立节点进程 |
| 生命周期 | 运行期限或 SIGINT 停止新工作，取消并 shutdown/join 正式 Session 的发送 owner |
| `MESHWAY_PROFILE` | 0 关闭；非零为采样分母，复用 `meshway_logging::profile` |

事件批量只合并一轮已就绪包的调度与 Session 提交；TUN 及 UDP 仍逐包 syscall。当前没有实现 `recvmmsg/sendmmsg`，也没有实现 `UDP_SEGMENT`／`UDP_GRO`。正式 UDP provider／Session 的逐包接口、完成与取消边界未被替代；本页不报告内核批量 I/O 的收益。

## 运行

需要本机锁定的 Rust／Zig 工具链，以及支持 `/dev/net/tun` 的 Linux Docker engine。脚本使用普通 Docker bridge 上两个一次性容器的独立 network namespace；仅这些容器拥有 `NET_ADMIN`，没有 host network、privileged 或宿主路由／DNS 修改。

```sh
mise run perf:dataplane

mise run perf:dataplane --batch 1 32 128 --profile 0 128 \
  --repeats 3 --seconds 15 --omit 3 --mtu 1400 --payload 1200 \
  --udp-rate 500M --cpus 2 --memory 512m
```

每个配置分别运行 TCP 单流和 UDP 定速率 iperf3，至少两次重复；重复之间反转配置顺序。默认每次先省略 2 秒 warmup，再测量 8 秒。release 交叉构建缓存独立于其他任务，位于 `target/dataplane-perf`；测量使用报告目录中不可变的二进制副本。Docker userspace 使用锁定 node-lab OS 源码对应的 immutable image，若尚无该 image，则在本工作区私有输出目录构建；不部署或修改常驻 lab。

`raw` 的 veth MTU 与内层 TUN MTU 相同，保留内核默认 offload；`plain` 的外层 veth 保留 1500 MTU、TUN 不启用 offload。两者经过同一内部 Docker bridge。TCP 应用写入块与实际 TCP 分段不是同一概念，TCP 不通过 Mbps／假定包长反推 pps。UDP 应用 payload 由 `--payload` 固定，IPv4／UDP 头额外占 28 字节。

## 报告契约

结果位于当前工作区 `.meshway/perf/dataplane/<run>/`。测量锁及构建缓存也归当前工作区所有。

| 文件／字段 | 内容 |
| --- | --- |
| `results.json` | schemaVersion、Git revision／dirty、源码 digest、binary SHA-256、参数、环境与逐次样本 |
| `summary.md` | 成功样本中位数，按 stage、batch、profiling、TCP／UDP 分组；失败数量另列 |
| `bin/meshway-dataplane-perf` | 本次固定 Linux release 二进制 |
| `<sample>/a.stdout`、`b.stdout` | 两个 plain 节点的 readiness；raw 没有节点进程 |
| `<sample>/a.stderr`、`b.stderr` | 日志及 profiling 原始 JSON |
| `<sample>/iperf-client.json`、`iperf-server.json` | iperf3 原始输出；相邻 `.stderr` 保存诊断 |
| `environment` | 内核、CPU 信息、目标架构、Rust／Zig／iperf3、image 来源、每容器 CPU／内存配额、MTU／包长／速率 |
| `goodputMbps` | 接收端实收应用 payload 吞吐；不计封装头 |
| `packetsPerSecond`、`lostPercent` | UDP 接收端交付报文数／秒和丢包；TCP 为 null |
| `cpuSeconds`、`cpuPercent` | 两个容器 cgroup CPU 增量；百分比为 CPU 秒／完整客户端墙钟时间 × 100，100% 等于一核 |
| `allocations`、`copiedBytes`、`peakMemoryBytes` | 当前未测，明确为 null |

吞吐采用 iperf3 排除 warmup 的接收区间。CPU 覆盖完整客户端区间，包括 warmup 和采集开销，包含节点、iperf3 与容器内辅助命令；不是仅 Node 的 CPU。固定 CPU quota 不等于绑核或独占宿主，结果只代表记录的环境。失败样本与原始日志保留，指标不填零，脚本返回失败。

Profiling 单独记录启用和关闭的样本。`plain.tun.read`、`plain.udp.send`、`plain.udp.recv`、`plain.tun.write` 包装 future 的同步 poll 工作，不计 await 挂起时间；正式 UDP／Session 的更深层指标也保留在同一输出中。自耗时只扣除实际采到的嵌套 scope，应结合 samples、calls 与采样分母解释，不将采样累计纳秒直接当作整次 CPU 时间。最终吞吐取 profiling 关闭样本。

iperf3 结果取接收端 `server_output_json.end.sum_received`，并核对 UDP 实收字节数等于交付报文数乘以固定 payload。不能用 iperf3 3.16 的 UDP `end.sum` 替代该字段；该字段可能在真实收到数据时仍报告零字节。Profiling 报告为累计快照，汇总只取每个节点最后一份，不累加同一进程的多份历史快照。

## L2 实测参考

2026-10-06，报告 `.meshway/perf/dataplane/20261006t222145z-cb007812/summary.md`，源码 revision `a00e12cf`。20 个样本全部成功，每个配置／负载重复两次；每次省略 2 秒 warmup 后测量 8 秒。以下是该次运行的中位数，不跨运行挑选样本。

| 环境 | 值 |
| --- | --- |
| Kernel／架构 | OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`，aarch64 |
| Userspace | 锁定 Ubuntu 24.04，iperf3 3.16 |
| 编译 | Rust 1.98.0、Zig 0.17.0、cargo-zigbuild 0.23.4，release，`aarch64-unknown-linux-musl` |
| 资源 | 每个容器 2 CPU quota、512 MiB 内存；每个 Node 一个 current-thread runtime |
| 网络／负载 | 同一内部 Docker bridge，两独立 namespace；MTU 1400；TCP 单流，UDP 1200 字节 payload、500 Mbps offered rate |
| 源码 SHA-256 | `419d2351f50c67e2e275db76435f42d29eb763a9cf135fc9f9dd60675e3484b4` |
| Binary SHA-256 | `96e74525c1b6a1d10c5de81e8bd8a228211d7a36c4a5eb8220418d819b6619ca` |

### Profiling 关闭

| 模式 | Batch | TCP Mbps | TCP CPU % | UDP Mbps | UDP pps | UDP 丢包 % | UDP CPU % |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| raw 内核参照 | — | 111991.93 | 140.87 | 411.80 | 42895.41 | 17.63 | 127.66 |
| plain 逐包事件 | 1 | 2353.79 | 156.87 | 394.06 | 41048.37 | 21.17 | 156.77 |
| plain 有界事件批量 | 32 | 2129.49 | 120.65 | 394.23 | 41065.87 | 21.16 | 151.78 |

CPU 为两容器合计，100% 表示一核；区间含 warmup。TCP pps 未测。raw TCP 保留内核 offload，其结果是该 VM／bridge 的内核参照；UDP 的 500 Mbps 是输入速率，该次 raw 自身已有丢包，不能将 plain 的全部丢包归因于转发器。Batch 32 在这次测量中 TCP CPU 较低，但吞吐没有高于 Batch 1；这组数据不证明更大 batch 总是更快。

### Profiling 启用，采样分母 128

| 模式 | Batch | TCP Mbps | TCP CPU % | UDP Mbps | UDP pps | UDP 丢包 % | UDP CPU % |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| plain | 1 | 2337.06 | 165.45 | 380.16 | 39599.73 | 23.96 | 159.45 |
| plain | 32 | 2103.76 | 120.66 | 383.56 | 39954.29 | 23.25 | 154.64 |

所有成功 profiling-on 样本、两个节点最后一份累计快照的采样自 CPU 汇总如下；不含未采样工作及最后一次报告后的尾段。

| 指标 | CPU 样本数 | 平均自 CPU ns／样本 | 采样自 CPU 合计 ms |
| --- | ---: | ---: | ---: |
| `plain.tun.read` | 154982 | 416.76 | 64.59 |
| `plain.tun.write` | 93700 | 1438.15 | 134.75 |
| `plain.udp.recv` | 153899 | 467.13 | 71.89 |
| `plain.udp.send` | 54766 | 478.36 | 26.20 |
| `transport.submit` | 54730 | 747.25 | 40.90 |
| `udp.recv.poll` | 153899 | 400.33 | 61.61 |
| `udp.send.poll` | 93873 | 1854.74 | 174.11 |

四个 `plain.*` 外层边界中，TUN write 的采样自 CPU 最大。下钻到正式 provider 后，`udp.send.poll` 是所有已报告指标中最大的采样自 CPU 项。`plain.udp.send` 的自耗时扣除了采到的嵌套 provider／提交工作，不能将它单独解释为 UDP 发送总成本。此次没有内核批量 I/O 或 GSO／GRO 的数字。
