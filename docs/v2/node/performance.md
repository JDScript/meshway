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

### L0 CPU 归因

`mise run perf:crypto:l0 -- --environment host`（或 `linux`）对正式 `meshway-circuit-perf` 做独立 release（保留函数调试信息）构建，固定 2／3 节点、1280B、1000 轮 warmup，分别保存 profile 关闭和 `MESHWAY_PROFILE=128` 的三次重复。macOS 另用用户安装的 `samply` 记录每拓扑 500,000 轮的 1 kHz 调用栈；Linux 复用锁定 node-lab image、zigbuild、无网络／只读容器和 2 CPU quota，镜像缺少 perf 时使用现有日志插桩。二进制、命令、原始 stdout／stderr、采样文件与 `results.json`／`summary.md` 位于 `.meshway/perf/crypto/<run>-l0-<environment>/`。

- 无插桩的线程 CPU／实际单向交付包是总预算；A／B send、receive 和 R forward 分别使用真实角色计时，不均分总量。
- 原生栈采样的 self-time 是叶函数权重；inclusive 的父子项不可相加。函数内联、未解析符号及样本窗口必须在归因报告说明；初始化／warmup 不混入稳态归因。
- 日志插桩使用首尾累计差值、精确 calls 和采样平均时间，top 15 是已有 scope 的 self CPU 估计。它没有 `Frame::validate`、每次 BTreeMap 查找或 memcpy 的独立计时，不能伪造函数级细分或用 macOS 的占比代替 Linux。
- AEAD、含 key schedule 的相邻 HMAC、codec、其它 SHA256、flow／replay／guard、缓冲／复制与其它分别报告；无法分离的项保留所属父项并标为未独立测得。插桩前后的 CPU 差值用于展示 observer effect，不归一化伪装成精确的生产成本。

`scripts/perf/crypto_analysis.py` 读取两环境 L0、原语 `results.json` 和 samply 的预符号化 sidecar。对每个主线程去除最初 200ms／最后 100ms，以 thread CPU delta 加权；用未插桩总预算乘原生采样比例，得到可加总但仍有统计误差的函数／类别估计。按调用栈将嵌套 codec 和缓冲工作划归自身类别，再识别 HMAC、AEAD、其它 SHA、guard 和 flow／policy；其余（含 harness 计时／fixture、通用分配和未归类包装）保留为其它。JSON 保存所有叶函数和 top 15；Markdown 汇总类别、角色、现有插桩 scope 和每条替换路线的算术差值。

#### CPU 归因实测

2026-10-07 UTC，Apple M3 Pro／macOS 27.0、Rust 1.98.0。报告 `20261007T041204Z-ce95fe75-l0-host` 的两拓扑各 6 个日志对照进程及各一次原生采样均成功；源码 digest 首尾一致。下表以每个单向交付 1280B IP 包计，合计所有路径角色；类别为原生采样比例乘 profile-off 总 CPU。两拓扑分别有 21,815／36,139 个有效 CPU 样本，约 4.0%／4.1% 的叶样本未解析（保留其父级归因）；不是逐调用精确计时。

| 类别 | 2 节点 µs／包（占比） | 3 节点 µs／包（占比） |
| --- | ---: | ---: |
| AEAD seal | 2.691（12.82%） | 2.764（8.55%） |
| AEAD open | 2.817（13.42%） | 2.877（8.90%） |
| 相邻 HMAC，含 key schedule | 7.743（36.89%） | 15.557（48.10%） |
| Frame validate／encode／decode | 0.057（0.27%） | 0.199（0.62%） |
| Packet parse／checksum | 0.158（0.75%） | 0.214（0.66%） |
| BTreeMap／flow／replay／policy | 1.788（8.52%） | 2.664（8.24%） |
| guard／authority | 1.538（7.33%） | 2.205（6.82%） |
| PacketBuffer／池／内存移动 | 1.917（9.13%） | 2.551（7.89%） |
| 其它，含 harness／通用分配／未归类包装 | 2.279（10.86%） | 3.308（10.23%） |
| **profile-off 总 CPU** | **20.989** | **32.339** |

未采到稳态的独立其它 SHA256 调用，不补零；SHA256 压缩叶函数（`sha2::sha256::compress256`）分别占 36.58%／47.68%，归属 HMAC。其次是 ChaCha `ApplyBlocksCtx`（15.39%／10.19%）和 soft Poly1305 `compute_block`（8.90%／5.85%）。完整 top 15、所有叶函数和复制调用者保存在 `.meshway/perf/crypto/assessment.json`／`.md`。`memmove` 中 `prepare_send_at`＋`commit_receive` 的调用者合计约 0.964／1.456 µs，包含 Rust 聚合值移动，不能等同于 IP payload 复制，也不能直接视为全部可省成本。

