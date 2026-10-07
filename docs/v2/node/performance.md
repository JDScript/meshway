# 分层性能测量

性能脚本显式运行，不进入 `mise run check` 或普通行为测试。每层只回答自身处理边界的成本；本机接纳、对端接收、真实业务交付分别计量。

## 层级

| 层级 | 处理步骤 | 状态与入口 |
| --- | --- | --- |
| L0 | 进程内受保护 circuit：真实编译器许可、SecureNode 握手、Noise、相邻 MAC、端点流准入与输出 guard；没有 socket、TUN 或控制面 I/O | 已实现；[L0 受保护 circuit](#l0-受保护-circuit)，`perf:circuit` |
| L1 | 正式协议 provider 的原生 socket I/O，对照公共 datagram Session；代理协议保留自身 TLS／WS | 已实现；[L1 承载对照](#l1-承载对照)，`perf:transport` |
| L2 | 两个 Linux namespace 中的真实 TUN、Tokio、正式 UDP provider 与 Session，原样转发明文 IP 包 | 已实现；`perf:dataplane` 的 `plain` |
| L3 | L2 I/O 加正式 SecureNode、真实认证／加密、路径证明与重放处理 | 已实现；[L3 实现与实测](#l3-实现与实测)，`perf:dataplane` 的 `secure` |
| L4 | L3 加 connectivity Manager 的真实邻接获取、承载调度与生命周期 | `perf:dataplane` 的 `manager`；与 L3 共用流水线驱动 |
| L5 | 完整 meshwayd、Control／Policy、真实执行资格、Manager、SecureNode、TUN 与系统业务应用 | [常驻三节点 lab](/v2/node/docker-lab)；验收状态见[验收范围](/v2/node/validation) |

L2 的 `raw` 是同一隔离网络中直接运行 iperf3 的内核参照，不经过 Meshway 代码；它不是 L0。L2 `plain` 不启用认证、加密、策略或控制面，也不代表 Meshway 业务数据面的吞吐。

同一 L2/L3 harness 另含 `wg-direct`／`wg-relay` [内核 WireGuard 参照](#内核-wireguard-参照)，用于同环境、配额与应用口径的实现差值；这不是额外的 Meshway 功能层。

## 共同报告口径

每次运行保存程序 hash、源码 digest、工具链、平台、参数与逐次原始样本；失败样本保留而不填零。启用 `MESHWAY_PROFILE` 时另保留抽样参数和首尾累积记录，并与插桩关闭的同一程序比较吞吐／进程 CPU，校验 observer 开销与计数边界；细层采样计时含插桩成本，不能直接当作未插桩程序的精确 CPU 占比。采样变量与输出字段见[运行日志](/v2/logging#显式分层性能采样)。

## L0 受保护 circuit

```sh
mise run perf:circuit
```

`scripts/perf/circuit.py` 显式构建 `tools/meshway-circuit-perf` 的 release 程序（缓存 `target/circuit-perf`）。单同步线程通过真实 Policy 编译器与 `tools/meshway-static-authority` 的静态签发建立 2–8 节点链，使用正式 `SecureNode` 完成 Noise／MAC circuit 握手与 TCP 流建立，再运行双向稳态包流。每包保留端点解析与业务流准入、Noise、相邻 MAC、codec、当前执行资格检查与原输出 guard；测量范围包含内存接收／交付，不包含 socket、TUN 或控制面 I/O。输入复制到池缓冲模拟 TUN 读入，计入源角色时间；后续角色传递同一分配。52-byte 输入为纯 IPv4/TCP ACK，1280-byte 输入携带 1228-byte TCP payload；握手与 warmup 不进入稳态窗口。

默认矩阵为 2／3／5 节点 × 1280／52 字节 × profile 关闭与 1/128 采样，各重复三次。角色成本使用线程 CPU 时钟与单调时钟；接收角色包含上游 frame release 检查，终点包含 guarded 明文交付。报告写入执行脚本所在工作树的 `.meshway/perf/circuit/<run>/`：`results.json` 保存源码 digest、Git revision、二进制 SHA256、工具链、平台、参数、每角色 calls／CPU／wall、逐段帧字节与中位数／p10／p90；`summary.md` 另列 profile 采样的阶段分解。采样分解带 observer effect，不能加总当作 profile 关闭时的 CPU 预算。

### L0 历史 v4 实测参考

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

### L0 v5 流水线内联对照

`20261007T011142Z-68ec103e` 完成 `perf:circuit` 默认 36 格矩阵，全部成功、源码 digest 前后一致。对照 `20261007T000738Z-f0d352b7` 的 v5 单段实现；相同 macOS／Apple M3 Pro、每格三次独立进程重复、profile 关闭。L0 使用同一流水线的 inline 路径，不创建 worker。CPU 为完整 harness 的线程 CPU／实交付 IP 包；Mbps 按 IP 字节计算，区别于下文 iperf 应用 goodput。

| 节点 | IP 字节 | pps 前 → 后 | IP Mbps 前 → 后 | CPU µs／包前 → 后 | guard／包前 → 后 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1280 | 53,343 → 57,190 | 546.24 → 585.63 | 18.598 → 17.479 | 16 → 15 |
| 3 | 1280 | 36,643 → 37,987 | 375.23 → 388.99 | 27.248 → 26.308 | 23 → 21 |
| 5 | 1280 | 22,537 → 23,402 | 230.78 → 239.64 | 44.317 → 42.698 | 37 → 33 |
| 2 | 52 | 140,193 → 143,431 | 58.32 → 59.67 | 7.129 → 6.972 | 16 → 15 |
| 3 | 52 | 91,447 → 92,414 | 38.04 → 38.44 | 10.917 → 10.784 | 23 → 21 |
| 5 | 52 | 55,210 → 55,639 | 22.97 → 23.15 | 18.077 → 17.939 | 37 → 33 |

guard 为开启采样时的真实调用计数，表中四舍五入，包含极少的维护检查。L0 没有 carrier／TUN 适配器，`carrier.*` 不适用。新 binary SHA-256 为 `50e8d452102f7a0db23239bda7f108ff996da0640f4accdce45ce333507c392d`；完整报告位于工作树 `.meshway/perf/circuit/20261007T011142Z-68ec103e/`。

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
| `--io per-packet\|batch` | plain 的逐包 syscall 与内核批量 I/O 对照；默认逐包，harness 对两者分别测量 |
| `--role a\|b` | 节点角色与 TUN 地址；公共 `NodeOptions` 是后续阶段的节点边界 |
| `--bind`、`--peer` | 当前 namespace 中的真实 UDP 本地／对端 socket 地址 |
| `--mtu` | 节点允许 1280–65000；harness 限制 1280–1472，避免默认 1500 字节外层 MTU 分片 |
| `--batch` | 每方向一轮最多处理 1–1024 个立即就绪的包，然后显式 yield；默认比较 1 与 32 |
| 发送预算 | 所有 batch 变体保持 1024 帧、`MTU × 1024` 字节、1 个在途操作 |
| runtime | 每个节点一个 Tokio current-thread runtime；两个独立节点进程 |
| 生命周期 | 运行期限或 SIGINT 停止新工作，取消并 shutdown/join 正式 Session 的发送 owner |
| `MESHWAY_PROFILE` | 0 关闭；非零为采样分母，复用 `meshway_logging::profile` |

`per-packet` 仅合并就绪包的事件调度，TUN／UDP 保持逐包 syscall；`batch` 使用正式 TUN `recv_batch`／`send_batch`、UDP `adopt_batch` 与 Session `submit_batch`／`recv_batch`。TUN 拆合、UDP GSO／GRO／mmsg 的实际能力和成功调用次数分别记录；只处理就绪前缀，不等待凑批。两种模式使用相同帧／字节／操作预算，完成与取消语义见[承载会话](/v2/node/transport#本机接纳与完成)。

### TUN 批量接口与能力验证

`meshway-tun::Options::with_batch_io(true)` 显式请求 Linux `IFF_VNET_HDR`。创建时通过安全的 tun-rs 配置入口探测 `CSUM + TSO4 + TSO6`，再探测附加的 `USO4 + USO6`；基础 offload 不可用时关闭新建描述符并创建逐包设备。`Tun::capabilities()` 返回实际启用值。默认 TUN 选项仍为逐包模式，L2 `plain --io batch` 显式启用该接口。

- `recv_batch(&mut PacketBatch)` 每次交付最多 32 个完整 IP 包／64 KiB，不等待凑批。一个内核超包的剩余分段留在有界接收状态中；逐包 `recv` 也可消费这些分段。当前超包上限为 65,535 字节／1,024 段，超限返回错误。
- `send_batch(&[&[u8]])` 返回一次写入的前缀包数；调用者对剩余后缀继续使用原输出 guard。一次调用至多写一个超包，不兼容时写一个普通包。此结果只表示本机内核接纳。
- TCP／UDP 的 IPv4／IPv6 拆分重建长度与校验和；TCP 同时调整序号及末段标志。合并只接受相邻、同流、校验和有效且序号／IPv4 ID 连续的包，短尾段结束合并。IP 分片、IPv4 options、IPv6 扩展头、TCP SYN／RST／URG 等不参与合并。
- 运行库仍使用独立包分配；这里不声明单缓冲或零复制已完成。

OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`、aarch64 的独立无网络容器验证结果：`IFF_VNET_HDR`、`CSUM`、`TSO4`、`TSO6`、`USO4`、`USO6` 均成功启用。真实内核 TCP IPv4／IPv6 各发送 8,192 字节，观察到至少 5 个规范化分段，逐段序号、长度与校验和通过；双栈 TCP 三包合并写入后应用完整收到 3,000 字节，IPv4／IPv6 UDP 三包合并写入均由内核恢复为三个应用报文。对应入口为 `meshway-tun` 的显式隔离测试 `offload_capabilities_and_normalized_packet_io`；原逐包设备生命周期／双栈 I/O 测试同时通过。L2 吞吐与 CPU 对照见[内核批量实测](#l2-内核批量实测)。

## 运行

需要本机锁定的 Rust／Zig 工具链，以及支持 `/dev/net/tun` 和内核 WireGuard 的 Linux Docker engine。脚本使用普通 Docker bridge 上两个或三个一次性容器的独立 network namespace；按模式只授予所需容器 `NET_ADMIN`，没有 host network、privileged 或宿主路由／DNS 修改。

```sh
mise run perf:dataplane

mise run perf:dataplane --batch 1 32 128 --profile 0 128 \
  --repeats 3 --seconds 15 --omit 3 --mtu 1400 --payload 1200 \
  --udp-rate 500M --cpus 2 --memory 512m
```

每个配置分别运行 TCP 单流和 UDP 定速率 iperf3，至少两次重复；重复之间反转配置顺序。默认每次先省略 2 秒 warmup，再测量 8 秒。release 交叉构建缓存独立于其他任务，位于 `target/dataplane-perf`；测量使用报告目录中不可变的二进制副本。Docker userspace 使用锁定 node-lab OS 源码对应的 immutable image，若尚无该 image，则在本工作区私有输出目录构建；不部署或修改常驻 lab。

`raw` 的 veth MTU 与内层 TUN MTU 相同，保留内核默认 offload；`plain` 的外层 veth 保留 1500 MTU，TUN offload 按 `--io` 选择。两者经过同一内部 Docker bridge。TCP 应用写入块与实际 TCP 分段不是同一概念，TCP 不通过 Mbps／假定包长反推 pps。UDP 应用 payload 由 `--payload` 固定，IPv4／UDP 头额外占 28 字节。

## 报告契约

结果位于当前工作区 `.meshway/perf/dataplane/<run>/`，构建缓存归当前工作区所有。测量使用 Git 公共目录所在主工作区的 `.meshway/perf/dataplane/measurement.lock`；其他工作树占用时轮询等待，避免同时运行两次 harness。

| 文件／字段 | 内容 |
| --- | --- |
| `results.json` | schemaVersion、Git revision／dirty、源码 digest、binary SHA-256、参数、环境与逐次样本 |
| `summary.md` | 成功样本中位数，按 stage、batch、profiling、TCP／UDP 分组；另列 L2→L3 与 secure−WireGuard 差值、失败数量 |
| `bin/meshway-dataplane-perf` | 本次固定 Linux release 二进制 |
| `<sample>/a.stdout`、`b.stdout` | plain／secure 端点的 readiness；raw／WireGuard 没有 Meshway 节点进程 |
| `<sample>/a.stderr`、`b.stderr` | 日志及 profiling 原始 JSON |
| `<sample>/iperf-client.json`、`iperf-server.json` | iperf3 原始输出；相邻 `.stderr` 保存诊断 |
| `environment` | 内核、CPU 信息、目标架构、Rust／Zig／iperf3、image 来源、每容器 CPU／内存配额、MTU／包长／速率 |
| `goodputMbps` | 接收端实收应用 payload 吞吐；不计封装头 |
| `packetsPerSecond`、`lostPercent` | UDP 接收端交付报文数／秒和丢包；TCP 为 null |
| `cpuSeconds`、`cpuPercent` | 所有样本容器 cgroup CPU 增量（直连两容器、中继三容器）；百分比为 CPU 秒／完整客户端墙钟时间 × 100，100% 等于一核；内核 CPU 归属限制见下节 |
| `results[].wireguard` | 各接口的内核类型／MTU、AllowedIPs、端点、路由、握手时间和双向传输字节；不保存密钥 |
| `cpuByRole` | 每个 A／R／B 角色的同区间 cgroup CPU 秒与百分比；历史报告缺失时表示未测，不从总量分摊 |
| `allocations`、`copiedBytes`、`peakMemoryBytes` | 当前未测，明确为 null |
| `capabilitiesInitial`、`capabilitiesFinal` | plain／secure 的实际 UDP 能力与成功 GSO／GRO／mmsg 次数；Manager 尚未导出 socket 计数，明确为 null；`tunBatch` 另记录 secure／manager 的 TUN vnet_hdr 实际模式 |
| `workers` | secure／manager 每节点实际配置的 crypto worker 数；0 为 inline，独立于 owner runtime |
| `syscallsPerPacket` | 当前为 null；`/proc` 进程 I/O 计数不能作为包含 UDP sendmsg／recvmmsg 的完整 syscall 计数 |

吞吐采用 iperf3 排除 warmup 的接收区间。CPU 覆盖完整客户端区间，包括 warmup 和采集开销，包含节点、iperf3 与容器内辅助命令；不是仅 Node 的 CPU。固定 CPU quota 不等于绑核或独占宿主，结果只代表记录的环境。失败样本与原始日志保留，指标不填零，脚本返回失败。

Profiling 单独记录启用和关闭的样本。`plain.tun.read`、`plain.udp.send`、`plain.udp.recv`、`plain.tun.write` 包装 future 的同步 poll 工作，不计 await 挂起时间；正式 UDP／Session 的更深层指标也保留在同一输出中。自耗时只扣除实际采到的嵌套 scope，应结合 samples、calls 与采样分母解释，不将采样累计纳秒直接当作整次 CPU 时间。最终吞吐取 profiling 关闭样本。

iperf3 结果取接收端 `server_output_json.end.sum_received`，并核对 UDP 实收字节数等于交付报文数乘以固定 payload。不能用 iperf3 3.16 的 UDP `end.sum` 替代该字段；该字段可能在真实收到数据时仍报告零字节。Profiling 报告为累计快照，汇总只取每个节点最后一份，不累加同一进程的多份历史快照。

## L2 内核批量实测

报告 `.meshway/perf/dataplane/20261007t002750z-1171958b/`（UTC），源码 `791e5e299a3af19c1ac98ec6b5f149586ef429ff`，启动时工作树 clean、测量期间源码未变。相同二进制、两次独立重复，中位数，profile 关闭；共 20 个 raw／plain／secure 样本全部成功。命令：

```sh
mise run perf:dataplane -- --batch 32 --profile 0 --repeats 2 \
  --seconds 6 --omit 1 --mtu 1280 --payload 1200 --udp-rate 2G \
  --cpus 2 --memory 512m
```

| 模式 | TCP Mbps | TCP CPU % | UDP Mbps | UDP pps | UDP 丢包 % | UDP CPU % | syscall／包 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| raw 内核参照 | 110,317.61 | 129.63 | 1,451.64 | 151,212.39 | 27.39 | 151.97 | 未测 |
| plain per-packet | 2,142.13 | 121.18 | 1,053.27 | 109,715.76 | 47.33 | 191.27 | 未测 |
| plain batch | 3,667.94 | 152.99 | 1,123.59 | 117,040.54 | 43.81 | 192.28 | 未测 |

Batch 的 TCP 吞吐增加 **71.2%**，CPU 增加 31.81 个百分点；UDP pps 增加 **6.7%**，CPU 增加 1.01 个百分点。CPU 为两容器合计，100% 表示一核，含 warmup。UDP 输入为 2 Gbps，raw 自身也有 27.39% 丢包，不能把全部丢包归因于转发器。TCP pps 未测，`/proc` I/O 计数不覆盖完整 UDP syscall 口径，因此不推算 syscall／包。

### 实际能力与使用证据

| 能力 | OrbStack 结果 | 证据 |
| --- | --- | --- |
| TUN `IFF_VNET_HDR`／`CSUM` | 已启用 | 每个 batch 节点首尾 capability 均为 true；双栈内核包校验和验证通过 |
| TUN `TSO4`／`TSO6` | 已启用 | 双栈真实 TCP 拆分与三包合并交付测试通过 |
| TUN `USO4`／`USO6` | 已启用 | 双栈 UDP 三包合并后恢复三个应用报文 |
| UDP `UDP_SEGMENT`／`UDP_GRO` | 已启用且实际使用 | TCP 样本累计 263,553 次 GSO 发送、263,101 个 GRO 超包；UDP 样本分别为 76,798／59,771 |
| UDP `sendmmsg`／`recvmmsg` | 实际成功 | TCP 样本累计 26,527／148,589 次；UDP 样本累计 136,663／71,755 次 |
| mmsg／逐包回退 | 隔离测试通过 | 显式禁用上层能力后验证双栈消息边界、来源、短尾段与截断 |

计数为每种负载的两次重复、两个节点退出时累计值之和，包含准备／warmup／回复；它们证明实际使用，不能当作测量区间内的精确 syscall／应用包比值。L2 应用负载使用 IPv4，IPv6 offload 由独立内核测试证明；能力启用不表示每类负载都使用全部 offload。

环境为 OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`、aarch64、锁定 Ubuntu 24.04／iperf3 3.16；Rust 1.98.0、Zig 0.17.0、cargo-zigbuild 0.23.4，Linux musl release。每容器 2 CPU quota／512 MiB，每节点 current-thread runtime，MTU 1280，UDP payload 1200 字节，省略 1 秒 warmup 后测量 6 秒。Node binary SHA-256 为 `0d73b792ae3dd615ae9ee735a6d5458198d931926ed64f520b4556b231b09972`；完整源码 digest、fixture hash、image 来源、首尾能力和逐次原始样本见 `results.json`。按唯一运行 label 核对无残留容器或 network。

## 内核 WireGuard 参照

`perf:dataplane` 在同一次运行内测量 raw／plain／wg-direct／wg-relay／secure-direct／secure-relay／manager-direct／manager-relay，复用锁定 Ubuntu 工具镜像、内部 bridge、每容器 CPU／内存配额、`--mtu`、TCP 单流、UDP payload／输入速率和 iperf3 测量区间。WireGuard 只使用内核实现与锁定的 `wireguard-tools`，不存在 userspace 替代路径；内核不支持时样本失败。

| 模式 | 拓扑与处理 |
| --- | --- |
| `wg-direct` | A、B 各一个 `wg0`，地址 `100.127.0.1/32`／`100.127.0.2/32`，peer AllowedIPs 仅为对端 `/32`；iperf3 路由走 `wg0` |
| `wg-relay` | A 的 `wg0` ↔ R 的 `wg0`、R 的 `wg1` ↔ B 的 `wg0`；R 只在自己的 namespace 开启 IPv4 forwarding，将两个端点 `/32` 分别路由到对应隧道。A、B 没有彼此的直接 peer |

每个样本创建新的随机密钥对，在该样本内固定；私钥生成、文件存储均在容器私有 tmpfs 的 0700 目录，文件权限 0600，仅公钥经内存传给对端。报告不保存密钥，容器删除时文件随 tmpfs 消失。所有 WG 容器仅添加 `NET_ADMIN`，无需 `/dev/net/tun`；每个接口均检查 `wireguard` 内核类型、MTU 和实际路由，样本结束核对每段握手及双向传输计数。各重复反转配置顺序；raw／WG 的 batch/profile 均显示 0（不适用），不为每个 Meshway batch/profile 重复内核参照。

### 差值的适用范围

- `summary.md` 以同次运行的 secure-direct−wg-direct、secure-relay−wg-relay 中位数相减，分别给出 TCP／UDP Mbps、UDP pps、丢包百分点与 cgroup CPU 百分点；缺失或失败的参照不补零。开启 profiling 的 secure 差值仍单独标注，正式结论取关闭插桩样本。
- WireGuard 是内核实现，可用多核 softirq／kworker 和内核 offload；Meshway 使用 userspace owner／crypto worker，经 TUN 与正式批量 socket I/O。下方历史数据保留当时单线程、逐帧 I/O 的实现边界。相同容器配额不是相同内核执行预算或独占 CPU。
- CPU 仍为样本容器 `cpu.stat usage_usec` 之和。WireGuard 加解密与网络处理可在 softirq／kworker 中运行，部分 CPU 可能记在其他 cgroup／宿主上下文，并不完整受容器 quota 约束；这里没有 VM 全局归因测量。**内核 CPU 是未完整测得，不是 0；CPU 差值仅是容器记账差值，不能直接称为密码算法成本差。**
- WireGuard 本身无 Meshway 式中继语义。`wg-relay` 的 R 终止两条隧道，解密 IP 包、路由并重新加密；Meshway R 对端到端 ciphertext 验 MAC／准入并转发，不解密业务内容。中继吞吐之差同时含信任模型与处理步骤差异。
- WireGuard 稳态 Data 的固定额外开销为 32B（16B 数据头＋16B AEAD tag），另有 padding、外层 UDP/IP。Meshway 当前 v5 Data 每段固定新增 **86B**，见[会话数据帧](/v2/node/dataplane#会话密钥与紧凑数据帧)。下方历史 v4 实测采用当时的可变长前缀，其 139B 示例不能用于当前 v5，也不能把吞吐差值全部归于头部大小。

### WireGuard 同环境实测

2026-10-06，运行 `20261006t234736z-dfa88ba3`，原始证据位于执行工作树的 `.meshway/perf/dataplane/<run>/`。使用以下参数，六种模式各测 TCP／UDP、每格两次独立重复，24/24 样本成功；表格为同次运行的中位数。

```sh
mise run perf:dataplane -- --batch 32 --profile 0 --repeats 2 \
  --seconds 6 --omit 1 --mtu 1280 --payload 1200 \
  --udp-rate 2G --cpus 2 --memory 512m
```

| 环境／来源 | 值 |
| --- | --- |
| Kernel／架构 | OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`，aarch64 |
| Userspace | 锁定 Ubuntu 24.04，iperf3 3.16，wireguard-tools `1.0.20210914-1ubuntu4` |
| Image ID | `sha256:9cd28af9b7c80732023e87eefef6be2b40293db4aa644d88e4a6b7929e4cf8f9` |
| 编译 | Rust 1.98.0、Zig 0.17.0、cargo-zigbuild 0.23.4，release，`aarch64-unknown-linux-musl` |
| 资源／网络 | 每容器 2 CPU quota、512 MiB；同一内部 bridge、独立 namespace；MTU 1280 |
| 负载／区间 | TCP 单流；UDP 1200B payload、2 Gbps 输入；warmup 1 秒＋测量 6 秒；Meshway batch 32、profile 关闭 |
| 源码 | `5b6968782da71bd44dd523c59c8ec9c632a30997`；启动时 dirty 仅为本页和 docker-lab 文档修改，测量源码 digest 前后一致 |
| 源码 SHA-256 | `ab03824736fddce5b07e0fe57764c7359f4ce72b2ea12bc3ea5632084578c8cc` |
| Node binary SHA-256 | `3dcd7a9cf965677f777104ec36811f33378f48faa53a1ad15cc7193af6fd0ae1` |
| Fixture binary SHA-256 | `a0bb6dfdfc2471e38a460afb457b42e78b06221e7e482f6d4c3df8a94dcfeb28` |

| 模式 | TCP Mbps | TCP cgroup CPU % | UDP Mbps | UDP pps | UDP 丢包 % | UDP cgroup CPU % |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| raw | 91842.76 | 130.00 | 1488.93 | 155096.93 | 25.52 | 152.70 |
| plain | 2229.39 | 121.52 | 1107.73 | 115388.29 | 44.61 | 188.27 |
| wg-direct | 2741.54 | 29.07 | 1826.14 | 190222.43 | 8.68 | 150.34 |
| wg-relay | 4853.50 | 23.61 | 1708.83 | 178003.06 | 14.54 | 130.42 |
| secure-direct | 321.90 | 137.27 | 467.61 | 48709.13 | 76.61 | 360.51 |
| secure-relay | 198.76 | 148.98 | 421.37 | 43893.16 | 78.93 | 442.21 |

CPU 按全部样本容器 cgroup 相加，100%＝一核，区间包含 warmup；WireGuard 的低 TCP cgroup CPU 不代表内核加解密成本只有这些。TCP pps 未测。UDP 2G 输入超过此次各实现的实际接收吞吐，使差值不再被 500M 输入遮蔽；raw 自身仍有 25.52% 丢包，不能把 secure 的全部丢包归因于认证／加密，也不能把本表直接当作无损容量或硬件 pps 上限。

**Meshway secure−WireGuard：**由未四舍五入的中位数相减，CPU／丢包单位为百分点。

| 对照 | TCP Δ Mbps | TCP Δ cgroup CPU pp | UDP Δ Mbps | UDP Δ pps | UDP Δ丢包 pp | UDP Δ cgroup CPU pp |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| secure-direct − wg-direct | −2419.64 | +108.20 | −1358.53 | −141513.31 | +67.93 | +210.17 |
| secure-relay − wg-relay | −4654.74 | +125.36 | −1287.46 | −134109.90 | +64.39 | +311.78 |

此次 WG relay TCP 两次为 4485.23／5221.77 Mbps，高于 direct 的 2771.47／2711.62 Mbps。每条隧道均有握手和双向字节证据，R 的两段路由与转发已核对；仍未隔离内核 offload、软中断／worker 调度与 CPU 分布对该现象的影响，不能据此宣称中继比直连更高效。报告保留真实值，不选择性重测或用文献数字替换。执行前按 30 秒间隔等待已有测量锁，并在完整运行期间保持互斥；按本次唯一 label 核对，无残留容器或 network。

## L2 实测参考

2026-10-06，报告 `.meshway/perf/dataplane/20261006t222145z-cb007812/summary.md`，源码 revision `a00e12cf`。20 个样本全部成功，每个配置／负载重复两次；每次省略 2 秒 warmup 后测量 8 秒。以下是该次运行的中位数，不跨运行挑选样本。

| 环境 | 值 |
| --- | --- |
| Kernel／架构 | OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`，aarch64 |
| Userspace | 锁定 Ubuntu 24.04，iperf3 3.16 |
| 编译 | Rust 1.98.0、Zig 0.17.0、cargo-zigbuild 0.23.4，release，`aarch64-unknown-linux-musl` |
| 资源 | 每个容器 2 CPU quota、512 MiB 内存；每个 Node 一个 current-thread runtime |
| 网络／负载 | 同一内部 Docker bridge，两独立 namespace；MTU 1400；TCP 单流，UDP 1200 字节 payload、500 Mbps offered rate |
| 源码 SHA-256 | `ccae7ef4d232d4ee…`（完整值见 `results.json`） |
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

## L3 实现与实测

`--stage secure` 工具入口已实现，组合正式 `SecureNode`、`meshway-transport-udp` provider 与公共 `Session`。`secure-direct` 为 A→B，`secure-relay` 为 A→R→B；只有 A、B 打开真实 TUN。R 的隔离容器没有 TUN device 和 `NET_ADMIN`，驱动只调用正式会话认证与重放检查并转发 ciphertext；业务流仅在端点准入。

### 处理边界

每个节点在自身 namespace 的 0700 私有目录中生成 0600 种子文件，私有材料不进入共享 fixture、stdout 或报告。公开 fixture 包含固定 network／revision／publication、编译与过期时刻、节点地址／监听端口／公钥库存、完整路径及 demand。各进程使用同一 compiler 独立编译，核对完整 `CompiledPublication` 的 SHA-256，再仅签发本节点 identity／snapshot。静态签发只存在于 `tools/meshway-static-authority`，使用真实时钟与原 `ensure_current`；issuer drop 撤销本进程旧快照。

iperf3 的 TCP 控制流和 UDP 负载分别使用精确 `5201` 端口 demand。A 在应用启动前分别发起 Noise circuit；逐验证者 MAC、flow 准入、重放处理和原完整路径回复归唯一 `SecureNode`。固定 Native UDP carrier 经 `set_carriers` 校验。输出复用正式 `carrier::send`／`carrier::deliver`：UDP 的原 output guard 保留到 Session 发送 poll 和完成，TUN 交付每次恢复 poll 都检查原 guard。

每个节点保留一个 current-thread owner runtime，crypto 使用正式 `pipeline::Pool`。工具的 `--workers 0` 使用 inline；非零值配置有界线程数。harness 默认 1 worker，对应默认两核配额中的 owner＋worker；单节点二进制默认可用核数减一、最少一。TUN／UDP 使用批量接口和实际 offload 能力；就绪输入经过 owner 准备、worker 计算、保序 owner 提交，再按 channel 批次发送／交付。SecureNode 使用默认 limits；L3 Session 帧数／操作数预算与 plain 相同，字节容量额外容纳 secure envelope 和握手。

### L4 Manager 边界

`--stage manager` 使用相同静态签发、SecureNode 和 worker pool，经 `Host::open`、`Host::install` 安装 capture 与执行 epoch，再取得正式受保护 SocketFactory 并调用无穿透的 `Manager::start`。工厂的 data scope 必须匹配已安装的执行，不能借 bootstrap scope 替代。不挂接 Server control relay，也不创建第二个 Control 签发者。公开 fixture 经真实 compiler 产生静态 Native UDP endpoint／完整路径，Manager 执行真实邻接认证、scope、选择和健康维护。业务输出使用 `Manager::send_batch`，控制输出使用独立逐帧入口，接收使用 Manager 的实际事件队列。

`manager-direct` 为 A→B，`manager-relay` 为 A→R→B。各节点使用 Host 自己创建的 capture TUN，端点直接复用该设备；R 保有 capture，但不读取本机业务或向 TUN 交付明文。readiness 的 `captureTun` 记录实际设备，R 的业务 `tun` 字段为 null。L4 各 namespace 需要 `NET_ADMIN` 和 TUN device；L3 的 R 仍无 TUN。L4 增量包含正式 underlay 安装／保护和 Manager 生命周期，仍不包含生产 Control／Server 交互、daemon 自动需求、DNS 和安装协调状态机，这些属于 L5。

### L3 流水线与 L4 实测

报告 `20261007t013659z-d3a3ee2c`，源码 `2d0b7584`，测量期间源码未变；启动时 dirty 仅为文档。相同二进制、每容器 2 CPU／512 MiB、每节点 1 owner＋1 crypto worker、TUN vnet_hdr 开启。环境为 OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`／aarch64、锁定 Ubuntu 24.04／iperf3 3.16、Rust 1.98.0／Zig 0.17.0、Linux musl release。命令：

```sh
mise run perf:dataplane -- --batch 32 --profile 0 128 --repeats 2 \
  --seconds 6 --omit 1 --mtu 1280 --payload 1200 --udp-rate 2G \
  --cpus 2 --memory 512m
```

下表为 profile 关闭的成功样本中位数，n 为实际成功次数。CPU 为全部角色 cgroup 之和，100%＝一核，含 warmup；TCP pps 未测。L3 前值来自指定 v5 基线 `20261007t001557z-44250076`，参数相同；L4 此前无实现实测，前值不填零。

| 层／路径 | n TCP／UDP | TCP Mbps 前 → 后 | TCP CPU % 前 → 后 | UDP pps 前 → 后 | UDP Mbps 前 → 后 | UDP CPU % 前 → 后 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| L3 direct | 2／2 | 312.65 → 480.50 | 125.37 → 143.82 | 50,203 → 52,427 | 481.95 → 503.30 | 353.35 → 281.54 |
| L3 relay | 2／2 | 237.56 → 467.80 | 138.66 → 204.00 | 52,031 → 54,220 | 499.50 → 520.52 | 449.66 → 367.40 |
| L4 direct | 2／2 | 未测 → 439.23 | 未测 → 135.50 | 未测 → 49,183 | 未测 → 472.16 | 未测 → 279.01 |
| L4 relay | 2／1 | 未测 → 353.00 | 未测 → 172.58 | 未测 → 41,257* | 未测 → 396.07* | 未测 → 350.50* |

*L4 relay UDP 仅有一次 profile-off 成功样本，不构成两次重复的稳定吞吐结论。* L3 direct／relay TCP 分别较基线增加 53.7%／96.9%；UDP pps 增加 4.4%／4.2%，UDP CPU 降低 71.81／82.26 个百分点。此次 UDP 丢包分别为 L3 direct 74.38%、relay 73.99%、L4 direct 75.62%、L4 relay 单次 80.21%；2 Gbps 输入已超过接收能力，不能称为无损容量。

| 路径 | guard／源包前 → 后 | `carrier.send.poll` 自 CPU µs／帧前 → 后 | `carrier.deliver.poll` 自 CPU µs／帧前 → 后 |
| --- | ---: | ---: | ---: |
| L3 direct | 43.42 → 11.64 | 5.095 → 0.899 | 5.781 → 2.746 |
| L3 relay | 70.29 → 17.01 | 4.957 → 0.857 | 6.095 → 2.541 |
| L4 direct | 未测 → 25.82 | 未测 → 1.230 | 未测 → 2.259 |
| L4 relay | 未测 → 45.21 | 未测 → 1.438 | 未测 → 2.583 |

这些值只取成功的 profile-on 样本。guard 使用全角色精确调用数／源包调用数；carrier 使用采样平均 poll 自 CPU × 精确 poll 次数，再除以发送 MAC 帧数或成功 TUN 交付数。同名静态 metric 一起汇总，包含准备与 warmup；这是有 observer effect 的估计，不是每次 syscall 的精确 CPU。L3 发送适配器达到约 1 µs／帧，TUN 交付仍约 2.5–2.7 µs／帧；后者的 1 µs 目标尚未达到。L4 还包含 Manager／underlay 的真实检查，不能把其 guard 总量归于 SecureNode 或当作 worker 读取 authority。

本轮 **58/60** 样本成功，整体报告 `passed=false`。两个失败均为 manager-relay UDP：`b32-p128-udp-r0` 和 `b32-p0-udp-r1`，节点已 ready，iperf 启动后 A（后一例还包括 R）返回 `Unavailable`；吞吐／CPU 字段保持 null。当前证据不足以区分具体健康失效或选链变化原因，工具此时终止样本，尚未完成高负载失链恢复验收。原 stderr、iperf 输出和失败状态全部保留。

此前完整运行 `20261007t011939z-3ae6552b` 为 **44/60** 成功：L4 的 16 次均因 Host 未安装执行 scope 而在启动时返回 `Underlay`；修正为正式 Host 安装后重新运行完整矩阵，没有替换旧报告。该轮 L3 direct／relay TCP 为 531.13／489.86 Mbps；本页统一采用后一次完整矩阵，避免跨轮挑选最好结果。

最后一轮 binary SHA-256 为 `aa2177174ab317cb0c74e14ea812b8ff885f9d9b48823fb5ad105acd3d26786f`，source digest 为 `49f49790e7ffb126fbdcc0403293da77c80279dab391c40708127607b90c9529`。报告与固定二进制位于工作树 `.meshway/perf/dataplane/<run>/`。两轮均使用共享 measurement lock 串行执行；生产 runtime conformance 状态独立见[验收矩阵](/v2/node/validation)。

Profiling 关闭用于正式吞吐；开启时采用 `MESHWAY_PROFILE=128`、`MESHWAY_PROFILE_DETAIL=1`，导出 `secure.send/receive`、`crypto.*`、`control.guard`、`codec.*` 和 TUN／UDP 边界。secure 驱动在停止并 join I/O 后输出最终累计快照。guard／包定义为所有节点精确 `control.guard` 的 `calls_started` 之和除以端点 `l3.packet.origin` 的同字段之和，包含准备、warmup、空闲与最终输出检查；不是每个成功交付包的固定算法次数。

### L3 实测参考

同一次运行内 raw／plain／secure-direct／secure-relay 使用相同容器配额、MTU、负载与区间；每格为两次独立重复的中位数，profile 关闭。CPU 为所有样本容器 cgroup CPU 之和除以客户端区间（100%＝一核，含 warmup）；UDP 输入固定 500 Mbps，因此 UDP 的差别主要体现在 CPU 与丢包，而不是吞吐。

| 模式 | 节点 | TCP Mbps | TCP CPU % | UDP Mbps | UDP pps | UDP 丢包 % | UDP CPU % |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| raw 内核参照 | 2 | 111,723 | 125 | 451 | 46,948 | 9.9 | 124 |
| plain（L2） | 2 | 2,181 | 121 | 431 | 44,942 | 13.7 | 146 |
| secure-direct（L3） | 2 | 314 | 128 | 442 | 46,056 | 11.5 | 308 |
| secure-relay（L3） | 3 | 213 | 142 | 421 | 43,805 | 15.9 | 420 |

运行 `20261006t232708z-3d779bfe`，源码 `70c6e220`（含 `90bc1aec` 同步段 Reading）。同参数的前一次运行 `20261006t232019z-cedb8714`（源码 `b99cebed`，不含 `90bc1aec`）为 secure-direct TCP 245 Mbps、UDP CPU 327%；secure-relay TCP 185 Mbps、UDP CPU 435%：共享 guard 读数在真实 I/O 下带来 TCP +28%／+15%，UDP CPU −19／−15 个百分点。

L2→L3 差值（后一次运行）：secure-direct TCP −85.6%，UDP CPU +162 pp；secure-relay TCP −90.2%，UDP CPU +274 pp。TCP 单流在 secure 下没有用满两核配额，吞吐同时受逐包串行等待影响，不能全部归因于密码运算。

profile 开启（1/128）时 secure-direct 的采样自 CPU 最大项依次为：`carrier.deliver.poll` 5.1 µs／次、`crypto.verify_mac_chain` 4.6、`carrier.send.poll` 4.4、`crypto.append_mac` 4.3、`crypto.decrypt_parse` 3.9、`crypto.encrypt` 3.6、`secure.send` 3.3、`secure.receive` 3.1、`flow.admit` 1.0；`secure.maintain` 整次运行只有 8 次。精确 `control.guard` 调用为 secure-direct 约 44 次／源包、secure-relay 约 72 次／源包，其中 SecureNode 操作各 5 次，其余来自 carrier 适配器与 Session 在每次发送／交付 poll 的输出 guard。采样含 observer effect，不能加总当作 profile 关闭的 CPU 预算。

secure 节点退出时报告 TUN 上无 demand 承载的内核维护包（IGMP／ICMPv6）与逐包准入拒绝的计数；两者按 daemon 语义计数丢弃，不中止节点。本次运行 28/28 样本成功。

### 测量环境

| 环境 | 值 |
| --- | --- |
| Kernel／架构 | OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`，aarch64 |
| Userspace | 锁定 Ubuntu 24.04，iperf3 3.16 |
| 编译 | Rust 1.98.0、Zig 0.17.0、cargo-zigbuild 0.23.4，release，`aarch64-unknown-linux-musl` |
| 资源 | 每容器 2 CPU quota、512 MiB；每 Node 一个 current-thread runtime |
| 网络／负载 | 内部 Docker bridge、独立 namespace；MTU 1280，batch 32；TCP 单流；UDP 1200 字节 payload、500 Mbps 输入 |
| 区间 | warmup 1 秒＋测量 6 秒；每配置两次重复 |
| 源码 revision | `70c6e22053b03b739a6ed7ce67dcc436ceb02900`，启动时工作树 clean |
| 源码 SHA-256 | `ccae7ef4d232d4ee…`（完整值见 `results.json`） |
| Node binary SHA-256 | `3dcd7a9cf965677f…`（完整值见 `results.json`） |
| Fixture binary SHA-256 | `a0bb6dfdfc2471e3…`（完整值见 `results.json`） |

按本次唯一 label 查询，无残留测试容器或 network。以上历史 L3 结果是单流、2 CPU 配额的隔离 VM 数据，不是硬件保证；不包含随后增加的 worker／批量接线或 L4 Manager。
