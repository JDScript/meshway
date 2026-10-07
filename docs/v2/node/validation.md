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
| `test:runtime:linux` | Native UDP 静态端点 | off | — | daemon 整组：下文[共同场景](#runtime-共同场景) | 当前 wire 已通过；`20261007T213446Z-7b1b18a3` |
| `test:runtime:linux` | Native TCP | off | — | daemon 整组：共同场景 | 当前 wire 已通过；`20261007T214309Z-a2b9e93f` |
| `test:runtime:linux` | Native UDP 静态端点 | resolv-conf | — | 共同场景＋[DNS 场景](#dns-场景) | 未通过；`20261007T215139Z-6b08959d`、`20261007T215819Z-790fc5f7` |
| `test:runtime:linux` | Native UDP 静态端点 | resolved | — | 共同场景＋DNS 场景 | 当前 wire 已通过；`20261007T220446Z-5360cb6e` |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋[穿透场景](#穿透场景) | 未通过；`20261007T221353Z-bd8c8c29`、`20261007T221747Z-6137d668` |
| `test:runtime:linux --traversal nat --traversal-fallback` | Native UDP＋A–R Native TCP 退路 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋穿透场景＋同路径 TCP 退路 | 未通过；`20261007T222406Z-706619eb`、`20261007T222802Z-896b1580` |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | resolved | 双 IPv4 端口保持型 SNAT | 整组组合 | 未通过；`20261007T223053Z-6d3d9c48`、`20261007T223451Z-5fe61a8d` |
| [常驻三节点 lab](/v2/node/docker-lab) | Native UDP 显式端点 | resolved | — | 六向双栈系统解析／echo、路径诊断、关闭插桩的同路径 TCP 吞吐对照 | 未验收（当前整组）；2026-10-07 已有下述 L5 局部证据；共享 bridge 不构成 NAT 证明 |

以下范围未验收：外层 IPv6 NAT、端点相关映射、映射轮换、NAT 与 resolv-conf 的组合、VLESS／Trojan 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换、出口转发／NAT、完整 TCP conntrack、动态 MTU／路径 MTU 发现及分片重组、整机重启与系统 resolver 服务停机间隙、userspace／SOCKS5。

常驻 lab 的 L5 原始证据目录为 `.meshway/perf/l5/20261007T162418Z-main-f39ba2b6-2cpu/`。该目录的两次 TCP 接收吞吐分别约 318／364 Mbps；协调者另报告空闲宿主复测 1,367 Mbps，独立报告路径待补，不能把该数值归入前述目录或用吞吐局部证据替代整组验收。

### 当前验证证据

`validate:matrix` 为每行独立保留报告并汇总 `matrix.json`／`summary.md`；通过结论同时要求 fixture `passed`、进程成功退出和本 run 无遗留资源。`--short` 的报告标记 `scope=diagnostic-short`：只运行 30 秒持续／纯接收窗口（退路 5 秒后触发），不要求已经跨原需求期限，未测到的延长为 null。其他场景仍执行，包括真实 instance 失效恢复；短模式不是整组验收，不能更新下述历史／当前通过状态。并发运行还需读取各行启动 load，不能与低负载串行结果不加区分地合并。

矩阵报告使用 `3887256b` 的生产源码与 `36a20fa3` 的满 MTU fixture，包含 wire v5、流式流水线、SSE、回执分类、Manager 查询节流／批前缀消费与 ring-only TLS。全部运行严格串行，前一行结束并清理后才启动下一行；失败行保留首跑并独立复跑一次，两次失败才记为未通过。报告记录入口前 `uptime`、1／5／15 分钟 load、耗时、二进制 hash、源码指纹与清理结果；本轮所有入口的 1 分钟 load 均低于 6。更早的重叠 fixture／高负载运行不用于本表结论。

TUN 报告位于 `.meshway/validation/tun/<报告 ID>/report.json`；runtime 位于 `.meshway/validation/runtime/<报告 ID>/report.json`；packet 原始报告位于执行 worktree 下的 `.meshway/validation/packet/<报告 ID>/report.json`，并在仓库同名目录保留副本。入口记录为 `invocationEvidence`；runtime 另记录实际启动前的 `loadBeforeRuntime` 和 `runtimeElapsedSeconds`。packet 的 `daemonReport` 指向整组通过的 `20261007T212412Z-807e82ca`。

packet／runtime 各次运行的构建、导入及结束源码指纹一致：`052e82e75417c17457c5e40951e2d1098147e2be56d3156f89a5cf465eb653c6`。TUN 使用较小的专属输入集合，指纹为 `477af3a17bc873c2346314c1dc27145230821df287fb91b22b6995932ce7b066`。入口记录汇总位于 `.meshway/validation/v1/20261007-main-3887256b/`；报告中各 `passed` 原样保留。

### 未通过边界

- **静态 UDP／resolv-conf**：两次均完成 DNS 启动／重启、双栈满 MTU／大 TCP、125 秒持续及纯接收流、Access 撤销／恢复。失败发生在实际 HTTP 中断之前：A 为 `Outage` ping 等待首个成功样本，准备日志已到 `installed`／`path_handshake`；R 或 B 等待 `http-outage` 的会合期限先到，其他 actor 随后退出。第二次首个 peer 退出后才出现的 `first_hop_channel` 不能作为最初故障原因。没有控制中断与恢复整组通过证据。
- **NAT／off**：首跑的 hostname／双栈 IP／direct ping 完成，但共享 ping 已确认订阅后反复停在 `path_handshake`，60 秒 setup timeout。复跑共享观察和 125 秒包流完成，随后 `state-traffic-attempt-a.json` 中 A 的已认证、具有业务依赖的 UDP link 对应 `0.0.0.0:35000` socket，但 `endpoints=[]`，未满足有效 STUN 映射 State 证明。
- **NAT＋TCP 退路／off**：两次均在 Controller 的同一 STUN State 校验失败。首跑为 `state-fallback-attempt-r.json` 中 R 的 socket 候选为空；复跑在首次业务 carrier 检查即出现 B 的 socket 候选为空（`state-initial-carrier-attempt-b.json`）。首跑 Controller 退出后 A 的纯接收流也停止；不能用该后续现象单独判定退路续租的根因，整组退路仍未通过。
- **NAT／resolved**：首跑共享 ping 经长时间 `path_handshake` 后取得新样本，但首次业务 carrier 校验中 A 的 socket 候选为空（`state-initial-carrier-attempt-a.json`）。复跑 hostname／双栈 IP／direct ping 完成，共享订阅已确认且多次 `installed`，随后持续 `path_handshake`，没有进入 `path_established`，最终 60 秒 setup timeout。共享观察切换缺口仍可复现，卡点已缩小到完整路径握手阶段；尚不能归因为结果发布停滞。

上述候选为空均来自真实 Server State：link 仍为 `authenticated` 且 `businessDependencies > 0`。它不等同于包流从未接通，也不满足完整穿透验收。失败报告保留 controller／A／R／B、准备阶段和 State，穿透报告另保留路由器证据；故障定位不替代修复与重新验收。

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