Linux 使用 OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`、aarch64、锁定 Ubuntu 镜像 `46773dba…`，每容器 2 CPU quota。镜像没有 perf，使用现有 1/128 插桩。默认报告 `20261007T041746Z-e9770af4-l0-linux` 和固定 guest CPU 0 的补测 `20261007T042914Z-29702971-l0-linux` 使用同一 binary `cd9b0c63…`，各 12/12 成功、源码未变。

| 环境／affinity | 2 节点 profile off／128 µs/包 | 3 节点 profile off／128 µs/包 |
| --- | ---: | ---: |
| macOS，默认 | 20.989／20.769 | 32.339／33.881 |
| Linux，默认 | 23.635／26.436 | 51.157／98.560 |
| Linux，guest CPU 0 | 61.755／65.265 | 95.544／92.996 |

固定 guest CPU 仍未消除环境差异；未测宿主频率、vCPU 调度／steal 的贡献，不把这种变化归因于代码。固定 CPU 0 的采样估计中，2／3 节点 AEAD seal 为 7.304／6.929 µs、open 为 7.632／7.310 µs、HMAC 为 22.494／45.272 µs；guard 为 9.113／12.478 µs，codec 为 0.966／1.861 µs。它们含 observer effect，不能当作 profile-off 的可加总分区。Linux 的 BTreeMap、replay、Frame validate、池与复制未独立计时，保留在父 scope；函数映射及 top 15 scope 见同一 assessment。

### 原语微基准

```sh
mise run perf:crypto
# 仅主机，或调整固定迭代／批次重复数：
mise run perf:crypto -- --environment host --iterations 100000 --repeats 5
```

`tools/meshway-crypto-bench` 的显式 example 使用工具自身的 dev-dependencies；运行基准不会切换生产后端，实际选择见[就地加密](/v2/node/dataplane#单缓冲所有权与就地加密)。`scripts/perf/crypto.py` 在 host 与 aarch64 Linux 的无网络、只读、去除 capabilities 的独立容器中运行同一固定迭代实现，复用 dataplane 的 zigbuild／锁定镜像及跨 worktree measurement lock。基准不进入 `mise run check`；严格 Clippy 仍检查该 example。

两个 crypto 测量任务均接受 `--cpu-set 0`，可选固定 Linux guest CPU；CPU quota 仍由 `--cpus`（L0 固定 2）控制。这不固定物理宿主核心，也不提供宿主独占；报告保留 affinity、首尾宿主 load average 与全部批次分位数。跨环境和大幅波动的样本不以单一倍率归因于实现。

| 项目 | 测量边界 |
| --- | --- |
| AEAD | ChaCha20-Poly1305 0.10.1／0.11.0 与 ring 0.17；每次创建 key object，seal／open 分别测量；in-place 复用分配，alloc 包含输入 clone／分配／释放；open.inplace 包含恢复密文复制 |
| HMAC | Hmac／SHA256 0.12／0.10 与 0.13／0.11、ring HMAC-SHA256；每次重新 key 与预 key clone／context 分别测量 |
| 候选相邻认证 | BLAKE3 keyed hash 截 16B；三个 AEAD 实现对空明文、frame AAD、`zero32 || sequence LE64` nonce 生成 16B tag；只测原语，不代表 wire 已选择或协议已验收 |
| 长度 | 原始 52／1280B；另以 `.v5` 标识相同 IP 长度：HMAC／BLAKE3 为完整 MAC transcript（69B domain／字段＋`IP bytes + 16` ciphertext），HMAC 保留正式实现的六次 update 边界；AEAD-tag 为真实 54B Data header＋ciphertext，计入 sequence 字段更新，nonce 与该 sequence 一致 |
| 时钟与样本 | 每批固定迭代，用线程 CPU 与单调墙钟；计时外校验跨实现 ciphertext／tag／plaintext 一致，预热 1000 次；报告独立批次中位数／p10／p90，不解释为逐操作尾延迟 |
| 可比性 | copy baseline 单独列出，不自动扣除；seal／tag 在每个测量流递增 nonce，open 重复验证固定公开测试密文；不用于真实秘密 |

原语窗口不含 Frame 校验、policy、重放或 tag 比较；HMAC 各字段与 BLAKE3 连续 transcript 在窗口外准备，AEAD-tag 的现成 wire AAD 只在每次调用更新 sequence。候选构造在生产中可能需要的 transcript 布局调整不由这些数字代替。

报告目录 `.meshway/perf/crypto/<run>/` 包含不可变 host／Linux 二进制、原始输出、`results.json` 和 `summary.md`。JSON 记录 schemaVersion、Git revision／dirty、源码 digest、工具链／RUSTFLAGS、平台／镜像／quota、二进制 SHA256、命令、逐批原始 CPU／wall／iterations 和聚合值；失败记录保留原因，未测不填零。x86_64 无本机环境时明确未测。Lock 变化随实现提交核对，不由新版本数字推断性能。

本工具锁定的 aarch64 后端：chacha20 0.9.1 只有显式 `chacha20_force_neon` 才选择 NEON，0.10.2 在 `target_feature=neon` 时默认选择 NEON；Poly1305 0.8.0／0.9.1 均没有 aarch64 NEON 后端。sha2 0.10.9 在 aarch64 需要 `asm` feature，0.11.0 默认通过 `cpufeatures` 检测 SHA2 后选择 `aarch64_sha2`，否则 soft；Apple ARM64 的 SHA2 检查恒为 true，Linux 使用 `getauxval(AT_HWCAP)`。本工具不强制全局 target CPU／backend cfg，0.11 AEAD 显式启用 zeroize，保留 key 清零成本。依赖中的平台实现可含审计过的 native／unsafe，本工具自身 `forbid(unsafe_code)`。

路线比较只使用同环境实测差值：每个单向交付包有一次 seal、一次 open、`2 × (节点数 − 1)` 次相邻 MAC。端点各承担一次 AEAD 与一次 MAC，中继承担两次 MAC。仅替换 AEAD／HMAC 实现、预 key HMAC、改变相邻 MAC 算法分别计算；微基准差值是被替换原语预算的算术估计，不是改后 L0／真实 I/O 的已验收吞吐。guard、flow、replay、nonce 与输出边界不因原语加速而省略。

非密码成本的解释边界：当前 Data 路径已就地编解码，不调用握手用 `frame.base()`，不能再把旧 v4 的 body 复制／重编码当作现行待消除项。代码路径中源和终点各调用两次 `Frame::validate`，每个中继四次；重复结构校验仍是候选，不表示整项 CPU 均可删除。现行 owner 已共享只读 Context，并通过字段借用避免稳态 circuit remove／insert；业务 flow 路径仍有独立的所有权与 clone，见[owner 诊断](#owner-路径与-tcp-拥塞诊断)。PacketPool 的加锁／归还、脏区清零、真实边界 guard 及 flow／replay 最终提交分别保留语义。L0 还包括每包 fixture 构造、角色计时和交付账本；减少这些 harness 成本不会自动加速生产 SecureNode。

#### 原语实测与路线预算

主机采用 `20261007T041129Z-22f1f285` 的成功 host 样本（5 批）；该次 Linux 工具激活失败原样保留。Linux 采用固定 guest CPU 0／2 CPU quota 的完整补测 `20261007T042800Z-e67a4e49`（7 批）。每批 100,000 次，Rust 基准源码相同；脚本和 binary digest 分别记录。Linux 原默认 affinity 的完整报告 `20261007T042418Z-fde40664` 也保留，不跨轮择优拼接。下表是所选两组的 CPU 中位数，µs/op；AEAD 为 in-place（open 含恢复密文复制），MAC／候选 tag 的 `.v5` 行按实际 IP 长度对应的 transcript／wire AAD 计。

| 原语 | macOS 52B | macOS 1280B | Linux 52B | Linux 1280B |
| --- | ---: | ---: | ---: | ---: |
| AEAD 0.10.1 seal／open | 0.261／0.271 | 2.524／2.609 | 0.295／0.294 | 2.754／2.781 |
| AEAD 0.11.0 seal／open | 0.493／0.469 | 2.085／2.127 | 0.508／0.512 | 2.184／2.281 |
| ring AEAD seal／open | 0.215／0.222 | 0.917／0.888 | 0.238／0.240 | 0.978／0.937 |
| HMAC SHA2 0.10，new／clone | 0.893／0.589 | 3.572／3.350 | 0.939／0.624 | 4.028／3.499 |
| HMAC SHA2 0.11，new／clone | 0.174／0.125 | 0.693／0.641 | 0.180／0.129 | 0.699／0.641 |
| ring HMAC，new／prekey | 0.208／0.157 | 0.658／0.607 | 0.275／0.230 | 0.750／0.720 |
| BLAKE3 keyed tag | 0.190 | 1.395 | 0.195 | 1.407 |
| 空明文 AEAD-tag，0.10／0.11／ring | 0.229／0.333／0.231 | 1.114／1.201／0.530 | 0.250／0.376／0.246 | 1.194／1.303／0.571 |

原始长度的 HMAC／SHA256、alloc seal／open、copy baseline 和各行 p10／p90 均保存在完整报告。Linux 固定 guest CPU 后仍有大幅批次波动：1280B 旧 HMAC new 的 p10–p90 为 3.816–7.481 µs，ring seal 为 0.938–3.282 µs；不能把中位数称为稳定的物理 CPU 上限。x86_64 与 aws-lc-rs 未测。

以下只对同环境原语中位数做加减，AEAD 均选择 ring，正数为每个单向 1280B 包的**原语预算节省**。MAC 选择中的“新”指 HMAC 0.13／SHA2 0.11；不是已更换生产实现的 L0 数字。

| 路线 | macOS 2／3 节点节省 µs | Linux 2／3 节点节省 µs |
| --- | ---: | ---: |
| (i) AEAD 切换＋新 HMAC，每次 new | 9.085／14.842 | 10.277／16.935 |
| (ii) AEAD 切换＋旧 HMAC 预 key | 3.771／4.213 | 4.677／5.735 |
| (ii) AEAD 切换＋新 HMAC 预 key | 9.189／15.049 | 10.394／17.168 |
| (iii) AEAD 切换＋相邻 ring AEAD-tag | 9.411／15.494 | 10.533／17.446 |
| (iii) AEAD 切换＋相邻 BLAKE3 | 7.681／12.034 | 8.862／14.103 |

例如路线 (i) 在 macOS 的源／终点分别省 4.485／4.600 µs，每个中继省 5.757 µs；Linux 对应原语加减为 5.105／5.173／6.657 µs。macOS 52B 两节点的路线 (i)／新预 key (ii)／AEAD-tag (iii) 分别省 1.535／1.632／1.421 µs，Linux 为 1.629／1.730／1.496 µs。所有路线含同 repeat 加减的分位数，但这些不是置信区间。尤其 Linux L0 与原语采集期间执行条件不一致，不用原语差值推算其替换后 L0 总 CPU。

这组实测首先支持无 wire 变化的密码后端替换：单纯预 key 旧 software SHA256 的收益远小于硬件 SHA256；仅升级 AEAD 0.11 不能获得 ring 的大包成本，52B 还更慢。相邻 AEAD-tag 相比新预 key HMAC 在 macOS 两节点 1280B 仅多省 0.222 µs，52B 则多花 0.212 µs；BLAKE3 大包也高于硬件 HMAC。算法／wire 改变仍需独立决定与实现验收，当前没有由这些微基准得出的必要性。

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

### L0 密码后端替换

`20261007T050103Z-be5131d5` 为 ring 0.17 AEAD＋预 key HMAC 0.13／SHA2 0.11 的默认 36 格矩阵，全部成功、源码 digest 首尾一致；实现提交 `b4d6410b`。平台仍为 Apple M3 Pro／macOS 27.0、Rust 1.98.0，单线程 inline、每格三次重复中位数、profile 关闭。H 基线为 `20261007T011142Z-68ec103e`，main 基线为 `20261007T024211Z-21ee1281`，不跨轮选取最好样本。

| 节点 | IP 字节 | pps H → main → 新后端 | CPU µs／包 H → main → 新后端 |
| ---: | ---: | ---: | ---: |
| 2 | 1280 | 57,190 → 46,722 → 113,796 | 17.479 → 19.862 → 8.786 |
| 3 | 1280 | 37,987 → 32,378 → 80,179 | 26.308 → 29.814 → 12.468 |
| 5 | 1280 | 23,402 → 20,965 → 52,439 | 42.698 → 47.399 → 19.064 |
| 2 | 52 | 143,431 → 125,873 → 177,586 | 6.972 → 7.761 → 5.628 |
| 3 | 52 | 92,414 → 85,786 → 123,957 | 10.784 → 11.602 → 8.059 |
| 5 | 52 | 55,639 → 49,491 → 79,033 | 17.939 → 19.987 → 12.651 |

2／3 节点 1280B 的 A send／receive 为 3.678／4.272、3.784／4.451 µs，B 为 3.503／4.175、3.596／4.373 µs；三节点 R 为 3.177 µs／包。profile 的精确 guard 调用仍为 15／21／33 次／单向交付包，包含所有路径角色及极少维护；公开操作的五次真实检查、nonce／replay／flow／输出 fence 未改。每跳 wire 字节仍为 1366／138。宿主首尾 load average（1／5／15 分钟）为 5.30／5.39／6.37 → 3.84／4.49／5.77，未固定物理核心或频率，不把跨轮全部差值视为纯代码收益。binary SHA-256 为 `c2b0b2e5f73f312f1afee3373cd907a3454dbcafa6c2370d7de2cc361a2b41a6`。

#### 新后端剩余 CPU 预算

独立归因报告 `20261007T050435Z-5d4b300c-l0-host` 使用 `perf:crypto:l0 -- --environment host`：每拓扑三次 profile off／128 对照与一次 500,000 轮原生栈采样，全部成功。下表为 1280B、每单向交付包、所有路径角色合计；按原生 self-time 权重乘同次未插桩总 CPU，方法与[CPU 归因](#l0-cpu-归因)相同。有效样本 8,972／11,984，未解析叶样本 6.9%／7.1% 保留父级归因；函数内联和短窗口仍有统计误差。

| 类别 | 2 节点 µs／包 | 3 节点 µs／包 |
| --- | ---: | ---: |
| ring AEAD seal＋open | 1.719 | 1.619 |
| 预 key 相邻 HMAC | 1.274 | 2.469 |
| PacketBuffer／池／内存移动 | 1.502 | 2.134 |
| flow／BTreeMap／replay／policy | 1.062 | 1.366 |
| guard／authority | 1.181 | 1.614 |
| Packet parse／checksum | 0.369 | 0.435 |
| Frame validate／codec | 0.020 | 0.090 |
| 其它，含 harness 计时／fixture／通用分配 | 1.785 | 2.337 |
| **未插桩总 CPU** | **8.912** | **12.063** |
| **非密码合计** | **5.918** | **7.976** |

密码部分约占 33.6%／33.9%。`prepare_send_at`＋`commit_receive` 的 memmove 调用者合计 0.749／1.175 µs，属于上表内存移动，不再另加；它包含聚合值移动，不是全部可删除的 payload copy。没有采到独立稳态其它 SHA256，不补零。profile=128 总 CPU 为 9.458／12.479 µs；宿主一分钟 load 范围 3.28–5.38。原生采样实际出现 ring seal／open 叶函数与硬件 SHA2 的 NEON 内联路径；完整 top 15、角色与复制调用者存于 `.meshway/perf/m-validation/budget.json`。这是 macOS L0 的分区，不作为 Linux 调度／I/O CPU 的替代。

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
- TUN 拆合实现归固定版本 tun-rs：单次 read 后使用 `VirtioNetHdr`／`gso_split`，写入前使用 `GROTable::apply_gro` 复用其 wireguard-go 合并规则。TCP／UDP 的 IPv4／IPv6 拆分重建长度与校验和；TCP 调整连续序号，FIN／PSH 仅保留在末段。Meshway 检查 flags、实际 IP／transport header、GSO 类型、offset、MTU 和分段数量，不信任 kernel 的 `hdr_len`；CHECKSUM_PARTIAL 使用 tun-rs 的 checksum 原语。接收槽按真实分段大小分配、复用，最多 1,024 段；已拆分包直接移交 `PacketBatch`，剩余分段由设备状态持有。保留 UDP 零 checksum 编码为 `0xffff`、非首段清 CWR 并更新 checksum 的边界处理；未协商 TSO_ECN，不声明 ECN TSO 支持。
- GRO 要求同地址族、源／目标地址与端口；TCP 还要求 ACK 值相同、TCP options 长度与内容一致、序号相接，标志只能是 ACK 或 ACK+PSH。IP ToS／Traffic Class、TTL／Hop Limit 及 IPv4 DF／保留位必须相容。输入 IP 长度与实际缓冲长度一致，并在合并前校验传输层 checksum。IP 分片、IPv4 options、IPv6 扩展头、TCP SYN／FIN／RST／URG／ECE／CWR、纯 ACK 无 payload 不合并；PSH 和短尾段只能位于聚合末尾。UDP 仅向同流的最后一组合并，短尾后不再追加，且依赖实际 USO 能力。
- tun-rs 允许 TCP 按序号向前拼接，允许跨输入中的其他流寻找同流；不要求 IPv4 ID 连续、IPv6 flow label 或 TCP window 相同。Meshway 额外保留 IPv4 header checksum、UDP length 和 TCP reserved／NS 位的严格合并准入检查，并对候选输入前缀重新求 GRO，直到首个聚合恰好覆盖整个前缀，保证一次调用只写这一组且返回真实合并包数；不会将非连续输入索引误报为已写前缀。该返回值可用于逐批报告，不能由 capability 推算合并数。
- `send_multiple` 会遍历多个输出且只返回字节总数，因此适配层使用 `apply_gro` 后的一次异步写入。发送缓冲预留 `VIRTIO_NET_HDR_LEN` headroom，并遵守 `ExpandBuffer` 的容量约定。公开发送接口借用不可变包，需复制到可变 staging；tun-rs 内部 GRO 也有候选包复制，GSO 拆分需写入独立包缓冲，不声明零复制。混合流前缀收缩时需要重新准备候选；常规完整可合并前缀只求 GRO 一次。

OrbStack `7.0.14-orbstack-00380-ga7e0a2dc9535`、aarch64 的独立无网络容器验证结果：`IFF_VNET_HDR`、`CSUM`、`TSO4`、`TSO6`、`USO4`、`USO6` 均成功启用。真实内核 TCP IPv4／IPv6 各发送 8,192 字节，观察到至少 5 个规范化分段，逐段序号、长度与校验和通过；双栈 TCP 三包合并写入后应用完整收到 3,000 字节，IPv4／IPv6 UDP 三包合并写入均由内核恢复为三个应用报文。对应入口为 `meshway-tun` 的显式隔离测试 `offload_capabilities_and_normalized_packet_io`；原逐包设备生命周期／双栈 I/O 测试同时通过。L2 吞吐与 CPU 对照见[内核批量实测](#l2-内核批量实测)。

## 运行

需要本机锁定的 Rust／Zig 工具链，以及支持 `/dev/net/tun` 和内核 WireGuard 的 Linux Docker engine。脚本使用普通 Docker bridge 上两个或三个一次性容器的独立 network namespace；按模式只授予所需容器 `NET_ADMIN`，没有 host network、privileged 或宿主路由／DNS 修改。

```sh
mise run perf:dataplane

