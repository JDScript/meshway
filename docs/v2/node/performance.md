# 分层性能测量

性能脚本显式运行，不进入 `mise run check` 或普通行为测试。每层只回答自身处理边界的成本；本机接纳、对端接收、真实业务交付分别计量。

## 层级

| 层级 | 处理步骤 | 状态与入口 |
| --- | --- | --- |
| L0 | 进程内受保护 circuit：真实编译器许可、SecureNode 握手、Noise、逐验证者 MAC、流准入与输出 guard；没有 socket、TUN 或控制面 I/O | 已实现；[L0 受保护 circuit](#l0-受保护-circuit)，`perf:circuit` |
| L1 | 正式协议 provider 的原生 socket I/O，对照公共 datagram Session；代理协议保留自身 TLS／WS | 已实现；[L1 承载对照](#l1-承载对照)，`perf:transport` |
| L2 | 两个 Linux namespace 中的真实 TUN、Tokio、正式 UDP provider 与 Session，原样转发明文 IP 包 | 已实现；`perf:dataplane` 的 `plain` |
| L3 | L2 I/O 加正式 SecureNode、真实认证／加密、路径证明与重放处理 | 定义；`secure` 待实现／由其他工作包提供 |
| L4 | L3 加 connectivity Manager 的真实邻接获取、承载调度与生命周期 | 定义；`manager` 待实现／由其他工作包提供 |
| L5 | 完整 meshwayd、Control／Policy、真实执行资格、Manager、SecureNode、TUN 与系统业务应用 | [常驻三节点 lab](/v2/node/docker-lab)；验收状态见[验收范围](/v2/node/validation) |

L2 的 `raw` 是同一隔离网络中直接运行 iperf3 的内核参照，不经过 Meshway 代码；它不是 L0。L2 `plain` 不启用认证、加密、策略或控制面，也不代表 Meshway 业务数据面的吞吐。L3／L4 枚举预留仅为工具扩展接口，当前请求这些阶段会在创建资源前失败。

## 共同报告口径

每次运行保存程序 hash、源码 digest、工具链、平台、参数与逐次原始样本；失败样本保留而不填零。启用 `MESHWAY_PROFILE` 时另保留抽样参数和首尾累积记录，并与插桩关闭的同一程序比较吞吐／进程 CPU，校验 observer 开销与计数边界；细层采样计时含插桩成本，不能直接当作未插桩程序的精确 CPU 占比。采样变量与输出字段见[运行日志](/v2/logging#显式分层性能采样)。

## L0 受保护 circuit

```sh
mise run perf:circuit
```

`scripts/perf/circuit.py` 显式构建 `tools/meshway-circuit-perf` 的 release 程序（缓存 `target/circuit-perf`）。单同步线程通过真实 Policy 编译器与 `tools/meshway-static-authority` 的静态签发建立 2–8 节点链，使用正式 `SecureNode` 完成 Noise／MAC circuit 握手与 TCP 流建立，再运行双向稳态包流。每包保留解析、业务流准入、Noise、逐验证者 MAC、codec、当前执行资格检查与原输出 guard；测量范围包含内存接收／交付，不包含 socket、TUN 或控制面 I/O。52-byte 输入为纯 IPv4/TCP ACK，1280-byte 输入携带 1228-byte TCP payload；握手与 warmup 不进入稳态窗口。

默认矩阵为 2／3／5 节点 × 1280／52 字节 × profile 关闭与 1/128 采样，各重复三次。角色成本使用线程 CPU 时钟与单调时钟；接收角色包含上游 frame release 检查，终点包含 guarded 明文交付。报告写入执行脚本所在工作树的 `.meshway/perf/circuit/<run>/`：`results.json` 保存源码 digest、Git revision、二进制 SHA256、工具链、平台、参数、每角色 calls／CPU／wall、逐段帧字节与中位数／p10／p90；`summary.md` 另列 profile 采样的阶段分解。采样分解带 observer effect，不能加总当作 profile 关闭时的 CPU 预算。

### L0 实测参考

Apple M3 Pro、macOS、Rust 1.98.0，profile 关闭，每格为三次独立进程重复的中位数；guard 次数来自 profile 采样的精确调用计数。`36ca2117` 把每次执行资格检查改为单次时钟读取；`90bc1aec` 让每个同步操作共用入口一次 `Reading`，并按截止调度其他 circuit 的维护扫描。

| 节点 | IP 字节 | pps `8f7d0c46` | pps `36ca2117` | pps `90bc1aec` | 中继 µs/包 `8f7d0c46` → `36ca2117` → `90bc1aec` | guard 次数/包 `36ca2117` → `90bc1aec` |
| ---: | ---: | ---: | ---: | ---: | --- | --- |
| 2 | 1280 | 21,314 | 37,222 | 46,126 | — | 87 → 16 |
| 2 | 52 | 28,194 | 68,860 | 109,774 | — | 87 → 16 |
| 3 | 1280 | 12,792 | 22,965 | 27,215 | 28.0 → 14.9 → 12.1 | 142 → 24 |
| 3 | 52 | 17,766 | 38,239 | 58,175 | 21.2 → 9.6 → 6.1 | 142 → 24 |
| 5 | 1280 | 6,931 | 11,827 | 14,581 | 29.9 → 17.1 → 13.6 | 252 → 40 |
| 5 | 52 | 8,415 | 17,302 | 23,778 | 25.1 → 12.0 → 8.5 | 252 → 40 |

`36ca2117` 之前的 profile 采样中，`control.guard`（执行资格检查）是最大的 self CPU 项，每个单向包在 2／3／5 节点分别触发 87／142／252 次，与包长无关；`90bc1aec` 后每个公开 send／receive 保留入口、提交、产出与 postcheck 共 5 次真实检查；MAC verify／append 随路径节点数增长；`frame.base()` 重复编码的独立成本约 1 µs/包。线上帧字节与[字节账本](/v2/discussions/packet#用一条公式覆盖路径的每一段)公式逐段一致。

## L1 承载对照

```sh
mise run perf:transport --protocol all --payload 64 1200 --batch 1 32 \
  --messages 200000 --warmup 1000 --repeats 3
```

`scripts/perf/transport.py` 显式构建 `tools/meshway-transport-perf` 的 release 程序（缓存 `target/transport-perf`），共用正式 provider／Session 实现，交替执行 raw／datagram 次序。同仓库测量互斥，每次运行保存独立二进制快照。结果位于仓库公共目录 `.meshway/perf/transport/<run>/`：二进制、`results.json`、`summary.md` 及各次 stdout／stderr。表格为各模式中位数，比值为有效配对重复的比值中位数。

| 协议／封装 | raw 基线 | Meshway datagram 对照 |
| --- | --- | --- |
| `tcp`（Native TCP） | 原生 TCP 字节流收发 | 公共 framing 封装／恢复 datagram |
| `udp`（Native UDP） | 原生 UDP 报文收发 | UDP 承载 datagram，并在对端恢复 |
| `vless-tcp`（VLESS TCP/TLS） | 完整 VLESS＋TLS 协议 I/O | 同一协议栈承载并恢复 datagram |
| `vless-ws`（VLESS WebSocket/TLS） | 完整 VLESS＋WS＋TLS 协议 I/O | 同一协议栈承载并恢复 datagram |
| `trojan`（Trojan TCP/TLS） | 完整 Trojan＋TLS 协议 I/O | 同一协议栈承载并恢复 datagram |

- 负载为本机固定 payload 批量 echo；两种模式使用相同 socket 参数、stream 缓冲上限、连接条件与批量策略。raw 在底层不支持 vectored I/O 时采用同样有界的批量字节缓冲。
- 吞吐按发起端实际收回的业务 payload 计，排除连接建立和 warmup；消息速率是逻辑消息交付计数；RTT 从批次提交到各条 echo 交付；CPU 时间来自完整子进程（含 warmup／初始化）。分配、复制及峰值内存尚无计数器，明确为 null。
- 报告把 Mesh 认证、端到端加密、路由、动态 ABI 与 TUN 标为未启用；这些增量成本由 L2 以上分层测量。新增协议须补齐同样的 raw／datagram 对照；其他项目的整机结果只作外部参考。

L1 尚未覆盖的对照轴：每邻接单连接与有界小连接池、接近实际 MTU 的负载与多流、隔离环境中的延迟／丢包／乱序／断连、动态库适配调用、排队取消与承载切换等生命周期、快慢通道与单通道故障等跨流干扰。比较连接数量时须保持全局排队预算一致，并先满足流映射稳定、不重复交付、未认证序号不推进重放窗口等正确性条件。

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
