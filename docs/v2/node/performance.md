# 分层性能测量

性能脚本显式运行，不进入 `mise run check` 或普通行为测试。每层只回答自身处理边界的成本；本机接纳、对端接收、真实业务交付分别计量。当前结果是记录环境中的测量，不是硬件保证。

## 层级定义

| 层级 | 处理步骤 | 入口 |
| --- | --- | --- |
| L0 | 进程内真实编译器许可、SecureNode 握手、Noise、相邻 MAC、端点流准入与输出 guard；无 socket／TUN／控制面 I/O | `perf:circuit` |
| L1 | 正式 provider 原生 socket I/O 对照公共 datagram Session；代理协议保留自身 TLS／WS | `perf:transport` |
| L2 | Linux namespace、真实 TUN、Tokio、正式 UDP provider 与 Session，转发明文 IP 包 | `perf:dataplane --modes plain-per-packet plain-batch` |
| L3 | L2 I/O 加正式 SecureNode、认证／加密、路径证明与重放 | `secure-direct`／`secure-relay` |
| L4 | L3 加 connectivity Manager 的真实邻接获取、承载调度与生命周期 | `manager-direct`／`manager-relay` |
| L5 | 完整 meshwayd、Control／Policy、执行资格、Manager、SecureNode、TUN 与业务应用 | [三节点 lab](/v2/node/docker-lab)；[验收范围](/v2/node/validation) |

`raw` 是同一隔离网络直接运行 iperf3 的内核参照，不经过 Meshway，也不是 L0。`plain` 没有认证、加密、策略或控制面，不代表业务数据面的吞吐。`wg-direct`／`wg-relay` 是同环境内核 WireGuard 参照，不是额外功能层。

## 运行方式与报告契约

### Netmap 增量投影

```sh
mise run perf:netmap -- --nodes 1000 --group-size 10 --samples 3    # 100 个十节点分组
mise run perf:netmap -- --nodes 1000 --group-size 1000 --samples 3  # 全互访
```

`tools/meshway-netmap-perf` 直接调用正式 Policy 编译入口，对确定性的 1000 节点测量：`--group-size` 个节点一组，组内 topology／Access 互通（等于节点数即全互访）。分别比较仅 revision 变化、一个节点改名、一个节点自身能力变化（Node 自触发事件，如滚动升级的 acquire）、全局 DNS domain 变化；每轮先计时增量与完整编译，再在计时外合并保留输出并核对逐字段一致性。输入指纹包含全局字段、主体绑定、地址分类与每个可见／出口依赖；先判定受影响节点，再构造完整 netmap。全局变更可以保守重编全部节点。

只有 allow 规则时，topology 连通分量按规则线性求出（每条规则的两端选择器各至少命中一个节点且不止一个节点时合并为一个分量）；存在 block 规则时逐对判定。规范摘要直接把序列化流送入 SHA-256（运行时选择 CPU 指令），不分配整份序列化副本。

2026-10-10 在 Apple M 系列开发机上（F2 分支，release，3 次中位数，毫秒；括号内为改动前 `03492b6e`）：

| 场景 | 十节点分组：评估节点／增量 | 全互访：评估节点／增量 |
| --- | --- | --- |
| 仅 revision | 0／32（2,610） | 0／649（842） |
| 一个节点改名 | 10／48（2,607） | 1000／8,064（11,969） |
| 一个节点能力变化 | 10／48（2,630） | 1000／8,174（11,884） |
| 全局 DNS domain | 1000／2,326（5,215） | 1000／8,385（11,852） |

Node 自触发事件只重编其 netmap 引用了该节点的节点（输入指纹即反向依赖：可见集合含该节点者）。全互访时这就是全部节点：每份 netmap 都含全部 peer 的身份与能力，单份约 1.5 MB，一次重编要序列化并摘要约 1.5 GB，耗时由此主导，并在 Server 的唯一写事务中发生。可支持规模的边界因此是“可见集合 × 节点数”而不是节点数；全互访千节点的单次重编约 8 秒，超出这一规模需要共享 peer 段（尚未实现）。

stdout JSON 保存源码 revision、节点／分组／重复参数、每轮毫秒、中位数、实际评估节点数和一致性结果。计时包含指纹、topology 索引及投影构造，不含 SQL、签名、网络 I/O 或真实包流；这是编译器测量，不能替代 Server 事务耗时或 L5 吞吐。Server 将相同指纹与 netmap 更新放在同一事务，缺少指纹时完整编译。

### 二进制体积