mise run perf:dataplane --batch 1 32 128 --profile 0 128 \
  --repeats 3 --seconds 15 --omit 3 --mtu 1400 --payload 1200 \
  --udp-rate 500M --cpus 2 --memory 512m
```

每个配置分别运行 TCP（默认单流，`--tcp-streams N` 可指定 1–128 条并行流）和 UDP 定速率 iperf3，至少两次重复；重复之间反转配置顺序。UDP 始终为一条流，TCP 并行数保存于报告 options 与真实 iperf 命令。默认每次先省略 2 秒 warmup，再测量 8 秒。release 交叉构建缓存独立于其他任务，位于 `target/dataplane-perf`；测量使用报告目录中不可变的二进制副本。Docker userspace 使用锁定 node-lab OS 源码对应的 immutable image，若尚无该 image，则在本工作区私有输出目录构建；不部署或修改常驻 lab。

`--modes` 可选择本页列出的模式，默认仍为全部。相同 Meshway 模式／拓扑复用容器；同一 batch／profile／repeat 的 TCP 和 UDP 连续使用同一节点进程与 circuit，只重启 iperf3。跨重复或 profile 配置重启节点并使用新材料。`--workers` 默认按 CPU quota 取 `max(ceil(cpus)-1, 1)`，可显式指定，0 为 inline。

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

iperf3 结果取接收端 `server_output_json.end.sum_received`，并核对 UDP 实收字节数等于交付报文数乘以固定 payload。不能用 iperf3 3.16 的 UDP `end.sum` 替代该字段；该字段可能在真实收到数据时仍报告零字节。Profiling 原始输出为累计快照：复用进程时，TCP 结束请求显式 checkpoint，UDP 结束停止并 join 节点后取得 final snapshot；报告中的 UDP 累积 calls／时间取两者之差，不重复累计 TCP。`max_wall_ns` 仍是进程累计最大值，不解释为 UDP 区间最大值。原始连续日志保存在 `*-nodes/`，各样本另保存其结束边界的副本；收到最终快照即退出，无固定五秒静默等待。

`nodeThreads` 使用每节点 Linux task CPU 计数的前后差值，按自己的快照墙钟区间计算 `cpuActivePercent`；worker 的值可相加。其余墙钟比例同时含阻塞、可运行但未获 CPU、quota 限流，不能称为精确的线程休眠占比。该口径与包含 iperf／内核记账的 cgroup CPU 分开。每样本另记录宿主首尾 load average；配额不提供宿主独占。

### 流水线墙钟观察

profile 开启时，`pipeline.submit_to_ready.wall`、`pipeline.ready_to_release.wall` 与 `pipeline.release_to_credit_drop.wall` 分别观察提交后至 crypto 完成、完成后至 owner 取出保序前缀、取出后至最终 TX/TUN credit 释放。按调用线程每 128 次抽取一次，跨线程只传递单调时刻，不推断 CPU；最后一项含 owner commit、批内等待及 I/O，错误／取消也可能释放 credit，须核对同次拒绝计数。`l3.input_head_to_prepare.wall` 只观察各输入批首包，不能称为全部包的平均排队。

`pipeline.submit_to_worker.wall` 与 `pipeline.worker.wall` 将提交→完成进一步分成调度／排队和 crypto 工作墙钟；两者嵌套于 `submit_to_ready`。`l3.tun.read.batch`、`l3.udp.read.batch`、`l3.prepare.batch` 记录各入口实际批次。`l3.backpressure` 记录未消费输入时的 credit／quantum 等待次数，不能当作丢包；`l3.reject` 记录真实准入拒绝类别。

`carrier.release_to_admission.wall` 与 `carrier.admission_to_completion.wall` 为抽样批次的 release→Session 准入、准入→本机完成；`transport.admission_to_first_poll.wall` 与 `transport.first_poll_to_write.wall` 为 Session 内部准入→首次 poll、首次 poll→首个成功 raw write 前缀，后者包含 readiness 等待、原 guard 与 syscall。`carrier.tun.write.wall` 包含一次成功 TUN 写入及其 readiness 等待。这些批次指标与 per-packet credit 指标互相嵌套，不直接相加，也不是内核 syscall-only 精确计时。

`pipeline.dispatch.batch`、`pipeline.release.batch`、`carrier.send.batch` 与 `carrier.tun.written.batch` 为实际非空批次的累计次数，桶为 1／2–4／5–8／9–16／17–32；TUN 使用真实返回的已写前缀数。TCP checkpoint 与 UDP final 之间取差值，避免复用进程时重复累计。关闭 profile 不读取这些墙钟或分配采样记录。

`networkCounters` 保存每容器 `/proc/net/snmp`、`/proc/net/dev` 的客户端前后值及差值，并保留 `ss -u -a -n -m` 的实际 socket buffer／排队与 `/proc/net/udp{,6}` 的逐 socket drops。它们覆盖 warmup 与边界采集，不等同于 iperf 排除 warmup 的区间；GRO socket 丢弃计数也不能按一个 counter 等同一个内层 IP 包。`tcpStreams` 原样保存每条 iperf sender 的 RTT、重传与 cwnd 字段；cwnd 最大值不是平均在途窗口，不能直接除以平均 RTT 作为吞吐上限。

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

每个节点保留一个 current-thread owner runtime，crypto 使用正式 `pipeline::Pool`。工具的 `--workers 0` 使用 inline；非零值配置有界线程数。harness 默认按 CPU quota 取核数减一、最少一；单节点二进制默认按可用核数减一、最少一。TUN／UDP 使用批量接口和实际 offload 能力；就绪输入经过 owner 准备、worker 计算、保序 owner 提交，再按 channel 批次发送／交付。SecureNode 使用默认 limits；L3 Session 帧数／操作数预算与 plain 相同，字节容量额外容纳 secure envelope 和握手。

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

### 流式完成与核数对照

两核报告 `20261007t022912z-b3d48a09` 为 40/40 成功，四核报告 `20261007t023624z-224b2f3f` 为 20/20 成功。两次使用同一 Linux binary SHA-256 `93d5e0073e22ebfe16d776ccc7c7f99760d966548daac3aeacf4e145947acc72`，源码 digest `5e0d36c1823654c9f2c8bae191100a4b1b13e477b61fd9463c6b9df3db331271`；运行前后源码未变。代码基于 `2fcc2b10`，包含流式完成队列、32 个全生命周期 credit 和控制消息预留，不包含随后独立的 TUN codec／UDP provider 改动。

```sh
mise run perf:dataplane -- --modes secure-direct secure-relay manager-direct manager-relay plain-batch \
  --batch 32 --profile 0 128 --repeats 2 --seconds 6 --omit 1 \
  --mtu 1280 --payload 1200 --udp-rate 2G --cpus 2 --memory 512m
