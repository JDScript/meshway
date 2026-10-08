# 验收范围

本页是 Node 隔离验收范围的唯一定义位置。其他页面只描述行为与限制，并引用这里的状态。入口命令、参数与报告位置见[开发服务、测试与环境](/v2/development#隔离-linux-验收与镜像)。

## 状态含义

| 状态 | 含义 |
| --- | --- |
| 当前 wire 已通过 | 使用当前数据面 wire（版本见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)）完成整组验收 |
| 历史通过未重验 | 曾在先前 wire 或先前实现上通过整组验收；结论不自动扩展到当前 wire |
| 未验收 | 没有整组通过证据；不能声明已支持 |
| 未通过 | 当前实现已运行该组，但至少一个必需检查失败；保留原始失败报告 |

库测试、单独内核测试、镜像构建成功或常驻 lab 的部分功能检查都不替代对应行的整组结论。

## 验收矩阵

可执行行 ID 来自 `scripts/validate_matrix.py` 的 `ROWS`：`tun`、`packet`、`udp/off`、`tcp/off`、`udp/resolv-conf`、`udp/resolved`、`nat/off`、`nat+fallback/off`、`nat/resolved`，顺序对应下表九个隔离入口。常驻 lab 不属于一次性矩阵。统一运行、并发预算与远程证据位置见[矩阵入口](/v2/development#validation-matrix)。

| 入口 | 承载 | DNS | NAT | 覆盖行为 | 状态 |
| --- | --- | --- | --- | --- | --- |
| `test:tun:linux` | — | — | — | 真实 TUN 设备创建与双栈包 I/O | 当前 wire 已通过；`20261007T212324Z-904a137e` |
| `test:packet:linux` | Native TCP | off | — | A→R→B 双栈小包／满 MTU UDP、MTU+1 拒绝、daemon 多 MSS TCP、R 只转发受保护帧、原路径回复 | 当前 wire 已通过；`20261007T212330Z-f7e9a2e9` |
| `test:runtime:linux` | Native UDP 静态端点 | off | — | daemon 整组：下文[共同场景](#runtime-共同场景) | 当前 wire 已通过；Planck `20261008T033613Z-abbd4545` |
| `test:runtime:linux` | Native TCP | off | — | daemon 整组：共同场景 | 当前 wire 已通过；Planck `20261008T033656Z-7a35b80e` |
| `test:runtime:linux` | Native UDP 静态端点 | resolv-conf | — | 共同场景＋[DNS 场景](#dns-场景) | 当前 wire 已通过（两次）；Planck `20261008T032749Z-3ba0ca12`、`20261008T050153Z-a1a8eebf` |
| `test:runtime:linux` | Native UDP 静态端点 | resolved | — | 共同场景＋DNS 场景 | V1 串行：当前 wire 已通过，`20261007T220446Z-5360cb6e`；Planck 旧候选未通过，本轮未重验，见[剩余边界](#remaining-boundaries) |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋[穿透场景](#穿透场景) | 当前 wire 已通过（两次）；Planck `20261008T032749Z-289b9780`、`20261008T050153Z-3e001994` |
| `test:runtime:linux --traversal nat --traversal-fallback` | Native UDP＋A–R Native TCP 退路 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋穿透场景＋同路径 TCP 退路 | 当前 wire 已通过（两次）；Planck `20261008T032749Z-4299bfde`、`20261008T050153Z-d30caa20` |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | resolved | 双 IPv4 端口保持型 SNAT | 整组组合 | 当前 wire 已通过（两次）；Planck `20261008T032749Z-d13bd2a9`、`20261008T050153Z-1978b05b` |
| [常驻三节点 lab](/v2/node/docker-lab) | Native UDP 显式端点 | resolved | — | 六向双栈系统解析／echo、路径诊断、关闭插桩的同路径 TCP 吞吐对照 | 未验收（当前整组）；2026-10-07 已有下述 L5 局部证据；共享 bridge 不构成 NAT 证明 |

以下范围未验收：外层 IPv6 NAT、端点相关映射、映射轮换、NAT 与 resolv-conf 的组合、VLESS／Trojan 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换、出口转发／NAT、完整 TCP conntrack、动态 MTU／路径 MTU 发现及分片重组、整机重启与系统 resolver 服务停机间隙、userspace／SOCKS5。

常驻 lab 的 L5 原始证据目录为 `.meshway/perf/l5/20261007T162418Z-main-f39ba2b6-2cpu/`。该目录的两次 TCP 接收吞吐分别约 318／364 Mbps；协调者另报告空闲宿主复测 1,367 Mbps，独立报告路径待补，不能把该数值归入前述目录或用吞吐局部证据替代整组验收。

### 当前验证证据

`validate:matrix` 为每行独立保留报告并汇总 `matrix.json`／`summary.md`；通过结论同时要求 fixture `passed`、进程成功退出和本 run 无遗留资源。`--short` 的报告标记 `scope=diagnostic-short`：只运行 30 秒持续／纯接收窗口（退路 5 秒后触发），不要求已经跨原需求期限，未测到的延长为 null。其他场景仍执行，包括真实 instance 失效恢复；短模式不是整组验收，不能更新下述历史／当前通过状态。并发运行还需读取各行启动 load，不能与低负载串行结果不加区分地合并。

当前六个 runtime 行的 V2 证据使用候选 `f909e875`（基于 main `a52fb267`），机器为 Planck x86_64、目标 `x86_64-unknown-linux-musl`。四个原失败行均在两个独立完整 run 中通过；静态 UDP/off 与 TCP/off 另完成回归检查。两轮均为 `scope=acceptance`、125 秒持续／纯接收窗口，未使用短模式。入口记录 `uptime` 和 load，不等待负载门槛。

十份 runtime 报告的构建／导入／结束源码指纹均为 `a7b5bd38d268c763badd737a52b3363467f46b7967f9bea5db9d447bd7c6ab34`，二进制 hash 集合一致。所有必需检查通过，Controller／A／R／B 成功退出；每行精确清理与远程 supervisor 的独立容器／网络复核均无遗留。通过结论包含真实 STUN State、完整路径诊断、实际包流、回执、撤销恢复和旧输出拒绝，不能仅由 saved/applied 或候选非空推导。

TUN、packet 和表中 V1 的静态 UDP/resolved 证据来自 `3887256b` 生产源码与 `36a20fa3` 满 MTU fixture，使用 macOS/OrbStack ARM64 串行执行。其 packet/runtime 指纹为 `052e82e75417c17457c5e40951e2d1098147e2be56d3156f89a5cf465eb653c6`，TUN 专属输入指纹为 `477af3a17bc873c2346314c1dc27145230821df287fb91b22b6995932ce7b066`。汇总位于 `.meshway/validation/v1/20261007-main-3887256b/`；原始通过及失败 verdict 均保留。

### 远程矩阵证据 {#remote-matrix-evidence}

| V2 run | 并发 | 完整行 | 结果 |
| --- | ---: | --- | --- |
| `20261008T032743Z-897a549e` | 4 | NAT/off、NAT＋TCP 退路/off、NAT/resolved、UDP/resolv-conf、UDP/off、TCP/off | 六行全部通过；总耗时 1,044.5 秒 |
| `20261008T050147Z-0c5809cf` | 4 | 前四行独立复跑 | 四行全部通过；总耗时 566.6 秒 |

远程原件位于执行 clone 的 `.meshway/validation/matrix/<run>/`；本地原样归档位于仓库 `.meshway/validation/v2/20261008-nat-regressions/planck/<run>/`。每行路径为 `rows/<row>/<报告 ID>/report.json`，含 JSON、pcap、进程／清理信息与源码指纹；总报告保留 `matrix.json`、`summary.md`、`remote.json`。入口及进度说明见[开发环境](/v2/development#validation-matrix)。

### 剩余边界 {#remaining-boundaries}

本轮四个目标行没有剩余必需检查失败。**Planck 的静态 UDP/resolved 单独组合未在 V2 重验**：旧候选矩阵 `20261007T234146Z-c299a3b8` 与串行复跑 `20261008T002050Z-1d853e58`／行报告 `20261008T002332Z-de9c1b24` 曾在系统解析、ping 首样本或会合检查失败，不能由本轮 NAT/resolved 的通过替代。其他未验收范围见矩阵下方列表。

V1 及中间候选的 STUN 候选为空、完整路径握手／状态发布不收敛等失败证据保留在各自原始报告和 V2 私有归档中；它们不被改写为通过。短模式只证明其工具和诊断行为，仍不计入本表完整验收。

## 入口

### TUN

```sh
mise run test:tun:linux
```

宿主交叉编译，容器仅运行预编译测试；使用固定 digest 镜像、独立 network namespace，只授予 TUN device 与 `NET_ADMIN`。报告位置见 [development](/v2/development#数据、报告与回收)。

### 受保护包流

```sh
mise run test:packet:linux
```

宿主编译 `meshway-packet-fixture`，使用 Server、A、R、B 四个独立容器／network namespace 和一个 internal bridge。Server 角色运行真实 Router／HTTPS／生成管理客户端；各 Node 独立持钥接入、取得执行快照，经正式 Native TCP provider 连接。Node 不持有管理 token 或 Server TLS 私钥。

双栈 UDP 同时覆盖小包与恰好填满默认内层 MTU 1366 B 的包：IPv4 payload 为 1338 B，IPv6 为 1318 B。相同 socket 再发送 MTU+1：默认内核 PMTU 策略返回 `EMSGSIZE`，或由内核生成、被正式包解析器拒绝的分片不得到达 echo 应用；随后再次验证满 MTU echo，确认拒绝不会停止 owner。各地址族的实际拒绝方式记录于 `mtu-conformance` 日志。此用例不声明分片重组或动态路径 MTU 发现可用。

入口还运行一组 Native TCP／DNS off 的真实 daemon conformance，复用下节的生命周期 driver，并在 `daemonReport` 引用独立报告。真实 daemon 上重复双栈 UDP 边界，并用同一 TCP socket 验证双栈 16 KiB（跨多个 MSS）的逐字节 echo。这样既覆盖受保护包流，也覆盖 daemon 按已安装 TUN MTU 分配包池的回归：修复前固定 1280 B 的池无法接纳 1366 B 包，仅运行直接 SecureNode fixture 不能证明该 daemon 边界。报告位置见 [development](/v2/development#数据、报告与回收)。

### daemon 与内核资源

```sh
mise run test:runtime:linux --build-only
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --dns-backend off
```

宿主编译实际 `meshwayd`、`meshway` 和验证 actors。Node 经 CLI 上线并由真实 Server 批准；验证程序使用 OS UDP／TCP socket，包处理由 daemon 的正式运行组件执行。容器使用独立 IPv4／IPv6 internal bridge，不发布宿主端口，不使用 host network 或 privileged。参数与报告位置见 [development](/v2/development#隔离-linux-验收与镜像)。

#### runtime 共同场景

- 关闭系统 DNS 的 Mesh-only 网络，强制 A→R→B 及原路径回复。
- 内层双栈 UDP／TCP，包含上述满 MTU／MTU+1 UDP 和 16 KiB TCP echo；同一 socket 跨需求添加；至少 125 秒持续续租；独立 UDP 流停止发包后的入站活动续租。
- Access 撤销／恢复；中转取消 IPv6 地址后继续转发 IPv6 密文；拒绝路由保留；离线 `down` 清理。
- 控制中断恢复：关闭 Node HTTP 接纳直至实际 instance／execution 到期，再恢复联系；无需再次 `up`，原身份、进程和 operator revision 保持，取得新 instance、重新 applied 并上报 State，以新的 OS socket 恢复原路径包流，过期的旧 TCP／cipher 被拒绝。
- `meshway ping`：hostname／双栈 Mesh IP 选择、direct 与有序 via、独立 ICMP 权限、用户与 Node 共享任务及独立取消、SIGINT、控制中断后的订阅终态、新 instance 下重新探测。
- 源、终点的新交付计数及中转 TUN 的零明文交付分别记录。

单独的内核测试另检查同目的地址的普通／受保护 socket、旧池拒绝边界、journal 恢复、外部同优先级规则冲突，以及 HTTPS bootstrap 在显式 down 前后的联系。

#### DNS 场景

```sh
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp \
  --dns-backend resolved --dns-image-manifest /absolute/private/os/image-manifest.json
```

覆盖真实 `getaddrinfo`、短名称、可见性 REFUSED、分域 UDP／TCP、系统 UDP→TCP 截断重试、进程重启、长时包流、失效阻断及 DNS／接口恢复。DNS 场景在测试 namespace 内使用私有 mount namespace 与私有 D-Bus／resolver，不改写 Docker 的 resolver bind 源；固定 OS 资产见 `dev/dns-fixture/README.md`。

#### 穿透场景

```sh
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --traversal nat \
  --traversal-image-manifest /absolute/private/os/image-manifest.json
```

- 两个独立 IPv4 端口保持型 Linux conntrack SNAT；UDP 发布端点为空，STUN、互探及认证使用实际承载 socket。
- 内层双栈 UDP／TCP 强制沿 A→R→B，持续与纯接收业务跨越至少 125 秒及原需求期限。
- `--traversal-fallback` 仅预授权 A→R 的 Native TCP 拨号，R→B 保持 Native UDP；TCP 入口初始不可达，业务持续 30 秒后开放该入口并阻断精确 A–R UDP 地址对；原 TCP socket、两个内层 TCP circuit 的 CID 与完整路径跨切换保留。
- State、双方接受的固定 offer、双侧抓包、conntrack、防火墙计数和应用回包共同构成证据。拓扑与固定 OS 资产见 `dev/traversal-fixture/README.md`。