自 `d073668a` 起，workspace release profile 为 `lto = "fat"`、`codegen-units = 1`、`panic = "abort"`、`strip = "debuginfo"`；优化级别沿用 Cargo release 的 3。lab 与性能工具的普通 release 构建共同使用该配置。保留符号表用于函数归因，移除 DWARF 调试信息；保留 DWARF 的原生采样需要同时覆盖 debug／strip，并在报告中保留覆盖值（见 [L0 CPU 归因](#l0-cpu-归因)）。panic 会终止进程，不执行 unwind 析构；普通错误、取消和正常退出仍走既有 owner 的清理／join 路径。跨此提交的历史性能数字必须标明 profile 变化，并用相同工具重新对照。

体积测量与 L0–L5 运行时性能分开报告。保存原始产物的文件字节数、SHA-256、target triple、源码 revision／digest、完整 release profile、features、Rust／Zig 版本、构建命令和墙钟耗时；冷构建、缓存构建及分析工具触发的重构建分别标注。MiB 为 `bytes / 1024²`，文件体积不等于常驻内存或 `.text` 大小。

证据位于 `.meshway/perf/binary-size/<run>/`，每种配置保存 host 与 `aarch64-unknown-linux-musl` 的 `meshwayd`／`meshway` 快照、构建 stdout／stderr、crate 前 25 项和函数前 30 项，以及完整 JSON 符号表。基本入口：

```sh
mise exec -- cargo install cargo-bloat --locked
mise exec -- cargo build --release --locked -p meshway-daemon -p meshway-cli --bins
mise exec -- cargo bloat --release --locked -p meshway-daemon --bin meshwayd --crates -n 25
mise exec -- cargo bloat --release --locked -p meshway-daemon --bin meshwayd -n 30 --wide
mise run build:node:linux
```

`cargo-bloat 0.12.1` 分析时强制 `strip=false`，其 File 列不能替代正式 release 文件体积；正式产物在分析前单独保存。Linux 分析沿用锁定 Zig linker：分析工具调用的 `cargo build` 需转交同参数的 `cargo zigbuild`，不能拿 host 符号表代表 Linux。crate／函数占比按可归属的 `.text` 符号计，不把内联、单态化、合并符号或未归属的 native 代码当作完整依赖成本。生成客户端类型还可能出现在 serde 单态化函数中，按符号名称统计只能给出可见部分，不能与 crate 行直接相加。

### L0 受保护 circuit

```sh
mise run perf:circuit
mise run perf:crypto
mise run perf:crypto:l0 -- --environment host
```

`scripts/perf/circuit.py` 构建 `tools/meshway-circuit-perf` release（缓存 `target/circuit-perf`），经真实 Policy 与工具专属静态签发建立 2–8 节点链。默认 2／3／5 节点 × 1280／52 B × profile 0／128，各三次独立进程重复；先完成 Noise／MAC 握手、TCP 建流与 warmup，再测双向稳态。52 B 是 IPv4/TCP ACK，1280 B 含 1228 B TCP payload。输入复制到池缓冲模拟 TUN，后续角色移交同一分配；每包包含解析、flow、crypto、codec、资格检查与 guarded 交付。

`.meshway/perf/circuit/<run>/results.json` 保存角色 calls／线程 CPU／wall、逐段帧字节、中位数／p10／p90；`summary.md` 列采样阶段。CPU／包合计全部路径角色，Mbps 按 IP 字节计算，区别于 iperf 应用 goodput。默认同步 inline；显式 `--workers N` 使用正式 streaming Pool、全链同时仅一包，测交接而非饱和并行容量，此时总 CPU 改用进程时钟，角色仍为 owner 线程时钟，报告记录 `cpuClock`。

### L0 CPU 归因

`CARGO_PROFILE_RELEASE_STRIP=none mise run perf:crypto:l0 -- --environment host`（或 `linux`）对正式 `meshway-circuit-perf` 做独立 release 构建。脚本设置 `CARGO_PROFILE_RELEASE_DEBUG=1`，调用时的 `STRIP=none` 覆盖普通 release 的 strip 设置，保留函数调试信息；只设置 debug 不会抵消显式 strip。固定 2／3 节点、1280B、1000 轮 warmup，分别保存 profile 关闭和 `MESHWAY_PROFILE=128` 的三次重复。macOS 另用用户安装的 `samply` 记录每拓扑 500,000 轮的 1 kHz 调用栈；Linux 复用锁定 node-lab image、zigbuild、无网络／只读容器和 2 CPU quota，镜像缺少 perf 时使用现有日志插桩。二进制、命令、原始 stdout／stderr、采样文件与 `results.json`／`summary.md` 位于 `.meshway/perf/crypto/<run>-l0-<environment>/`。

`mise run perf:crypto:attribute -- --crypto <run>… --host-l0 <run> --linux-l0 <run> --output <file>`（`scripts/perf/crypto_analysis.py`）从 samply sidecar 排除主线程最初 200 ms／最后 100 ms，以 thread CPU delta 加权；原生 self-time 比例乘同轮未插桩总 CPU，按 AEAD、含 key schedule 的 HMAC、codec、其它 SHA、flow／replay／guard、缓冲／复制与其它分类，保存所有叶函数和 top 15。inclusive 父子项不可相加；未解析叶保留父级归因。日志 scope 没有覆盖每次 map／validate／memcpy，不能伪造函数细分；采样估计含 observer、内联与窗口误差，不能用 macOS 占比代替 Linux。

### 原语微基准

`tools/meshway-crypto-bench` 的显式 example 使用工具自己的 dev-dependencies，不切换生产后端。`scripts/perf/crypto.py` 在 host 与锁定 Linux 无网络只读容器运行固定迭代；默认预热 1000 次，支持 `--iterations`／`--repeats`。crypto 两工具接受 `--cpu-set 0` 固定 guest CPU，但不固定物理核；quota 和首尾 load 分别记录。结果位于 `.meshway/perf/crypto/<run>/`。

| 轴 | 契约 |
| --- | --- |
| AEAD | ChaCha20-Poly1305 0.10.1／0.11.0、ring 0.17；每次构造 key，seal／open 分测；in-place 复用分配，alloc 含 clone／分配／释放，open.inplace 含恢复密文复制 |
| MAC | HMAC／SHA256 0.12／0.10、0.13／0.11 与 ring；new 和预 key clone/context 分测 |
| 候选 | BLAKE3 keyed 截 16 B；空明文 AEAD、frame AAD、`zero32 || sequence LE64` 生成 tag；不代表 wire 已选择 |
| 长度 | 原始 52／1280 B；`.v5` 按真实 transcript：MAC 为 69 B domain／字段＋IP＋16 B ciphertext tag，保留 HMAC 六次 update；AEAD-tag 为 54 B Data header＋ciphertext，每次更新 sequence |
| 计量 | 固定批次线程 CPU／wall；计时外跨实现逐字节核对；中位数／p10／p90 是批次分位数，不是逐操作尾延迟；copy baseline 单列、不自动扣除 |

原语窗口不含 Frame 校验、policy、replay 或 tag 比较；MAC transcript 在窗口外准备。seal/tag 在每个测量流递增 nonce，open 重验固定公开密文。路线预算按每单向包一次 seal、一次 open、`2 × (节点数 − 1)` 次 MAC 做同环境加减，只是替换原语预算，不能推算改后真实 I/O 吞吐。锁定后端的 feature、zeroize 与编译 flags 保存在报告；不用跨版本微基准代替生产验证。

### L1 承载对照

```sh
mise run perf:transport --protocol all --payload 64 1200 --batch 1 32 \
  --messages 200000 --warmup 1000 --repeats 3
```

`scripts/perf/transport.py` 构建正式 provider／Session 消费者 `tools/meshway-transport-perf`（`target/transport-perf`）。Native TCP／UDP、VLESS TCP/TLS／WS/TLS、Trojan TCP/TLS 分别对照原生协议 I/O 与公共 datagram 封装；raw 仍执行代理协议握手／TLS。两模式共享 socket 参数、连接、缓冲与批量上限；无 vectored I/O 时 raw 也用有界字节缓冲。固定 payload 批量 echo，交替 raw/datagram 顺序；吞吐按发起端实收业务 payload，排除建连／warmup，RTT 从批次提交至每条 echo，CPU 含初始化／warmup。

公共仓库 `.meshway/perf/transport/<run>/` 保存固定二进制、stdout／stderr、JSON／Markdown；表格为中位数，比值为有效配对重复的比值中位数。Mesh 认证、端到端加密、routing、动态 ABI 与 TUN 均标未启用；分配／复制／峰值内存为 null。

### L2–L4 运行

需要锁定 Rust／Zig 与支持 TUN、内核 WireGuard 的 Linux Docker engine。独立内部 bridge 上使用两个或三个一次性 namespace，按需授予 `NET_ADMIN`／TUN device；无 host network、privileged 或宿主路由／DNS 修改。复用锁定 node-lab OS 的 immutable image，缺少时在私有输出目录构建；不部署常驻 lab。

```sh
mise run perf:dataplane
# 单模式诊断预设：batch 32、profile 0、一次重复、4 秒
mise run perf:dataplane -- --quick --modes manager-relay
# 正式配对验收；不加 --quick
mise run perf:dataplane -- --modes secure-relay manager-relay \
  --repeats 2 --seconds 6 --omit 1 --profile 0 --cpus 2 \
  --mtu 1280 --payload 1200 --udp-rate 500M
```

`--modes` 普通默认仍为全部九种模式，batch 默认 `[1,32]`、profile `[0,128]`、repeats 2、seconds 8、omit 2。`--quick` 默认单个 `manager-relay`、batch `[32]`、profile `[0]`、repeats 1、seconds 4；可以显式改为另一个单模式／单 batch／单 profile。报告顶部标注 **“diagnostic quick run，不作最终矩阵”**。quick 不能展开多模式或 batch/profile 矩阵。

| 参数 | 行为 |
| --- | --- |
| `--traffic tcp udp` | 默认 TCP 后 UDP；可单选；TCP 单流，`--tcp-streams 1–128` 可并行，UDP 始终一条流 |
| `--workers 0–256` | 默认 0 inline，与正式 Pool 一致；[策略依据](#有界输出队列实测) |
| `--cpus`／`--memory` | 每容器配额，默认 2／512m；不等于独占核 |
| `--mtu`／`--payload` | harness MTU 1280–1386（外层 veth 1500 − IPv4/UDP 28 − v6 Data 开销 86），默认 1280（与 CI 一致）；UDP payload 默认 1200，IPv4/UDP 另占 28 B；TCP 应用块不等于实际分段 |
| `--batch` | 每方向一轮立即就绪上限 1–1024，实际 secure／manager 至多 32；不等待凑批 |
| `--udp-buffer` | 默认 0 使用正式 provider；非零仅改变 L3 双方向 socket 请求，不改 Manager／plain |
| `--flow-idle-seconds` | fixture 默认 60；长窗口可显式提高，不能用读取延长 flow／授权，也不改 daemon 默认 |
| `--memory-detail` | 每 250 ms 流式保存 cgroup memory.stat、所有进程 status／smaps_rollup；采样进程开销仍计入 cgroup |
| `--allocator system\|mimalloc` | 工具默认 system；mimalloc 仅启用工具 feature；分配器统计只在 profile 开启时读取；不代表 daemon 已部署 |

release 交叉缓存 `target/dataplane-perf`；报告使用不可变二进制副本。同模式／拓扑复用容器，同一 batch/profile/repeat 的 TCP、UDP 共用 node／circuit，只重启 iperf。跨重复或 profile 重启 node、使用新材料；奇数重复反转配置顺序。fixture／node 期限覆盖连续两种负载。

### L2 实现

plain 两端真实非持久化 TUN 地址为 `100.127.0.1/30`、`100.127.0.2/30`，经正式 UdpTransport／Session 原样转发 IP；source 只匹配显式 peer，不是身份认证。截断／发送完成错误阻断样本。`per-packet` 仅批量调度，`batch` 使用正式 TUN／UDP／Session 批量 I/O；两者均为 1024 帧、`MTU × 1024` 字节、一个在途操作预算。每节点 current-thread runtime，期限或 SIGINT 后停止新工作、取消并 join Session owner。raw 的 veth MTU 与内层相同且保留 offload；plain 外层 veth 为 1500。

### TUN 批量接口与能力验证

`Options::with_batch_io(true)` 显式请求 Linux `IFF_VNET_HDR`，安全 tun-rs 入口先探测 CSUM＋TSO4＋TSO6，再探测 USO4＋USO6；基础能力失败则关闭新描述符并创建逐包设备。实际值由 `Tun::capabilities()` 返回；默认 Tun options 仍逐包。

- `recv_batch` 至多 32 完整 IP 包／64 KiB，不等待凑批；超包剩余分段留在有界状态，逐包 recv 也能消费。单超包至多 65,535 B／1024 段，超限错误；分段槽按真实大小复用。
- `send_batch` 返回一次写入的真实前缀包数，至多一个超包，不兼容时一个普通包；后缀继续携带原 guard，只代表本机接纳。不能用 `send_multiple` 的总字节数伪装前缀完成。
- 拆合归固定 tun-rs 的 VirtioNetHdr／gso_split／GROTable，重建双栈长度／checksum／TCP 序号，FIN／PSH 留末段；检查 flags、真实 header、GSO 类型、offset、MTU、段数，不信任 kernel hdr_len。UDP 零 checksum 编为 `0xffff`，非首段清 CWR 并更新 checksum；未协商 TSO_ECN。
- GRO 要求同流、TCP ACK/options 相同且序号相接，标志限 ACK 或 ACK+PSH；IP ToS／Traffic Class、TTL／Hop Limit、IPv4 DF／保留位相容，长度与 checksum 正确。IP 分片、IPv4 options、IPv6 扩展头、SYN／FIN／RST／URG／ECE／CWR、纯 ACK 不合并；PSH／短尾只在末段。UDP 只向同流最后一组追加，短尾后停止，并依赖实际 USO。
- tun-rs 可按序号向前拼接、跨其它流寻找同流，不要求 IPv4 ID／IPv6 flow label／TCP window 相同。Meshway 额外校验 IPv4 checksum、UDP length、TCP reserved／NS，必要时收缩候选并重新 GRO，确保第一组合恰好覆盖完整输入前缀，不误报非连续索引。
- staging 保留 virtio headroom 和 ExpandBuffer 容量约定；不可变发送输入要复制到 staging，GRO 内部和 GSO 分段也有复制，不声明零复制。常规完整前缀只求 GRO 一次。

OrbStack 独立容器已验证上述六种能力，双栈真实 TCP 8192 B 至少五段的序号／长度／checksum，以及 TCP 三包合并交付 3000 B、UDP 三包恢复三个报文。入口为 `offload_capabilities_and_normalized_packet_io`，逐包生命周期／双栈 I/O 同时验证。L2 应用流是 IPv4；IPv6 能力由独立测试证明。

### L3 实现与实测

`secure-direct` 为 A→B，`secure-relay` 为 A→R→B，仅端点打开 TUN；R 无 TUN device／NET_ADMIN，只执行正式认证／重放并转发 ciphertext。每 namespace 0700 私有目录生成 0600 种子，秘密不进入 fixture／stdout／报告；公开 fixture 记录路径、demand、材料公钥、编译／到期与 publication digest。每进程用同一真实 compiler 独立编译核对摘要，进程内期望内容仅由 `tools/meshway-desired-fixture` 提供，它不签发租约。

iperf TCP 控制流与 UDP 负载使用精确 5201 demand，启动前建立 Noise circuit，正式 SecureNode 独占 flow／replay／原完整路径回复。每节点 current-thread owner＋正式 Pool，guard 随输出保留至 Session／TUN 最终 poll；准备→crypto→保序 owner 提交→有界 TX／TUN。SecureNode 使用默认 limits，L3 Session 帧／操作预算与 plain 相同，字节另容 secure envelope。

### L4 Manager 边界

Manager 复用上述驱动与静态签发，经过正式 `Host::open/install`、capture 和受保护 SocketFactory；data scope 匹配真实安装，不借 bootstrap 绕过。固定 Native UDP endpoint 经真实编译，执行邻接认证、选择、健康与 scope；发送走 Manager，接收走真实事件队列，持续更新实际 carrier，旧输出不换绑。各节点使用 Host capture，R 保有 capture 但不读本机业务／交付明文（`captureTun` 有值，业务 `tun=null`）。不含 Server／Control I/O、自动 demand、DNS、安装协调，这些属于 L5。

fixture 的 topology 把被测路径的每条边（A–B 或 A–R、R–B）声明为 `keepalive: true`，Manager 按[常驻规则](/v2/configuration-reference/topology#邻接的建立与常驻)在启动时建立并保持邻接；fixture 在全部邻接出现后才发起 circuit 握手。daemon 对双向可拨边的首包 interest／中继 hint 按需获取不在本层测量，邻接获取延迟不计入任何样本。

### 内核 WireGuard 参照

只使用内核 WireGuard 与锁定 wireguard-tools；不支持时样本失败，无 userspace 替代。direct 两端 `wg0`；relay 是 A↔R.wg0、R.wg1↔B，两端无直接 peer，R 仅在私有 namespace 开 IPv4 forwarding。AllowedIPs 仅对端 `/32`。每样本新 key，私钥留容器 0700 tmpfs／0600 文件，不保存到报告；核对接口类型、MTU、路由、每段握手与双向字节。raw/WG 的 batch/profile 为 0（不适用），每负载／重复仅测一次。

WireGuard relay 解密 IP、routing 后再加密；Meshway relay 不解密业务。WG Data 固定 32 B＋padding，Meshway v5 每段 86 B，均另有外层 UDP/IP。信任模型、offload、内核多核执行与用户态边界不同，不能把差值全称为密码／头部成本。WG softirq／kworker 的 CPU 可能记在样本 cgroup 外且不完全受其 quota 限制；未测 VM 全局 CPU，**未完整测得不等于零**。

### 共同报告口径

结果在执行工作树 `.meshway/perf/dataplane/<run>/`；共享测量锁在 Git 公共目录所在主工作区 `.meshway/perf/dataplane/measurement.lock`，其他 worktree 占用时轮询。测量前检查 `uptime`，当前收尾对照优先选择宿主一分钟 load <4；若连续 >6 超过10分钟，可在 <8 时使用明确标注的共享负载例外，保存等待历史、准入门槛及各样本首尾 load。构建／全量检查与 Docker 测量分开，锁不保证宿主独占；测量中负载再次升高仍按真实值保留。

| 文件／字段 | 契约 |
| --- | --- |
| `results.json`／`bin/` | schemaVersion、revision／dirty、源码 digest 首尾、工具链／平台／flags、二进制 SHA256、参数、不可变二进制与逐次样本 |
| `summary.md` | 成功样本中位数，按 mode／batch／profile／traffic；L2→L3、secure−WG 同组差值；失败数量、quick 标记 |
| `<sample>/{a,r,b}.stdout/.stderr` | readiness、日志、原始 profile；raw/WG 无 node 进程 |
| `failure`／`nodeExitCodes`／`processLogs` | 失败样本的原因、各 node 退出码，以及每个角色 stdout／stderr 的有界尾部（至多 40 行／16 KiB，去掉 profile 快照）；仅失败样本有 `processLogs`，使只收 `results.json` 的 artifact 也保留 node 自己的错误；成功样本不附带日志 |
| `iperf-client.json`／`iperf-server.json` | 原始应用结果，旁边 stderr 保留失败原因 |
| `goodputMbps` | 接收端实际应用 payload，不含封装；iperf 排除 warmup 区间 |
| `packetsPerSecond`／`lostPercent` | UDP 实交付报文/s／丢包；TCP pps 为 null，不按假定包长倒算 |
| `cpuSeconds`／`cpuPercent`／`cpuByRole` | 所有角色 cgroup 增量／完整客户端墙钟，100%＝一核，包含 warmup、iperf、采样与辅助命令；历史缺角色不均分 |
| `nodeThreads` | Linux task CPU 前后差，按自己的快照墙钟算活跃率；余下墙钟含阻塞、调度和 quota，不能称精确休眠占比 |
| `memoryByRole`／`*.memory.csv/.events` | 每 250 ms cgroup current、真实 node VmRSS、OOM 事件；缺失为 null，采样峰值不等于连续峰值，RSS 与 cgroup 不能相加 |
| `*.memory.detail` | 显式详细分项，经独立 stderr 流写宿主，不积累在容器 tmpfs；非原子读取，失败保留 |
| `capabilitiesInitial/Final`／`tunBatch` | 实际 TUN 模式、UDP offload/mmsg 成功次数与 `buffers`（读回的 socket 缓冲及请求值）；Manager 的 offload 计数为 null，其 `buffers` 来自已登记的 socket，fixture 无 instance 时为空，实际值看 `networkCounters` 的 `ss` |
| `networkCounters`／`tcpStreams` | SNMP、dev、ss、逐 socket drops 首尾与差；TCP sender RTT／重传／cwnd 原值，max cwnd 不是平均窗口 |
| `wireguard` | 每接口类型、MTU、AllowedIPs、endpoint、路由、握手／双向计数，不含私钥 |
| `allocations`／`copiedBytes`／`peakMemoryBytes`／`syscallsPerPacket` | 未测为 null；`/proc` I/O 不覆盖完整 syscall；不由 capability 倒推实际次数 |

UDP 权威接收项为 `server_output_json.end.sum_received`，验证字节等于实交付报文数×payload；iperf3 3.16 的 `end.sum` 可错误为零，不能替代。失败样本和日志保留、指标不填零、脚本返回失败；不择优替换历史失败。

profile 开关分组，正式吞吐取 0。TCP checkpoint、UDP 停止并 join 后 final snapshot 做累计差分，同名 metric 先聚合；max_wall_ns 仍是进程累计最大，不冒充 UDP 区间最大。原连续日志在 `*-nodes/`，收到 final 即退出。poll scope 不计 await 挂起，self CPU 只扣实际采到的嵌套 scope；精确 calls、samples、分母与 observer 开销必须一起解释。

### 流水线墙钟观察

| 指标 | 边界 |
| --- | --- |
| `pipeline.submit_to_ready.wall` | 提交→crypto 完成；内部 `submit_to_worker` 为排队／调度，`worker` 为计算墙钟 |
| `pipeline.ready_to_release.wall` | 完成→owner 取出保序前缀 |
| `pipeline.release_to_credit_drop.wall` | 取出→最终 TX/TUN 完成、失败或取消，含 commit／批内等待／I/O |
| `l3.input_head_to_prepare.wall`／`manager.receive_to_consume.wall` | 仅输入批首→prepare、Manager 接收批首→消费，不是全部包均值 |
| `carrier.release_to_admission.wall`／`admission_to_completion.wall` | 抽样批次 release→Session 接纳、接纳→本机完成 |
| `transport.admission_to_first_poll.wall`／`first_poll_to_write.wall` | 接纳→首 poll、首 poll→首个成功 raw write，含 readiness／guard／syscall |
| `carrier.tun.write.wall` | 一次成功 TUN 写入及 readiness 等待 |

跨线程只传单调时刻，每线程每128次抽样，不推断 CPU。各项互相嵌套，不直接相加；丢弃／取消可释放 credit，须核对拒绝计数。批次 histogram 桶 1／2–4／5–8／9–16／17–32，记录实际非空 dispatch／release／carrier send／TUN 已写前缀；不把 capability 或桶上界当作真实合并数。`l3.backpressure` 是保留输入的等待，不是丢包。profile-on 每秒 `pipelineOccupancy` 记录输入、credit、TX/TUN capacity／jumbo、flow/circuit/task，属于有界性观察，不是权限或交付回执。关闭采样不读取这些时钟／快照。

## 当前结论

以下各表统一保留指定最终报告，不能跨运行择优拼接。环境／实现不同的行不是配对 A/B。OrbStack 为 aarch64 kernel `7.0.14-orbstack-00380-ga7e0a2dc9535`、Ubuntu24.04、iperf3 3.16、Rust1.98／Zig0.17／cargo-zigbuild0.23.4、Linux musl；Planck 为 AMD Ryzen9 9950X／kernel6.8／Docker29.8，多用户共享；CI x86_64 为 `perf.yml` 自托管 runner（AMD EPYC 7543，kernel 6.8，构建在 tmpfs），UDP 2G 输入下约 208k pps 为输入率上限而非容量。L2–L4 通常每容器2 CPU／512 MiB、MTU1280、payload1200；UDP输入率和成功数另列。

### L0 最终同步矩阵

| 环境／报告 | 1280 B：2／3／5节点 pps | 52 B：2／3／5节点 pps | 范围 |
| --- | ---: | ---: | --- |
| Apple M3 Pro macOS，`20261007T080533Z-7b11af1b` | 116,395／81,295／54,603 | 203,539／138,736／90,776 | inline、默认36格全部成功 |
| Planck，`planck-20261007T070124Z-c7b84c03` | 140,454／98,093／61,793 | 未在本页汇总 | `5a30247d`，1280B CPU 7.12／10.19／16.18 µs |
| OrbStack | 当前最终同步矩阵未测 | 未测 | 历史插桩CPU不能替代 |

### 原生 ARM64 L0（jp-tyo） {#l0-arm64}

`perf.yml` 运行 `38977113530`（push，`f499a809`）的两个 job 用同一源码各自原生构建、运行默认 `perf:circuit` 矩阵：`perf-arm64` 在 jp-tyo 自托管 runner（Oracle Ampere，Neoverse N1 4 核，kernel 6.17，`aarch64-unknown-linux-gnu`），`perf` 在 Planck 自托管 runner（x86_64，32 逻辑 CPU，kernel 6.8）。下表取 profile 关闭、每格三个独立进程的中位数；CPU／包为 harness 线程 CPU／实际交付 IP 包（含全部路径角色），A.send／A.recv 为发送端 A 每次调用的 owner 线程 CPU。报告为该运行的 artifact `perf-arm64-38977113530` 与 `perf-38977113530`。

| 节点／IP 字节 | ARM64 pps | ARM64 CPU ns／包 | ARM64 A.send／A.recv ns | Planck x86 pps | Planck x86 CPU ns／包 | Planck x86 A.send／A.recv ns | pps 比值 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2／1280 | 66,884 | 14,947 | 5,930／6,850 | 171,733 | 5,823 | 2,287／2,705 | 0.39 |
| 2／52 | 112,266 | 8,906 | 3,665／4,274 | 265,344 | 3,768 | 1,590／1,836 | 0.42 |
| 3／1280 | 50,495 | 19,789 | 6,025／6,869 | 121,911 | 8,201 | 2,337／2,747 | 0.41 |
| 5／1280 | 33,788 | 29,591 | 6,084／6,972 | 76,732 | 13,031 | 2,385／2,844 | 0.44 |

原生 ARM64 的单线程 L0 吞吐约为同运行 Planck x86 的 0.4（各格 0.39–0.44，3／5 节点 52 B 为 80.7k／52.2k pps，比值 0.41／0.43），A 的发送与接收 CPU 各约为 x86 的 2.5–2.6 倍，节点数增加时比值不变差。两台 runner 均非独占，ARM64 job 仅在 load1 ≤ 2.0 时开始测量（本次各格测前 load1 0.24–1.78）。arm64 job 只跑 L0＋原语（原语仅在手动运行指定 `crypto` 层时，本次 push 运行没有），无 L3／L4，不能据此推算 ARM64 真实 TUN／UDP 吞吐。

### L1 当前结果范围

| 环境 | 已有能力 | 最终数值 |
| --- | --- | --- |
| host／Linux | 五协议 raw/datagram、payload／batch／重复对照与报告 | 本页未发布最终同环境矩阵，不能由 L2 推算 |

### L2 内核批量实测

| 环境／模式 | TCP Mbps | UDP pps | 输入／重复 | 报告 |
| --- | ---: | ---: | --- | --- |
| OrbStack 2 CPU plain per-packet | 2,142.13 | 109,716 | 2G，2次 | `20261007t002750z-1171958b` |
| OrbStack 2 CPU plain batch | 3,667.94 | 117,041 | 同轮2次，TCP +71.2%，UDP +6.7% | 同上 |
| Planck 2 CPU plain batch | 17,222 | 208,224 | 2G，2次 | `planck-20261007t070435z-a3f07ece` |

OrbStack 同轮 raw 自身丢包27.39%，不能把全部 UDP 丢包归给转发器；batch TCP／UDP CPU 为152.99%／192.28%。实际 GSO／GRO／mmsg 成功次数在报告，双栈禁用能力回退由隔离测试验证。后续 P 的 plain TCP 为10,506 Mbps，但同轮 UDP仅一次成功，见历史索引。

### L3 流水线与 L4 实测

| 层／环境／路径 | TCP Mbps | UDP pps | UDP输入／成功数 TCP/UDP | 报告 |
| --- | ---: | ---: | --- | --- |
| L3 OrbStack 2 CPU direct | 1,693.69 | 141,833 | 2G，2/2 | `20261007t082302z-a2aff6a7` |
| L3 OrbStack 2 CPU relay | 1,653.76 | 49,612 | 500M，2/2 | `20261007t204126z-cbe7469c`，inline／mimalloc |
| L3 Planck 2 CPU direct | 2,292 | 199,927 | 2G，2/2 | `planck-20261007t070435z-a3f07ece` |
| L3 Planck 2 CPU relay | 2,127 | 195,072 | 2G，2/2 | 同上 |
| L3 CI x86_64 2 CPU direct（`2254d0f9`，wire 6／netmap） | 2,705／2,705 | 208,262／208,313 | 2G，2/2，丢包 ≤0.03% | `perf-38143155889`／`20261011t133440z-e33c27b2` |
| L3 CI x86_64 2 CPU relay（同上） | 2,637／2,590 | 208,334／208,218 | 2G，2/2，丢包 ≤0.05% | 同上 |
| L3 Planck 2 CPU direct（`c72a498a`→`289e3058`，M4） | 2,724→2,727 | 104,154→104,164 | 1G（输入上限），2/2，丢包 ≤0.01% | `20261015t071207z-7d3a9547`→`20261015t071647z-4e21cce5`；L3 不产生出口流，[出口流维护](#出口流维护与真机-cpu)不改变这一层 |

P 两核报告10/12成功，relay与plain UDP各一次严格接收账本失败；历史relay为1,482.99 Mbps／96,874 pps（UDP仅一次成功）。direct在其窗口达到1 Gbps／100k pps，但2G有丢包，不是无损容量。P的四核完整矩阵12/12成功，direct为1,675.20 Mbps／139,511 pps、relay为1,427.12／95,674；配额增大不等于专用核。新relay使用500M输入，不能以49,612 pps解释为容量回退。

Planck使用旧`5a30247d`及1 worker，L3 direct／relay RTT4.3／2.9 ms、重传786／322；P／Y默认inline在该主机未重验。不能将跨架构数字视为同版本比较。

### L4 最终配对

| 环境／路径 | TCP Mbps | UDP pps | UDP输入／成功数 TCP/UDP | 报告 |
| --- | ---: | ---: | --- | --- |
| OrbStack 2 CPU direct | 1,468.05 | 49,025 | 500M，2/2 | `20261007t112743z-6f3b2103`，历史Y |
| OrbStack 2 CPU relay | **1,628.89** | 48,034 | 500M，2/2 | `20261007t204126z-cbe7469c`，inline／mimalloc |
| Planck 2 CPU relay（`9c6e4cb3`，M2） | 2,468／2,422 | 104,162 | 1G，2/2，丢包 0.003%／0% | `planck-20261013t211518z-904289c2`；改动前 `e5a56b27` 2,229／2,289、重传 512／534，见[接收缓冲](#接收缓冲) |
| CI x86_64 2 CPU relay（`2254d0f9`，wire 6／netmap） | 2,422／2,386 | 207,690／208,242 | 2G，2/2，丢包 0.30%／0.05% | `perf-38143155889`／`20261011t133440z-e33c27b2`；同轮 L3 relay 2,637／2,590，L4/L3 TCP 逐重复 **0.918×／0.921×** |

最终relay报告使用源码`a549bdba`，固定binary `ff48e9332c727ab7be5dcea93dbb5646ff35c9122717e9a70d5a7521e310dd61`，源码digest首尾一致。命令为上方正式配对参数另加 **`--batch 32 --allocator mimalloc`**；其他参数不变。8/8成功，L4/L3 TCP中位数比 **0.985×**，逐重复配对为 **0.968×／1.002×**，均达到≥0.9目标。L3／L4 TCP cgroup CPU为234.60%／257.38%，全部TCP重传0；RTT分别17.43／22.91 ms和16.47／15.69 ms，仍不构成低延迟验收。UDP丢包分别4.71%／7.74%，不是无损容量。最高节点RSS为24.45／30.82 MiB，全部OOM事件为0。

本工作包构建／检查退出后测量，所有样本使用共享锁。此前连续高load超过10分钟的等待证据使本轮可用<8例外；实际首尾一分钟load **3.92–5.37**，Manager两次TCP分别在<4的准入窗口开始。`results[].measurementGate` 保存门槛／连续高负载证据／等待历史，原始`hostUptimeInitial/Final`保存实际值；未获得独占CPU。

改动前同 allocator 的配对 `20261007t194733z-3512cfe3` 为 **0.677×**（947.86／1400.59 Mbps，首尾 load 7.22–7.55；改后 6.70–7.07，差值不能全部归于代码）。含 batch 1 的完整矩阵仍有严格 UDP 账本／readiness 失败，上述结论只取独立的 batch32 完整配对，未拼接失败样本。

### Manager 队列归因

Y的历史batch1 500M L4/L3为0.844×，2G batch32为0.795×。profile128诊断 `20261007t105907z-45f2114a`→`20261007t113645z-ea3b934c` 中，R的选链→首次poll从208.35降至12.96 µs，release→credit从290.85降至120.64 µs，而接收批首→事件消费从169.38增至1,646.66 µs；更深队列吸收突发也增加等待。Session接纳→首poll约0.23 µs，不是逐批必经后台任务。

当前事件消费者直接移交就绪批前缀，并在已有两批积压时提前给消费者调度机会，不等到1024槽耗尽；原硬上限、FIFO、部分消费预约、控制优先、32-credit和AA查询节流均保留。L3/L4工具同时对齐正式daemon：新输入出队后直接进入32包／200 µs owner量子，省去中间额外yield；处理／输出后仍让出，暂停业务时控制事件不触发此捷径。**这项驱动对齐消除的是harness额外调度成本，不能全部宣传为daemon加速。**

同为batch32、profile128、4+1秒quick、system allocator，且都已含批队列改动，移除中间yield前后为`20261007t195140z-b8ef50ff`→`20261007t195437z-a1d41cb4`：R接收批首→消费 **3671.20→33.40 µs**，输入→prepare **28.35→2.51 µs**，选链→首poll **19.31→6.97 µs**，release→credit **190.86→93.91 µs**。后测消费后占用为20,031次空、2,454次9–32、7次33–64，未观察到>64；真实waker回调3476、批发布22495、poll64306，不能互称。各墙钟含采样／排队且互相嵌套，不能加总为CPU。行为定义见[Manager控制预算](/v2/node/connectivity#udp-控制消息预算)。

### WireGuard 同环境参照

| 环境／路径 | TCP Mbps | UDP pps | UDP输入 | 报告 |
| --- | ---: | ---: | --- | --- |
| OrbStack 2 CPU direct | 2,741.54 | 190,222 | 2G | `20261006t234736z-dfa88ba3` |
| OrbStack 2 CPU relay | 4,853.50 | 178,003 | 2G | 同上 |
| Planck 2 CPU direct | 4,780 | 208,108 | 2G | `planck-20261007t070435z-a3f07ece` |

每格两次、profile off；OrbStack同轮旧v4 L3 direct／relay为321.90／198.76 Mbps，不能与今日L3作同二进制差值。WG relay高于direct的现象保留真实值，尚未隔离内核调度／offload原因。Planck 8 CPU／MTU1420／4G校准为5,117 Mbps／414k pps，raw107.8 Gbps；相对[wg-bench](https://github.com/cyyself/wg-bench) 7950X 5.64 Gbps低9%，不是无配额netns复现。

### L5 常驻 lab 实测

| 场景 | 实测 | 范围 |
| --- | --- | --- |
| TCP单流，2次 | 277／330 Mbps；重传554／896；RTT3.5／5.9 ms；max cwnd213–222 KB | `20261007T085207Z-main-4c4e231f` |
| UDP 200M／500M | 70／128 Mbps；丢包56%／74% | 原始接收口径见同报告，不作无损容量 |
| `meshway ping` 5次 | 全回复，RTT0.33–0.52 ms via lax | 完整sha→lax→nyc |
| 内核ping | 首包约1.1 s建circuit；稳态0.5–5 ms | 与独立DiagnosticPermit测量分开 |

OrbStack aarch64、每节点0.5 CPU／384 MiB、iperf3 3.16、TCP `-t6 -O1`、UDP payload1200；源码`4c4e231f`。该报告尚不含随后P的有界输出队列／inline默认；不能用L3/L4推算最新daemon或把构建视为部署。历史v4同路径TCP49 Mbps。

当前 main（`1271be5d`，含 V2 的 STUN 刷新／State 采样修复）在同一 lab、每节点 **2 CPU／512 MiB**、同路径 sha→lax→nyc 的复测：

| 场景 | 实测 | 范围 |
| --- | --- | --- |
| TCP单流，2次 | **1321／1525 Mbps**；重传17／61；RTT8.2／6.6 ms；max cwnd约1.4 MB | `20261008T055333Z-main-1271be5d-2cpu` |
| UDP 500M | 接收475 Mbps；丢包4.9% | 同报告，`sum_received` 口径 |
| `meshway ping` 5次 | 全回复，RTT0.29–0.35 ms via lax | 完整sha→lax→nyc |

入口 1 分钟 load 2.29、5 分钟 5.08（无构建，仅有一个休眠的验收编排进程）；daemon 二进制 `6dc10d36…` 经 `node:lab versions` 按 PID／hash 核验。与 `4c4e231f` 行的差异包含 CPU quota 0.5→2 和 M／O／P／Y／PT 的实现变化，不是配对 A/B。当前实际部署／行为验收以[lab](/v2/node/docker-lab)及[矩阵](/v2/node/validation)为准。

| 混合承载场景 | 实测 | 范围 |
| --- | --- | --- |
| sha→lax VLESS TCP/TLS，lax→nyc Native UDP | TCP 接收 **1157／1143 Mbps**，重传160／203，sender RTT16.4／23.2 ms；UDP 500M 接收 **456.6 Mbps**、丢包8.68%；测后诊断5/5、RTT0.32–0.74 ms via lax | `20261008T063234Z-main-1271be5d-2cpu-vless`；同 daemon／2 CPU／512 MiB，**高负载诊断，非空闲性能结论** |

等待180秒仍未达到1分钟load<2.5，三次样本首尾load **7.10–7.36**；原始等待历史、客户端／接收端JSON及PID／hash在报告中。本工作包测量时未运行构建。UDP按server `end.sum_received`计量：342,717,600 B＝(312,736−27,138)×1200。相对于上方Native UDP的1321／1525 Mbps、RTT8.2／6.6 ms与475 Mbps／4.9%丢包，这组数据受不同宿主负载与旧版流式续接行为影响，不能归因成VLESS单独开销。

部署 main `11d65768`（含 W2 续接修复，daemon `f04dd1ce…`）后在 revision 10 重做同一布局：

| 场景 | 实测 | 范围 |
| --- | --- | --- |
| sha→lax VLESS TCP/TLS，lax→nyc Native UDP | TCP 接收 **1266／1304 Mbps**，重传218／285，sender RTT12.0／17.6 ms；UDP 500M 接收 **453 Mbps**、丢包9.3%；测后诊断5/5 via lax | `20261008T071549Z-main-11d65768-2cpu-vless`；入口1分钟load 3.16（等待3分钟未低于2.5，无构建） |

持续业务下连续 7 次 State 采样（3 分 7 秒，跨越至少三次 60 秒执行续租）VLESS link ID 保持 `834a83a5…` 不变，lax↔nyc UDP link ID 同样不变；业务停止后链路按既有规则释放，新需求再建立新 link ID。与同机 Native UDP 边的 1321／1525 Mbps 相比，TCP 低约 5–15%、RTT 高约 4–11 ms、UDP 丢包 4.9%→9.3%；内层 UDP 跨 TCP 承载的队头阻塞是预期行为，但两组入口负载不同，不是配对 A/B。

revision 7 的State实际认证通道为sha↔lax `vless`、lax↔nyc `native_udp`，两个方向的`meshway ping`各5/5，回复沿完整via lax路径。刷新publication时VLESS link ID改变、随后短暂消失，UDP link ID保持；`1271be5d`的`renew_link`只保留Native TCP／UDP，尚未部署W2修复。因此本轮不构成流式长时连续性验收，实验后已回滚到revision 8（等价于原revision 6）。日志、State序列、材料约定及进程内复现索引位于私有`.meshway/w3-lab-vless/`；当前布局见[lab](/v2/node/docker-lab#混合承载配置)。

### 有界输出队列实测

默认 **`MESHWAY_DATAPLANE_WORKERS=0`（inline）**。P在同一pending实现、硬件密码后端、32-credit下的单流quick曲线：2 CPU的worker0/1/2/3 TCP为**1859.8／1216.4／980.8／974.4 Mbps**，4 CPU为1770.9／1198.7／896.2／927.4。单在途L0两节点1280B跨线程墙钟从**8.97→24.54 µs**，三节点12.47→35.75 µs，通知／交接超过当前crypto计算；≤128 B继续inline。该曲线为单次诊断，默认另有P两次矩阵支撑，不宣称所有CPU都最优。

只有预期获得 **≥4个独占核且AEAD重新成为主要成本** 时，才值得显式试验>0；须重新做同机profile-off矩阵、CPU／交接／RTT和生命周期验证，不能按quota自动增worker。H/I的池保留为可选实现：统一owner/crypto职责、按circuit保序、32个全生命周期credit、控制优先与取消/join边界，并支持未来高crypto占比平台复用；inline和线程池执行同一crypto实现。

### Owner 路径与 TCP 拥塞诊断

缓冲默认值与特权请求见[UDP批量接口](/v2/node/transport#udp-批量接口)；本段 O／P 数字为当时 OrbStack 上普通请求的结果；Planck 上的对照见[接收缓冲](#接收缓冲)。O中扩大buffer消除TCP样本外层接收溢出，direct／relay从939.84／726.16升至1633.46／1464.09 Mbps、重传归零，但RTT升至13–23 ms；不是低延迟优化。P quick将7 MiB降至1 MiB／256 KiB／64 KiB，TCP1796.6→1194.6／881.7／764.4，RTT13.85→8.56／2.78／1.04 ms，后两者重传125／1905。较低RTT用吞吐／重传换取。

Circuit稳定分配和共享Context后原生L0两／三节点总CPU为8.235／11.164 µs，memmove调用者0.164／0.186 µs；包含聚合移动，不是全部可删payload copy。map、flow、replay、真实guard及最终提交仍保留；小包inline不授予ACK特殊权限。

### 接收缓冲

M2（`e5a56b27` → `9c6e4cb3`）。Planck（Ryzen 9 9950X、kernel 6.8、宿主 `net.core.rmem_max`＝212,992，与 it7 相同），每容器 2 CPU、MTU 1280、batch 32、profile 0，每格两次；“netem”为两端 eth0 各 `tc netem delay 30ms limit 100000`（60 ms RTT），“不对称”另把 A 设为 8 CPU、B 为 0.5 CPU。改动前 socket 读回 416 KB（`rb425984`）；7 MiB（`a25f2d26`）读回 14 MiB，最终 4 MiB 读回 8 MiB（端点与 L4 的 R 有 `NET_ADMIN`，强制请求生效）。

| 场景（Mbps；重传） | 改动前 416 KB | 7 MiB | **4 MiB（最终）** |
| --- | ---: | ---: | ---: |
| L3 direct TCP，1 ms RTT | 2,540／2,492；618／668 | 2,615／2,708；0 | **2,674／2,699；0** |
| L4 relay TCP，1 ms RTT | 2,229／2,289；512／534 | 2,462／2,426；0 | **2,468／2,422；0** |
| L3 direct UDP 1G 丢包 % | 0.001／0.453 | 0.011／0 | 0.024／0.042 |
| netem TCP | 266／315 | 409／410 | **410／412** |
| netem UDP 300M 丢包 % | 0.021／0 | 0／0 | 0／0 |
| netem 不对称 TCP | 377／400；0／64，B socket 丢 10／10 | 410／411；0 | **411／412；0** |
| netem 不对称 UDP 300M 丢包 % | — | 0／0 | 0／0 |

- 默认缓冲下所有行的 Meshway socket 丢包（`ss` 的 `d`）为 0；UDP 行残余的少量丢包在接收 namespace 的 iperf socket（`UdpRcvbufErrors` 增量），不在 Meshway socket。
- 4 MiB 与 7 MiB 在全部格上的差异都在两次重复的波动内。1 ms RTT 满载时内层 TCP 的 sender RTT 两者都约 10 ms（改动前 2.0–2.5 ms）：缩小缓冲没有降低满载时延。两者的内层 max cwnd 都约 4.3 MB，小于两种缓冲，推断排队量由内层窗口而不是 socket 缓冲决定（未单独测量排队位置）。
- 接收方溢出的修复只靠缓冲。M2 另实测过按通道的发送节奏（令牌桶，允许速率为近期峰值的两倍）：在 7 MiB 缓冲下，20 ms 记忆让 60 ms RTT 的内层 TCP 降 14%（每窗都重新爬升），100 ms 记忆与无节奏相同；只有 416 KB 缓冲（模拟没有特权）时 20 ms 记忆消除了接收方丢包。**pacing 在 7 MiB 缓冲下无收益**，未保留。
- netem 下 manager-direct fixture 两种版本都起不来（双方同时拨号 `existing_rank` 后 `Context`），40 秒以上的 UDP 窗口紧跟 TCP 时以 `State` 退出；均与缓冲无关，未作为样本。CPU 配额不是独占核；共享宿主首尾一分钟 load 3.2–7.2，逐样本见 `results[].hostLoad*`。

报告与脚本在 `.meshway/validation/m2/`（`perf/`、`runs.txt`、`logs/r4.list`、`samples*.txt`）：最终对照 4 MiB `20261013t211518z-904289c2`、`20261013t212217z-e0e0ecc9`、`20261013t212309z-334276f9`、`20261013t212454z-a2a529b8`、`20261013t212546z-944777aa`，同轮 7 MiB `20261013t210933z-8b199dcb`、`20261013t212058z-f02425d3`、`20261013t212150z-aacae5ac`、`20261013t212336z-0c688328`、`20261013t212428z-2de6e09d`；改动前 `20261013t192411z-9034b130`、`20261013t194224z-e6ff6eb8`、`20261013t200659z-7b0ca5be`、`20261013t200923z-960e09ad`；发送节奏的测量报告列在 `runs.txt`。

### 出口流维护与真机 CPU

M4（`c72a498a` → `89def831`）。真机只读观察 alpha.11 release（未部署新版本）：it7（AMD EPYC Genoa VM，2 vCPU，kernel 6.8，内层 TCP BBR）向 planck 的 Mesh 地址发 iperf3 30 秒，248 Mbit/s：

| 项 | 实测 |
| --- | --- |
| 网络 owner 线程（TUN 读、准入与加密、UDP 发送和 worker 其余工作都在这一个线程） | 76.8%（user 46.2、sys 30.6） |
| 两个 Tokio worker（回程 ACK 的 UDP 接收，约每次 `recvmmsg` 一个报文） | 14.4%／13.6%；其他线程 <1% |
| 批量 I/O | eth0 发送 26.3k 包／秒；owner 每秒 1,884 次 `sendmsg`（每次约 14 个 GSO 段）；TUN 每秒 1,048 次读（TSO 聚合，约 21 段）；每秒 3,574 次 TUN 写，即逐个回程纯 ACK（纯 ACK 按设计不 GRO） |
| 每外层包 owner CPU | 约 29 µs（user 17.6、sys 11.7） |
| perf 全部线程 | meshwayd 用户态 49.5%、内核 41.2%、vdso 2.9%、`nft`／`sh` 子进程 5.5% |
| 内层 TCP | RTT 88.8 ms（最小 72.5）、`sndbuf_limited` 25.4%；32 段重传全部为 DSACK（乱序，不是丢包）；`tcp_wmem` 上限 4 MiB |

GSO、TUN vnet_hdr／TSO 与 recvmmsg 都在用，单 owner 线程没有锁争用。真机 Mesh 正向因此同时受两点限制：owner 接近饱和，以及内层发送缓冲在排队抬高的 RTT 下封顶。发布产物没有符号表，用户态函数无法在真机归因（同源重建能复现布局但同样无符号）。空闲时进程仍占 15.6% 核，子进程另 9.8%：出口角色每秒审计经 `/bin/sh` 闸门执行 `nft`，实测每秒 5.45 次，其中 4 次来自出口审计（两次 dump × 两张表），1 次来自 egress capture 回读。

**出口流隔离项按包扫描**：每次准入经 `maintain_circuits` 执行 `FlowTable::expire_exits`，扫描全部流并对每个隔离项（最多 4,096，保留 120 秒）加锁检查。最近有出口流时（每个 echo、每个经出口的 DNS 查询都是一条流），每个包都付这份线性成本，与该包是否经出口无关。真机复现：it7 经出口 ping 60 秒（300 条 echo 流）后，Mesh 正向降到 207 Mbit/s，owner 升到 93.6%（user 64.4%），约每包多 13 µs（约 43 ns／隔离项）。fixture 采样（下述构建，`perf` 1999 Hz）：发送端 A 52.7% 的样本在这一处，出口 E 16–21%。

| 提交 | 不变量／唯一 owner | 改动 |
| --- | --- | --- |
| `289e3058` | 每个包仍检查本流存活，失效即拒；隔离只会更长；wire v6、逐跳 MAC、有界流水线不变／dataplane `FlowTable` | 失效出口流转隔离项与到期隔离项清除按 100 ms 调度（见[数据面](/v2/node/dataplane)）；待发 `Retire` 计数，drive 不再逐个扫描隔离项 |
| `08ff561c` | 每个回程段仍单独匹配投影并准入／underlay 出口 I/O owner | 出口 TUN 以 offload 批量 I/O 创建，逆 NAT 回包以聚合占一个内核队列槽 |
| `89def831` | 审计周期与检查项不变／underlay 出口 backend | 每秒审计读一次两张表（此前两次），每个出口节点每秒少 2 个 `nft` 进程 |

fixture 前后对照：`exit/off` 行第 16 阶段（A→R→E，每容器 2 CPU，Planck；iperf3 TCP 20 秒）。daemon 构建为 **dev profile 覆盖 opt-level 3、关闭 debug assertions／溢出检查、保留行表**（`CARGO_PROFILE_DEV_*` 环境变量；不是 release 的 fat LTO／单 codegen unit／panic abort，矩阵默认是未优化 debug 构建），前后同一覆盖、交替运行，Mbit/s 为 各 5 次运行的中位数（范围）：

| 指标 | 改动前 `c72a498a` | 改动后 `89def831` |
| --- | ---: | ---: |
| Mesh 正向 | 669（597–718） | **1,996**（1,962–2,016） |
| Mesh 反向 | 839（693–873） | **2,015**（2,005–2,064） |
| 出口正向 | 761（620–765） | 882（873–890） |
| 出口反向 | 1,030（957–1,061） | 1,362（1,334–1,380） |
| 出口反向时 E 的出口 TUN TX 丢包／次 | 69–133 | 0 |
| 压力步（出口 TCP 旁每 0.2 秒一条新 echo 流） | 672（660–689），echo 100% | 777（756–780），echo 100% |
| 压力步 echo 最小 RTT（ms） | 38.2（35.1–38.6） | 27.5（26.0–29.1） |

改动前一次运行（base-4）在第 16 阶段之后的租约阶段失败（`new exit flow refused within the lease: lan`，产品代码未改，其第 16 阶段计入）；改动后一次运行（a3-2）在出口反向时遇到 planck 宿主约 8.4 小时一次的 RTNL 停顿（08:23:57Z，与 D3 预测的 08:23:58Z 一致：DNS `observe_deadline` 4.1 秒、撤回包平面、iperf3 超时），不计入，另补一次。出口反向／Mesh 反向的比值由 1.20–1.39 变为 0.66–0.68，是 Mesh 反向提高的结果，不是出口回退。改后采样（`perf` 499 Hz、DWARF 栈）：A 的 owner 线程以 ChaCha20-Poly1305 封装 8.4%、HMAC-SHA256 7.6%、内存复制 5% 为首，另有 10.2% 在 vdso 时钟读取（每包多次 `Instant::now`／`SystemTime::now`）；E 在出口正向时 12.5% 在 vdso，两个 Tokio worker（出口 driver 与出口 I/O owner）各约 26%。

- 第一项提交单独（`289e3058`，两次）即得到 Mesh 正向 1,976、反向 2,030、出口正向 907、出口反向 1,356：吞吐增益全部来自隔离项扫描。改后 Mesh 正反向约为同环境 L3 secure-direct（2,724，release）的 0.73 倍，其余是 relay 一跳、daemon 每轮工作与构建差异。
- 出口反向：改动前 E 的出口 TUN 发送队列每次丢 69–133 个包（`ip -s link` TX dropped，等于该次重传数）；改后为 0。fixture 没有 WAN 时延，这些丢包在 1 ms RTT 下只损失少量吞吐；真机的 cubic 发送方在 73 ms RTT 下每次丢包都要很久恢复，planck 一次 20 秒出口反向 tun1 丢 243 个包、重传 136 个（98 Mbit/s），这就是真机出口反向 81–161 波动的来源。改后剩余的出口反向重传（372–3,157 次）不在任何可见计数上，未定位。
- 出口 echo 延迟：每个 echo 是一条新出口流，要等一轮安装（nft 事务、journal、conntrack 确认），fixture 无负载时也约 35 ms（真机 Mesh 73 ms、出口 112 ms）。压力步中 echo 最小 RTT 由 38.2 ms 降到 27.5 ms（第一项单独为 36.7），与审计少用一半 `nft` 进程一致（同一进程信号量串行全部 nft 调用）。
- 出口正向当时受出口 driver 逐包串行写出口 TUN 限制（每包两次跨任务交接并等待写入结果，压力步与正向运行都记录 `Exit queue full`），已由 X11 处理，见下文[出口正向写路径](#exit-write-path)。
- A 每秒回读约 7,000 条直连路由的 capture 表（fixture 的国家级规则集），路由 dump 与解析约占其样本 8%；真机 it7 没有这类规则集。

报告：`.meshway/validation/m4/`（`rows/` 每次运行的第 16 阶段 JSON，`extract.py`／`summary.txt` 汇总，`field/` 真机观察脚本与原始输出，`prof/` 采样归因）。下一项按样本排序：出口 driver 的逐包串行写入，以及每包的时钟读取。

### 空闲 CPU 口径与真机归因 {#idle-cpu}

P2，alpha.21。**口径**：空闲 CPU 取 ≥ 60 秒窗口内每线程 `/proc/<pid>/task/<tid>/schedstat` 运行时间之差（与 `stat` 的 utime+stime 一致），给出每线程与进程合计；出口节点另计已回收子进程（`nft`／`sh`）。`top -H` 的 3 秒快照不作门槛：两个 Tokio worker 之间的工作窃取使同一负载在 3 秒窗口里集中到一个线程，it7 最忙线程 3 秒值中位 4.7 %、p90 6.5 %、最大 12.5 %，而 60 秒均值 3.1–3.2 %；出口能力每约 20 秒刷新一次（`STATE_REPORT_TTL_SECONDS / 3`，建删一个探测 TUN 并执行 `nft`），该秒进程合计再高约 5 个百分点。

**真机归因**：发布产物无符号表。以同一 tag、`CARGO_PROFILE_RELEASE_STRIP=none` 重建一份（`.text` 与发布件不逐字节相同），对齐两者 `.eh_frame` 的 FDE 序列（函数顺序与长度）把发布件地址映射到重建件符号（17,062 个函数中 16,593 个可映射），再读真机 `perf record --call-graph dwarf` 的栈。真机只读：`perf`、`bpftrace`、`/proc`，不换二进制、不重启。

**it7 结果**（2 vCPU EPYC VM，承担出口角色，四条中继边按需建立、空闲时为 `relayIdle`；alpha.21 release；单位为一个核的百分比）：进程 7.0–7.3 %；两个 Tokio worker 合计 6.3 %，各约 142 次唤醒／秒；其余 8 个线程合计 1.0 %；进程之外的 `nft`／`sh` 子进程另约 4.2 %。

| 来源 | 线程 | CPU |
| --- | --- | ---: |
| (a) 真实转发：`activeFlows` 0，没有中继 circuit；经 meshwayd socket 的 UDP 2–3 包／秒 × 每包约 29 µs（[M4](#出口流维护与真机-cpu)） | worker | ≤ 0.02 |
| (b) 通道维护：6 个 peer 中 5 个 `direct` VLESS，健康 Ping／keepalive、穿透与拨号任务（TCP 2–4 段／秒） | worker | 0.5 |
| (c1) DNS owner 每 250 ms 一次 observe（[数据面](/v2/node/dataplane)）：每次两轮锚点证明（links、addresses、routes 共 10 次 netlink dump）、约 23 次 resolve1 D-Bus 调用、namespace 校验 | worker | 4.7 |
| 其中：锚点证明解析全部 link 时 `netlink-packet-route` 0.33 对 bridge／TUN 的每个 IFLA_INFO_DATA 属性预先格式化整段 payload（`format!("{payload:?}")`，属性数 × payload 长度） | worker | 1.4 |
| (c2) 出口与 egress capture 每秒审计（`nft -j list table` 三张表的 JSON 解码与比对） | worker | 0.9 |
| (c2) 同上的子进程管理：`sh` 闸门＋`nft` 每秒 3 次、每次写一次 `exit-child.json` 栅栏；出口能力 20 秒探测 | blocking 线程 | 1.0 |
| (c3) netlink 监视、Status 与其余 | worker | 0.2 |
| (d) 未解释 | — | ≤ 0.3 |
| 进程外：`nft` 每次约 12 ms CPU（读入宿主全部规则集缓存，含 Docker 的 iptables-nft 表），每秒 3 次 | 子进程 | 4.2 |

对照（同一时刻、同一可执行文件）：Planck 容器（出口角色，无中继，容器 netns 无 bridge，32 核 x86）两个 worker 0.39／0.40 %，唤醒次数与 it7 相同（各约 135 次／秒），每次唤醒约 29 µs，it7 约 225 µs。lab lax（Apple Silicon 上的 OrbStack，宿主负载噪声大）1.2–2.5 %／worker。唤醒频率处处相同，没有忙轮询或电平分支；it7 的差额来自每次唤醒更贵：上表 1.4 个百分点的预先格式化（只在宿主 netns 有 bridge／TUN 时出现；planck 容器内一次 link dump 解码 26 µs，构造 it7 的接口组合后 528 µs，只读 index／名称／别名 0.6 µs）、宿主 netns 更多路由与地址的 dump、2 vCPU 虚拟机的系统调用与 epoll 锁竞争（`queued_write_lock_slowpath` 占全部样本 5 %）。中继边空闲时不产生开销。

报告：`.meshway/validation/p2/`（窗口与 3 秒峰值脚本、`perf`／`bpftrace` 原始输出、符号化栈、FDE 对齐脚本、link dump 基准）。

### 出口正向写路径 {#exit-write-path}

X11：已安装流的包不再经出口 driver。driver 写出某流的一个包后，该流此后的包（投影已 ACK、FIFO 中无同流包）由包处理 owner 每轮至多 32 个交给出口 I/O owner 的有界队列，I/O owner 每次至多 64 个、逐包检查投影／guard／权限后以 offload 合并写入出口 TUN（与回程的批量读对称）；准入仍逐包。行为定义见 [Exit 设计 · 出口队列](/v2/node/exit-design#exit-queue)。

fixture 前后对照，口径与上节相同（`exit/off` 第 16 阶段，Planck，每容器 2 CPU，dev profile 覆盖 opt-level 3，前后交替运行，Mbit/s 为各 5 次的中位数（范围），括号外为接收速率）：

| 指标 | 改动前 `75fdaad3`（alpha.12） | 改动后 X11 |
| --- | ---: | ---: |
| Mesh 正向 | 1,997（1,943–2,019） | 2,028（1,999–2,047） |
| Mesh 反向 | 2,051（1,924–2,074） | 2,066（2,052–2,073） |
| 出口正向 | 890（810–904） | **1,764（1,750–1,785）** |
| 出口正向／Mesh 正向 | 0.45（0.42–0.45） | **0.86（0.86–0.88）** |
| 出口反向 | 1,355（1,301–1,372） | 1,374（1,354–1,375） |
| 压力步（出口 TCP 旁每 0.2 秒一条新 echo 流），echo 回复 | 767（724–788），100% | **1,685（1,656–1,721），100%** |
| 本阶段 `Exit queue full` 告警（次） | 12（每次） | **0（每次）** |

- E 在出口正向时的线程 CPU（`watch/`，6 秒采样）：改前包处理 owner 线程 84.5%、两个 Tokio worker（driver 与出口 I/O owner）各约 53%；改后 owner 线程 93.8%（Mesh 正向时同为 94%，即同一瓶颈），两个 worker 各约 18%。出口正向剩余的差距是每包多出的出口准入与一次包复制。
- 出口反向不受本项影响，E 的出口 TUN TX 丢包前后均为 0。压力步 echo 最小 RTT 中位数 26.2 → 30.2 ms：同一时间出口 TCP 流的速率翻倍，路径上的排队随之增加。

报告：`.meshway/validation/x11/`（`rows/` 每次运行的第 16 阶段 JSON，`extract.py`／`summary.txt` 汇总，`watch/` 每次 iperf 的线程 CPU）。

### 匿名驻留与分配器对照

musl system工具的Manager／Secure OOM复现，节点RSS约495／501 MiB，其中匿名约487／495 MiB；另一个R为519.33 MiB RSS、511.19 MiB匿名并有换出页。iperf匿名不足0.4 MiB，不能解释节点RSS。OrbStack启动参数`cgroup.memory=nokmem,nosocket`关闭部分内核记账，kernel/sock/slab零值不代表实际无占用；记账边界见[内存](/v2/node/dataplane#进程与容器内存记账)。

仅TUN复用不足以消除Manager OOM；mimalloc工具加TUN复用的65+1秒独立窗口：4 CPU Manager UDP118,763 pps／RSS32.74 MiB，Secure direct154,441／23.77 MiB；2 CPU Manager TCP1320.18 Mbps／34.70 MiB。全部OOM/max/Swap为0，匿名页平台化。关闭allocator统计的复验为Manager122,410 pps／RSS33.82 MiB、cgroup45.83 MiB；稳定不依赖读取统计。此为单次内存回归，不代替完整矩阵。生产选择仅Linux musl daemon入口，库不装全局allocator；原生C分配不自动替换。

### 二进制体积（ARM64 实测）

报告 `20261007T192625Z`，Apple M3 Pro／macOS 27.0，Rust 1.98.0、Zig 0.17.0、cargo-zigbuild 0.23.4。基线使用 `60cab2bb` 的冻结源码归档，结果使用 `2a58dc9f` 的 ring HTTP 装配和上述 release profile；两种 target 均在该主机构建。最终 host 产物另按冻结后的源码重建核对，SHA-256 与测量快照一致。

| Target | 程序 | 基线 MiB | 结果 MiB | 缩小 |
| --- | --- | ---: | ---: | ---: |
| `aarch64-apple-darwin` | `meshwayd` | 15.724 | 6.722 | 57.25% |
| `aarch64-apple-darwin` | `meshway` | 3.245 | 1.725 | 46.84% |
| `aarch64-unknown-linux-musl` | `meshwayd` | 23.426 | 12.264 | 47.65% |
| `aarch64-unknown-linux-musl` | `meshway` | 2.703 | 1.544 | 42.90% |

两个程序共同构建的首次命令墙钟：host **396.9 → 343.3 秒**，Linux **396.8 → 364.0 秒**。各阶段使用独立的 Cargo target 缓存，复用已安装工具链、registry 和 Zig 缓存；宿主并非独占，时间包含当时调度／资源竞争。`cargo-bloat` 因 strip 覆盖触发的重构建独立记录，未混入这两行时间。

host／Linux 的 `.text` 分别为 **8,485,968 → 4,888,396 B**、**17,477,828 → 10,774,972 B**。以下为 Linux 的 crate 归属，KiB／占 `.text`；LTO 会改变内联、单态化与符号归属，需连同总大小阅读。

| 归属 | 基线 KiB／% | 结果 KiB／% |
| --- | ---: | ---: |
| aws-lc-sys | 490.9／2.88 | 0／0 |
| ring | 145.6／0.85 | 131.6／1.25 |
| reqwest＋hyper 系列 | 522.6／3.06 | 186.5／1.77 |
| Hickory 系列 | 678.7／3.98 | 426.1／4.05 |
| zbus | 838.4／4.91 | 310.5／2.95 |
| Tokio | 752.0／4.41 | 479.8／4.56 |
| serde_json | 292.3／1.71 | 534.7／5.08 |
| meshway-control-client | 41.6／0.24 | 64.0／0.61 |
| meshway-protocol，共享 DTO | 108.5／0.64 | 214.6／2.04 |

正常依赖图和 lockfile 中的 aws-lc-rs／aws-lc-sys 均已移除。生成操作按源码方法名匹配 `runtime::Client` 的可见符号，Linux 为 20,600 → 27,196 B；含共享 DTO 名称的可见单态化为 786,468 → 500,076 B。它们与上表重叠，生成类型的独占成本未能独立分离。Server 的 reqwest JSON 支持限测试，SSE 使用的 stream 保留在 control-client；Hickory、zbus 和 Tokio 的已使用能力保留。完整两 target 的 crate 前 25、函数前 30、全部符号 JSON、SHA-256、源摘要与命令位于 `.meshway/perf/binary-size/20261007T192625Z/`。

### Release profile 的 L0 对照

同一 Apple M3 Pro／macOS 27.0、Rust 1.98.0，基线 `60cab2bb` 与结果 `2a58dc9f` 均运行 `mise run perf:circuit` 默认 36 格矩阵，全部成功，源码摘要首尾一致。下表取 profile 关闭、每格三个独立进程的中位数；CPU 为 harness 线程 CPU／实际单向交付 IP 包，包含所有路径角色。L0 的正式数据面与测量源码相同，此处比较 release profile。

| 节点 | IP 字节 | 基线 → fat LTO pps | 基线 → fat LTO CPU µs／包 |
| ---: | ---: | ---: | ---: |
| 2 | 52 | 203,136 → 217,362 | 4.921 → 4.600 |
| 2 | 1280 | 119,683 → 124,215 | 8.338 → 8.048 |
| 3 | 52 | 144,692 → 151,280 | 6.911 → 6.606 |
| 3 | 1280 | 94,589 → 89,202 | 10.572 → 11.205 |
| 5 | 52 | 91,177 → 96,032 | 10.966 → 10.413 |
| 5 | 1280 | 56,331 → 59,041 | 17.722 → 16.930 |

3 节点／1280B 短窗口出现约 6% CPU 上升，因此对同一对不可变二进制追加 **9 组交错顺序、100,000 轮、1000 轮 warmup、无插桩**的配对复测。基线／fat LTO 的 CPU 中位数为 **10.935／10.505 µs／包**；逐组 CPU 比值的中位数为 **0.9674**，8/9 组下降，未复现短窗口回退。其余默认格 CPU 中位数下降约 3.5%–6.5%。保留 fat LTO；这些结果只覆盖 L0，宿主调度／频率未固定。

完整报告为同一体积证据目录中的 `baseline-l0/`、`final-l0/`、`l0-paired-3n1280/`。两程序 SHA-256 分别为 `4ed1dd399fdbbb1fd8fef233a17c030e1dd76837140494e65c1d61bf502dce7f` 与 `7677bb88eb512484fbdeb2828f80459e098f4a62164052f81590e9081fab9441`；逐次样本和 p10／p90 原样保留。

## 已知限制

- L4/L3 ≥0.9已在上述batch32配对达到；两batch完整矩阵仍有严格UDP账本不一致、readiness `Context`及system TCP超时，根因未全部解决。满载低延迟和relay无损100k pps未验收，500M输入不能证明更高容量。P/Y/Planck的OOM证据保留，allocator长窗口不将历史失败改为成功。
- quota不是绑核或独占；历史宿主load曾达8–17，VM uptime epoch变化也未隔离，不能把跨轮差值全部归于实现。当前表保留当时版本，最新x86／L5及全部承载／DNS组合需独立重验。
- UDP namespace计数含iperf与warmup，不等于外层socket丢包；GRO counter也不一一对应内层包。P relay/plain外层socket drops为0时应用socket仍可溢出。CPU／吞吐／采样区间不同，不能直接相除作精确单包成本。
- 动态ABI、连接池与全局相同排队预算、多流干扰、延迟／丢包／乱序／断连、排队取消／承载切换尚无完整L1性能对照。新增协议仍需原生同栈参照。
- 精确syscall/包、分配/复制计数、连续内存峰值、物理频率／steal与内核全局CPU未测；缺项保留null。D3内核到内核全链路零复制未完成，剩余位置见[单缓冲所有权](/v2/node/dataplane#单缓冲所有权与就地加密)。

## 历史对照

仅保留里程碑和全部既有报告ID；逐轮分析、二进制hash、命令与失败原文在对应 `.meshway/perf/{circuit,crypto,dataplane,l5}/<run>/` 及Git。报告ID大小写按原文保留；范围端点不代表中间报告被删除。

<details>
<summary>历史里程碑与证据索引</summary>

- v4 L0 guard优化：2节点1280B 21,314→37,222→46,126 pps，3节点12,792→22,965→27,215；提交`8f7d0c46`／`36ca2117`／`90bc1aec`。v5 inline两节点53,343→57,190：`20261007T000738Z-f0d352b7`、`20261007T011142Z-68ec103e`；同步复测46,722：`20261007T024211Z-21ee1281`。
- 旧密码归因2／3节点20.989／32.339 µs、HMAC占36.89%／48.10%：`20261007T041204Z-ce95fe75-l0-host`；Linux默认／CPU0：`20261007T041746Z-e9770af4-l0-linux`、`20261007T042914Z-29702971-l0-linux`。原语：`20261007T041129Z-22f1f285`、`20261007T042800Z-e67a4e49`、`20261007T042418Z-fde40664`；ring大包AEAD与硬件SHA优于旧soft后端，未证明改wire必要。
- ring＋预key HMAC两节点113,796 pps：`20261007T050103Z-be5131d5`；新预算8.912／12.063 µs、密码约34%：`20261007T050435Z-5d4b300c-l0-host`。owner前后：`20261007T052308Z-a6f2a00a-l0-host`、`20261007T055628Z-75769819-l0-host`；完整矩阵`20261007T062320Z-48ad2bef`，L0最终`20261007T080533Z-7b11af1b`。
- L2初始：`20261006t222145z-cb007812`；内核批量`20261007t002750z-1171958b`。v4真实L3 direct245→314、relay185→213 Mbps：`20261006t232019z-cedb8714`、`20261006t232708z-3d779bfe`；同环境WG：`20261006t234736z-dfa88ba3`。
- H单worker L3 direct312.65→480.50、relay237.56→467.80 Mbps：`20261007t001557z-44250076`、`20261007t013659z-d3a3ee2c`（58/60）；Host安装前失败`20261007t011939z-3ae6552b`（44/60）。I两／四核：`20261007t022912z-b3d48a09`、`20261007t023624z-224b2f3f`；失链证据`i-health-baseline-1791338937`确认旧carrier视图；解析／UDP失败`20261007t021741z-0b97f862`。
- M新后端L3 direct771.01 Mbps／115,975 pps、relay623.28／86,356：`20261007t045428z-fb7362ea`；分阶段采样`20261007t050813z-b1ce80b9`。
- O多流／核数／inline八报告范围`20261007t051734z-4307336a`至`20261007t053026z-0200744d`；墙钟`20261007t053152z-bac25c11`、`20261007t060407z-be091915`；buffer对照`20261007t061008z-a0f6db21`（23/24）。高负载两／四核`20261007t061708z-72652c1e`、`20261007t061912z-03ba3d5f`；采样`20261007t062112z-6577db6f`；OOM／重跑`20261007t055034z-5020f03e`、`20261007t055307z-ae3ede86`。
- O验证后两／四核direct1530.35／1083.72 Mbps：`20261007t064600z-c4127ba8`、`20261007t064755z-170866c9`；packet／runtime证据`20261007T063254Z-cc0cccab`、`20261007T063546Z-96479efe`。
- P worker quick范围`20261007t074329z-397fd216`至`20261007t074527z-e4778389`；L0交接`20261007T080857Z-e671b22c`至`20261007T080905Z-c3ca6a5d`；buffer quick`20261007t074633z-295115be`至`20261007t074726z-d6536890`。正式两／四核`20261007t082302z-a2aff6a7`（10/12）、`20261007t082458z-6c901bcd`（12/12）；45秒`20261007t081406z-3c7a3514`；L4 OOM`20261007t074912z-93fccd5f`、静态malloc探针OOM`20261007t083121z-e2b2daf5`；packet`20261007T082003Z-a1ff1005`。
- Y旧L4 relay411.12 Mbps：`20261007t103344z-3281e8e2`；500M／2G正式`20261007t112743z-6f3b2103`、`20261007t113249z-4f1fbe81`（各23/24）；前后等待`20261007t105907z-45f2114a`、`20261007t113645z-ea3b934c`。
- 匿名驻留OOM：`20261007t114803z-ecc32829`、`20261007t115323z-c3a556d2`、`20261007t115938z-1ad9d6ec`；TUN复用未消除Manager OOM：`20261007t120135z-c468fa67`；mimalloc对照`20261007t115612z-a39f29af`；最终65秒UDP／TCP`20261007t120528z-4e6dbc7a`、`20261007t120850z-b929c046`；readiness失败`20261007t120831z-c24ecf26`；profile0统计关闭复验`20261007t121817z-371ba8a5`。
- Planck旧后端：crypto `planck-20261007T051345Z-51ab43c8`、L0 `planck-20261007T050400Z-6ba8cc86`、2 CPU `planck-20261007t051440z-9e6608ff`（direct1967 Mbps）、4 CPU `planck-20261007t051807z-677968b0`（8/12含OOM）。新`5a30247d`：L0 `planck-20261007T070124Z-c7b84c03`、2 CPU `planck-20261007t070435z-a3f07ece`、4 CPU `planck-20261007t070907z-4a6a085e`（direct2339 Mbps）、WG校准`planck-20261007t071046z-078a1483`。AVX2／SHA-NI经断点核实，sha2升级在x86没有ARM旧soft路径那样的增益。
- L5 `20261007T085207Z-main-4c4e231f`：TCP277／330 Mbps，旧v4同路径49 Mbps；早于P部署。原始私有分析另位于crypto assessment、`m-validation`／`o-validation`／`p-validation`目录，不以构建或测试数量替代业务验收。
- PT基线`20261007t194733z-3512cfe3`（4/4 TCP）；quick队列／驱动对照`20261007t195014z-e08810ee`、`20261007t195140z-b8ef50ff`、`20261007t195349z-3bd58940`、`20261007t195409z-21d2ce07`、`20261007t195437z-a1d41cb4`；readiness失败`20261007t195216z-ee78d209`；中断的门槛包装器诊断`20261007t193208z-bc24b15d`仅保留单次TCP，不作矩阵。
- PT默认两batch完整system矩阵`20261007t200717z-b0529f91`（14/16）；同参数加mimalloc `20261007t203245z-9d0dfd94`（15/16，batch32 TCP比1.0355）、`20261007t203839z-f49921ac`（14/16，readiness／UDP账本失败）；最终独立batch32 `20261007t204126z-cbe7469c`（8/8，0.985×）。同版本fmt／严格workspace Clippy、connectivity／dataplane／daemon测试（4线程）、脚本与文档检查通过；Linux双栈packet证据`20261007T195548Z-860ac766`。保留所有失败，不跨轮拼样本。

</details>