# 同一二进制，另一次完整运行将 --cpus 改为 4、--profile 改为 0。
```

每容器两核时为一个 owner＋一个 worker，四核时为一个 owner＋三个 worker。其余环境与上一节相同；正式值为 profile 关闭、两次重复中位数。前值统一取 `20261007t013659z-d3a3ee2c`。CPU 为所有角色 cgroup 相加，100%＝一核，包含 iperf 和 warmup。

| 模式 | TCP Mbps 前 → 两核 → 四核 | UDP pps 前 → 两核 → 四核 | 两核 TCP／UDP CPU % | 四核 TCP／UDP CPU % |
| --- | ---: | ---: | ---: | ---: |
| L3 direct | 480.50 → 558.21 → 582.98 | 52,427 → 48,662 → 74,388 | 184.62／319.31 | 197.88／413.95 |
| L3 relay | 467.80 → 402.41 → 518.72 | 54,220 → 43,373 → 62,611 | 218.46／402.07 | 273.27／511.87 |
| L4 direct | 439.23 → 452.61 → 506.30 | 49,183 → 65,292 → 86,523 | 169.24／334.37 | 190.61／463.15 |
| L4 relay | 353.00 → 323.00 → 337.07 | 41,257* → 38,246 → 58,013 | 205.98／397.59 | 233.75／524.10 |
| L2 plain batch | — → 3,592.15 → 3,672.20 | — → 110,879 → 103,458 | 156.49／192.41 | 159.16／195.16 |

*前值只有一个成功 UDP 样本。* 两核／四核 L3 direct UDP 丢包为 75.80%／63.12%，L4 relay 为 81.36%／71.65%；2G 输入不是无损容量测量。**L3 direct 的 1 Gbps TCP／100k pps UDP 底线在两核和四核均未达到。** 四核 L3 direct TCP 仅比两核高 4.4%，增加 worker 并没有解除主要串行／等待限制。两核、四核记录的宿主一分钟 load average 分别为 4.08–8.66、4.88–14.62；同宿主还有其他工作流，CPU quota 并非独占 CPU，不能将跨轮变化全部归于实现。

#### L4 失链定位

使用旧报告的固定 binary 重放，证据为 `.meshway/perf/dataplane/i-health-baseline-1791338937/`。失败样本 `manager-relay-b32-p128-udp-r0` 中，A 于 `02:09:26.730Z` 发生正常的双向拨号选链替换，旧 channel 仍有约 5.71 秒健康期；`02:09:27.128Z` A 因 `Unavailable` 停止。R 的旧邻接到 `02:09:32.737Z` 才健康到期，且该样本仅处理了少量 circuit 控制帧。另一失败 TCP 样本具有相同顺序。

根因是 L4 工具仅在启动时调用 `set_carriers`，后续忽略 Manager 的选链变化，拿旧 carrier 输出；后来的健康到期是节点退出的后果，不能据此归因为高负载 Ping/Pong 饥饿。驱动现在持续取得 Manager 的实际调度视图，新准入使用新选择；旧输出在 carrier 不匹配时计数丢弃，不重新绑定。生产 daemon 原本就更新该视图。

控制接收优先和发送容量预留独立增强了满载边界，见[控制消息预算](/v2/node/connectivity#udp-控制消息预算)。新两核的四个 relay UDP 样本及四核的两个样本均保持进程运行，没有 `Unavailable`；健康期限与有效 Pong 条件未放宽。这些六秒负载样本不替代长期失效／恢复验收。

#### 分阶段成本与忙闲边界

以下取两核、profile=128、UDP 两次的采样，使用精确 calls 加权；µs 为含 observer effect 的 CPU 估计。owner 准备／提交、worker 整体为 inclusive 每次调用；carrier 为 self CPU／实际发送或交付帧，不能与 inclusive 行相加当作总预算。

| 路径／角色 | owner send | owner prepare RX | owner commit RX | worker crypto 整体 | carrier send self／帧 | TUN deliver self／帧 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| L3 direct A | 4.68 | — | — | 7.96 | 1.53 | — |
| L3 direct B | — | 1.78 | 4.73 | 7.70 | — | 3.33 |
| L3 relay R | — | 2.02 | 2.25 | 9.05 | 1.76 | — |
| L4 direct A | 4.58 | — | — | 8.30 | 2.28 | — |
| L4 direct B | — | 1.78 | 4.80 | 7.94 | — | 2.92 |
| L4 relay R | — | 1.99 | 2.40 | 11.90 | 2.32 | — |

L3 direct A 的 worker 内，AEAD 约 2.85 µs、相邻 MAC 约 3.80 µs；B 的解密约 2.86 µs、MAC 验证约 3.92 µs。全负载汇总 TUN deliver self 为 L3 direct 3.12 µs／帧、relay 3.07，未达到约 1 µs／帧目标。当前 carrier 已复用同一批切片列表和同 epoch 的 guard reading，但本次没有改 TUN 内部 codec，亦未取得精确的合并率／syscall 分阶段计数。

profile 关闭的 L3 direct 线程 CPU 活跃比例如下；四核 worker 列为三个 worker **总和**，括号为平均每 worker。余下墙钟时间包含阻塞和调度／quota 等待，不能据此区分具体睡眠原因。

| 核数／负载 | A owner % | A worker % | B owner % | B worker % |
| --- | ---: | ---: | ---: | ---: |
| 2／TCP | 44.1 | 37.9 | 49.8 | 39.4 |
| 2／UDP | 57.1 | 55.2 | 44.5 | 39.5 |
| 4／TCP | 42.3 | 43.0（14.3） | 52.3 | 43.7（14.6） |
| 4／UDP | 69.4 | 78.5（26.2） | 62.1 | 58.8（19.6） |

这组数据表明 crypto 算力未用满。剩余串行点包括端点 owner 准入／提交、较小就绪前缀的 carrier／TUN 成本，以及宿主等待已释放前缀 I/O 的路径。独立有界 TX pending、worker 排队及 ready→release→syscall 的墙钟分解仍未完成；不能声称完整无阻塞流水线已验收，也不能把内核丢包和调度等待按这些 CPU 采样作精确归因。

#### L0 同步路径重测

`20261007T024211Z-21ee1281` 完成默认 36 格矩阵；profile 关闭、每格三次中位数，对照上文 `20261007T011142Z-68ec103e`。L0 不使用线程池或 carrier／TUN，不能证明流式调度收益。

| 节点 | IP 字节 | pps 前 → 后 |
| ---: | ---: | ---: |
| 2 | 1280 | 57,190 → 46,722 |
| 3 | 1280 | 37,987 → 32,378 |
| 5 | 1280 | 23,402 → 20,965 |
| 2 | 52 | 143,431 → 125,873 |
| 3 | 52 | 92,414 → 85,786 |
| 5 | 52 | 55,639 → 49,491 |

这次 L0 数值下降，未隔离宿主负载／CPU 频率的贡献，不将其解释为确定的算法回退或忽略。binary SHA-256 为 `c0ee2b44c2b8f371b6f90482900415c92d6cd9ae3435ab555d1fea346f908647`。此前试跑 `20261007t021741z-0b97f862` 的失败报告保留：包括 profile 差分遇到同名计时项 null 的解析问题，以及两个 UDP 实收字节与包数相差一包的样本；没有补零或替换旧样本。本节采用后续完整成功矩阵。

### L3 密码后端替换

`20261007t045428z-fb7362ea` 使用同一新后端，12/12 成功、源码 digest 首尾一致；正式吞吐取 profile 关闭、两次重复中位数。对照 `20261007t022912z-b3d48a09`，仍为 OrbStack aarch64、锁定 Ubuntu 镜像 `46773dba…`、每容器 2 CPU／512 MiB、每节点一个 owner＋一个 worker、真实 TUN vnet_hdr 与 UDP 批量 I/O。命令：

```sh
mise run perf:dataplane -- --modes secure-direct secure-relay plain-batch \
  --batch 32 --profile 0 --repeats 2 --seconds 6 --omit 1 \
  --mtu 1280 --payload 1200 --udp-rate 2G --cpus 2 --memory 512m
```

| 模式 | TCP Mbps 前 → 后 | UDP pps 前 → 后 | 后 TCP／UDP cgroup CPU % | 后 UDP 丢包 % |
| --- | ---: | ---: | ---: | ---: |
| secure-direct | 558.21 → 771.01 | 48,662 → 115,975 | 138.49／341.46 | 44.32 |
| secure-relay | 402.41 → 623.28 | 43,373 → 86,356 | 173.32／386.07 | 57.85 |
| plain-batch | 3,592.15 → 6,289.31 | 110,879 → 116,502 | 115.54／160.50 | 44.07 |

**直连 UDP 已达到 100k pps 底线，TCP 仍未达到 1 Gbps；中继两项均未达。** 2G 输入下存在显著丢包，这不是无损容量。TCP 的两个直连样本为 777.96／764.07 Mbps。plain 同时明显变化，且基线未包含后来独立的 TUN／UDP 改动，因此相对该基线的 L3 提升不能全部归因于密码后端。CPU 为所有角色 cgroup 之和、包含 iperf 与 warmup；TCP pps 未测。

宿主样本首尾 1／5／15 分钟 load 范围为 5.31–6.19／5.27–5.54／6.40–6.48；共享 measurement lock 保证本仓库测量互斥，不提供宿主独占。源码基于 `d0e60ba9` 加 `b4d6410b` 的后端改动，binary SHA-256 为 `8454abfa0c20dc717b8488bc5af43931e564f803d87e796242cf3ef7c810ffd8`，完整固定二进制与逐次原始样本保存在当前工作树报告目录。后端的一致性由 Snow 双方向／长度／nonce 边界逐字节对照、独立 HMAC 固定 Data／Probe transcript 向量，以及原 session／Linux packet conformance 验证。

#### 新后端逐节点阶段成本

补充采样 `20261007t050813z-b1ce80b9` 仅运行 secure-direct／secure-relay、profile=128，其余参数不变；8/8 成功，与正式吞吐报告为同一 binary。宿主一分钟 load 为 3.04–4.28；profile-on 直连／中继 TCP 中位数为 803.04／639.52 Mbps，UDP 为 124,537／90,526 pps。这次负载条件变化使采样吞吐高于 profile-off，不能解释为 observer 没有开销，也不替换正式数字。

下表为 UDP 两次重复按精确 calls 加权的 CPU 估计。owner／worker 列为每次对应操作的 inclusive µs；AEAD／MAC 列是 worker 的内部项；carrier 为 self CPU／实际发送或交付帧。少量 TCP 控制包的反向路径缺采样标为不适用，不填零。各列互相嵌套、含 observer effect，不能全部相加当作未插桩总成本。

| 路径／角色 | owner send | owner prepare RX | owner commit RX | worker 整体 | worker 内 AEAD／MAC | carrier send self／帧 | TUN deliver self／帧 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| direct A | 4.765 | — | — | 3.226 | 1.177／0.821 | 0.936 | — |
| direct B | — | 1.707 | 4.679 | 2.804 | 1.133／0.871 | — | 1.100 |
| relay A | 4.674 | — | — | 3.160 | 1.169／0.809 | 0.923 | — |
| relay R | — | 1.893 | 2.233 | 2.828 | —／1.667（验证＋生成） | 1.099 | — |
| relay B | — | 1.733 | 4.700 | 2.803 | 1.128／0.873 | — | 1.090 |

相对旧分阶段数据，direct A worker 从 7.96 降至 3.23 µs，B 从 7.70 降至 2.80，R 从 9.05 降至 2.83；端点 owner 准备／提交仍是主要串行工作。新采样每次 guard 为 0.218–0.223 µs；端点 flow.admit 自 CPU 为 0.929–0.952 µs／调用、policy 为 0.184–0.198、IP parse 为 0.216–0.262；Data encode／decode 为 0.163–0.188。worker 自身包装／移动约 0.80–1.06 µs／操作，不能把这项称为纯调度等待。所有逐角色原始 scopes、calls 与采样数见同一 `budget.json`。

正式 profile-off 直连 TCP 的 A owner／worker 活跃比例为 43.9%／16.0%，B 为 49.7%／17.5%；UDP 对应为 75.7%／30.4%、69.1%／27.5%。TCP worker 已显著空闲，继续增加密码算力不足以证明能达到 1 Gbps。剩余工作包括 owner flow／replay／guard、聚合值移动／context clone、pool／就绪前缀与 TUN／carrier I/O；ready→release→syscall 的墙钟排队、内核丢包与宿主调度尚无独立精确归因，不能从线程未活跃比例推断具体等待原因。

同一 Linux image 的独立无网络只读容器读取 `AT_HWCAP=0x2fb3ffff`，SHA2 与 ASIMD bits 均为 true；锁定 sha2 0.11／cpufeatures 0.3 的 aarch64 Linux `getauxval` 分派选择硬件 SHA2，未设置强制 backend／target-cpu。原始探测命令与结果保存于 `.meshway/perf/m-validation/hwcap.json`。本轮没有重跑原语工具，以正式 L0／L3 和实际 HWCAP 验证后端。

### Owner 路径与 TCP 拥塞诊断

诊断与优化使用 `perf/owner-path` 的同一 L3 harness，基线为 `7eac33a6`／密码后端 M，MTU 1280、batch 32、UDP payload 1200／2G、warmup 1 秒＋测量 6 秒，每格两次。测量互斥锁覆盖全部 Docker 运行；主仓库并行检查及宿主其它负载不受该锁约束，以下保留环境差异，不跨轮选取最好样本。

#### 单流、多流、核数与 inline

矩阵为 `20261007t051734z-4307336a` 至 `20261007t053026z-0200744d` 的八个独立报告，32/32 TCP／UDP 样本成功。下表 TCP 为 profile-off 中位数；RTT 为 iperf 各流 `mean_rtt` 的中位数，CPU 为节点线程自己的活跃比例。宿主一分钟 load 为 4.62–7.86。

| CPU quota | worker | TCP 流数 | Mbps | RTT µs | A owner／worker % | B owner／worker % |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1 | 1 | 777.71 | 1830 | 46.1／16.9 | 51.5／18.5 |
| 2 | 1 | 4 | 771.04 | 1926 | 49.0／17.1 | 54.0／17.5 |
| 2 | 0 | 1 | 779.78 | 1754 | 47.1／0 | 48.6／0 |
| 2 | 0 | 4 | 675.66 | 2359 | 47.8／0 | 52.1／0 |
| 4 | 1 | 1 | 711.62 | 1850 | 45.0／16.3 | 50.2／18.2 |
| 4 | 1 | 4 | 726.15 | 2282 | 52.5／18.5 | 57.8／19.7 |
| 4 | 0 | 1 | 717.39 | 1876 | 47.1／0 | 49.1／0 |
| 4 | 0 | 4 | 741.39 | 2269 | 48.9／0 | 53.1／0 |

多流共享同一 owner、circuit 和全局 32-credit 窗口，因此多流没有突破 1 Gbps **不能单独证明 owner CPU 饱和**。TCP owner 只使用约半核，inline 和增加 quota 均没有解除限制；后续 socket 对照确认主要限制是突发 UDP 接收溢出引起 TCP 拥塞窗口收缩，叠加流水线／I/O 往返延迟。CPU 串行成本仍限制消除丢包后的满载吞吐。

#### 原生 owner self-time 与剩余预算

`perf:crypto:l0 -- --environment host` 的前／后报告为 `20261007T052308Z-a6f2a00a-l0-host`／`20261007T055628Z-75769819-l0-host`。每拓扑三次 profile-off／128 对照与一次 500,000 轮 samply，均成功。后者 2／3 节点有效样本为 8,173／11,437，未解析叶为 569／847。归因器优先识别已解析的 ring AEAD 汇编叶，避免优化后的 DWARF buffer 父帧把 AEAD 误归入内存工作；两侧使用同一规则重算。

下表为每个单向 1280B 交付包的全路径 CPU µs，原生 self-time 权重乘同轮未插桩总 CPU，仍含采样／内联归因误差。

| 类别 | 2 节点前 → 后 | 3 节点前 → 后 |
| --- | ---: | ---: |
| AEAD | 1.736 → 1.665 | 1.557 → 1.561 |
| 相邻 HMAC | 1.262 → 1.291 | 2.455 → 2.446 |
| 缓冲／内存移动 | 1.604 → 1.057 | 2.128 → 1.189 |
| flow／map／replay／policy | 0.917 → 1.314 | 1.802 → 1.478 |
| guard／authority | 1.151 → 1.207 | 1.661 → 1.711 |
| Packet parse／checksum | 0.371 → 0.069 | 0.052 → 0.412 |
| Frame／codec | 0.057 → 0.012 | 0.117 → 0.027 |
| 其它，含 harness／分配 | 1.787 → 1.620 | 2.322 → 2.340 |
| **总 CPU** | **8.885 → 8.235** | **12.094 → 11.164** |
| **非密码** | **5.886 → 5.279** | **8.081 → 7.157** |

`prepare_send_at`＋`commit_receive` 的 memmove 调用者合计从 0.779／1.110 降至 0.164／0.186 µs，不另加到上表。Packet／flow 等类别的明显权重转移受内联归因影响，不能称为某个函数精确加速倍率。后测 **owner 路径 top 15 self-time** 如下；过滤条件为调用栈含 prepare-send／prepare-receive／commit-receive，单位仍是全路径每交付包，不是 inclusive CPU。

| 叶函数（泛型名缩写） | 2 节点 µs／包 |
| --- | ---: |
| `mach_absolute_time` | 0.381 |
| `_platform_memmove` | 0.344 |
| `pthread_mutex_unlock` | 0.128 |
| `FlowTable::prepare` | 0.109 |
| `pthread_mutex_lock` | 0.086 |
| `profile::Scope::drop` | 0.053 |
| `flow::Tuple::cmp` | 0.050 |
| circuit BTreeMap root `Option::as_mut` | 0.049 |
| `admit_circuit_carrier` | 0.048 |
| `slice::Iter<[u8; 2]>::next` | 0.047 |
| `ExecutionSnapshot::clone` | 0.043 |
| `CancellationToken::is_cancelled` | 0.042 |
| flow deadline `OccupiedEntry::remove_kv` | 0.040 |
| `ExecutionSnapshot` drop glue | 0.039 |
| `ExecutionSnapshot::reading` | 0.036 |

实现采用一次稳定 Circuit 分配、稳态字段借用与共享只读 Context；未替换 BTreeMap 的哈希算法、未省略 Frame 校验或真实 guard。Replay check 为常数时间位查询，推进最多移动固定 16 个 u64；本次没有把它改成另一套窗口。短包 inline 使用 128B 明文阈值：52B ring seal／open 原语各约 0.24 µs、预 key HMAC 约 0.13 µs，远小于下表数十微秒交接；它保留相同 lane 序号、credit、验证和 owner 提交。

默认 L0 最终矩阵 `20261007T062320Z-48ad2bef` 为 36/36 成功、源码未变。2／3／5 节点 1280B 为 119,562／77,894／56,778 pps，CPU 为 8.353／12.446／17.572 µs；52B 为 171,170／140,058／91,297 pps，CPU 为 5.719／7.089／10.940 µs。guard 仍为 15／21／33 次，wire 仍为 86 B。该轮宿主负载高于原生归因轮，不用原生较低 CPU 替换默认矩阵；52B 两节点未表现为稳定加速。

#### 墙钟与批次边界

`20261007t053152z-bac25c11` 的改动前 direct TCP，A／B 的 submit→ready 为 28.3／55.2 µs，ready→取出为 50.7／36.8，取出→credit 释放为 75.1／57.5。批首输入等待为 19.7／7.7 µs，结合 owner 准备成本，A→B 在已观察的 userspace 段约 **0.34 ms**；这是两节点角色均值拼接的数量级估计，混有 ACK、未逐包关联，也不含尚在内核或 provider 预取队列中的等待。

`20261007t060407z-be091915` 为 owner 改动后、扩 buffer 前的细分，profile=128、TCP 两次累计样本加权平均，单位 µs。worker 列是纯计算路径的墙钟（含 observer），ready→取出包含保序与调度，credit 列含 commit／批内等待／输出 I/O。子列不能重复相加。

| 角色 | 输入批首→prepare | submit→worker | worker | ready→取出 | 取出→credit 释放 | Session 首 poll→首成功 write | TUN write |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| direct A | 33.4 | 41.2 | 1.7 | 41.5 | 65.2 | 36.3 | 1.8 |
| direct B | 12.8 | 39.4 | 1.7 | 22.5 | 51.8 | 22.1 | 7.3 |
| relay R | 19.4 | 53.4 | 1.4 | 37.2 | 61.9 | 22.2 | — |

Session 准入→首次 poll 只有约 0.08–0.19 µs；没有观测到这里存在毫秒级队列。源／终点 owner 边界、worker 交接和较小输出前缀仍有可优化空间，但以上用户态区间不足以精确归因相对 plain 的全部约 1 ms RTT 增量。UDP provider 一次接收可预取多份 GRO、随后每次交付至多 32 帧，TUN GSO 剩余分段亦有自身有界存储；其剩余驻留、内核排队和 guest 调度未获得逐包关联的时钟，不把缺失区间填成零或全称作 worker 休眠。

实际非空批次分布（同一采样、两次合计；桶依次为 **1／2–4／5–8／9–16／17–32**）：

| 负载／角色／边界 | 各桶批次数 |
| --- | --- |
| TCP A TUN read | 161／547／1,061／2,455／38,824 |
| TCP A owner prepare | 2,992／9,818／14,158／10,078／38,786 |
| TCP A worker dispatch | 1,469／5,263／154,656／0／0 |
| TCP A ready release | 6,175／16,955／27,375／57,923／16,993 |
| TCP A carrier send | 3,976／10,002／22,605／51,791／15,808 |
| TCP B UDP read | 699／2,214／8,491／23,617／30,030 |
| TCP B owner prepare | 28,209／26,146／15,536／29,710／29,217 |
| TCP B ready release | 39,663／42,724／76,394／41,614／7,403 |
| TCP B TUN 已写前缀 | 21,991／43,651／83,177／35,273／4,042 |
| UDP A TUN read | 40／80／218／255／65,997 |
| UDP A owner prepare | 2,003／6,211／16,399／12,749／61,090 |
| UDP A ready release | 7,820／26,161／49,512／97,181／21,901 |
| UDP B UDP read | 735／1,654／5,498／13,655／51,288 |
| UDP B owner prepare | 3,371／8,042／17,268／28,857／45,241 |
| UDP B ready release | 18,348／41,869／96,169／65,785／6,494 |
| UDP B TUN 已写前缀 | 18,345／41,861／96,168／65,784／6,494 |

TUN read 多为大批，与输入侧 TSO 超包拆分一致；不能将 17–32 桶当成等待凑齐 32 包的证据。输出侧 worker 小组与 ready 前缀会形成更多小批次；实现只发送已就绪前缀，没有 GSO 聚合等待计时器。TCP ACK 的无 payload TUN 写入仍逐包，不能通过把它计为 GRO 大包掩盖 syscall。

#### 丢包定位与 socket buffer 对照

扩 buffer 前，实际 UDP socket `rb/tb` 均为 229,376 B。profile-off direct TCP 两次 B 的 `RcvbufErrors=InErrors` 为 **66／70**，A 为 0；relay 两次 R 为 **34／32**、B 为 **16／10**。各角色 TUN errors/dropped 均为 0，`SndbufErrors`／checksum errors 也为 0。`/proc/net/udp` 的 :41000 drops 对应这些溢出，不能把 GRO 丢弃计数直接等同于内层 TCP 重传数。采样 TCP 的端点 `l3.reject.packet` 还出现 4–154 次；credit／quantum 计数表示输入保留的背压，不是满队列主动丢包。

依据该证据，Linux batch UDP 对默认 buffer 采用普通 **7 MiB SO_RCVBUF／SO_SNDBUF** 请求；参照 wireguard-go `conn/controlfns.go` 的 7 MiB 目标，但不使用它的 BUFFORCE 路径。实际 Linux `ss` 返回 rb/tb **14,680,064 B**；显式非零配置、原 descriptor、mark 和 guard 保持各自约束。行为定义见[UDP 批量接口](/v2/node/transport#udp-批量接口)。

同机紧邻对照的 TCP 字段如下，n=2、profile-off。RTT／cwnd 为每次原始值；cwnd 单位 B。后测 `20261007t061008z-a0f6db21` 的 TCP 全部成功，但一次 direct UDP 违反严格接收字节／报文数核对，整体 23/24，失败原样保留，不以其 UDP 作为完整矩阵结论。

| 路径 | buffer 前 → 后 Mbps | 前 → 后 RTT µs | 前 → 后 retransmits | 前 → 后 max snd cwnd B |
| --- | ---: | --- | --- | --- |
| direct | 939.84 → 1633.46 | 1343／1273 → 12994／22554 | 443／488 → 0／0 | 214900／191568 → 4305368／4280808 |
| relay | 726.16 → 1464.09 | 1407／1925 → 18008／19793 | 303／340 → 0／0 | 201392／201392 → 4285720／4369224 |

后测 TCP 的所有角色 UDP 溢出、TUN 丢包及 profile-on 端点拒绝均为 0；吞吐变化支持“接收突发丢包→cwnd 受限”的归因。**扩大 buffer 并没有降低满载 RTT**：cwnd 增长到约 4.3 MB 后，持续无 pacing 的 TCP 形成更深内核排队，RTT 达 13–23 ms。超过 1 Gbps 的吞吐样本不能同时宣传成低延迟改进。2G UDP 仍超过真实服务能力，源 TUN 与接收 socket 均可丢包，不是无损容量。

后续完全相同二进制 `4f681377…` 的两核／四核完整复测为 `20261007t061708z-72652c1e`／`20261007t061912z-03ba3d5f`，均 12/12 成功；CPU 为所有角色 cgroup 合计。两轮宿主一分钟 load 分别约 11–13／12–17，明显高于前述扩 buffer 对照的 5–8。

| 模式 | 两核 TCP Mbps／UDP pps | 四核 TCP Mbps／UDP pps | 两核 TCP／UDP CPU % | 四核 TCP／UDP CPU % |
| --- | ---: | ---: | ---: | ---: |
| secure-direct | 939.26／70,146 | 1240.19／81,558 | 252.24／333.63 | 271.22／350.88 |
| secure-relay | 791.55／55,012 | 820.59／59,326 | 300.83／367.89 | 285.28／362.48 |
| plain-batch | 6006.69／116,893 | 7820.20／129,498 | 197.53／200.32 | 199.79／184.65 |

因此 1 Gbps 单流已在有效两核样本和四核完整复测中出现，但**两核底线尚不能在这组高共享负载复测下稳定声明通过**；100k pps 也不能用早期低负载值代替本轮未达值。`20261007t062112z-6577db6f` 的同 binary profile-on 完整 12 格另行保留，不替换正式吞吐。较早四核 `20261007t055034z-5020f03e` 有一次 relay UDP 退出 137，节点没有最终 stderr；完整重跑 `20261007t055307z-ae3ede86` 通过，但首次异常的内部停止位置未独立定位，不能宣称根因已修复。所有原始失败、JSON、线程与 kernel counters 保存在对应报告和 `.meshway/perf/o-validation/`。

#### 验证结束后的同参数复现

本工作树的构建／行为验证全部退出后，按预定顺序再运行一次完整两核／四核矩阵，分别为 `20261007t064600z-c4127ba8`／`20261007t064755z-170866c9`，各 12/12 成功。binary 仍为 `4f6813779c1c5ea612aefd316d8ee47ec1ab5a02efb2c7e755c95c19f97f86f2`；源码 digest `28530772c54a8fadca3c581abfde0097642e933f63b6c82305f210620a3eae81` 的变化仅来自归因脚本。宿主一分钟 load 为约 8–10／9–14，仍不是独占环境。下表对照用户指定的 M 报告 `20261007t045428z-fb7362ea`，全部为 profile-off、两次中位数。

| 模式 | TCP Mbps：M → 两核 → 四核 | UDP pps：M → 两核 → 四核 | 两核 TCP／UDP CPU % | 四核 TCP／UDP CPU % |
| --- | ---: | ---: | ---: | ---: |
| secure-direct | 771.01 → 1530.35 → 1083.72 | 115,975 → 101,958 → 87,769 | 296.86／358.03 | 250.72／349.04 |
| secure-relay | 623.28 → 1245.58 → 1080.88 | 86,356 → 72,570 → 73,821 | 351.95／388.91 | 331.14／414.20 |
| plain-batch | 6289.31 → 8862.87 → 7033.54 | 116,502 → 124,933 → 118,135 | 200.82／181.54 | 201.39／197.22 |

**两核 direct 两次均达到 1 Gbps／100k pps 底线**：TCP 1541.28／1519.42 Mbps，UDP 103,316／100,601 pps；relay TCP 1175.39／1315.78 Mbps，UDP 未达 100k。direct TCP 的 RTT 为 17,045／16,618 µs，重传 0／0，cwnd 最大值 4,321,332／4,274,668 B；relay 为 20,939／19,317 µs、重传 0／0。TCP 所有角色的 UDP 溢出为 0。UDP 2G 仍丢包，direct 两核丢包中位数 50.70%，不声明无损 100k 容量或 UDP 相对 M 加速。

四核后半程再次明显变慢：direct TCP 两次 1477.80／689.64 Mbps，RTT 17,973／47,105 µs，重传仍为 0／0；relay 1455.44／706.32 Mbps。其 TCP 中位数虽超过 1 Gbps，但不是每次均达标，UDP 也未达 100k。核数与吞吐变化混有共享调度／频率／负载，不能据此断言增加 worker 会降速，也不能隐去前述高负载未达样本。

当前实现已通过 fmt／workspace all-targets all-features 严格 Clippy，dataplane／connectivity／daemon／transport／underlay 组合测试（4 test threads），独立 Linux UDP 批量／buffer 测试、`test:packet:linux` 和 `test:runtime:linux -- --transport native-udp --dns-backend off`。packet 证据为 `20261007T063254Z-cc0cccab`，真实 daemon/CLI runtime 为 `20261007T063546Z-96479efe`。这些检查证明原 owner／guard／路径与生命周期回归，不替代其它承载或 DNS 组合验收。剩余性能边界为满载内核排队、用户态预取队列的逐包驻留、独立 TX pending，以及共享负载下的吞吐稳定性。

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

### x86_64 同口径参照

同一 harness、同参数在原生 x86_64 Linux 主机（AMD Ryzen 9 9950X，kernel 6.8，Docker 29.8，`x86_64-unknown-linux-musl`）上的结果；源码 `d0e60ba9`（ring／硬件 SHA2 后端替换之前）加 harness 修复 `335015a1`／`b40fc85d`。主机为多用户共享机，各窗口 1 分钟 load 2.3–8.6，不是独占环境；aarch64 对照列来自前述 OrbStack VM 报告，期间源码不同，跨架构差值不能全部归因于 CPU。

| 口径 | aarch64 VM | x86_64 主机 |
| --- | ---: | ---: |
| 原语 1280 B：AEAD 0.10.1 seal／ring AEAD seal | 2.524／0.917 µs | 1.639／0.488 µs |
| 原语 1280 B：HMAC sha2 0.10 预 key／sha2 0.11 预 key | 3.350／0.641 µs | 0.609／0.610 µs |
| L0 2／3／5 节点 1280 B（inline，profile off） | 46,722／32,378／20,965 pps | 99,178／75,209／49,947 pps |
| L3 secure-direct 2 CPU：TCP／UDP | 558 Mbps／48,662 pps | 1,967 Mbps／159,426 pps |
| L3 secure-relay 2 CPU：TCP／UDP | 402 Mbps／43,373 pps | 1,836 Mbps／158,570 pps |
| plain-batch 2 CPU：TCP／UDP | 3,592 Mbps／110,879 pps | 17,224 Mbps／207,543 pps |
| 内核 WireGuard direct／relay 2 CPU：TCP | 2,742／4,854 Mbps | 4,525／4,249 Mbps |

x86_64 的 RustCrypto 后端经调试器断点核实为 AVX2（ChaCha／Poly1305）与 SHA-NI（sha2 0.10／0.11），ring 为 AVX2 与硬件 SHA；因此 x86 上 sha2 升级无增益，ring AEAD 仍约 3× 于 RustCrypto。UDP 行在 x86 上接近 2G 输入速率上限（raw 207,799 pps），丢包 22–24%，不是无损容量。4 CPU 配额矩阵 8/12 成功：secure UDP 接收端在 512 MiB 下 3 次退出 137（其中 1 次有 Docker OOM 事件），plain-batch TCP 1 次发送未完成；根因未定，失败样本完整保留。报告：crypto `planck-20261007T051345Z-51ab43c8`、L0 `planck-20261007T050400Z-6ba8cc86`、L3 2 CPU `planck-20261007t051440z-9e6608ff`、4 CPU `planck-20261007t051807z-677968b0`（本地 `.meshway/perf/<kind>/`）。

#### x86_64 复测：`5a30247d`

同一主机、同参数复测 main `5a30247d`（ring AEAD／硬件 HMAC、流式释放、7 MiB UDP socket 缓冲、Circuit 稳定分配）。各窗口 1 分钟 load 1.5–4.8；72/72 样本成功。L0 2／3／5 节点 1280 B：140,454／98,093／61,793 pps（7.12／10.19／16.18 µs/包）。

| 2 CPU／容器 | TCP Mbps | UDP pps | CPU% TCP／UDP | TCP 平均 RTT | 重传 | 最大 cwnd |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| secure-direct | 2,292 | 199,927 | 267／336 | 4.3 ms | 786 | 2.98 MB |
| secure-relay | 2,127 | 195,072 | 392／451 | 2.9 ms | 322 | 1.17 MB |
| plain-batch | 17,222 | 208,224 | 208／162 | 0.20 ms | 16,103 | 0.51 MB |
| 内核 WireGuard direct | 4,780 | 208,108 | 64／165 | 1.0 ms | 0 | 2.21 MB |

4 CPU／容器（3 worker）：secure-direct 2,339 Mbps／208,231 pps，secure-relay 2,197／208,222——TCP 只比 2 CPU 高 2–3%，UDP 到达 2G 输入上限 208,333 pps、丢包 0.05%。较 `d0e60ba9`：secure-direct TCP +16.5%、UDP +25.4%；secure-relay +15.9%／+23.0%。UDP 命名空间 `Udp.RcvbufErrors` 在 secure 样本为 0–858（含 warmup），plain／WireGuard 更高。

WireGuard 校准（`--cpus 8 --mtu 1420 --udp-rate 4G`）：wg-direct TCP 5,117 Mbps、UDP 414k pps；raw 107.8 Gbps。对照 [wg-bench](https://github.com/cyyself/wg-bench) 的 7950X 5.64 Gbps 低 9%；仍有容器配额与 Docker bridge，不是无配额 netns 复现。内核 WireGuard 的 softirq CPU 可能记在 cgroup 之外。报告：L0 `planck-20261007T070124Z-c7b84c03`、2 CPU `planck-20261007t070435z-a3f07ece`、4 CPU `planck-20261007t070907z-4a6a085e`、校准 `planck-20261007t071046z-078a1483`。

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
