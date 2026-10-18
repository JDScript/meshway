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

## macOS 实验验收 {#macos}

以下是 [Apple Host](/v2/node/apple-design) 的 VM 验收行，不属于 `validate_matrix.py` 的 Linux 入口。VM 仍为 A2 薄壳，A3 升级被来宾 GUI 授权阻塞；不得用原生 Rust 单测、Swift 编译／签名或 A2 URLSession 证据代替这些行。环境与操作见 [macOS lab](/v2/node/macos-lab)，私有记录位于 A3 工作树 `.meshway/macos-lab/a3/`。

| 行 | 必需场景 | 状态 |
| --- | --- | --- |
| `macos/ipc` | App／probe → 签名认证 XPC → Rust IPC 13 的 Status、Up、Down、断连取消与有序 stop／join | 未验收（VM）；真实 Node control-only 分派已有原生单测，VM 尚未替换薄壳 |
| `macos/enrollment` | enrollment key 接入、验签与保存 netmap、实际 applied／running／peer | 未验收 |
| `macos/mesh` | utun ↔ Linux peer 的双向 IPv4／IPv6 ping、TCP、UDP；Rust provider underlay 绕行 | 未验收 |
| `macos/dns` | 系统 resolver 解析 Mesh 名、UDP／TCP DNS 与撤回 | 未验收；本机服务地址待决定，未安装 NE DNS |
| `macos/offline` | Server 停机，签名租约内持续通信 | 未验收 |
| `macos/lease` | 短租约实际到期后包被丢弃且不落物理网络；Server 恢复后自动恢复 | 未验收 |
| `macos/restart` | stopTunnel/startTunnel 后在 Server 离线时从 netmap.json 恢复，保持租约期限 | 未验收 |

## 验收矩阵

可执行行 ID 来自 `scripts/validate_matrix.py` 的 `ROWS`：`tun`、`packet`、`udp/off`、`tcp/off`、`vless/off`、`trojan/off`、`udp/resolv-conf`、`udp/resolved`、`nat/off`、`nat+fallback/off`、`nat/resolved`，顺序对应下表十一个隔离入口。常驻 lab 不属于一次性矩阵。统一运行、并发预算与远程证据位置见[矩阵入口](/v2/development#validation-matrix)。

默认矩阵另含 `exit/off`（真实 A→R→E IPv4 出口与单出口 DNS）、`exit6/off`（同一拓扑双栈、E 承载 IPv6／NAT66，见 [exit6/off 行](#exit6-off)）、`nat/relay`（直连不可达时经第三节点中继，见[中继兜底行](#relay-row)）、`nat/reconnect`（Server 与 daemon 重启后打洞边自愈，见[重启自愈行](#reconnect-row)）、`roam/direct`、`roam/nat`、`roam/relay`、`roam/flap`、`roam/exit`（运行中更换上联，见[漫游行](#roam-row)及其后四行）与四个 `netmap/*` 生命周期行（见[netmap 生命周期行](#netmap-rows)）；出口详见[出口场景](#exit-scenario)。

| 入口 | 承载 | DNS | NAT | 覆盖行为 | 状态 |
| --- | --- | --- | --- | --- | --- |
| `test:tun:linux` | — | — | — | 真实 TUN 设备创建与双栈包 I/O | 当前 wire 已通过；`20261007T212324Z-904a137e` |
| `test:packet:linux` | Native TCP | off | — | A→R→B 双栈小包／满 MTU UDP、MTU+1 拒绝、daemon 多 MSS TCP、R 只转发受保护帧、原路径回复 | 当前 wire 已通过；`20261007T212330Z-f7e9a2e9` |
| `test:runtime:linux --transport native-udp --reported-endpoints` | Native UDP 本机上报端点（无 configured／STUN） | off | — | daemon 整组：下文[共同场景](#runtime-共同场景)；B 的 listener 端口在其 daemon 首次绑定时被占用：Status 报告原因后再占 3 秒，释放后无需重启即绑定、清空 `lastError`，端口归 daemon（U1） | W5 全长通过；Planck `20261008T185416Z-08dce814`；历史静态端点：`20261008T033613Z-abbd4545` |
| `test:runtime:linux` | Native TCP | off | — | daemon 整组：共同场景 | W5 全长通过；Planck `20261008T185416Z-9a4a35f1` |
| `test:runtime:linux --transport vless` | VLESS TCP/TLS | off | — | 共同场景；仅 R listener、A/B 拨入，精确叶证书验证 | W5 全长通过；Planck `20261008T185416Z-b31bfda5` |
| `test:runtime:linux --transport trojan` | Trojan TCP/TLS | off | — | 共同场景；仅 R listener、A/B 拨入，精确叶证书验证 | 当前 wire 已通过（两次）；Planck `20261008T060730Z-58ae84d3`、`20261008T061612Z-c9edd384` |
| `test:runtime:linux` | Native UDP 静态端点 | resolv-conf | — | 共同场景＋[DNS 场景](#dns-场景) | 当前 wire 已通过（两次）；Planck `20261008T032749Z-3ba0ca12`、`20261008T050153Z-a1a8eebf` |
| `test:runtime:linux` | Native UDP 静态端点 | resolved | — | 共同场景＋DNS 场景 | V1 串行：当前 wire 已通过，`20261007T220446Z-5360cb6e`；Planck 旧候选未通过，本轮未重验，见[剩余边界](#remaining-boundaries) |
| `test:runtime:linux --transport native-udp --traversal nat` | Native UDP 协调穿透 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋[穿透场景](#穿透场景) | W5 全长通过；Planck `20261008T185416Z-cabbce4c` |
| `test:runtime:linux --transport native-udp --traversal nat --traversal-fallback` | Native UDP＋A–R Native TCP 退路 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋穿透场景＋同路径 TCP 退路 | W5 短模式通过：`20261008T184559Z-c77e0683`；既有全长证据：`20261008T032749Z-4299bfde`、`20261008T050153Z-d30caa20` |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | resolved | 双 IPv4 端口保持型 SNAT | 整组组合 | 当前 wire 已通过（两次）；Planck `20261008T032749Z-d13bd2a9`、`20261008T050153Z-1978b05b` |
| [常驻三节点 lab](/v2/node/docker-lab) | Native UDP 显式端点 | resolved | — | 六向双栈系统解析／echo、路径诊断、关闭插桩的同路径 TCP 吞吐对照 | 未验收（当前整组）；2026-10-07 已有下述 L5 局部证据；共享 bridge 不构成 NAT 证明 |
| `validate:matrix --rows roam/direct` | Native UDP 已发布 listener（两个方向） | off | A 的两个端口保持型 SNAT 上联；A 经独立 IPv6 网络连 Server | 运行中换上联（soft／hard／dual-stack × A 拨号／listener）的首个回复、丢包、TCP、circuit／通道连续性与无关通道；dual-stack 只改 IPv4 默认路由时 A 自己判为 path 并反应，见[漫游行](#roam-row) | 通过；R3 合入 X13 后 Planck 23 行矩阵 `20261016T113938Z-2b403235` 行 `20261016T114617Z-13a2f6cb`；合入前 `20261016T110000Z-ef91dc08` 行 `20261016T110648Z-47047c57`（main 上 dual-stack 两格不达门槛） |
| `validate:matrix --rows roam/nat` | Native UDP 协调穿透（两端空端点） | off | A 两个、B 一个端口保持型 SNAT（端口相关过滤） | A 换上联后由 A 发起的重新穿透：首个回复 ≤ 5 秒、同一 circuit、B 接受时替换旧半边，见 [roam/nat 行](#roam-nat-row) | 通过；Planck 22 行矩阵 `20261016T001640Z-78eb74de` 行 `20261016T002910Z-06f28a33` |
| `validate:matrix --rows roam/relay` | 同上＋公网中继 R | off | 同上 | 空窗内新流经 R 且留在 R、旧流空窗等于重新穿透、恢复后新流直连，见 [roam/relay 行](#roam-relay-row) | 通过；同一矩阵行 `20261016T002745Z-01c614f4` |
| `validate:matrix --rows roam/flap` | Native UDP 已发布 listener | off | A 的两个端口保持型 SNAT 上联 | 2 次（soft）、合并窗内 3 次（burst）、5 次（hard，链路 down/up）快速往返：每次 path 变化一次反应、报告有界、无 circuit 退役、跟随最终来源，见 [roam/flap 行](#roam-flap-row) | 通过（main 同样通过）；同一矩阵行 `20261016T002641Z-4d5f6d62` |
| `validate:matrix --rows roam/exit` | Native UDP 已发布 listener（A 拨号） | off | A（B 的出口）的两个端口保持型 SNAT 上联，出口服务 T 在 WAN | 出口节点漫游：能力 `probing`→`ready`、旧出口 TCP 连接结束、新流经新上联、出口角色保持并可撤回、B→A Mesh 流不受影响，见 [roam/exit 行](#roam-exit-row) | 通过；合入后同一矩阵行 `20261016T115223Z-dbf8ee30`；合入前 `20261016T111250Z-273a0df4`（main 上 `down` 时出口退役一直冲突、行失败） |
| `validate:matrix --rows exit/off` | Native UDP；声明 Native TCP 能力 | 三端 resolved 后端（MagicDNS＋一个 `useWithExit` split） | IPv4 出口 masquerade | A→R→E；能力／安装、公网与 LAN、长流／仅接收、撤回、HTTP 中断内离线出口与租约到期全撤／恢复、journal；单出口 DNS（第 9 阶段）；域名出口与可信关联（第 10 阶段）；方案切换含 `global:E`（第 11 阶段）；ICMP 错误、PMTU 与窄通道 MSS／fragmentation needed（第 12 阶段）；`meshway ping` Mesh 外目标：公网 IP 经 E、direct、域名经 E resolver 与关联、撤权后 denied（第 13 阶段）；`auto` 按 A 的地区选方案（第 14 阶段）；echo 寿命、出口投影容量与出口角色恢复（第 15 阶段）；出口与 Mesh 吞吐并排、echo 安装下的出口 TCP 流（第 16 阶段）；A 的 7,000 区间节点侧分割 | 当前 wire 已通过（含 DNS、域名出口阶段与节点侧分割）；R3＋X6 矩阵 `20261012T115405Z-61e14403` 行 `20261012T115835Z-1812d1b8`；D2（第 13 阶段）矩阵 `20261013T160631Z-761b2963` 行 `20261013T161638Z-b6eb53a0`；X6 Planck 矩阵 `20261012T101704Z-ae64c486` 行 `20261012T102856Z-a05feedb`，见[出口场景](#exit-scenario) |
| `validate:matrix --rows exit6/off` | Native UDP；内层双栈 | 三端 resolved 后端 | IPv4 masquerade＋IPv6 NAT66（`exitFamilies: ["ipv4","ipv6"]`） | A→R→E 的 IPv6：两族能力与 IPv6 前置条件、TCP／UDP／ICMPv6 与 ULA LAN、PMTU／`traceroute -6`、MSS 与 1280 以下通道、AAAA 关联、方案切换、运行时只有 IPv4（`EACCES`）与静态只 IPv4 出口、撤回与 quarantine，见 [exit6/off 行](#exit6-off) | 通过（隔离 fixture；真实宿主 IPv6 出口未验收）；Planck 24 行矩阵 `20261017T213601Z-935b9b88`（`752673fe`）行 `20261017T214444Z-e8b51be7`；单行 `20261017T102314Z-37ac6ad7` 行 `20261017T102526Z-3a7dd7bf` |

以下范围未验收：外层 IPv6 NAT、隔离 fixture 中的端点相关映射（仅有下述[实网证据](#real-nat)）、两端都在端点相关映射 NAT 后、映射轮换、NAT 与 resolv-conf 的组合、VLESS WS/TLS 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换、文件后端（resolv-conf）出口 resolver、resolved 代理 stub 缺失（systemd < 248 或 `DNSStubListener=no`）时的出口 resolver 不可用（仅组件测试）、`dns.overrideLocal` 出口 resolver（仅组件测试）、显式多出口 DNS plan、出口 IPv6、完整 TCP conntrack、中继到下一跳更窄时的 ICMP 回报（只有首跳通道上限可知，见[通道 MTU](/v2/node/dataplane#channel-mtu)）、出口对外部超限回包的错误回报、Mesh IPv6 packet too big 的真实包流（仅组件与 A→R→B 运行测试）及分片重组、userspace／SOCKS5。整机重启与 resolved 停机间隙不属于隔离矩阵，由系统服务安装单独实测：重启（含 Server 不可达）通过，systemd-resolved 重启后 5 秒内、停止再启动后 10 秒内恢复 Mesh 名称解析，见 [Linux 安装与系统服务](/v2/install#验收记录)；`udp/resolved` 行另在隔离 namespace 内重启 resolved 并要求 5 秒内再次解析 Mesh 名称。

常驻 lab 的 L5 原始证据目录为 `.meshway/perf/l5/20261007T162418Z-main-f39ba2b6-2cpu/`。该目录的两次 TCP 接收吞吐分别约 318／364 Mbps；协调者另报告空闲宿主复测 1,367 Mbps，独立报告路径待补，不能把该数值归入前述目录或用吞吐局部证据替代整组验收。

### 实网 NAT 证据 {#real-nat}

2026-10-13，候选 `f1aa2250`：us-lax-it7（x86_64 公网、无 NAT、无 listener，只能拨出）与 meshway-vm（arm64，经 macOS Cloudflare WARP 出网：同一 socket 对不同目的得到不同外部端口，即端点相关映射）。证据 `.meshway/node-lab/n2a/`。

| 项 | 之前（main `1852b477`） | 之后 |
| --- | --- | --- |
| 邻接 | `unreachable`，双向 `meshway ping` 超时；it7 的 INIT 发往 STUN 候选端口，被 WARP 过滤 | `direct native_udp`，双向 `meshway ping` 5/5；it7 从 VM 的签名 punch 学到来源端口（候选 `:54017`，实际 `:53572`）。同时与两个 WARP 后的 lab 容器（main 版本，仅作响应方）直连 |
| 外层分片 | it7→VM 超过 1,357 B 的报文本机分片后全部丢失 | 全部带 DF，抓包 0 个分片 |
| 通道 MTU | 无 | it7→VM `1444/1330`（内层 DF ping 外层 1,444 B 5/5、1,445 B 0/5），VM→it7 `1500/1386`（偏大：OrbStack 不保留 DF；VM 默认路由限 MTU 1,300 后为 `1300/1186`，见 [OrbStack 与 WARP](/v2/install#orbstack-warp)） |
| TCP 吞吐（iperf3 20 s） | 无通道 | VM→it7 6.4 Mbit/s（62 次重传）；it7→VM 0（内层 TCP 1,366 B 超过 1,330，被外层丢弃；数据面 MSS 限制属后续工作）；`-M 1250` 时 it7→VM 101 Mbit/s。裸路径：VM→it7 117 Mbit/s、it7→VM 679 Mbit/s |

开发 Server 发布第二个 STUN 观测点后（2026-10-13 00:54），netcheck 在 VM 判定 `endpoint-dependent`、在 it7 判定 `endpoint-independent`；it7 换装中继修复后 30 分钟内 sync 0 次 `Network` 失败（此前每次失败都与中继 300 秒连接寿命到期精确对应）。

VM→it7 方向 Mesh TCP 远低于裸路径的原因已定位（2026-10-15）：满长外层报文在 Mac 的 WARP 隧道上被分片并乱序，节点发送端顺序与 socket 计数正常；处置与前后对照见 [OrbStack 与 WARP](/v2/install#orbstack-warp)。

### 中继兜底行 {#relay-row}

`nat/relay` 是 5 节点行。在 `nat/off` 的拓扑（A、R 各在端口保持型 SNAT 后，B 公网）上完成共同启动与 70 秒持续流后：

1. router-a 丢弃 A 与 B 之间、router-r 丢弃 R 与 B 之间的全部报文：A 没有直连，R（R1）保留与 A 的通道、失去 B。
2. 两个公网中继在这一步才 enroll；节点 ID 较小的作 R3（与 A、B 都有拓扑，只与 B 常驻，因此 A 与它没有通道），另一个作 R2（与 A、B 都常驻）。
3. Server State 显示 R2 与 A、B，R3 只与 B，R1 只与 A 有已认证通道后，Controller 去掉 `A→B via R` 并允许 A–B 直连。Policy 的候选为 `[A, B]` 与经 R1、R2、R3 的三条两跳路径；只有 R2 与两端都有通道。

| 检查 | 要求 |
| --- | --- |
| CLI ping | `meshway ping b` 的完整路径为 `[A, R2, B]` |
| 新流 | 新的 UDP／TCP 5 元组完成回显（同一排序；不加入 `[A, R1, B]` 上的旧 circuit） |
| 切换时间 | 提交新内容到首个中继回复 ≤ 30 秒 |
| status | A 的 peer B 为 `relay`、`via = [R2]` |
| 未用中继 | 结束时 R3 的报告仍没有与 A 的通道 |
| 节点 ID | `scenario-relay-roles.json` 中 R3 在 R2 之前（R1 与 R2 的先后随 enrollment 而定，一并记录） |
| 路由器证据 | 两个路由器各有其两条 B IP 对 DROP 规则，A→B 方向计数 > 0（A 确实尝试过直连），没有更宽的规则 |

经 `[A, R1, B]` 建立的旧流保持原路径，R1 失去 B 后不再有回复，本行不再检查它们。

结果：`8f6bb90b` 单行 `20261013T065956Z-a803e97f` 失败——R1 的节点 ID 在 R2 之前，ping 经 R2 成功，而新 UDP 流按 Policy 顺序加入了 A 在 `[A, R1, B]` 上的旧 circuit 并被固定在该路径（据此修复为“新流只加入排序位置不低于所选路径的 circuit”）；`94babca6` 单行 `20261013T071339Z-51b9968f`（行 `20261013T071540Z-fc1cd00e`，提交后 1,555 ms 经 R2 回复，R3 `idle`，A→B 直连尝试被丢 16 个）；`1a5a1274` 全量 17 行 `20261013T091200Z-d6fd0e7c`（行 `20261013T092053Z-8fe259a9`，R1 在 R2 之前，1,185 ms）。N3 的 3 节点版本：`991ca758` `20261013T053059Z-11b6d36a`、`3e1142ee` 全量 `20261013T060913Z-86100acd`。证据 `.meshway/validation/n4/`、`.meshway/validation/n3/`。

这一行用路由器丢包造成“直连不可能”，不是两端各自的端点相关映射＋端口相关过滤；后者的实网形态见下节 lax↔planck。

### 重启自愈行 {#reconnect-row}

`nat/reconnect` 在 `nat/off` 的拓扑与共同启动、70 秒持续流之后，把 A–R 变成“端点相关映射的 A ↔ 公网 R”：第一次断开时 router-a 对 A 的新 UDP 流改用随机端口映射（`SNAT --random-fully`，每个目的一个外部端口），router-r 放行全部入站 UDP（DNAT 到 R，R 等同公网主机）。A 的 STUN 映射因此对 R 无效，只有协调穿透加来源学习（R 从 A 的签名 INIT 学到实际端口）能连通。A–R 是按需边（R 能经 TCP 反向拨入 A）：没有通道时 A 每 2 秒发一次 `meshway ping r` 维持需求。

依次四个阶段，每个都要求 A 的 status 中 R 重新为 `direct`（新通道）且距扰动 ≤ 60 秒：

| 阶段 | 扰动 | 计时起点 |
| --- | --- | --- |
| `nat` | router-a 丢弃 A↔R UDP 直到 A 的 status 失去 R，再恢复并清空 A 的全部映射（conntrack） | 恢复 |
| `server` | 真实 Server 原地重启（同一数据库与密钥、新 boot；instance、State、事实全部丢失）；A、R 取得新 instance、State 恢复 `current` 后，同 `nat` 断开再恢复 | 恢复 |
| `restart-r` | 只重启 R 的 daemon | 重启指令 |
| `restart-a` | 只重启 A 的 daemon | 重启指令 |

路由器证据：随机映射与入站 DNAT 规则计数 > 0，两次断开的 DROP 计数 > 0，Server 阶段的恢复晚于 Server 重新服务；各阶段时间单调。证据 `controller/scenario-reconnect-timings.json`、`traversal-packets.json` 的 `reconnect`。

| 候选 | 结果 |
| --- | --- |
| `97051237`（main `e7b9ff0d` 产品代码＋本行） | 单行 `20261013T131635Z-426130a9` 未通过（复现）：`nat` 0.5 秒恢复；Server 重启后 A、R 的每次尝试在激活时都以 `target_not_authorized` 丢弃（穿透 socket 的 Underlay 授权仍指向旧 instance），双方从不发 punch，180 秒内未恢复 |
| `ec24b778` | 单行 `20261013T132830Z-d8e85fdf`：`nat`／`server`／`restart-r` 分别 1.1／10.8／7.3 秒恢复；`restart-a` 未恢复——新进程没有业务需求（A–R 为按需边），fixture 据此改为无通道时持续 ping |
| `7c0febbb` | 单行 `20261013T134136Z-fbcbd4ea` 通过：`nat` 1.2 秒、`server` 10.4 秒（A、R 在 Server 重新服务后 1.6／2.4 秒取得新 instance，8.3／8.9 秒 State `current`）、`restart-r` 7.3 秒、`restart-a` 9.6 秒；R 从 A 的签名 INIT 学到随机映射端口（候选 `:35000`／`:1391`／`:62265`，实际 `:24025`／`:22396`） |
| `db193112`（全量矩阵） | `20261013T135826Z-fe73e26e` 通过：`nat` 10.5 秒、`server` 1.4 秒、`restart-r` 10.4 秒、`restart-a` 9.4 秒 |

### 漫游行 {#roam-row}

`roam/direct`（`--traversal roam`）是独立的 6 容器拓扑，资源全部为本行自有的 internal bridge 与容器：A 有两个上联（uplink1／uplink2，各经一个端口保持型 SNAT 路由器到 WAN；router-a1 另把 WAN 上的 UDP 35000 转发给 A）、一个与 C 共享的 LAN，以及只有 A 与 Controller 的 IPv6 网络 `srv6`（A 经它连 Server，如 `meshway-vm` 经 IPv6 连 dev Server；STUN 与对端仍为 IPv4）；Controller、公网 B 与 C 在 WAN 上。topology 只允许 A–B 与 A–C，每条边只有一方能拨出（常驻边）；租约 1,800 秒，避免租约换代混入测量。A 对 B 持续 100 pps 带序号 UDP 回显、10/s ping 与一条每 50 ms 一行的 TCP echo，对 C 同样运行（无关通道）。两个阶段各三格，扰动前要求 B、C 都已回复：

| 阶段 | A–B 方向 | soft | hard | dual-stack |
| --- | --- | --- | --- | --- |
| `dialer` | A 拨号 B 的已发布 listener | A 的默认路由改到 router-a2，再删除 uplink1 的地址 | 断开 uplink2，重新接入 uplink1 并设默认路由 | 接回 uplink2 并等 6 秒后，只把 IPv4 默认路由改到 router-a2：A 的链路、地址与到 Server 的 IPv6 路由都不变 |
| `listener` | B 拨号 A 在 router-a1 上发布的 listener | 同上 | 同上 | 同上 |

计时起点为新默认路由生效（A 自己的时钟，与流量记录同一时钟）。每格观察 12 秒；门槛（D14，合入条件）：首个内层 UDP 回复 ≤ 3 秒、A 的业务 circuit 集合不变（Server State 的 `paths`）、TCP 连接未重置、A–C 通道 ID 不变；dual-stack 格另要求 A 在 1 秒内把变化分类为 path 并有一次 Manager 漫游反应。另记录 ping 与 TCP 的首个回复、双向 UDP 丢包、最长中断、channel ID 与 remote、daemon 的漫游 debug 事件。

| 格 | main：首个 UDP／最长 TCP 中断／候选发布，A–B 通道 | R1：首个 UDP／最长 TCP 中断／候选发布，A–B 通道 |
| --- | --- | --- |
| dialer/soft | 0.43／0.67／0.96 s，新通道（B 的 `probe_recovery` 握手） | 0.21／0.66／0.31 s，同一通道跟随到 router-a2 的映射 |
| dialer/hard | 0.32／0.66／4.73 s，新通道 | 0.25／0.70／0.46 s，同一通道 |
| listener/soft | 7.25／12.5（窗口内 TCP 无回显）／0.97 s，B 健康失效后重拨 | 0.53／0.67／0.43 s，B 的拨号 socket 经挑战跟随，同一通道 |
| listener/hard | 5.82／6.44／4.69 s | 0.26／0.64／0.46 s，同一通道 |

候选发布：A 在路径变化撤回旧 STUN 观测后第一份带候选的 State 被 Server 接受的时刻（自新路由起）；listener 阶段 B 在其后 1–2 ms 收到 A 的新事实（dialer 阶段 B 不拨号 A，不记录）。hard 格在 main 上的 4.7 秒来自无路由窗口内失败的 State 报告按 5 秒重试，R1 在随后的路径变化（新链路有路由）时立即重试。

dual-stack 格（R3）：

| 格 | main `34987b0d`＋本行 fixture（Planck `20261016T085917Z-6ee48043`） | R3（Planck `20261016T110000Z-ef91dc08`） |
| --- | --- | --- |
| dialer/dual-stack | 分类 refresh、无漫游反应（不达门槛）；首个 UDP 0.48 秒、TCP 最长中断 0.72 秒——B 在 A 的下一次例行健康 Ping 后挑战跟随 | path（新路由后 0.25 秒）与一次漫游反应；首个 UDP 0.62 秒、TCP 最长中断 0.73 秒：B 在 0.26 秒挑战新来源，第二个挑战 Ping 的 Pong 使通道在 0.61 秒跟随（第一个未得到应答；合入 X13 后的矩阵 `20261016T113938Z-2b403235` 中 dialer 格 0.28 秒、listener 格 0.57 秒，即这一次多等一个挑战间隔在两格间不定出现，原因未查；main 上 B 由 A 的例行 Ping 触发挑战，0.48 秒） |
| listener/dual-stack | 同上；0.42／0.72 秒 | path（0.25 秒）与一次反应；0.28／0.32 秒 |

soft／hard 四格在 A 经 IPv6 连 Server 后两边仍达标（地址或链路的变化本身即 path 类）。

两边四格的 circuit 都保持（main 的空窗也短于 15 秒无回复上限），TCP 都未重置，A–C 通道与流量不受影响（R1 四格 LAN 丢包 0）。main 的 listener 两格不达门槛；R1 四格达标，UDP 丢包 25–58 个（100 pps），路由器计数见各行 `router-*` 证据。

| 运行 | 结果 |
| --- | --- |
| 本机 main 基线：main `0af9393e`（产品代码同 `f007bde1`）＋本行 fixture，`20261015T174443Z-0af0b4fb` | listener 两格未达门槛（上表左列） |
| 本机 R1 `1aa1d67f`，`20261015T181918Z-bc4928d6` | 四格达标（上表右列）；此前 `f274753d` 的 `20261015T173450Z-9b555759` 同样达标，hard 格候选发布 6.7 秒（已修） |
| 合入 main（alpha.13）后 Planck 19 行矩阵 `20261015T220339Z-2b25a4b8`（`b1b9d75c`，并行 8） | 19 行全部通过；`roam/direct` 行 `20261015T221317Z-40f317f5` 首个 UDP 回复 0.24／0.27／0.24／0.06 秒，TCP 最长中断 ≤ 0.70 秒，同一 circuit 与通道，候选发布 0.31–0.47 秒 |
| Planck 19 行矩阵 `20261015T212445Z-21b27563`（R1 `2eae8716`，并行 8，墙钟 1,066 秒） | 19 行全部通过；`roam/direct` 行 `20261015T213122Z-49aac8c7` 首个 UDP 回复 0.24／0.03／0.23／0.06 秒（dialer soft／hard、listener soft／hard），TCP 最长中断 ≤ 0.67 秒，同一 circuit 与通道，候选发布 0.11–0.45 秒，B 收到候选 7／109 ms，LAN 丢包 0 |
| Planck 此前两次（`20261015T193138Z-e662edb4`、`20261015T204321Z-92dbe6a6`） | `roam/direct` 均通过；其余 runtime 行分别因新内核测试超出 DNS 锚点测试的任务预算、新增内核测试地址挤占接口替换地址而失败（fixture 已修）；`20261015T211031Z-1660d855` 中 `exit/off` 的组件测试一次 Netlink 错误（之前两次通过），变化观测随后改用独立 netlink socket |

R1 之前的候选：A 的阶段切换（A 增加 listener）后一次 Manager apply 失败、重启邻接，C 看到新 boot 退役 A–C circuit 而 A 一侧保留，A→C 空等 15 秒（`20261015T172722Z-c3cc3381`）；本节点换 boot 的对称处理见 [X12](#x12-change)，apply 失败的 owner 未能定位，Manager 现记录意外结束的 owner。另一候选在 B 已经跟随 A 的新映射后又因“remote 不在候选集合”旁路拨号（`20261015T172314Z-d76d6adc`），现只在 remote 原是已发布候选时才拨。证据 `.meshway/validation/r1/`。

真机（R1 程序部署到常驻 lab 与 `meshway-vm`）：

| 环境 | 扰动 | 观察 |
| --- | --- | --- |
| lab：lax 接入自有 bridge `meshway-r1-lab-roam` | lax 默认路由改到新 bridge（soft） | path 类变化，控制连接重连；sha／nyc→lax 5/s ping 无中断，lab bridge 上的通道不变 |
| 同上 | 断开 lax 的 lab bridge（hard），8 秒后接回 | sha、nyc 的通道在断开后 0.11 秒内经挑战跟随到 `192.168.97.1:<同端口>`（宿主转发的新来源），接回时 1 ms 内跟随回原地址；ping 最长中断 sha 无、nyc 0.62 秒，channel ID 不变，无 circuit 退役 |
| `meshway-vm`（systemd） | 在 eth0 增加第二地址，再把默认路由的 `src` 改为它（保留 `mtu 1300`），随后精确恢复 | 原记为“两次 path 类变化”，有误：OPS13 以 debug 复核，三次都分类为 refresh（VM 经 IPv6 连 Server，IPv4 第二地址带 `IFA_F_SECONDARY` 被当作临时地址）；sha→VM ping 无中断；运行 main 的 us-lax-it7 在恢复后约 12 秒失去与 VM 的通道、30 秒内恢复（对端没有拨号端跟随） |

lab 与 VM 结束后恢复原程序与网络；私有记录在 `.meshway/validation/r1/lab/`、`.meshway/validation/r1/vm/`。

R3 真机（`meshway-vm`，同一扰动，sha 与 it7 运行 alpha.14、只读；VM 以 debug 运行，结束后恢复 alpha.14 程序与 `RUST_LOG`）：

| VM 程序 | 分类（加地址／改 `src`／恢复） | VM→it7 最长中断（加地址／改 `src`／恢复） | it7→VM | 本机反应 |
| --- | --- | --- | --- | --- |
| alpha.14（OPS13 两次） | refresh ×3 | 1.65 秒（恢复）、1.03 秒 | 1.66、0.83 秒 | 无；只靠 it7 挑战跟随 |
| alpha.14（R3 复测 `before-alpha14`） | refresh ×3 | 0.28／1.03／0.60 秒 | 0.30／1.03／0.56 秒 | 无 |
| R3 `9b83d08e`（`after-r3-1`、`after-r3-2`） | path ×3（两次） | 0.21／0.41／0.42、0.39／0.41／0.45 秒 | 0.21／0.41／0.43、0.39／0.41／0.60 秒 | 每次撤回 10 个 STUN 观测并在约 0.15 秒内重新发布、4 条 UDP 通道漫游探测；it7 的 VM 端点 `:47656`→`:47832`→`:47656` 跟随，通道不变 |

私有记录 `.meshway/validation/r3/vm/`；矩阵证据 `.meshway/validation/r3/planck/`。Mac 切换 Wi-Fi（只有映射变化）由用户操作，未执行。

### roam/nat 行 {#roam-nat-row}

`roam/nat`（`--traversal roam --netmap-scenario roam-nat`）在漫游行的拓扑上把 B 放进自己的 NAT（router-b），三台路由器都是端口保持型 SNAT、只放行本侧主机发起的流的回包，没有任何转发端口；A、B 的 Native UDP listener 都没有发布端点、都能拨出，A–B 为常驻边，只能经协调穿透连通。单一阶段两格：soft（A 默认路由改到 router-a2 再删 uplink1 地址）与 hard（断开 uplink2、接回 uplink1 并设路由；soft 结束后 router-b 忘记它为 router-a1 保留的回包（conntrack 删除），使 hard 也是 B 的 NAT 没见过的映射）。每格观察 25 秒；门槛：首个内层 UDP 回复 ≤ 5 秒（D14 收紧）、circuit 集合不变、TCP 未重置、A–C 通道不变、A 取得新通道、新穿透首个 prepare 的一方是 A、B 在仍持有旧通道时接受（旁接受）。另记重新获取时刻（A 的新通道安装）。

| 格 | main `b9f7108d`＋本行：首个 UDP／重新获取／最长 TCP 中断 | R2：首个 UDP／重新获取／最长 TCP 中断 |
| --- | --- | --- |
| soft | 5.24／5.23／6.45 秒：A、B 各等 6 秒健康窗口失效后才重新协调 | 3.14／3.13／3.17 秒：A 在探测无应答 1.5 秒后结束旧通道并 prepare，B 在旧通道旁接受（offer 方向与旧通道相反） |
| hard | 5.80／5.79／6.42 秒 | 2.83／2.83／6.70 秒 |

两边 circuit 都保持、TCP 未重置；TCP 最长中断在空窗约 3 秒时为 3.2 或 6.4–6.7 秒（内核 RTO 退避的下一次重传）。中间候选 `ee16929b` 的 soft 格 4.9／5.3 秒：B 以“承载不同”拒绝了与其旧通道方向相反的 offer，已修。

### roam/relay 行 {#roam-relay-row}

`roam/relay`（`--netmap-scenario roam-relay`）在 `roam/nat` 的拓扑上增加公网中继 R（WAN 上的已发布 listener，A、B 都拨它；A–R、R–B 常驻），无 `via`，Policy 的候选为 `[A, B]` 与 `[A, R, B]`。除主流量外 A 每 250 ms 用一个新 5 元组发一个 UDP 报文（每个都是新流）；A 的 status 失去对 B 的直连时启动第二条 100 pps UDP 流（“中继流”），恢复直连 2 秒后再启动第三条（“恢复后流”）4 秒；R 的收包计数每 250 ms 采样。门槛在 `roam/nat` 的之外：旧直连流的最长中断 ≤ 重新获取＋1 秒；空窗内（失去直连 0.25 秒后至恢复前 0.25 秒）至少有一个新流，且每个新流都在 1 秒内经 R 得到回复，R 在中继流启动前的收包速率 > 0；直连恢复后中继流仍经 R（R 收包 ≥ 150 pps）且最长中断 ≤ 1 秒；恢复后流得到回复且让 R 的收包增加 < 50 pps（不经 R）；扰动前的 circuit 全部仍在（空窗内新增一条经 R 的）。

| 格 | main：旧流中断／重新获取，空窗新流，中继流 | R2：旧流中断／重新获取，空窗新流，中继流，恢复后流 |
| --- | --- | --- |
| soft | 6.48／6.47 秒；空窗内 5 个新流 0 个经 R——它们加入了下一跳已无通道的直连 circuit 并一起等待；中继流同样等到直连恢复，R 无流量 | 2.99／2.98 秒；2/2 经 R；中继流恢复后仍经 R（200 pps，最长中断 0.08 秒）；恢复后流 406 个回复、R 增加 2.7 pps |
| hard | 6.47／5.84 秒；同上 | 2.91／2.58 秒；2/2 经 R；同上（200 pps、0.10 秒）；R 增加 2.0 pps |

即 D7 的语义成立：已有流不迁移、空窗内断且空窗就是重新穿透时间；空窗内的新流经策略中继并留在中继，恢复后的新流回到直连。

### roam/flap 行 {#roam-flap-row}

`roam/flap`（`--netmap-scenario roam-flap`）用漫游行的拓扑、A 拨号公网 B 的一个阶段，两个上联始终接入：soft 在 0.4 秒间隔内切换默认路由 router-a2→router-a1；burst 在一条 shell 命令内连切三次（a2→a1→a2，远小于 250 ms 合并窗）；hard 以 uplink1 链路 down／up 加路由切换往返 5 次（0.4 秒间隔，约 2 秒）。门槛：分类出的 path 变化数等于 Manager 的漫游反应数（burst 恰为 1，其余 1 至切换次数）；State 报告 ≤ path 变化数 × 16 ＋ 1；无 circuit 退役、circuit 集合不变；B 的通道最终指向 A 最后一次切换所用路由器；首个回复 ≤ 3 秒、TCP 未重置、A–C 不变。

| 格 | main：path 变化／反应／报告（上限），B 跟随次数 | R2：同左 |
| --- | --- | --- |
| soft（2 次） | 2／2／4（33），2 | 2／2／4（33），0（a2 只停留 0.4 秒，挑战未完成即回到原来源） |
| burst（3 次） | —（R2 新增） | 1／1／2（17），1 |
| hard（5 次） | 5／5／10（81），5 | 4／4／6（65），4（两次切换落在同一合并窗） |

main（R1）已满足本行的不变量；R2 的改动未改变它（每次 path 变化的报告实测 2 份：撤回旧观测、带新映射）。

| 运行 | 结果 |
| --- | --- |
| 本机基线：main `b9f7108d`＋三行 fixture `6cbe366a`，`baseline-nat-1` 行 `20261015T230204Z-f1530c30`、`baseline-rf-1` 行 `20261015T230421Z-3c5fe0a6`／`20261015T230549Z-8f506fc3` | `roam/nat` 两格超过 5 秒；`roam/relay` 空窗新流与中继流不经 R；`roam/flap` 通过 |
| 本机 R2 `r2-3`：`roam/nat` `20261015T232835Z-340b4faf`、`roam/relay` `20261015T233003Z-80dcd05e`；`r2-1`：`roam/direct` `20261015T231657Z-1ba2e939`、`roam/flap` `20261015T232141Z-20335112` | 四行全部通过 |
| Planck 22 行矩阵 `20261016T001640Z-78eb74de`（`0ff3e157`，并行 8，墙钟 1,066 秒） | 22 行全部通过。`roam/nat` 首个回复 3.68／2.73 秒（soft／hard），`roam/relay` 3.34／3.39 秒、空窗新流 4/4、5/5 经 R、中继流恢复后 205／203 pps 经 R、恢复后流不经 R；`roam/flap` path 变化／反应／报告 2/2/4、1/1/2、4/4/6；`roam/direct` 0.28／0.06／0.28／0.09 秒 |
| Planck 此前 `20261015T233743Z-e3b061a6`（`57a4d256`，四个 roam 行与其余行并发） | 20 行通过；`roam/flap` 与 `exit/off` 创建网络失败：`all predefined address pools have been fully subnetted`（每个 roam 行 4–5 个 bridge）。矩阵此后让 roam 行逐个运行 |

证据 `.meshway/validation/r2/`。

### roam/exit 行 {#roam-exit-row}

`roam/exit`（`--netmap-scenario roam-exit`）用漫游行的拓扑、A 拨号公网 B 的一个阶段：A 开启 `exitEnabled`、`ip_forward=1`，是 B 的出口；B 的方案 `exit` 只有一条 IP 规则（WAN 上的服务 T 的 /32 经 A 出口，回退 direct，无 DNS 计划），Access 允许 B→T 的 UDP 42424／TCP 42425／ICMP。T 是 WAN 上的普通容器（UDP／TCP 回显，记录每个新 UDP 来源与每个接受的 TCP 对端）。B 对 A 持续运行 Mesh 流量（100 pps UDP、ping、TCP），A 对 C 同样（无关通道）；每格前 B 对 T 启动一组新的出口流量（同样三种），扰动同 `roam/direct` 的 soft、hard 两格；观察 12 秒后 B 对 T 建一条新 TCP 连接（回显一行）与一个新 UDP 流（回显一个报文）。A 的出口能力（Server State `exitCapability`）每 0.2 秒采样。门槛：B→A Mesh 首个 UDP 回复 ≤ 3 秒、B 的 circuit 集合不缩小、Mesh TCP 未重置、A–C 不变；能力先报 `probing`、5 秒内回到 `ready` 且仍装有 IPv4；本格的出口 TCP 连接结束（报错，或新路由 3 秒后不再有回显）；新 TCP 与 UDP 流得到回复，T 看到的新 TCP 来源是 A 的新上联路由器；A 没有出口角色停止／失败记录；行结束时各节点 `down` 成功。

| 格 | main `34987b0d`＋本行 fixture（Planck `20261016T085917Z-6ee48043`） | R3（Planck `20261016T110000Z-ef91dc08`） |
| --- | --- | --- |
| soft | `probing` 0.57 秒、`ready` 1.65 秒；Mesh 首个 UDP 0.28 秒、TCP 最长中断 0.46 秒；出口 UDP 0.29 秒后以 router-a2 的地址继续（同端口），出口 TCP 停滞、无 RST；新 TCP／UDP 流成功 | `probing` 0.56 秒、`ready` 1.65 秒；Mesh 0.28／0.47 秒；出口 UDP 0.29 秒后以 router-a2 的地址继续，出口 TCP 停滞、无 RST；新 TCP 来源为 router-a2，新 UDP 成功；无出口角色失败记录 |
| hard | `probing` 0.14 秒、`ready` 1.49 秒；0.06／0.66 秒；同上（新来源 router-a1） | `probing` 0.42 秒、`ready` 1.51 秒；0.05／0.64 秒；出口 UDP 0.05 秒；新 TCP 来源为 router-a1 |
| 行结束 | A 的 `down` 未完成：出口 `maintain`／退役反复 `kernel ownership or routing protection conflict`（内核为同一原始 tuple 新建的条目被判为外来），`meshway down` 超时，行失败 | 各节点 `down` 成功，出口清理完成 |

| 运行 | 结果 |
| --- | --- |
| main 基线：`34987b0d`＋fixture `28ebb42c`，Planck `20261016T085917Z-6ee48043`（`roam/exit` 行 `20261016T090336Z-2531ae82`、`roam/direct` 行 `20261016T090455Z-bfec3635`） | `roam/exit`：两格测量项均达门槛，行结束时 A 的 `down` 失败；`roam/direct`：dual-stack 两格不达门槛 |
| R3 中间候选 `1a65b3bd`，Planck `20261016T091146Z-e6a4d826` | `roam/exit`、`roam/direct`、`exit/off` 通过；`udp/off` 的新内核测试失败：IPv6 临时地址与 Server 路由同在一个接口，它是该路由的首选源，删除它确实改变了路由（测试已改为另一接口） |
| R3 `9b83d08e`，Planck `20261016T092905Z-255b4d91` | `udp/off`（含 `underlay-roam-family`）通过 |
| 合入 main（X13，`68fab20f`）后 Planck 23 行矩阵 `20261016T113938Z-2b403235`（`75b51cbc`，并行 8，墙钟 1,111 秒，无并发矩阵） | 23 行全部通过，`underlay-roam-family` 在 15 个 runtime 行均通过；`roam/direct` 六格首个 UDP 0.08–0.57 秒、TCP 最长中断 ≤ 0.72 秒；`roam/exit` 两格达标 |
| R3 Planck 23 行矩阵 `20261016T110000Z-ef91dc08`（`b425ccc9`，并行 8，墙钟 1,094 秒） | 23 行全部通过；`roam/direct` 六格首个 UDP 0.07–0.62 秒、TCP 最长中断 ≤ 0.73 秒；`roam/exit` 见上表；`roam/nat` 3.72／2.84 秒、`roam/relay` 3.64／2.76 秒 |

本机（Mac，负载 20–40）上的同一基线另见出口重新探测与 A 的全部流量一起停滞 6–9 秒（`baseline-1`）；Planck 上没有出现，视为宿主 CPU 饥饿，不作为 main 的行为结论。

### N3 实网证据 {#n3-real-network}

2026-10-13，`7376e3e0`（lab、VM、it7）与 `66897a30`（lab、VM、it7、planck 容器 `meshway-planck`）。证据 `.meshway/node-lab/n3/`。

| 项 | 结果 |
| --- | --- |
| lax（Docker）↔ meshway-vm（OrbStack machine） | 部署后无流量即 `direct native_udp`（lax 拨 VM 的本地候选 `192.168.139.66:35000`），`meshway ping` 双向 5/5（0.2–0.9 ms） |
| `* ↔ *` keepalive | 三容器六个方向在部署后数秒内 `direct`，05:09:47–05:15:54 的 13 次采样端点不变；it7、VM 与全部 lab 节点同样常驻 |
| 中继（lax 内用 tc 丢弃 VM 的来包） | 通道健康失效后（6 秒内）`meshway ping meshway-vm` 10/10 `via relay meshway-sha`、OS ping 5/5，status 为 `relay via meshway-sha` |
| 候选节奏 | 同一阻断下 lax 对 `192.168.139.66:35000` 的拨号尝试（每次 5 秒）起点间隔 6、7、9、13、21.8、37.9、69.1、133.6、261.6 秒；期间两次被其他工作流的配置修订（新 netmap）重新开始；解除阻断后约 4.5 分钟（当时的暂停）恢复直连 |
| lax、sha、nyc、VM ↔ planck 容器 | 双向 `unreachable`。planck 容器在 Docker masquerade 后（映射与端点无关、端口保持 `128.238.177.131:35000`），lax 等经 WARP（端点相关映射）。planck 宿主 `enp14s0` 45 秒抓包：发往 `104.28.152.187` 的 punch 139 个，来自 WARP 的入站 0 个；同期 lax 确实发出发往 `128.238.177.131:35000` 的 punch。即 lax 的报文在到达 planck 网卡之前被丢弃，planck 也就无从学习 lax 的映射端口；属于上文“端点相关映射 × 端口相关过滤”的不可直连形态 |
| it7 ↔ 全部节点 | `direct native_udp`；planck 容器与 it7 直连 |

未观测：WARP 更换出口 IP 后的恢复（本轮窗口内未发生）。netcheck 的过滤类型判定未实现（需要能从未联系过的地址／端口回包的观测点；开发 Server 的 STUN 端点为 Google、Cloudflare，不支持 RFC 5780 CHANGE-REQUEST）。

### N4 实网证据 {#n4-real-network}

2026-10-13，`94babca6`（lab 三容器、VM、it7、planck 容器 `meshway-planck` 与开发 Server）。证据 `.meshway/node-lab/n4/`。

| 项 | 结果 |
| --- | --- |
| 事实 | lax 的最后已知事实中 it7 的 `connected` 含 planck 与全部 lab 节点，planck 的只含 it7 |
| lax、sha、nyc、VM → planck | `meshway ping meshway-planck` 各 10/10 `via relay us-lax-it7`（RTT 101–145 ms）；lax status 为 `relay via us-lax-it7`。N3 时四路径截断去掉了 it7，lax→planck 超时 |
| 经 Mesh 的业务 | planck 容器 `python3 -m http.server 8080 --bind 100.64.0.6`，四个源 `curl http://100.64.0.6:8080/` 均 200（0.35–0.89 s），planck 日志记录四个 Mesh 源地址 |
| 无关修订不重置退避 | lax 内 tc 丢弃 VM 的来包后，lax 对 VM `lab-udp 192.168.139.66:35000` 的 `candidate_backoff` 为 failures 1–7（暂停 1→64 s）；09:02:04 修改 nyc 的 remark（revision 32，lax netmap serial 35→36）后下一次尝试仍按原节奏在 09:02:27 发生（failures 8、暂停 128 s），随后 failures 9（256 s），其间没有 `candidate_pacing` 重新开始事件 |

### X7 真实出口宿主 {#x7-exit-hosts}

2026-10-13，分支 `x7/exit-hosts` 的 `bede97a0` 构建（最终 `5183bf6e` 只另把直连公网网段计入出口接口、修正组件测试并整理 worker 代码，planck 最终运行 `df591c3f`）。证据 `.meshway/node-lab/x7/`。

| 项 | 结果 |
| --- | --- |
| planck 容器作出口（Docker-on-Linux，`172.18.0.2/16`，全部接口 `rp_filter=2`），修复前 | it7 经 planck 出口 `ping 128.238.177.1` 0/3；echo request 进入出口 TUN 并命中 `echo_flows`，容器 `IpForwDatagrams` 不变、`TcpExtIPReversePathFilter` +3。根因：出口 TUN 没有 IPv4 地址，loose `rp_filter` 对它的反向路径检查在反向查找未落到该 TUN 时一律 `EXDEV`（无流时 `ip route get 128.238.177.1 from 100.64.0.5 iif tun1 mark …` 即 `Invalid cross-device link`）。lab 的 OrbStack 容器 `rp_filter=0`，不做该检查 |
| 同上，修复后 | it7 经 planck：`ping` 4/4（约 110 ms），`IpForwDatagrams` +15、`IPReversePathFilter` 不变；lax 经 it7 中继到 planck 出口（公共规则 `128.238.0.0/16` → Exit planck via it7）：`ping` 4/4，`traceroute -I` 第 1 跳 planck 的 Mesh 地址 `100.64.0.6`、第 2 跳 Docker 网桥 `172.18.0.1`、第 3 跳目标；`curl smtp://128.238.2.93/` 收到 `220 duke.poly.edu ESMTP Postfix` 与 EHLO 回答。`128.238.177.1` 不提供 HTTP，所查 NYU `128.238/16` 主机中未找到开放 HTTP 的目标 |
| us-lax-it7（Docker 28 iptables-nft，`DOCKER-ISOLATION-STAGE-*`／`DOCKER` 链有限定网桥的 DROP，FORWARD `policy accept`）`exitEnabled=true` | 修复前报 `firewallConflict` 且整机 `synchronizing · not applied`、peer 全部 unreachable；修复后 running、applied（revision 30）、5 个 peer `direct`，`Exit unavailable (conntrack unavailable)`，Server State 同为 `unavailable`／`conntrackUnavailable`（`freshness=current`）。防火墙判定通过：it7 实际 `nft -j list ruleset` 的离线判定为无冲突；下一项真实缺口是 it7 未安装 `conntrack`。前后 it7 的外来 nft 对象（74 个，忽略计数器与 handle）完全相同；`exitEnabled` 已恢复为 `false`（revision 31） |

未完成：lax 经 it7 出口的 `curl ifconfig.me` 未执行。上表 `conntrackUnavailable` 是 M3 之前缺 `conntrack` 工具所致；M3 起出口经内核 ctnetlink 管理 conntrack，不再需要该工具，it7 上的实网复验待补。

### N5 实网证据 {#n5-real-network}

2026-10-13，lab 三容器、VM、us-lax-it7、planck 容器与开发 Server 统一在 `db193112`。证据 `.meshway/node-lab/n5/`。

修复前的根因证据：it7 journal（`it7-journal-1207-now.txt`）记录 12:27:51–12:57:42 开发 Server 经 Cloudflare 不可达（530／502），`renew_instance` 失败后 `acquire_instance` 取得新 instance；此后所有需要打洞的边（lab／VM ↔ it7）`unreachable`，13:47 部署前仍是（`before/`）。11:19:27 alpha.5 的 Server 重启同样使全部节点换 instance，之后只有两端都在其后重启过的边能重新打洞，与 11:16–12:07 的分区及“只重启一端无效、两端同时重启才恢复”一致。fixture 中同一序列的激活日志为 `target_not_authorized`（[重启自愈行](#reconnect-row)）。

| 扰动 | 结果（应直连的边 direct、lab／VM↔planck 为 `relay`／`relayIdle`，10 秒一次采样） |
| --- | --- |
| 部署本身的 Server 更新（14:05:21） | 14:05:38 只剩 VM↔it7 为 `relayIdle`，14:05:59 起全部达标（≤38 秒） |
| `dev.py restart server`（14:07:29） | 六个节点全部换 instance（`instanceId`／`generation` 均变）；已有通道不中断，12 个样本中 11 个达标；14:09:01 一个样本 planck 对 lax／nyc／vm 为 `unreachable`、对 sha 为 `offline`，下一样本恢复 |
| 重启 it7 的 meshwayd（14:10:02，其他节点保持 Server 重启后的新 instance） | 首个样本（+13 秒）起全部达标；修复前正是这一序列（Server 重启后只重启一端）无法恢复 |
| `docker restart meshway-lax`（14:12:24） | 首个样本（+11 秒）起全部达标 |

lab／VM 与 planck 之间设计上没有直连（两端端点相关映射），空闲时显示 `relay (idle) via us-lax-it7`，不再是 `unreachable`。

之后 `relayIdle` 改为要求路径上每个中继都报告与下一节点的通道，六台与 Server 重新统一到该提交，并做 10 分钟观察窗（`window/`，`check.py` 逐样本判定）。

### M1 实网证据 {#m1-real-network}

2026-10-13，lab 三容器、VM、us-lax-it7、planck 容器与开发 Server 统一在 `287e2385`（先 Node 后 Server）。证据 `.meshway/node-lab/m1/`。

| 检查 | 结果 |
| --- | --- |
| it7 → VM TCP，不限 MSS（VM 端 `iperf3 -R`，20 秒） | 升级前（main）0 bit/s（重现 N2a 基线）；只升 it7 后 373 Mbit/s；全部升级后 408／394 Mbit/s，`-M 1250` 对照 355 Mbit/s。it7 到 VM 的通道 `1395/1281`→`1415/1301`，it7 把 VM 的 SYN 限为 MSS 1261，两端路由缓存都没有 PMTU 例外 |
| it7 的 fragmentation needed | `ping -M do -s 1300 100.64.0.4`（1328）报告 `Frag needed and DF set (mtu = 1301)`（it7 自己的数据面合成），路由缓存 `mtu 1301`；`-s 1273` 3/3 |
| OPS4 的纯 IP 双出口（公共规则 `128.238.0.0/16 → planck`、`geoip:cn → it7`） | 六台 running／applied、`dns.egress` 全为 `noExit`、`getent` 各自本机解析；lax `traceroute -I 128.238.177.1` 第 1 跳 planck（100.64.0.6），`traceroute -I 1.2.4.8` 第 1 跳 it7（100.64.0.5），ping 均 3/3。main 下同一形态曾让六台 `blocked`（OPS4 rev 34） |
| 名称规则经两个出口（`nyu.edu → planck`、`example.net → nyc`，再加上述两条 IP 规则） | 保存 422 `dns_exit_plan_required`，路径 `/routing/egress/rules/1`，节点 lax、sha、vm、it7（planck、nyc 对自己的规则为 direct）；revision 不变 |

测试配置结束后以原 JSONC 源文 CAS 恢复（rev 41 与 rev 39 内容相同）。

### U1 启动期自愈证据 {#u1-startup-self-heal}

2026-10-13，只用 lab 三容器（`node:lab deploy`，每轮以不同 `RUST_LOG` 强制重建全部容器；开发 Server 未改动）。证据 `.meshway/node-lab/u1/`（`check.sh` 逐轮输出 `deploy-<n>-check.txt`，端口占用采样 `holder/`）。

| 检查 | 结果 |
| --- | --- |
| namespace inode 复用 | 八轮重建中 inode 多次复用：lax 得到 N5 中 nyc 冲突时的 `4026532721`，之后 nyc、sha 也先后得到它；`4026532512`、`4026532616` 同样在不同容器间复用。每次 cookie 都不同（96625 → 98254 单调），旧 `exit-child.json`／journal 以 `Archived the exit-child record of another network namespace`／`changed_namespace` 归档，无一次 `Underlay startup failed` 或冲突退避 |
| 升级过渡 | 第一轮从无 cookie 的旧记录升级时 inode 未复用，旧记录按 inode 规则归档；新记录即带 cookie |
| listener `EADDRINUSE` | 约三分之一的节点启动时 `0.0.0.0:35000` 绑定失败（`Listener unavailable; retrying its bind`，`attempts=1`），1 s 后 `Listener bound after retry`，随即以同一内容再 apply，Status 清空 `lastError`；套接字由 daemon 持有（`ss`），peers 全部 direct。旧构建同一现象需重启容器（ops5 nyc） |
| 修复中发现 | 第一版重试绑定后 Status 仍保留 `lastError: transport`（完整 apply 不清除先前 Degraded 的问题），deploy 因 sha 不收敛失败；`775fbeb5` 修复后重部署 |

Planck 18 行矩阵 `20261013T211635Z-273c8e3a`（`7a6dab15`）全部通过；`udp/off` 行 `20261013T211638Z-24df659d` 中 B 的 Status 报告 `held by meshway-runtime pid 1`（fixture 占住端口），释放后 3.3 秒绑定、`lastError` 清空、端口归 daemon。同一分支的前一次矩阵 `20261013T205702Z-9626e731` 中 `nat/resolved` 失败：A 在 access 撤销阶段 DNS owner 健康检查失败（`Backend`）后整体重装，之后 45 秒内首个 UDP 流无回显；此前通过的 `nat/resolved` 报告均无该 DNS 失败，listener 重试未出现，复跑通过，记为未定位的偶发。远程副本 `.meshway/validation/u1/remote/`。

`EADDRINUSE` 的占用者未在失败瞬间捕获（alpha.6）：采样显示失败前数毫秒同一 daemon 刚在 35000 绑定过一个 socket，失败约 200 ms 后已无进程持有该端口。后续（B2、H1）逐条核对了 Manager 的同端口改绑路径：首个 instance 到达的普通 listener → 协调 socket 改绑、内容变化与 Manager 重启都先 join 旧 owner（socket 同步关闭）再 bind；B2 前的接管路径和 H1 前“重试 owner 已绑定、未被接管的 socket 在 owner 退役后仍留在接管队列”两处会让新 bind 撞上本进程的旧 socket，均已改为先关闭再 bind，`manager::tests::netmap::rebind` 以真实同端口 bind 覆盖（旧顺序得到 `EADDRINUSE`）。alpha.8 常驻 lab 首次即绑定成功；alpha.6 那一次的确切占用者不再复现，重试仍是对其他进程占用的兜底。

### D3 系统 DNS 健康检查证据 {#d3-dns-health}

planck 2026-10-13 22:47:56Z、10-14 07:11:57Z、15:35:57Z、23:59:57Z 四次 DNS owner 阻断（前两次 `observe Backend` 后 `recover Conflict`，后两次只有 `observe Conflict`，均持续到进程重启）。四次都在宿主 WiFi（ath12k）重认证后的 PTK 重新协商结束时（3–82 ms 内）发生，间隔精确为 8 h 24 min，该处驱动的发送队列 flush 等满 5 秒；与宿主 CI、Docker 操作和 resolved 重启无关。原因链：宿主范围的 RTNL 占用使容器内全部 netlink 请求停顿，daemon 运行时停住，恢复时 2 秒 observe 期限（`Backend`）或锚点读取自己的 5 秒 netlink 期限（`Conflict`）已过；被中断的锚点读取把锚点 guard 永久结束，此后每次重证都以 `Conflict` 失败。证据 `.meshway/validation/d3/`。

本机 lab（OrbStack，三节点）以隔离容器拆除大量 dummy 接口制造宿主 RTNL 占用，复现与修复对照：

| 停顿 | main（alpha.10） | D3 |
| --- | --- | --- |
| 3.1 s／5.0 s | 三节点 `observe Backend` → `recover Conflict`，阻断至重启 | — |
| 6.5 s | lax `observe Backend`／`recover Conflict`；sha 另有出口 `audit` netlink 失败（10-13 22:47 签名）；nyc 只有 `observe Conflict`（15:35／23:59 签名）；均阻断 | 6.7 s：`check=observe_deadline`（`abandoned after 6677 ms`）或 `check=anchor_netlink`（`RTM_GETADDR` dump 失败），20 ms 内重证成功，约 0.6 s 内重装，三节点 running／ready |
| 12–13 s | — | 三节点 `observe_deadline` 后恢复；D3 第一版中一次 nyc 的 egress capture owner 校验读取失败即结束宿主内核归属（`anchor_ended`，阻断） |
| 6–7 s ×8（含 egress 修复） | — | 每次三节点 DNS 均自动恢复；lax 两次 `Egress capture verification unavailable` 后 `verified again; ownership kept`（6352 ms、12 ms），全部 running／ready |

其余候选（D3 构建）：`/etc/resolv.conf` 改为普通文件 4 秒 → `check=resolv_conf_mode`（`expected stub; found foreign`），Status `lastErrorDetail` 同文，改回后自动恢复；删除锚点 → monitor `anchor_link_changed`、DNS `resolved_link`（`Link 4 not known`）、`recover` 为 `anchor_ended`，保持阻断（归属真正丢失）；resolved 重启由 `nat/resolved` 的 `resolved-restart` 阶段覆盖；observe 与 apply 由同一 actor 任务串行执行，不会并发。单进程停顿（`SIGSTOP` 3 秒，8 次）在 main 上不触发失败：停顿期间内核已完成请求，不能代替 RTNL 占用。

egress capture owner 按同一不变量：回读失败只记为未知（30 秒上限），回读不一致或表消失才结束归属，规则见[源端 egress capture](/v2/node/runtime#源端-egress-capture-与切换)；`unreadable_verifications_keep_ownership_until_the_limit_and_a_change_ends_it`（失败若干次后一致→保持；不一致→结束；未知满 30 秒→`verify_unavailable`）、隔离测试 `verification_distinguishes_a_missing_rule_and_table_from_a_consistent_table`（一致→`Holds`；删除一条规则→`objects_changed`；删除表→`table_missing`）。

`dns_anchor_survives_suspend_and_owner_restart_until_explicit_removal` 新增：中断的读取与零期限读取后锚点不结束、下一次读取另开 socket 并成功（main 上 `an interrupted read ended the anchor`）。Planck `nat/resolved` 单行 `20261015T022035Z-05e74ab3`（行报告 `20261015T022219Z-b6110c78`，`ca9eec4f`）通过，含这两个锚点测试与 `resolved-restart` 阶段；含 egress 修复的 `fa23b64e` 复跑 `20261015T030829Z-3e88ac90`（行报告 `20261015T031141Z-1767bfe7`）通过。

### X11 出口写路径与出口审计／清理证据 {#x11-exit}

**写路径**：`exit/off` 第 16 阶段（优化 dev 构建，与 M4 同口径，前后交替各 5 次）出口正向中位数 890 → 1,764 Mbit/s，为 Mesh 正向的 0.45 → 0.86；本阶段 `Exit queue full` 告警 12 → 0 次（每次运行），压力步 767 → 1,685 Mbit/s 且 echo 100／100。数值与中间候选见[出口正向写路径](/v2/node/performance#exit-write-path)。

**审计读取失败与清理重试**：OPS11 真机 lab lax 12:06:53Z（alpha.11）与本机 lab 15:45:40Z（alpha.12，三节点同时）的签名相同：出口 owner 审计 `kernel netlink operation failed` 即停止角色，撤回时清理再失败，资源保留，此后每次探测 `ownershipConflict` 直到进程重启（本机 lab 的 lax、sha 持续 1 小时，nyc 的清理恰好成功而恢复）。修复后同一 lab（lax／sha／nyc，X11 构建）：

| 复现 | 结果 |
| --- | --- |
| lax CPU 限额 0.02 核 12 秒（`docker update --cpus`，使 `nft` 超出 2 秒预算，与宿主停顿时的失败形态相同） | `Exit role audit unavailable; retrying, the role and its flows kept` → 8.7 秒后 `verified again; ownership kept`（4 次失败），角色保持；main 构建的同一操作：DNS 健康检查失败撤回包平面，出口撤回失败，随后 egress capture `pause` 失败结束宿主内核归属（见剩余边界） |
| sha 限额 45 秒、nyc 限额 16 秒 | DNS 先失败撤回包平面，撤回出口角色时清理失败：`Exit role cleanup failed; resources kept, retrying`（1 s、2 s、4 s…），限额解除后第 7／第 4 次完成 `Exit role cleanup completed after retrying`；同时 egress capture `pause` 失败结束宿主内核归属，节点需重启（与出口无关，见剩余边界） |
| 宿主 RTNL 停顿（隔离容器拆除 17 万个 dummy 接口，约 8–11 秒，三次） | DNS `observe_deadline`／`anchor_netlink` 后恢复；出口审计读取未失败，三节点角色保持 |

组件测试（`exit/off` 组件阶段，同一隔离容器形态）：`linux::kernel::exit::tests::cleanup::exit_audit_read_failures_are_unknown_and_a_failed_cleanup_completes_on_retry`（注入 5 次读取失败→未知、角色保持，随后一致；外来元素→`flow_unowned`；清理第一次读取失败→不删除任何对象，重试完成移除；表已删除而未确认→下一次以重读确认完成；重新安装）与 `exit_recovery::netmap_exit_failed_cleanup_is_retried_and_the_role_installs_again`（角色链中外来规则：审计停止角色，清理在规则存在期间每次失败、`Host::exit_cleanup` 报 pending、安装新角色被拒；移除规则后清理完成、表消失、角色重装并确认一条 CT）。Planck 18 行矩阵 `20261015T195811Z-5b10ea68`（`d68cad6d`，并行 4）全部通过，`exit/off` 行 `20261015T200914Z-4b1882e0` 787.6 秒，含上述两项组件测试；并行 8 的 `20261015T193324Z-c16e4000` 中 netmap 三行因 Docker 地址池耗尽未开始，`exit/off` 第 16 阶段压力步中 E 一次投影退役的 nft 操作失败而停止角色（见剩余边界）。证据 `.meshway/validation/x11/`。

### X12 内核变更失败与本节点 boot 证据 {#x12-change}

**变更失败**：分类见[内核变更失败](/v2/node/runtime#kernel-change)。组件测试（`exit/off` 组件阶段 `changes-egress`／`changes-exit`，与 X11 组件同一隔离容器形态，另有本机 OrbStack 同形态运行）对每条变更路径各注入两种失败（测试专用工具故障：内核已应用而工具与下一次读取失败；内核未见到）：`linux::kernel::egress::tests::changes::failed_capture_changes_are_read_back_and_completed` 中 `stage` 已应用→目标、未应用→起点，`switch` 已应用→目标（其 stage 步）、未应用→起点，`pause`（switch 之后，带设备路由）两种都在起点路径上，六项均经重试完成且回读一致、无待提交门；之后外来删除一条规则→`objects_changed`→`verify` 不重试。`linux::kernel::exit::tests::changes::failed_exit_changes_are_read_back_and_completed`（真实 UDP echo、已确认 CT）中 `renew` 两种均续期成功，`retire` 两种重读为起点、重试后 CT 删除、journal 清空、审计一致，`maintain`（到期投影的 quarantine 写入失败）两种均完成；外来元素→`flow_unowned`→`verify`。另有纯逻辑测试 `verification::tests`（重读分类、1／2／4／8／16 秒退避与 30 秒上限、拒绝不重试、`stopped` 结束等待、`changePending` 发布与撤回）。

Planck 19 行矩阵 `20261016T051355Z-4b58fdf4`（`2861583a`，含 Manager 重建沿用 boot，并行 8，墙钟 1,073 秒）全部通过，`exit/off` 行 `20261016T052318Z-eaec7ac0` 组件十项通过；此前 `20261015T235403Z-98e9f030`（`17005fcd`，并行 8，墙钟 1,085 秒）全部通过；`exit/off` 行 `20261016T000237Z-fa4f6620`（804.7 秒）组件阶段十项全部通过，含 `changes-egress`（`stage-lost-target,stage-failing-start,switch-lost-target,pause-lost-start,switch-failing-start,pause-failing-start,foreign-rule-verify`）与 `changes-exit`（`renew-*-renewed`、`retire-*-start-completed`、`maintain-*-completed`、`foreign-element-verify`）；矩阵各行日志中没有变更重试（自然负载未触发）。

**lab 复现**（本机 OrbStack lab，X12 构建 release `76552504…`，X11 方法 `docker update --cpus 0.02`，`.meshway/validation/x12/lab/`）：

| 复现 | 结果 |
| --- | --- |
| sha 45 秒（`x12-throttle-3`） | DNS 失败撤回包平面，撤回时 `pause` 失败：`Egress capture change failed; read back, retrying`（`reread: unavailable` → 第 2 次 `start`）→ 23.8 秒后 `Egress capture change completed after retrying`（`attempts: 3`）；轮询期间 Status `changePending` `{"owner":"egressCapture","operation":"pause","attempts":3,…}`；恢复后一次 `pause` 失败重读为 `target`、548 ms 完成。宿主内核归属保持，三节点 running／ready，出口角色经清理重试后重装 |
| sha 45 秒（`x12-throttle-4`） | `pause` 失败一次、8.5 秒后完成；归属保持 |
| sha 45 秒（`x12-throttle-1`）、nyc 16 秒（`x12-throttle-2`） | 撤回在限额解除后发生，`pause` 未失败；`x12-throttle-1` 中出口审计连续 30 秒读不到而停止角色（见[剩余边界](#remaining-boundaries)），清理第 4 次完成后重装 |

X11 同一场景（main／X11 构建）：`Egress capture owner stopping … kernel ownership ends`，Host fail closed 直到重启。lab 结束后恢复为原 release `2d03609d…`（alpha.13），三节点 running、出口 ready。

**本节点换 boot**：网络 worker 的 Manager 重建（apply 失败后）就是 boot 变化的来源，现沿用原 boot：`manager::tests::netmap::rebind::configuration_changes_with_live_traversal_channels_keep_the_manager_and_its_boot`（traversal socket 与活通道上 3 轮配置变更：移除再加回本节点 listener、无关改名，每次 apply 成功、Manager 保持运行与原 boot；随后按 worker 的方式以原 boot 重建，对端的新通道读到同一 boot）。`netmap_lifecycle::own_new_boot_ends_every_circuit_at_once_and_its_close_reaches_the_far_end`（A→R→B）：A 首次绑定 boot、同一 boot 保留，换 boot 后 A 的 circuit 立即结束，`Close` 经 R 转发到 B，A 的下一个包立即发出新 INIT（1 ms，不等 15 秒）。R1 那次 apply 失败（`20261015T172722Z-c3cc3381`）：A 在一次 path 变化约 2 秒后 Manager apply 返回 `transport operation cancelled or timed out`（Manager 已停止或一次绑定超时），该构建尚无 `Connectivity owner ended unrequested` 日志，没有 `Datagram socket owner ended`，不能定位到具体 owner；此后的构建记录意外结束的 owner。R2 观察的 lab sha 配置变更（serial 34，alpha.13）同一签名：restart 分支只在 Manager 已停止（或失败早于发布新内容）时走到，`transport operation cancelled or timed out` 即 Manager 的根停止已被取消，即某个 owner 未经请求结束；该容器日志已随 lab 重新部署丢失，上述测试在测试 socket 工厂上未复现，具体 owner 仍未定位（下次由 WARN 的 `owner` 字段给出）。

### X13 宿主内核归属重证证据 {#x13-reproof}

规则见[宿主内核归属的结束与重证](/v2/node/runtime#ownership-end)。alpha.14 真机（OPS13，lab sha 限额 0.02 核 45 秒）3 次中 2 次：egress capture 回读连续 30 秒读不到 → `verify_unavailable` → 此后每次安装 `kernel ownership or routing protection conflict`、控制中继拒绝全部连接、DNS `anchor_ended`，直到 `docker restart`。原因是整个 Host 共用的一个 conflict token 被取消后再无更新途径，内核中没有外来对象，journal 也无异常。

隔离测试 `linux::kernel::tests::reproof::an_unavailable_ownership_end_is_proved_again_and_a_foreign_object_keeps_it_ended`（真实 Host、TUN、egress capture、控制中继；测试专用工具故障使全部 `nft` 读取失败）：30.9 秒后 `verify_unavailable`，期间安装被拒、无 TUN、中继不通；仍读不到时重证失败并保持暂态、下次时刻推后；恢复读取后重证成功，同一 installation、同一内容重装、同一 HTTP client 经中继再次连通，发布通道未关闭；再次暂态阻断后在 capture 块优先级内加一条外来规则，下一次重证读到它 → `conflict`／`reproof`，删掉规则后仍是冲突；新 Host（重启）沿用同一 journal；旧一期 monitor 在暂态阻断期间读到外来规则 → 直接 `conflict`／`monitor`。`exit_recovery::netmap_exit_failed_cleanup_is_retried_and_the_role_installs_again` 断言 `Host::exit_cleanups` 的现值在清理完成时即撤回。证据 `.meshway/validation/x13/local/`。

**lab 复现**（本机 OrbStack lab，X13 构建 release `231607ac…`，`docker update --cpus 0.02`，结束恢复 compose 原值 `--cpus 2`；Status 每 2 秒、nyc→sha ping 每 0.5 秒；`.meshway/validation/x13/lab/`）：

| 运行 | 限额 | 路径 | 限额解除 → 重证成功 → Status running／ready、ping 恢复 |
| --- | --- | --- | --- |
| `x13-1` | 60 s | X12 变更重试（`pause` 12.1 秒完成），归属保持 | 08:59:28.3 → —（未结束）→ 08:59:30（≤ 2 s），ping 08:59:28.5 |
| `x13-2` | 60 s | X12 变更重试（`pause` 25.9 秒完成），归属保持 | 09:02:34.3 → — → 09:02:36（≤ 2 s） |
| `x13-3` | 60 s | `verify_unavailable` 09:05:12.0，阻断期间第 1 次重证 `unavailable` | 09:05:38.3 → 09:05:38.78（0.5 s，第 2 次）→ 09:05:40（≤ 2 s），ping 09:05:39.1 |
| `x13-4` | 90 s | `verify_unavailable` 09:08:16.3 | 09:09:08.3 → 09:09:09.84（1.5 s，第 1 次）→ 09:09:10（≤ 2 s），ping 09:09:10.3 |
| `x13-5`、`x13-6` | 90 s | X12 变更重试（29.1／16.1 秒完成），归属保持 | ≤ 2 s |

6 次全部在限额解除后 2 秒内 running／ready，无需重启或 `up`；`exit.cleanupPending` 在 `Exit role cleanup completed after retrying` 之后的第一次轮询（≤ 2 s 轮询间隔）即消失（alpha.14：约 15 秒后）。60 秒限额下走哪条路径取决于 DNS 撤回时 `pause` 是否失败（失败时 owner 在变更重试中，周期回读不计时），90 秒使回读未知必然满 30 秒。lab 结束后恢复为原 release `cb647b05…`（alpha.14），三节点 running、CPU 为 compose 原值。

Planck 22 行矩阵 `20261016T101528Z-e959e64c`（`9a640af4`，并行 4，墙钟 1,471 秒）全部通过，`exit/off` 行 `20261016T102643Z-cf5618c7` 799.0 秒；矩阵各行日志中没有宿主内核归属结束（自然负载未触发）。同一提交的两次并行 8 矩阵各一行失败：`20261016T092812Z-67eae514` 的 `udp/resolved`（同机另一矩阵并发运行，三节点同时 `observe_deadline` 4.96 秒、Server 503，fixture 在 `outage-deadlines` 之后超时；单行复跑 `20261016T095226Z-a832ff54` 通过）；`20261016T095733Z-8ebca7ac` 的 `exit/off` 第 16 阶段 echo 94／100（压力步中 E 一次投影退役的 nft 操作失败、按 X12 重读重试 1 秒，与 X11 记录的并行 8 现象相同）。证据 `.meshway/validation/x13/`。

### X15 worker 忙循环与重证证据 {#x15-livelock}

规则见[网络 worker 的恢复优先级](/v2/node/runtime#ownership-end)。alpha.15 真机（OPS14，lab sha 限额 0.02 核 90 秒）3 次中 2 次限额解除后不自愈、sha meshwayd 约 110 % CPU：DNS owner 先自行重证（故障仍在、`recovered` 已为真，等待 hold），随后宿主内核归属暂态结束；worker 的 `biased` select 里 DNS 失败与恢复两个电平分支轮流立即就绪（恢复 → 重装在 capture 处因归属已结束失败、到不了 DNS hold → 失败 → …，约 37,000 次／秒），排在其后的到期重证永远选不到；`lastErrorDetail` 被 DNS 分支置空。

单测：`worker::lifecycle::tests::a_recovered_dns_fault_and_an_ended_term_reach_the_reproof_without_spinning`（DNS 已出错 → 已恢复 → 归属结束、1 秒后到期：恢复只处理一次，第 2 轮即选中重证，10 秒内、轮数上限 4）与 `…::a_due_reproof_is_selected_before_dns_facts_that_stay_ready`；DNS actor 测试断言已重证的故障不再是失败。把 `Health::failed` 改回电平语义时前者失败（`the worker spins`）。控制中继：`proxy::tests::a_relay_that_cannot_listen_retries_until_its_socket_is_free`、`…::a_relay_at_capacity_refuses_new_connections_and_keeps_current_ones`、`…::listener_retries_follow_the_u1_cadence`；分类：`kernel::tests::reproof::a_missed_drain_ends_ownership_as_unavailable`、`protection::tests::a_stopped_connection_records_an_unavailable_end_a_conflict_replaces`。证据 `.meshway/validation/x15/local/`。

**lab 复现**（本机 OrbStack lab，`docker update --cpus 0.02` 90 秒、恢复 `--cpus 2`；Status 每 2 秒、nyc→sha ping 每 0.5 秒、sha 日志实时跟随并按秒计四条循环消息、sha CPU 每 5 秒；`.meshway/validation/x15/lab/`）。第一轮（`b2955d06`）：

| 运行 | 路径 | 限额解除 → 重证成功 → running／ready |
| --- | --- | --- |
| `x15-1` | X12 `pause` 重试完成，DNS 自行恢复，归属保持 | 14:05:40.3 → — → 14:05:42 |
| `x15-2` | **OPS14 顺序**：DNS 14:07:29 失败、14:08:26.8 重证恢复，14:08:31.8 `change_unavailable` 结束归属；worker 处理恢复 1 次（重装在 capture 失败）后不再循环，重证 1 次 `unavailable` | 14:08:56.3 → 14:09:01.9（5.6 s）→ 14:09:03；四条循环消息全程 8 条（alpha.15：约 37,000 条／秒），sha CPU ≤ 14 % |
| `x15-3` | 出口审计 14:11:15 先停止角色，`verify_unavailable` 14:11:22.5 | 14:12:12.3 → 14:12:36.3（24 s）→ 14:12:38：worker 等一个排在已停止出口 owner 队列里、永不回答的探测 audit 73 秒，直到出口清理第 7 次完成 |
| `x15-4`、`x15-5` | `verify_unavailable`（`x15-4` 先 X12 `pause` 重试） | 4.1／0.8 s 重证，6／2 s running |

`x15-3` 另修（`49bf0c59`）：停止服务的出口 owner 立即丢弃已排队请求；失败的重证新一期丢下连接时不再记 `connection ended`。第二轮（`49bf0c59`）8 次：

| 运行 | 路径 | 限额解除 → 重证成功 → running／ready |
| --- | --- | --- |
| `x15-1`…`x15-4`、`x15-6`、`x15-7` | `verify_unavailable`，阻断期间重证 `unavailable` | 0.1／2.0／0.3／4.2／1.1／0.7 s 重证，≤ 2 s（`x15-4` 6 s）running |
| `x15-5`、`x15-8` | X12 `pause` 重试后 `verify_unavailable` | 0.5／0.8 s 重证，2 s running |

两轮 13 次中 12 次在限额解除后 10 秒内 running／ready（第一轮 `x15-3` 24 秒，原因已修，第二轮 8／8），无需重启或 `up`；四条循环消息每秒至多 4 条，限额解除后 sha CPU 至多 34 %。DNS 先恢复、归属后结束的顺序在 13 次中出现 1 次（第一轮 `x15-2`，修复后的构建上，单测覆盖同一顺序）。lab 结束后恢复为 alpha.15 release `136f3efc…`。

Planck 23 行矩阵 `20261016T150131Z-1f0563e0`（`0ecab400`，并行 8，墙钟 1,082 秒，同机无其他矩阵）全部通过。证据 `.meshway/validation/x15/`。

### S4 构建身份重编译与 DNS 故障边沿证据 {#s4}

规则见 [Netmap 设计 · Server 侧](/v2/node/netmap-design#server-侧) 与[网络 worker 的恢复优先级](/v2/node/runtime#ownership-end)。

**重编译**。OPS16（alpha.17 现场升级）lax 的已存 netmap（serial 38）由 alpha.16 编译、缺清除后保留的选择版本；Server 启动不重编译、续租只重签它，节点 CAS 连续 `version_conflict`。Server 单测（真实 App、加密存储、重开进程）：`app::tests::netmap::upgrade::a_start_by_another_build_recompiles_and_the_nodes_next_sync_receives_it`——同一构建重开不写任何 netmap／指纹行；换成“另一构建留下的”指纹与 `a` 的不同内容后重开，`a` 得到 serial +1 且 digest 等于本构建编译结果，`b` 保留 serial，`a` 持旧引用的 `sync` 立即收到新 serial；`node::selection::clear::a_start_by_another_build_restores_the_cleared_version_in_the_netmap` 按 OPS16 形状（清除后缺版本的旧 netmap）重开后 `selectionVersions` 恢复。Policy：`netmap::incremental::another_build_selects_every_node_and_unchanged_content_keeps_its_serial`。去掉启动重编译时两条 Server 测试失败。耗时（M3 Pro release，`mise run perf:netmap`）：1000 节点全量编译中位 2.37 s、同构建只算指纹 33 ms；6 节点 0.36／0.11 ms。证据 `.meshway/validation/s4/startup-cost/`。

**DNS 故障边沿**。OPS16 前 planck（alpha.16）23:03:54 实测 Status 为 `synchronizing`／`suspended`、`applied` null 而 Mesh 包照通，23:05 重启后消失；按日志与代码，该状态始于 17:59:57（16:49 的 OPS15 采样仍为 running）：日志 17:59:57.946 `DNS owner health check failed`（`observe_deadline`，`abandoned after 4490 ms`），7 ms 后 `DNS owner recovered its system installation`，没有 `Required DNS component failed`／`recovered`；22:29:56–22:30:10 的 502／ServerBusy 只经 Control 发布 `retrying` → `controlReady`（刷新 `observedAt`），无 Engine 调用。worker 的 `Health::failed` 把“已重证”的故障视为非失败，于是包平面未撤回也未重装，DNS 查询许可停在撤回、故障停在未清（Status 的安装不再 current）。lax 同时段日志已随 lab 重建丢失。单测 `worker::lifecycle::tests::a_fault_re_proved_before_the_plane_looked_is_one_failure_and_one_recovery`（未确认的已重证故障 → 失败 → 恢复 → 确认后不再失败 → 新故障再失败）与 DNS actor 的 `failed_owner_keeps_its_fault_while_it_retries_recovery_until_it_succeeds`（确认前已重证的故障仍是失败）；把 `Health::failed` 改回旧语义时两者失败，X15 的 `…without_spinning` 仍通过。证据 `.meshway/validation/s4/planck-evidence/`、`.meshway/validation/s4/unit/`。

Planck 23 行矩阵 `20261017T003157Z-9cccbf41`（`e7a01397`，并行 8，墙钟 1,099 秒，同机无其他矩阵）全部通过，无遗留容器／网络。证据 `.meshway/validation/s4/matrix/`。

### H4 撤回原因、重装日志、observedAt 与控制中继状态证据 {#h4}

规则见[撤回期间的原因](/v2/node/runtime#ownership-end)、[控制中继不结束归属](/v2/node/runtime#ownership-end)、Status 表的 `observedAt`／`controlObservedAt`／`controlRelay` 与[日志](/v2/logging#事件与字段)。

**根因（OPS17 lab，alpha.18，sha `--cpus 0.02` 30 秒）**。① 两轮都没有 `Required DNS component recovered`：撤回清理结束后，Control 在停顿期间收到的 offer 先于 DNS 恢复边沿被选中，以同一 netmap 重装并清掉故障，而日志只写在 DNS 恢复分支。② 撤回期间 `lastError` 为 null：worker 02:35:46.05 判定 DNS 失败后先 `reconfig(None)`、清理结束才发布撤回报告；清理等待 egress capture 的 pause 重试与出口退役共 31 秒（02:36:17.77 完成），其间网络报告仍是旧的 ready＋内容，DNS 故障使安装不再 current，Status 读作 `synchronizing`／`suspended`、原因 null（02:36:12–17 的读数）。并非报告被覆盖。③ `observedAt` 在安装不再 current 时取 Control 最后一次发布时间，停在故障前。

**修复后 lab**（lab release `c933b048…`，revision `54f188ea`＝本分支 `90a13fc0` 去掉 CLI 黑盒测试一行断言的改动，sha 30 秒 ×2，`.meshway/validation/h4/lab/`）：

| run | 限制／恢复 | Status 原因 | observedAt | 重装日志 | running |
| --- | --- | --- | --- | --- | --- |
| h4-1 | 03:46:27／03:46:57.1 | 03:46:27 起 `blocked`／`failed`／`dns` 带 `observe_deadline` 详情，03:46:51 起 `preparing`／`dns` 同一详情 | 03:46:29.580（撤回）→ 03:46:48.883（preparing）→ 03:46:57.294（running） | 03:46:57.29 `Required DNS component recovered; netmap reinstalled`（`trigger=offer`，`withdrawn_ms=27714`） | 03:46:58 |
| h4-2 | 03:49:13／03:49:43.3 | 03:49:13 `synchronizing`／`suspended`／`dns`（worker 尚未处理的故障，详情已在），此后同 h4-1 | 03:49:15.681（故障）→ 15.779（撤回）→ 40.782 → 43.560 | 03:49:43.56 同上（`withdrawn_ms=27781`） | 03:49:44 |

决定 1（phase 映射）后再跑一轮（lab release `215a58f6…`，revision `495dc00f`，`lab/decision1/`）：05:34:42 限制、05:35:13.07 恢复；05:34:43 起 `retrying`／`failed`／`dns` 带同一 `observe_deadline` 详情（CLI 标题 `retrying`），05:35:03 起 `synchronizing`／`preparing`／`dns`，05:35:13.16 `Required DNS component recovered; netmap reinstalled`（`trigger=offer`，`withdrawn_ms=27408`），05:35:13 running；不再出现 `blocked`。单测 `status::tests::a_transient_failure_retries_and_only_a_persistent_one_blocks`（DNS → `retrying`；持久 → `blocked`；出口清理重试中 → `retrying`），CLI `a_recovering_failed_plane_is_retrying_with_its_cause`。

两轮 `controlObservedAt` 都停在 03:44:58.533（Control 无变化），`observedAt` 只在三者变化时前进。中间轮次（`round1/`）暴露两处并已修复：清理因出口退役失败把原因改成无详情的 `underlay`（现保持最早原因，只有清理期间归属结束才改为 `underlay`）；DNS owner 先发布故障后发布详情，worker 在 22 µs 内读到空详情（现先发布详情）。`round0/` 为第一版。两轮中每次失败撤回恰一条 WARN、每次重装恰一条 INFO。

**控制中继**。Underlay 单测 `linux::proxy::tests::a_relay_that_cannot_listen_retries_until_its_socket_is_free`（发布 `retrying`、attempts 1、监听后清除）与 `…at_capacity_refuses_new_connections_and_keeps_current_ones`（`atCapacity` tasks／refused 1，接纳后清除）；隔离 NET_ADMIN 测试 `linux::tests::control_attachment_listens_once_a_held_socket_is_released`（进程启动时活 listener 占住路径：`attach_control` 成功、发布 `retrying`（`cannot listen: …`）、放开后 729 ms 经中继得到 HTTP 回答）。daemon 层（`.meshway/validation/h4/x14b/`，一次性容器）：占住 `underlay-bootstrap.sock` 后启动 meshwayd，IPC 就绪、Status `controlRelay.retrying`、CLI `Relay` 行，放开后下一次尝试回收并 `listening again`（attempts 5）；占位 socket 权限不是 `0600` 时从不回收、一直 `retrying`（`run-foreign-mode.txt`）。

Daemon 单测：`status::tests::observed_at_is_the_last_change_of_the_shown_state`、`…a_dns_fault_the_worker_has_not_reported_is_already_the_cause`；CLI `a_control_relay_that_does_not_serve_every_connection_is_shown`。Linux 无特权容器 `runtime::network`、`runtime::status`、`linux::proxy`、`linux::protection` 全部通过，X13 重证测试照常（`.meshway/validation/h4/local/`）。

Planck 23 行矩阵 `20261017T035656Z-d07a17c4`（`673c17be`，与本分支 `3076d3e9` 只差同一行测试断言，并行 8，墙钟 1,091 秒，同机无其他矩阵）全部通过。证据 `.meshway/validation/h4/matrix/`。

决定 1 之后的全量矩阵（同机无其他矩阵，每轮开始前 `docker ps` 只有常驻容器）：最终 `20261017T095424Z-df6f8adf`（`ec9374c7`，并行 8，墙钟 1,101 秒）23 行全部通过，证据 `.meshway/validation/h4/matrix13/`。此前各轮每轮一行失败、行各不相同，行启动 load 7–12：

| run | 失败行 | 签名 | 归因 |
| --- | --- | --- | --- |
| `20261017T053722Z-1a07d234`、`…060640Z-a914c818` | `roam/flap` | 结束时节点 `down` 超过 fixture 的 10 秒 | fixture：controller 在 `roam-complete` 即退出，`roam-down` 与 `roam-stop` 同时发出；节点的 Control 正有请求在飞时等其 10 秒连接超时。`20261017T091216Z-074139d9` 的 `roam/nat` 同一签名。已修：supervisor 等每个节点 `roam-down` 后才 `roam-stop`（至多 40 秒），controller 服务到 `roam-stop` |
| `…053722Z-1a07d234`、`…082502Z-2e0eefcf` | `nat/resolved` | 租约到期恢复后 `ping` 60 秒停在 `path_handshake`：R 等待 R→B 认证通道，B 的穿透 `prepare` 一直 `NotAvailable` | 中继侧通道获取；H4 未改动该路径。同一时段 main `f58def1b` 全量 `20261017T080104Z-70baa7e4` 全部通过 |
| `…064349Z-19673857` | `exit/off` | Docker `all predefined address pools have been fully subnetted` | 已知并发网络数（见 [roam/flap 行](#roam-flap-row)的 `20261015T233743Z-e3b061a6` 记录） |
| `…070252Z-9b56eb4d`、`…074258Z-6ebe6e29`、`…093221Z-b49150f4` | `exit/off` | 全局方案下新连接 3 秒内未经 E（circuit 3 秒才就绪）；echo 投影峰值 33（上限 32）；echo 299/300 | 高负载下的 fixture 计时与上限 |
| `…072236Z-ad684c7a`、`…084541Z-e7a66243`（并行 4） | `roam/relay` | `newFlowsThroughR`：0.6–0.75 秒的空窗内新流 0–1 个、未经 R 回复 | 空窗过短的门槛 |
| `…074258Z-6ebe6e29` | `nat/resolved` | 持续窗口初始 IPv4 UDP 无 echo | 高负载 |

这些签名都不经过 H4 改动的代码（Status 投影、失败撤回／重装的发布与日志、控制中继状态发布）。证据 `.meshway/validation/h4/matrix2/`–`matrix12/`、`matrix-main-baseline/`、`matrix4/`（修 fixture 前后 `roam/flap` 单行各 4／1 次通过）。

### 当前验证证据

`validate:matrix` 为每行独立保留报告并汇总 `matrix.json`／`summary.md`；通过结论同时要求 fixture `passed`、进程成功退出和本 run 无遗留资源。`--short` 的报告标记 `scope=diagnostic-short`：只运行 30 秒持续／纯接收窗口（退路 5 秒后触发），不要求已经跨原需求期限，未测到的延长为 null。其他场景仍执行，包括真实 instance 失效恢复；短模式不是整组验收，不能更新下述历史／当前通过状态。并发运行还需读取各行启动 load，不能与低负载串行结果不加区分地合并。

当前六个 runtime 行的 V2 证据使用候选 `f909e875`（基于 main `a52fb267`），机器为 Planck x86_64、目标 `x86_64-unknown-linux-musl`。四个原失败行均在两个独立完整 run 中通过；静态 UDP/off 与 TCP/off 另完成回归检查。两轮均为 `scope=acceptance`、125 秒持续／纯接收窗口，未使用短模式。入口记录 `uptime` 和 load，不等待负载门槛。

十份 runtime 报告的构建／导入／结束源码指纹均为 `a7b5bd38d268c763badd737a52b3363467f46b7967f9bea5db9d447bd7c6ab34`，二进制 hash 集合一致。所有必需检查通过，Controller／A／R／B 成功退出；每行精确清理与远程 supervisor 的独立容器／网络复核均无遗留。通过结论包含真实 STUN State、完整路径诊断、实际包流、回执、撤销恢复和旧输出拒绝，不能仅由 saved/applied 或候选非空推导。

TUN、packet 和表中 V1 的静态 UDP/resolved 证据来自 `3887256b` 生产源码与 `36a20fa3` 满 MTU fixture，使用 macOS/OrbStack ARM64 串行执行。其 packet/runtime 指纹为 `052e82e75417c17457c5e40951e2d1098147e2be56d3156f89a5cf465eb653c6`，TUN 专属输入指纹为 `477af3a17bc873c2346314c1dc27145230821df287fb91b22b6995932ce7b066`。汇总位于 `.meshway/validation/v1/20261007-main-3887256b/`；原始通过及失败 verdict 均保留。

### netmap 生命周期行 {#netmap-rows}

`netmap/offline` 与 `netmap/restart` 使用本机上报端点（`--reported-endpoints`，无 configured／STUN 端点），拨号候选只来自 peer 事实。`netmap/offline` 关闭 Node HTTP 3 分钟（三个 60 秒租约有效期，期间续租与过期判断都发生）；常驻 lab 另做 10 分钟离线复验。`netmap/restart` 关闭 HTTP 后先停止业务 125 秒（越过 120 秒按需邻接空闲关闭，也越过全部事实的新鲜度），A 的一次 `meshway ping` 与新 5-tuple 必须经空闲关闭后的重拨成功（`scenario-idle-redial-*` 证据），再重启三个 daemon，新邻接只能来自 `peer-facts.json` 中的最后已知事实，见[最后已知事实](/v2/node/connectivity#last-known-facts)。对照：同一 fixture 在未修复产品（main `83c462c7`）上 `netmap/restart` 未通过（`initial IPv4 UDP flow did not produce echo`；Planck `20261010T171854Z-8af9b94d`）。

#### 持续窗口锚点 {#traffic-anchor}

验收持续／纯接收窗口为 70 秒，由 netmap 常量决定而非历史 permit 期限：它长于一次 sync 续租周期（Server hold 50 秒＋最多 10% 抖动＋余量），且长于一次 60 秒租约有效期（各行使用 `--netmap-validity 60s`），因此同一 socket 必然跨过一次续租，并跨过其建立时所依据的租约截止。更长的窗口不增加新的状态转移。此前的 125 秒来自已删除的 demand permit 期限（60 秒）的两倍余量；2026-10-10 前的证据仍以 125 秒标注。目标是全量矩阵墙钟不超过 6 分钟。

| 候选 | 行 | 结果 |
| --- | --- | --- |
| `cde22065`（N1z-G） | 全量 16 行，Planck 矩阵 `20261010T182959Z-04119fe8` | 15 行通过（含四个 `netmap/*`：`20261010T183501Z-a323e57f`、`-6d74eb7e`、`-df693708`、`-b2efd34c`）；`exit/off` 未通过（E 的首个 State 报告被 400 拒绝，`prerequisite`，未复现） |
| `64056d2f`（同产品代码） | `exit/off` 单独复跑，矩阵 `20261010T193255Z-3c6bfa50` | 通过：`20261010T193827Z-8563eae8` |
| `717631ac`（F2） | 全量 16 行，Planck 矩阵 `20261010T235445Z-d7d66054`（并行 16，墙钟 390 秒，70 秒持续窗口，`netmap/offline` 3 分钟，`netmap/restart` 含空闲关闭后的离线重拨） | 16 行全部通过（`netmap/*`：`20261011T000016Z-74949d4b`、`-e271d31c`、`-473e6cca`、`-4033c3b4`） |
| `2f862f63`（P1） | 全量 16 行，GitHub Actions 于 Planck 运行 `20261011T054035Z-9a9dd74c`（并行 8） | 14 行通过；`exit/off` 在镜像 manifest 缺 `packages`（已修，见下一行）时立即失败；`nat+fallback/off` 证据阶段缺 TCP 退路 conntrack（runner 证据目录不可读，两次 Planck 复跑未复现，报错现已注明 router 与 dump 状态） |
| `d7de0cef`（P1） | 全量 16 行，Planck 矩阵 `20261011T061934Z-5187b512`（并行 8，墙钟 625 秒） | 16 行全部通过；`nat/off` 中 R 只接受 A 的 A–R offer（三次获取，均在 A prepare 后约 0.25 秒接受，R 未 prepare）；`nat+fallback/off` 两侧 router 各一条 ESTABLISHED `dport=35001` |
| `4923dff0`（F3 前段） | 全量 16 行，Planck 矩阵 `20261011T110213Z-a8b8aaf6` | 15 行通过；`nat+fallback/off`（`20261011T111342Z-b437eda9`）router-a 的 traffic 快照只有 TIME_WAIT 退路：router 每秒轮询 `traffic` 信号，controller 在信号后 0.65 秒撤销 Access，A–R TCP 退路随之按授权退役，router-a 快照晚于撤权 0.18 秒（router-r 早于撤权，为 ESTABLISHED）。P1 CI 的同一报错没有可读证据，无法单独确认，但与该时序一致 |
| `93b01c31`（F3） | 全量 16 行，Planck 矩阵 `20261011T121416Z-82e1063a`（并行 8，墙钟 601 秒） | 16 行全部通过；controller 在撤权前等待两个 router 的 `traffic-router-*` 快照会合，`nat+fallback/off`（`20261011T122216Z-42d55dfd`）两侧 router 均为同一条 ESTABLISHED `dport=35001` 流 |
| `c86321c2`（X5 前段） | 全量 16 行，Planck 矩阵 `20261011T140432Z-152583ab`（并行 8） | 15 行通过；`exit/off`（`20261011T141743Z-cf7bd61c`）DNS 阶段通过，随后 E 的租约在持续窗口内到期：DNS 阶段在 E 上增删路由模拟 resolver 断开，触发外部网络变化、控制连接重连，下一次 sync 长轮询（hold 最长约 55 秒）越过 60 秒租约。fixture 改为让 E 的上游 DNS 停止应答，不改 E 的路由 |
| `94a0b680`（X5） | 全量 16 行，Planck 矩阵 `20261011T143117Z-9dedf85e`（并行 8，墙钟 606 秒） | 16 行全部通过；`exit/off`（`20261011T143719Z-12d35cc0`）245.0 秒，F3 两次为 228.4／238.1 秒：DNS 阶段约 12 秒（其中 E 上游停止应答时 A 的查询失败约 10 秒），撤回／离线／到期／恢复各一次查询 |
| `b5644c06`（X5b 前段） | 全量 16 行，Planck 矩阵 `20261011T192758Z-85bc66e6`（并行 8，墙钟 603 秒） | 16 行全部通过；Server 的 sync hold 以节点持有的租约为界后，`exit/off`（`20261011T193721Z-49437239`，243.6 秒）恢复为在 E 上加删 unreachable 路由阻断其 DNS authority，E 的租约不再越过 |
| `e0449eae`（X5b） | 全量 16 行，Planck 矩阵 `20261011T195821Z-aa2222be`（并行 8，墙钟 640 秒） | 16 行全部通过；`exit/off`（`20261011T200533Z-24e26301`）256.4 秒 |
| `892113d5`（R2a 前段） | 全量 16 行，Planck 矩阵 `20261012T072359Z-a6533ad7`（并行 8） | 15 行通过；`netmap/expiry`（`20261012T072931Z-dd597b60`）R 报 `lease end left the TUN installed`：fixture 在 Status 去掉租约后立即检查 TUN，正落在 daemon 撤回的数十毫秒内，见[租约到期撤回](#lease-expiry-withdrawal)。同时出现的 503 `serverBusy` 是 fixture 关闭 HTTP 接纳的应答，不是 Server 负载 |
| `a6d7852b`（R2a） | 全量 16 行，Planck 矩阵 `20261012T075356Z-5ed32544`（并行 8，墙钟 582 秒） | 16 行全部通过（`netmap/*`：`20261012T075819Z-129899bf`、`20261012T075820Z-79b337bd`、`20261012T075825Z-bdc37377`、`20261012T075831Z-2bfa941f`；`exit/off` `20261012T075818Z-236d5371`）；此前 `netmap/expiry`＋`exit/off` 复跑 `20261012T074646Z-f70efd1a` 通过；证据 `.meshway/validation/r2a/` |
| `cccc2594`（X6） | 全量 16 行，Planck 矩阵 `20261012T101704Z-ae64c486`（并行 8，墙钟 910 秒，启动 load 约 16） | 16 行全部通过（`netmap/*`：`20261012T102857Z-08b3d1eb`、`-cd87dc7a`、`20261012T102901Z-219edb2b`、`20261012T102903Z-af5d254f`；`exit/off` `20261012T102856Z-a05feedb` 含第 10 阶段）；证据 `.meshway/validation/x6/` |
| `72144421`（R3） | 全量 16 行，Planck 矩阵 `20261012T105652Z-fcc0189b`（并行 8，墙钟 666 秒） | 16 行全部通过；`exit/off`（`20261012T110443Z-5d871fd5`，299.8 秒）A 由 7,000 区间 inline 集合计算并安装 7,002 个受管区间、7,040 条捕获表路由 |
| `879fff10`（R3＋X6 合并） | 全量 16 行，Planck 矩阵 `20261012T113016Z-e7abeae2` | 15 行通过；`exit/off`（`20261012T114014Z-f7d10cfa`）第 10 阶段通过，撤回恢复时 A 的内核监视器通知缓冲溢出（ENOBUFS，每批 256 条路由变更超出按缓冲计费的约 200 条），保护失效、恢复后 DNS 未回答；改为每 64 条等待后 `exit/off` 单行 `20261012T114921Z-806c1c28` 通过 |
| `c4018706`（R3＋X6） | 全量 16 行，Planck 矩阵 `20261012T115405Z-61e14403`（并行 8，墙钟 597 秒） | 16 行全部通过（`netmap/*`：`20261012T115835Z-4e0f7c87`、`20261012T115836Z-ec5cae2c`、`20261012T115843Z-7137264f`、`20261012T115845Z-6a853566`）；`exit/off`（`20261012T115835Z-1812d1b8`，265.5 秒，含第 10 阶段）；证据 `.meshway/validation/r3/matrix/` |
| `57dff7f5`（F4a 前段） | 全量 16 行，Planck 矩阵 `20261012T184725Z-20fb3fac`（并行 5，启动 load 约 13） | 15 行通过；`exit/off`（`20261012T190639Z-5db07be6`）第 10 阶段失败：关联到期后已建立流每包按新判定被丢弃（产品缺陷，由 F4a B3 修复） |
| `ecf5fe08`（F4a） | `exit/off` 单独复跑 `20261012T191840Z-522739fb`、`20261012T192852Z-d654e193`（启动 load 约 13／11，同时运行其他工作）；随后 `20261012T194315Z-fd9adf18` | 前两次未通过，失败与上行不同：`second named connection needed another Server round trip or failed`（第 10 阶段同名第二次连接）与 `exit lan UDP sequence 170: deadline has elapsed`（持续窗口 LAN UDP 单包超时）；第三次（load 约 4）通过，`exit/off` `20261012T194322Z-b990830c`。两次失败只在高负载下出现、未复现，不属于 `netmap/expiry` 的撤回时序 |
| `ecf5fe08`（F4a） | 全量 16 行，Planck 矩阵 `20261012T195053Z-bc48b6cc`（并行 8，墙钟 684 秒） | 16 行全部通过（`netmap/*`：`20261012T195514Z-dd968081`、`20261012T195514Z-bf4e7bdc`、`20261012T195522Z-3291ec0d`、`20261012T195522Z-f299ef78`；`exit/off` `20261012T195514Z-0965a9f8` 425.4 秒）；证据 `.meshway/validation/f4a/` |
| `4164e184`（R2d） | 全量 16 行，Planck 矩阵 `20261012T212926Z-abd9b9c8`（并行 8，墙钟 708 秒） | 16 行全部通过（`netmap/*`：`20261012T213906Z-80a410f5`、`20261012T213907Z-52fc69d3`、`20261012T213908Z-606792db`、`20261012T213919Z-8c87b3bf`；`exit/off` `20261012T213906Z-314988fd` 435.9 秒）；`netmap/expiry` 三个 daemon 在 Status 去掉租约时 TUN 均已删除（`scenario-expired-tun-removed-ms` 为 0）；证据 `.meshway/validation/r2d/` |
| `7cf62c52`（S3） | 全量 16 行，Planck 矩阵 `20261012T220402Z-33d41c74`（并行 8，墙钟 979 秒，开始负载 8.7–20.7） | 14 行通过，含 State 业务路径检查（A／R／B 各报告 A→R→B 及自身角色）；`exit/off`（`20261012T221422Z-665d0bf9`）第 10 阶段“first named connection did not leave through E”，`netmap/expiry`（`20261012T221429Z-4ea9cc44`）R 在租约到期时 TUN 尚未删除。两行复跑 `20261012T223042Z-07dc2864` 仍失败（`exit/off` 同一阶段；`netmap/expiry` 为 daemon-cleanup 测试 status 未收敛）；同一窗口以 main `1852b477` 复跑这两行 `20261012T224249Z-0dc077d7` 同样失败（`exit/off` 第 10 阶段“associated address did not follow its name rule”、`netmap/expiry` daemon-cleanup 未收敛），主机负载 12–24（其他租户矩阵并行），未归因于 S3 |
| `b4fe0619`（R2d） | 全量 16 行，Planck 矩阵 `20261012T221512Z-fd686a24`（并行 8，启动 load 13–19，同时运行其他工作流的矩阵）；两行复跑 `20261012T225154Z-d1dbb968`（load 12–24） | 14 行通过；`nat+fallback/off`、`nat/resolved` 在 fixture 建网时 Docker `all predefined address pools have been fully subnetted`，复跑时两行都在行前 `daemonCleanupTest`（`blocked_down_retries_real_dns_reservation_cleanup_without_going_online` 状态收敛超时）失败；均为共享主机负载／地址池，未进入产品阶段 |
| `7facc522`（C3） | 全量 16 行，Planck 矩阵 `20261012T224334Z-469128d0`（并行 8，墙钟 1,625 秒，启动 load 12–26） | 16 行全部通过；`exit/off`（`20261012T230753Z-f4b5dbb0`，751 秒）含第 11 阶段方案切换。此前单行 `20261012T215508Z-04ee3535` 切到 `global:E` 时 A 的 DNS 安装拒绝新 netmap（第 11 阶段改为 `exit-alt`，见[剩余边界](#remaining-boundaries)）；`20261012T220808Z-43169c17` 在宿主 load 21 下接入阶段超出行期限，复跑 `20261012T222615Z-3a71f2cb` 通过 |
| `b4fe0619`（R2d） | 全量 16 行，Planck 矩阵 `20261012T232408Z-75436f19`（并行 8，墙钟 912 秒，启动 load 5.4） | 16 行全部通过（`netmap/*`：`20261012T233115Z-501d0ae3`、`20261012T233117Z-e8cb6291`、`20261012T233120Z-363260bd`、`20261012T233127Z-f2c7f0b9`；`exit/off` `20261012T233115Z-04ff231d` 501.3 秒）；`netmap/expiry` 三个 daemon 的 `scenario-expired-tun-removed-ms` 为 0；证据 `.meshway/validation/r2d/` |
| `f2441b12`（C3 合入 main `7e07313e` 后） | 全量 16 行，Planck 矩阵 `20261013T000304Z-56e3679e`（并行 8，墙钟 847 秒） | 16 行全部通过；`exit/off`（`20261013T001400Z-28cda72a`，568 秒）含第 11 阶段 |
| `332ed126`（R2d 合入 main `7e07313e`，含 S3） | 全量 16 行，Planck 矩阵 `20261013T000021Z-6ab4b57c`（并行 8，行启动 load 8–16）；复跑 `20261013T002703Z-588fd38a`（load 3.5–16） | 首次 13 行通过：`nat+fallback/off`、`nat/resolved`、`netmap/offline` 中 A 的持续 UDP 在 instance 到期时中断超过 10 秒（A 的 A–R channel 因 scope 失效退役，重装等待一次 13 秒的穿透查询 RPC，经 fixture 503 接纳关闭后的控制中继 broken pipe 才返回）；复跑这三行通过，`exit/off`（`20261013T003322Z-30edd78d`）第 10 阶段 `first named connection did not leave through E`（与上行 S3 及其 main 对照同一签名）。两次都在其他工作流矩阵并行的高负载下，未在低负载复现 |
| `861d58d3`（R2d 合入 main `1f9f0442`，含 S3、C3） | 全量 16 行，Planck 矩阵 `20261013T013122Z-6b6cc50d`（并行 8，行启动 load 10–15） | 14 行通过；`exit/off`（`20261013T014519Z-170aaa14`）第 10 阶段同上签名；`netmap/revoke`（`20261013T014520Z-1e568deb`）A 在持续流开始时收到长度不符的 UDP 回显（`unexpected UDP echo length`），未复现 |
| `861d58d3`（同上） | 全量 16 行，Planck 矩阵 `20261013T015222Z-59ebfafc`（并行 8，墙钟约 800 秒，启动 load 7.2） | 16 行全部通过（`netmap/*`：`20261013T015704Z-a902d41d`、`20261013T015706Z-8ee141d1`、`20261013T015726Z-cf749863`、`20261013T015727Z-f77f06d8`；`exit/off` `20261013T015704Z-d47e6634` 491.1 秒，含第 11 阶段）；`netmap/expiry` 三个 daemon 的 `scenario-expired-tun-removed-ms` 为 0；证据 `.meshway/validation/r2d/` |
| `a8bd7eb3`（E1） | 全量 16 行 ×3，Planck 矩阵 `20261013T002917Z-b29b2dbd`／`20261013T010449Z-cec7e47f`／`20261013T012400Z-b319f63b`（并行 8，墙钟 823 秒起，开始负载 3–16） | 15／14／14 行通过。`exit/off` 含 ICMP 阶段（合并前编号 11，现为第 12 阶段）：`20261013T004350Z-bbe88ba0` 451.4 秒、`20261013T011243Z-62bc2868` 441.5 秒通过（F4a 为 425.4 秒，该阶段约 7 秒）；第三次在第 9 阶段读取 Resolver 查询日志早于其写入（取回的日志含该查询），复跑 `20261013T014606Z-da80439d` 通过。`netmap/expiry` 三次均为 B 或 A 在租约到期前已报 `suspended`（HTTP 关闭期间 503）而 TUN 尚在，与 S3 记录的同一现象一致，单行复跑 `20261013T005801Z-4859c4fa` 通过、与出口和 ICMP 无关；第二次 `udp/resolv-conf` 为 CLI 样本缺认证回复 channel；本机短模式 `exit/off` 矩阵 `20261013T001735Z-28a13a73` 通过（396 秒） |
| `6aaf979a`（E1 合入 main `1211538b`，含 S3、C3、R2d） | 三行 `exit/off`、`netmap/expiry`、`udp/resolv-conf`：Planck 矩阵 `20261013T021341Z-ba543719`（启动 load 6.5）；随后全量 16 行 `20261013T022738Z-789636f9`（并行 8，墙钟 898 秒，启动 load 6–8） | 三行通过（`exit/off` `20261013T021836Z-5bf70fb0`，493.0 秒，12 阶段）；全量 16 行全部通过，`exit/off`（`20261013T023637Z-8155b4dd`，518.3 秒）含第 11 阶段方案切换与第 12 阶段 ICMP 错误 |
| `45ec6e99`（N3 前段） | 全量 17 行（新增 `nat/relay`），Planck 矩阵 `20261013T053433Z-a688a9f6`（并行 8，墙钟 772 秒，启动 load 0.9–2.7） | 17 行全部通过（`nat/relay` `20261013T053955Z-8b43b4de` 146.1 秒） |
| `3e1142ee`（N3） | 全量 17 行，Planck 矩阵 `20261013T060913Z-86100acd`（并行 8，墙钟 766 秒，启动 load 1.6–2.7） | 17 行全部通过（`nat/relay` `20261013T061459Z-ba45e9f1` 150.7 秒；`exit/off` `20261013T061459Z-e26901ca` 491.8 秒）；证据 `.meshway/validation/n3/` |
| `1a5a1274`（N4） | 全量 17 行（`nat/relay` 为 5 节点），Planck 矩阵 `20261013T091200Z-d6fd0e7c`（并行 8，墙钟 753 秒，启动 load 2.0–5.9） | 17 行全部通过（`nat/relay` `20261013T092053Z-8fe259a9` 155.2 秒，R1 的节点 ID 在 R2 之前，提交后 1,185 ms 经 R2 回复）；证据 `.meshway/validation/n4/` |

| `db193112`（N5） | 全量 18 行（新增 `nat/reconnect`），Planck 矩阵 `20261013T135230Z-e8df696e`（并行 8，墙钟 803 秒，启动 load 2.7–5.0） | 18 行全部通过（`nat/reconnect` `20261013T135826Z-fe73e26e` 195.3 秒；`exit/off` `20261013T135832Z-7800f081` 514.4 秒） |
| `287e2385`（M1）、`1b8e8df2`（M1 合入 main `2ae855c4`，含 D2） | 全量 18 行，Planck 矩阵 `20261013T163418Z-ca02b918`（并行 8，墙钟 801 秒，启动 load 2.6–5.2）与 `20261013T170843Z-f7c75f4f`（墙钟 807 秒，启动 load 1.7–3.3） | 两次均 18 行全部通过；`exit/off` `20261013T164204Z-524400e3` 520.5 秒、`20261013T171640Z-150ac562` 532.4 秒，第 11 阶段切到 `global:E`、第 12 阶段窄 A–R 通道（`1400/1286`，MSS 1246，`TCP_MAXSEG` 1234，A 自己的 3/4 MTU 1286）；单行 `exit/off` `20261013T155956Z-2f06f59b` 527.0 秒；证据 `.meshway/validation/m1/` |
| `5183bf6e`（X7） | 全量 17 行，Planck 矩阵 `20261013T092932Z-0d781dd4`（并行 8，墙钟 778 秒，启动 load 1.2–2.9）；同产品行为的 `df591c3f` 矩阵 `20261013T090318Z-539d8762` 同样 17 行通过 | 17 行全部通过（`exit/off` `20261013T093730Z-4df28179` 504.3 秒，第 2 阶段 E 在 `ip_forward=0` 下 applied、打开 forwarding 后约 21 秒自动安装角色）；此前 `bede97a0` 矩阵 `20261013T071339Z-d4bdcc87` 16 行通过，`exit/off` 组件测试的投影目的 `8.8.8.8` 在无默认路由的隔离 namespace 中被新的逐流路由判定拒绝（测试改用 fixture 的外部服务并补拒绝断言），单行 `20261013T085053Z-f5b51954` 通过；证据 `.meshway/validation/x7/` |
| `8b6e952a`（AUTO-1） | 单行 `exit/off`，Planck 矩阵 `20261014T082132Z-2bce7aa7`（启动 load 7.2）；随后全量 18 行 `20261014T084353Z-cfeb8f5d`（并行 8，墙钟 864 秒，启动 load 0.9–5.7） | 单行通过（`20261014T082705Z-0e2cb3f6`，590.8 秒）；18 行全部通过（`exit/off` `20261014T084838Z-b54a09c8` 586.7 秒，含第 14 阶段：`auto` 依次解析为 `mesh`／`noTrustedRegion`、`exit`／`label`／US、`exit-alt`／`label`／JP、手动 `exit` 覆盖后回到 JP、`mesh`／`noRuleMatched`／FR；JP 切换后按名称建立的 TCP／UDP 流全部失败、旧关联消失；`routing.autoResolve` 五条）；证据 `.meshway/validation/auto1/` |

#### 租约到期撤回 {#lease-expiry-withdrawal}

`netmap/expiry` 关闭 Node HTTP（503）越过 60 秒租约。Status 的 `netmap` 按查询时的时钟判定租约（`now < expiresAt`），到截止即为 null、`dataPlane` 为 `suspended`；daemon 的 Engine 在同一截止由单调计时器触发 `expire`，依次撤回 connectivity、dataplane、DNS 与 underlay 并删除 TUN，耗时数十毫秒。因此 Status 先于 TUN 删除变化，属于预期时序，不是“到期未撤”。R2a 失败的 R（`20261012T072931Z-dd597b60`）：上次 sync 07:31:34.702，Connectivity 在 07:32:34.696 关闭，`Netmap lease ended; terminating the packet plane` 在 07:32:34.749，fixture 恰在这 53 毫秒内读到 null 并立即检查 TUN；S3 矩阵的 `netmap/expiry`（`20261012T221429Z-4ea9cc44`，fixture 修复前）是同一失败签名。行检查改为：Status 去掉租约后 2 秒内 TUN 必须删除并记录实际延迟（`scenario-expired-tun-removed-ms`），之后照旧检查拒绝路由与业务阻断。已排查：daemon 日志确认计时器在截止触发、撤回完成且无失败日志，A 同次在 07:32:34.818 撤回；503 来自 fixture 的接纳关闭，与 Server 负载无关。截止到撤回完成之间、worker 在撤回前处理中的至多一批（32 包）不逐包检查租约，这一窗口未单独测量。

### 远程矩阵证据 {#remote-matrix-evidence}

| V2 run | 并发 | 完整行 | 结果 |
| --- | ---: | --- | --- |
| `20261008T032743Z-897a549e` | 4 | NAT/off、NAT＋TCP 退路/off、NAT/resolved、UDP/resolv-conf、UDP/off、TCP/off | 六行全部通过；总耗时 1,044.5 秒 |
| `20261008T050147Z-0c5809cf` | 4 | 前四行独立复跑 | 四行全部通过；总耗时 566.6 秒 |

远程原件位于执行 clone 的 `.meshway/validation/matrix/<run>/`；本地原样归档位于仓库 `.meshway/validation/v2/20261008-nat-regressions/planck/<run>/`。每行路径为 `rows/<row>/<报告 ID>/report.json`，含 JSON、pcap、进程／清理信息与源码指纹；总报告保留 `matrix.json`、`summary.md`、`remote.json`。入口及进度说明见[开发环境](/v2/development#validation-matrix)。

### VLESS／Trojan TCP/TLS 证据

W2 使用候选 `fbde424e`（生产修复 `0593e19b`，基于 main `1271be5d`）在 Planck x86_64／`x86_64-unknown-linux-musl` 完成两轮独立全长运行：矩阵 `20261008T060724Z-15ece3bb`（495.7 秒）与 `20261008T061606Z-9875d228`（489.6 秒）。两轮各只选择 `vless/off`、`trojan/off`，并发上限为 4；四份行报告均为 `scope=acceptance`、125 秒持续／纯接收窗口，且必需检查全部通过。

四份报告的构建／导入／结束源码指纹一致：`e1ae2b2cba515e96db89569be6c881234a00f2de2961465c01bb0fc021728d6c`；`meshwayd` SHA-256 为 `4a2b7edced885f2266bf579de01c3ea92ff7f4ef6f2f323e2cc4cbcbbb338cd2`，完整二进制摘要集合也一致。Controller／A／R／B 均成功退出，R 的 TUN 明文交付为零，行清理及远程 supervisor 独立容器／网络复核均无遗留。配置证据确认 A/B 没有 listener、dialers 仅含所选协议，R 只有对应 listener 且 dialers 为空；相同材料的执行续接、HTTP 中断恢复、Access 撤销恢复、lease 失效及离线清理均经过共同场景。

四行短模式门槛（上述两行及 NAT/off、NAT＋TCP 退路/off）在矩阵 `20261008T055845Z-1b1b90ca` 全部通过，只作为诊断证据。W2 原始远程副本已归档至仓库根 `.meshway/w2-streaming/planck/<矩阵 ID>/`；远端保留 `/mnt/data/Meshway/src/.meshway/validation/matrix/<矩阵 ID>/` 的原件与二进制。TCP/TLS 的通过不扩展到 VLESS WS/TLS daemon 整组、TLS／WS 材料轮换或多路径切换。

### 剩余边界 {#remaining-boundaries}

本轮四个目标行没有剩余必需检查失败。**Planck 的静态 UDP/resolved 单独组合未在 V2 重验**：旧候选矩阵 `20261007T234146Z-c299a3b8` 与串行复跑 `20261008T002050Z-1d853e58`／行报告 `20261008T002332Z-de9c1b24` 曾在系统解析、ping 首样本或会合检查失败，不能由本轮 NAT/resolved 的通过替代。其他未验收范围见矩阵下方列表。

**planck 宿主 WiFi 的周期中断**（宿主问题，不在产品侧处理）：planck 宿主 ath12k 在每次 802.1X 重认证（约 8 h 24 min 一次）的 PTK 重新协商时 flush 约 5 秒，期间宿主范围的 netlink 停顿。D3 之后 planck 节点每次随之 DNS 健康检查失败、撤回包平面约 0.6 秒并自动重装，不再阻断至重启。 真机（alpha.11）2026-10-15 08:23:57Z 一次：宿主探针记录 4.11 s 停顿，planck 节点 `observe_deadline`（abandoned after 3918 ms）后 146 ms 恢复、279 ms 内重装 netmap，每 2 s 的 249 份 Status 中仅 1 份 `preparing`；证据 `.meshway/node-lab/ops10/d3-rekey/`。

**长于 30 秒的内核停顿仍 fail closed，停顿结束后自愈**（X13 之后）：变更失败与回读失败共用 30 秒上限（与 DNS owner 一致）。停顿期间每一次回读都失败满 30 秒时，出口审计照旧停止角色（lab sha 限额 45 秒一次：`operation: audit`、`check: verify_unavailable`，清理第 4 次完成后重装），egress capture 以暂态阻断结束宿主内核归属，Host 每 1–8 秒重证、停顿结束后自动重装（[X13](#x13-reproof)）；变更在起点反复失败满 30 秒为 `change_unavailable`，同样按暂态处理。控制中继监听失败与任务容量耗尽不结束归属，由中继自行重试／拒新保旧（X15，见[归属的结束与重证](/v2/node/runtime#ownership-end)）。X11 发现的 `pause` 失败（main 上停机到重启）与并行矩阵中出口 `retire` 超时停止角色已按[内核变更失败](/v2/node/runtime#kernel-change)处理，证据见 [X12](#x12-change)。

**节点持有的 peer 事实可早于其租约到期**（X8 发现，P1 已修复）：此前 Server 只在事实内容变化时唤醒 sync hold、hold 只以租约为界，而事实按报告者 `observedAt` + 60 秒到期，节点在 hold 期间或 Node HTTP 中断时可在租约到期前失去出口就绪事实，新出口流在本地被拒（`netmap_denied`）。现在 hold 也以节点持有事实的最早截止 − 5 秒为界（[sync hold](/v2/node/netmap-design#sync-transport)），且过期的出口就绪事实按最后已知值继续可选（[最后已知事实](/v2/node/connectivity#last-known-facts)）；`exit/off` 第 15 阶段等待 A 取得一次 sync 的绕过已去掉，第 8 阶段照旧断言 HTTP 中断期间直到租约到期前新出口流都能建立。P1 Planck 18 行矩阵 `20261015T062652Z-9a95285c`（`541e48b5`，并行 4）全部通过；单行 `exit/off` 复跑 `20261015T065227Z-9645d57c`（行 `20261015T065230Z-44e5de7b`）中 A 关于 E 的事实在首个路径租约到期前 7 秒过期，此后两条链路各 3 个新 UDP 出口流建立、无拒绝（main 上此时 `netmap_denied`）。同批另有两次失败：第 15 阶段 E 在角色重装进行中如实报告未就绪、A 据当前事实拒绝了一次出口 DNS 查询（`20261015T055930Z-782e79b3`；是 fixture 断言过严，第 15 阶段已改为中断窗界定，见[出口场景](#exit-scenario)）；两个 `exit/off` 并发时第 16 阶段 iperf3 超时（`20261015T065357Z-b98749cc`，与 P1 无关）。断言修改后（`f9115a91`）`exit/off` 单跑两次通过：`20261015T123419Z-c3f16022`（行 `20261015T123716Z-7753f918`，A 关于 E 的事实在首个租约到期前 3 秒过期，之后两条链路各 1 个新出口流成功）与 `20261015T125059Z-2a2f87b5`（行 `20261015T125103Z-83765480`，第 15 阶段中断窗 2.5 秒内 3 次查询失败 2 次、上限 7、窗外 0 次）。

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
- 内层双栈 UDP／TCP，包含上述满 MTU／MTU+1 UDP 和 16 KiB TCP echo；同一 socket 跨需求添加；至少一个[持续窗口](#traffic-anchor)（70 秒）的持续续租；独立 UDP 流停止发包后的入站活动续租。
- Access 撤销／恢复；中转取消 IPv6 地址后继续转发 IPv6 密文；拒绝路由保留；离线 `down` 清理。
- 控制中断恢复：关闭 Node HTTP 接纳直至实际 instance／execution 到期，再恢复联系；无需再次 `up`，原身份、进程和 operator revision 保持，取得新 instance、重新 applied 并上报 State，以新的 OS socket 恢复原路径包流，过期的旧 TCP／cipher 被拒绝。
- `meshway ping`：hostname／双栈 Mesh IP 选择、direct 与有序 via、独立 ICMP 权限、用户与 Node 共享任务及独立取消、SIGINT、控制中断后的订阅终态、新 instance 下重新探测。
- 源、终点的新交付计数及中转 TUN 的零明文交付分别记录。

单独的内核测试另检查同目的地址的普通／受保护 socket、旧池拒绝边界、journal 恢复、外部同优先级规则冲突，以及 HTTPS bootstrap 在显式 down 前后的联系。

容器重启边界由同一入口的两个独立 namespace（`underlay-restart-a`／`underlay-restart-b`）共享私有目录验证：A 写入真实负向 capture journal，B 在同 boot、新 netns 中归档原字节并重新建立、精确清理；B 随后把 A 的 journal 改写为 B 自身的 boot／device／inode（模拟 `docker stop`／`start` 复用 nsfs inode），必须因缺少驻留证据而归档并重新分配，而不是 Conflict。`daemon-owner-conflict` 使用真实 Control 签发快照及仍活着的 TUN owner，验证启动 Conflict、非空且递增的 `retryAt`，以及 owner 释放后无新 `up` 的自动恢复。`tools/meshway-runtime-fixture/src/recovery.rs` 的 HTTP 中断阶段保持同进程／同 namespace，以独立证明旧 lease 和输出失效；容器级 namespace 变体归上述内核／daemon 隔离场景，不由 HTTP gate 模拟。新增场景的整组验收以包含这些检查的行报告为准。

#### DNS 场景

```sh
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp \
  --dns-backend resolved --dns-image-manifest /absolute/private/os/image-manifest.json
```

覆盖真实 `getaddrinfo`、短名称、可见性 REFUSED、分域 UDP／TCP、系统 UDP→TCP 截断重试、进程重启、长时包流、失效阻断及 DNS／接口恢复。DNS 场景在测试 namespace 内使用私有 mount namespace 与私有 D-Bus／resolver，不改写 Docker 的 resolver bind 源；固定 OS 资产见 `dev/dns-fixture/README.md`。

#### 出口场景 {#exit-scenario}

```sh
mise run validate:matrix --rows exit/off --short
mise run validate:remote -- --ref <commit> --rows exit/off
```

`exit/off` 使用真实 Server／CLI／daemon 和独立身份的 A、R、E，仅 E 启用出口，仅 A 使用 Exit→E via R 的 profile。公网分类目标使用完全隔离 internal bridge 中的 `11.77.x.0/29`，另有显式授权的精确 RFC1918 LAN；不能把 TEST-NET 或 LAN 成功记为 Internet grant。三端均运行隔离的私有 D-Bus／resolved：A 的系统上游是 Resolver 容器（也是 A 的 `useWithExit` split 解析器；A 的方案以一个 7,000 个 `/20` 区间加该 /32 的 inline 集合规则把它定为 direct，A 由 artifact 计算并安装约 7,000 个受管区间），E 的系统上游是 external 网段内的隔离 DNS authority；配置开启 MagicDNS。Server 租约为 60 秒。

入口收集能力／前置条件、saved／applied 与 netmap 租约、投影（A 的 egress 规则与 fallback、三端相同的外部 Access、E 的 exit role、R 无 role）、A 实际安装的分割（egress journal 的计划、nft `managed4` 元素数与 IPC `netmap.capture` 三者一致且不少于 7,000，split /32 不受管，external 与 LAN 目的受管）、capture、原始与 NAT tuple、nft／conntrack、服务日志、R 密文、E 阻断对照、持续／仅接收窗口、`exitEnabled` 撤回、HTTP 中断与租约到期、同 namespace journal 恢复证据。HTTP 中断同时结束在途 sync 长轮询：此后三端不再取得租约，已有 TCP 与每两秒新建的 UDP 流在离线期间继续经出口往返，直至路径上首个租约到期才结束；E 在自身租约到期时撤回 role，owned nft 表与 conntrack 精确清理，A 保留负向 egress 门（`managed4` 非空、无活动 selector）；恢复 HTTP 后无第二次 up 取得新租约并以新 circuit 建立新流。投影 ACK、projection 容量拒绝等内部边界另运行同源码的正式组件测试；组件通过不能代替独立进程行通过。所有阶段（1–15）、进程退出与精确清理通过后该次行才通过。

第 2 阶段（出口前置条件，X7）：E 在 `ip_forward=0` 下启动，必须照常 running（applied 等于 saved、data plane ready），IPC 与 Server State 的 `exitCapability` 同为 `unavailable`／`ipForwardDisabled` 且没有安装角色；fixture 在 E 的 namespace 打开 forwarding 后不重启 daemon、不发新 netmap，E 的周期能力探测观测到 ready 后以同一内容安装角色（第 1、3 阶段：`ready`、`installedFamilies=[ipv4]`、applied 不变）。

第 9 阶段（DNS）：A 的计划为 `viaExit`（路径 A→R→E），E 为 `serveExitResolver` 且只服务 A；A 的 `split` 解析器决定为 `direct`。A 的系统 `getaddrinfo` 必须得到 E 的系统 resolver（resolved，上游为 external 网段内的隔离 DNS authority）的回答：A、双栈 A＋AAAA（AAAA 原样返回）、90 条 A 记录（超出 Mesh UDP 1232 bytes，只能经 Mesh TCP 路由完成）；authority 记录的唯一提问者是 E 的 external 地址。A 自己的 MagicDNS 名称本地解析，未知 MagicDNS 名称本地拒绝，两者都不出现在任何上游；`split.test` 名称只从 A 的 split 地址到达 Resolver。A 的系统上游就是 Resolver，它对任何名称都会给出标记地址：E 上加 unreachable 路由阻断其 DNS authority 时 A 的查询失败，Resolver 不收到该名称。E 的出口 role 撤回与 A 的租约到期后查询失败，HTTP 中断后的租约内查询成功，撤回恢复与租约恢复后查询再次成功。抓包中 `exit-dns` 字节只出现在 E 的 external 接口，A、R 与 E 的其他接口没有。

第 10 阶段（域名出口，X6）：在第 9 阶段的交换检查之后临时改为“名称规则 `*.name.exit-dns.test` → Exit E via R、其余 direct、Access 为 `domain:name.exit-dns.test`”。A 安装的捕获（egress journal）不含名称区地址，关联期间恰含该服务地址；不经名称直接连服务地址按 IP 规则 direct（A 无路由、服务无访问）；名称的第一次 `getaddrinfo` 一次即得到服务地址（E 的回答被 A 持有至签名关联应用），按名称的 TCP 连接只以 E 的地址到达服务；控制器计数到关联任务完成，同名第二次连接不增加；关联期间不经名称连该地址也按名称规则经 E；按名称建立、每 2 秒往返一次的 TCP 与 connected UDP 流持续 100 秒、越过关联 60 秒 TTL 的到期：到期后不经名称新连该地址失败（按 IP 规则 direct，A 无直连路径），A 的捕获仍含该地址，持有的 TCP 每次往返成功、UDP 至多丢 1 次、外部服务只见 E；流结束后该地址退出 A 的捕获；撤销 `domain:` 授权后按名称连接失败；恢复原配置后进入第 11 阶段。

第 11 阶段（方案切换，C3）：A 的 assignment 允许 `exit`（默认）、`exit-alt`（与 `exit` 规则相同的另一方案）、`mesh` 与 `global:E`。`meshway exit --json` 由 A 的 netmap 给出四个方案，人读输出保存为 `a/exit-view.txt`。A 上每秒一次往返的 TCP 与 connected UDP 流经 E 建立后，`meshway exit exit-alt` 在 Server 提交、A 应用带该版本的 netmap 后返回（`applied=true`、本机版本等于提交版本）；切换后该 socket 的 TCP 与 UDP 往返全部失败（旧选择的流随新 selection 结束），新连接经 E 到达服务，A 的捕获仍为默认捕获。`meshway exit off` 后 A 的捕获无受管区间、无默认捕获，连外部服务失败且服务不见新的 E 访问（`a/exit-off.txt`）；`meshway exit exit` 恢复后进入第 5 阶段。之后 `meshway exit global:E`：A 应用、保持 running（`useWithExit` split 解析器的地址是 A 分割的 direct 例外，A 的捕获为默认捕获但不含它），新连接经 E 到达服务，split 名称仍只由 A 自己的 split 地址问到 Resolver（M1；此前 A 的 DNS 安装以 “DNS resolver plan is incomplete” 拒绝该 netmap、停在 synchronizing）。

第 14 阶段（自动方案，AUTO-1，[自动方案](/v2/configuration-reference/routing#auto)）：在第 11 阶段之后临时改为“`exit` 与 `exit-alt` 各加一条名称规则 `*.name.exit-dns.test` → Exit E via R、fallback 为 direct；`auto` 为 US → `exit`、JP → `exit-alt`、fallback `mesh`；A 的 assignment 加 `auto` 并以它为默认”，A 的地区依次为未声明、US、JP、FR，由控制器经配置提交写入。A 无到外部服务的直连路径，因此连接只能经 E 的关联到达服务。① 未声明：`meshway exit auto` 后 `--json` 的 `auto` 为 `{profile: mesh, basis: noTrustedRegion, region: null}`，连外部服务失败且服务不见 E 的访问。② US：`{exit, label, US}`；E 回答的名称得到关联后，按名称与不经名称的连接都只以 E 的地址到达服务（E 从自己的投影重算 A 的 selection，否则握手失败），A 的捕获含该地址。③ 按名称建立、每秒往返的 TCP 与 connected UDP 流经 E 后改为 JP：A 应用后 `{exit-alt, label, JP}`，该流的 TCP／UDP 往返全部失败，不经名称连该地址失败（旧 generation 的关联已删除），流结束后该地址退出 A 的捕获；新名称经 E 到达服务。④ `meshway exit exit` 覆盖 `auto`（`auto` 为 null），`meshway exit auto` 回到 `{exit-alt, label, JP}`。⑤ FR：`{mesh, noRuleMatched, FR}`，A 的捕获不含该地址，连外部服务失败。⑥ `meshway exit exit` 后 `meshway exit --default`：持久选择清除（`requestedProfile` 为 null、`version` 保留为清除所用的版本、`cleared=true`），回到默认 `auto` 的 `{mesh, noRuleMatched, FR}`。Server 的 `routing.autoResolve` 活动（actor 为 system）依次为 `mesh`、`exit`、`exit-alt`、`exit-alt`、`mesh`、`mesh`，每次解析变化恰一条、手动覆盖没有；`meshway exit exit` 并恢复原配置后进入第 5 阶段。证据 `auto-profile.json`、`controller/auto-activities.json`。

第 12 阶段（ICMP 错误，E1）：拓扑另有 router 容器（只开 IPv4 转发，无 Meshway 进程），连接 `external` 网段与第二个公网分类网段 `far`；external 服务的 far 地址经 E → router → far 到达，router 的 far 接口 MTU 为 1280（其余 1500，服务自己的接口不变，因此 TCP 对端通告的 MSS 不限制 A）。E 上临时加一条 far 网段经 router 的路由，阶段结束删除。A 的原始 ICMP socket 记录 A 收到的全部 ICMPv4 错误。`traceroute -I`（每跳 1 个、默认并发探测、等待 2 秒）必须给出第 1 跳为 E 的地址（E 本机 time exceeded）、第 2 跳为 router、第 3 跳为目标，且后续跳 RTT 比第 1 跳多出不超过 20 ms（各探测的出口投影同轮安装）；64 KiB 的 TCP echo 经 router 的 fragmentation needed 收敛并完整返回，A 的路由缓存为 `mtu 1280`，`ping -M do -s 1300` 失败、`-s 1252` 成功，A 记录到来自 router 的 3/4、MTU 1280 错误；A 保持一条到 external 服务的 UDP 流时，router 伪造 5 条引用不存在 tuple 的 time exceeded（E 的 conntrack 无关联，内核丢弃）与 30 条引用该流 NAT 后 tuple 的错误：A 只收到后者，且在每流每秒 10 条的预算之内（至少 1 条）。数据面的引用匹配、来源规则与限速另由 `meshway-dataplane` 的流表、报文与 A→R→E 运行测试覆盖；伪造错误被数据面拒绝的计数（`related_counters`）不经 IPC 暴露，行内以 A 的接收结果为准。随后（M1，[通道 MTU](/v2/node/dataplane#channel-mtu)）把 A–R 链路两端 MTU 设为 1400：A 的 `status` 中到 R 的通道变为 `1400/1286`；A 的 MSS 表限为 1246；经 E 的 64 KiB TCP echo 不限 MSS 也完整返回（A 的 `TCP_MAXSEG` ≤ 1246，服务回包不超过 A–R 通道）；`ping -M do -s 1259` 失败并报告 `Frag needed … (mtu = 1286)`，A 的原始 socket 记录到来自目标地址的 3/4、MTU 1286 错误（A 自己的数据面合成），路由缓存 `mtu 1286`，`-s 1258` 成功；结束恢复 1500 并清空路由缓存。

第 15 阶段（echo 寿命、容量与出口角色恢复，X8；`dev/exit-fixture/echo.py`）：在第 12 阶段之后。① 容量：A 对 external 服务地址 `ping -i 0.2` 60 秒（300 个请求），同时每 2 秒一次 `traceroute -I -q 1 -w 2 -m 5`；每 0.5 秒读取 E 的出口 journal，投影数峰值不超过 32（容量 128 的四分之一），A 收到全部 300 个回复，traceroute 全部到达目标，E 的角色始终已安装。② CT 换代（OPS8 的触发）：A 以 0.5 秒间隔 ping 40 次，其间 E 两次 `conntrack -D -p icmp -s <A>` 删除该 echo 连接的条目（如 ICMP conntrack 超时），后一序号新建条目；回复至多少于删除次数，E 的角色保持安装，E 的日志没有新的 `Exit owner stopping`。③ 恢复：A 持续 ping 时删除 E 出口回程表中 A 的 `/32` 路由：E 的日志出现 `Exit role failed; withdrawing it alone`，角色在 25 秒窗口内重装，窗口内 E 每次采样都 running、data plane ready、applied 非空、sync connected 无错误，A 的 MagicDNS（本地）与普通名称（经 R 到 E 的 resolver）每 0.5 秒各解析一次：MagicDNS 全部成功；普通名称只允许在角色中断窗内失败——E 撤回角色时如实报告未就绪，A 按这一当前事实拒绝经 E 的出口 DNS 查询是正确的 fail closed，不是缺陷。中断窗用 E 自己的时间戳界定：起点为删除路由后 E 日志的 `Exit-owned kernel object removed outside its owner; stopping the exit role`，终点为 E 的 Status 中重装后第一份就绪能力（`observedAt` 晚于起点）所在 State 报告被确认（E 日志 `State candidate publication acknowledged`）再加 1 秒（Server 对内容变化立即回答 A 的 hold、A 应用该事实）。与窗有交集的查询可失败，失败次数不超过 `ceil(窗长 / 0.5 s) + 1`（相邻查询起点至少相隔 0.5 秒）；窗外的普通名称查询 100% 成功。窗、窗内查询数、失败数与上限记录在 `echo-capacity.json` 的 `recovery.dnsOutage`。A 的 ping 在重装后恢复。组件测试 `exit_recovery::netmap_exit_echo_reconnect_and_route_loss_stop_only_the_role` 在同一隔离容器形态下覆盖：CT 换代后旧序号退役与新序号安装不冲突、删除回程路由只停角色且清理精确、Host 的新 serial 照常安装、角色重装后投影可用。证据 `echo-capacity.json`、`components/recovery.log`。

第 16 阶段（出口吞吐与队列压力，X10；`dev/exit-fixture/throughput.py`）：在第 15 阶段之后。① A 对 E 的 Mesh 地址（同一 A→R→E 通道，Access `tcp:5201`、Mesh 路由经 R）与经 E 对 external 服务各跑 iperf3 TCP 20 秒，正向与反向（`-R`，反向即 E 或 external 发送、回程经 E 的 NAT 与出口 TUN）；记录接收速率、发送方重传与 A／E 的链路、SNMP、qdisc 计数，不作断言（数值属于宿主；矩阵为 debug 构建，只比较同一行内的比例）。② A 经 E 的一条 iperf3 TCP 流持续 20 秒，同时 A 以 0.2 秒间隔 ping 同一服务（每个 echo 请求是一条新出口流，E 在 TCP 流旁不断安装与退役投影）：每个 1 秒区间都有数据（不低于 0.1 Mbit/s），echo 回复不少于 95 %，E 的角色保持安装。抓包在本阶段前停止、之后以第二段续抓（各段的包数上限不被本阶段耗尽），第 7／9 阶段的检查读取全部分段。证据 `exit-throughput.json`。

当前状态：**当前 wire 已通过（含 DNS、域名出口阶段、跨关联到期的持有流、ICMP 错误与 PMTU 和 7,000 区间节点侧分割）**。X10 候选 `d4a79440`（出口队列溢出只丢包、内核操作不阻塞已安装流、回包批量与反压）：Planck 18 行矩阵 `20261015T035310Z-a73ab52d`（并行 4）全部通过，`exit/off` 行 `20261015T040519Z-94a728f4` 795.7 秒；第 16 阶段（debug 构建，Mbit/s，括号内为重传）Mesh 正向 36.9（0）、反向 44.6（16），出口正向 43.0（0）、反向 63.3（259），出口反向为 Mesh 反向的 1.42 倍；echo 安装下的出口 TCP 流 42.5（890），最低 1 秒区间 32.5，echo 100／100，无 `Exit queue full`；第 12 阶段 traceroute 三跳同一轮（增量 ≤ 0.005 ms）；第 15 阶段 300／300、峰值 27。main 产品代码上的同一阶段（`20261015T013949Z-847d86d7`）：出口反向 19.3（28），为 Mesh 反向 44.7 的 0.43；压力步第 2 秒起断流到结束（平均 1.0），E 的该流投影退役后同 tuple 的重传一律被拒。中间候选的失败：一轮把其后到达的安装挡在外（traceroute 增量 52／124 ms，`20261015T015512Z-07f6c942`）；等待项无操作时重新挂起而自旋（出口 TCP echo 第 112、113 个序号超时，`20261015T022901Z-4a752da1`、`20261015T024247Z-5aa3c241`）；等待项逐个开轮，排在一次退役之后的 traceroute 新流各自一轮（增量 52／110 ms，`20261015T033956Z-9ec02085`），现以重放并入下一轮修复。证据 `.meshway/validation/x10/`。H2 候选 `3a47c644`（方案选择清除、中继按整条链排序）：Planck 18 行矩阵 `20261014T210943Z-8bc0b498`（并行 4）全部通过，`exit/off` 行 `20261014T212105Z-5e1031c4` 592.0 秒，第 14 阶段 ⑥ 清除后为 `{mesh, noRuleMatched, FR}`、`routing.autoResolve` 共 6 条；第 12 阶段改为等待 A 每秒一次的 MSS clamp 刷新（H1 记为偶发的“还没有 MSS clamp 表”是 fixture 在刷新之前读表，同一失败在 `20261014T193015Z-255c88b7` 复现）。同一提交的其他矩阵失败为同机并行矩阵耗尽 Docker 地址池（`all predefined address pools have been fully subnetted`，三次）与一次第 10 阶段首个名称连接未经 E（`20261014T202904Z-1f0503dc`，同一提交另两次通过，未定位），证据 `.meshway/validation/h2/`。X8 候选（合入 H2 后）`909f2fd6`：Planck 18 行矩阵 `20261014T233036Z-e7000147` 全部通过（启动 load 1.3–4.0），`exit/off` 行 `20261014T233523Z-9bb7fd1e` 607.5 秒：第 15 阶段 300／300、投影峰值 27，CT 换代 40／40，删除回程路由后 0.92 秒内重装，ping 59／60。合入后的两次失败：组件测试的前提（重建的条目换 id）一次未成立——重建的条目占用释放的槽位时 id 相同，测试改为占用槽位并至多重复 5 次（`20261014T224036Z-ff16c760`）；第 8 阶段 HTTP 中断内新出口流在 A 的租约到期前约 20 秒被拒（`20261014T224410Z-50ef86a6`、`20261014T230212Z-3e940135`）：A 判定新出口流依据它持有的 E 的事实，事实的新鲜度只随 sync 刷新（Server 不为续期唤醒 hold），第 15 阶段恢复后 A 仍持有恢复前报告的事实，其到期早于 A 的租约；第 15 阶段改为等 A 在重装之后取得一次 sync 再结束。事实到期可早于消费者租约是现有行为，见下文剩余边界。X8 合入前的候选 `09bc15ad`：Planck 18 行矩阵 `20261014T220407Z-2c791342` 全部通过，`exit/off` 行 `20261014T221147Z-ccff6958` 695.7 秒：第 15 阶段 300／300、投影峰值 26，CT 换代 40／40，删除回程路由后约 0.5 秒只撤角色并在 0.92 秒内重装，ping 58／60。main 产品代码上的同一阶段：容量步 120／300、峰值 128（`20261014T185116Z-6c337811`）；CT 换代 50 秒后的 `maintain` 冲突重现 OPS8 全链（`Exit retirement failed` → Mesh TUN 被关闭 → `unexpected_del_route table=<underlay> prefix=48` → 控制中继与 DNS 失败，`20261014T200754Z-13ebca51`）。首个 X8 候选（无出口 cipher `Close`）在 CT 换代步只收到 10／40：A 一条出口未接纳的流使 A 一端的 circuit 保持，E 在最后一条 echo 流结束时退役 cipher，A 在其上等满 15 秒无回复（`20261014T201805Z-9a1900c0`），由出口 cipher 结束时向来源发 `Close` 修复。证据 `.meshway/validation/x8/`。H1 候选 `8ca58fe3`（含 AUTO-1）：Planck 18 行矩阵 `20261014T094553Z-37e2f01a` 全部通过，`exit/off` 行 `20261014T095018Z-9d788324` 580.7 秒，第 12 阶段 traceroute 三跳 68.70／68.94／69.28 ms（后续跳增量 ≤ 0.58 ms，断言 ≤ 20 ms；H1 之前 `20261014T081712Z-f3eb3cf8` 为 66.19／118.82／173.16 ms），组件测试一轮 8 条投影 54.4 ms、逐条 8 条 520.6 ms；同一候选的前一次矩阵 `20261014T092614Z-e970ee30` 中 `exit/off` 在第 12 阶段窄通道检查失败（A 的通道已到 1400/1286 时还没有 MSS clamp 表，traceroute 增量 0.25 ms 已通过），复跑通过，记为未定位的偶发，证据 `.meshway/validation/h1/`。E1 候选 `a8bd7eb3`：`exit/off` 行 `20261013T011243Z-62bc2868` 441.5 秒，ICMP 阶段（现为第 12 阶段）traceroute 依次为 E 的 Mesh 地址、router、目标，64 KiB TCP echo 经 router 的 10 条 fragmentation needed（MTU 1280）收敛（MSS 1228，A 缓存 `mtu 1280`），30 条注入错误到达 10 条（每流每秒预算），引用不存在流的 5 条没有到达；此前没有缓存例外修复的候选在第 8 阶段失败（A 的 egress owner 把该 PMTU 例外当作外来路由），证据 `.meshway/validation/e1/`。常驻 lab（E1 程序与 Server）：lax 以 `geoip:cn` → Exit sha（方案与公共规则两种形态），`traceroute -I 1.2.4.8` 显示 sha 的 Mesh 地址、docker 网关与其后各公网跳直至目标；公共规则形态下 sha 自身的 netmap 规则动作为 `direct`、DNS `serveExitResolver`、经 eth0 直出，路径检测 sha → `direct`、lax → Exit sha；真实路径约 1,356 字节的 MTU 在 sha 直出时也无 fragmentation needed（黑洞，与 Meshway 无关），lax 经 sha 的行为相同；（H1 之前）每个新 echo 流需一次单独的出口投影安装，经出口的 echo RTT 比 sha 直出高约 120–150 ms，默认等待 2–3 秒的 traceroute 在约 12 跳后超时（`-w 6` 可完整到达）；H1 起同一轮到达的投影合并安装，常驻 lab 尚未按 H1 重测；配置按 E1 键 CAS 移除恢复，证据 `.meshway/node-lab/e1/`。F4a 审阅修复候选 `c90d91af` 之后（含持有流检查与数据面保留判定）：Planck 16 行矩阵 `20261012T195053Z-bc48b6cc` 全部通过，`exit/off` 行 `20261012T195514Z-0965a9f8` 425.4 秒；此前第一次带持有流检查的矩阵 `20261012T184725Z-20fb3fac` 在第 10 阶段失败（关联到期后已建立流每包按新判定被丢弃），证据 `.meshway/validation/f4a/`。R3 候选 `72144421`：Planck 矩阵 `20261012T105652Z-fcc0189b` 行 `20261012T110443Z-5d871fd5`，299.8 秒（A 安装 7,002 个受管区间、7,040 条捕获表路由）（合入 X6 前）。R3＋X6 合并后（候选 `c4018706`）：Planck 16 行矩阵 `20261012T115405Z-61e14403` 全部通过，`exit/off` 行 `20261012T115835Z-1812d1b8` 265.5 秒，第 10 阶段由 A 安装的捕获证明关联地址进入受管区间。此前：X6 候选 `cccc2594`：Planck 16 行矩阵 `20261012T101704Z-ae64c486` 全部通过（并行 8，墙钟 910 秒），`exit/off` 行 `20261012T102856Z-a05feedb` 445.0 秒，第一次名称解析 0.29 秒、一次任务完成、第二次连接零完成；本机短模式 `20261012T101111Z-bcc3f73b` 通过（另一次短模式 `20261012T100612Z-0c20b833` 在域名阶段之后的第 5 阶段 `docker exec cat` 失败，未复现）；证据 `.meshway/validation/x6/`。常驻 lab（X6 候选程序与 Server）：lax 以 `geosite:openai` → Exit nyc via sha、其余 direct，`curl` api.openai.com 的 SYN 只在 lax TUN 与 nyc 出口 TUN／eth0、example.com 只在 lax eth0；openai.com 与 `172-64-154-211.nip.io`（direct 名称规则）共享的地址连接被丢弃、openai.com 的另一地址经 nyc；配置以原 JSONC CAS 恢复，lab 回到 main 程序，证据 `.meshway/node-lab/x6/`。此前（X5）：Planck 矩阵 `20261011T195821Z-aa2222be`（候选 `e0449eae`）行 `20261011T200533Z-24e26301`，256.4 秒；此前 `20261011T143117Z-9dedf85e`（`94a0b680`）。此前不含 DNS 阶段的全长证据：Planck 单行 `20261010T160621Z-de9a2fae`、矩阵 `20261010T162437Z-4bbe5e35` 行 `20261010T162742Z-e9ed64a0`。IPv6 出口、源／Access／成员撤销逐项、切网、文件后端出口 resolver 与常驻 lab 整组仍未覆盖。常驻 lab 局部检查（不替代整组）：2026-10-11 以 X5 候选临时配置 lax 经 Exit→nyc via sha，lax 的 `dig`／`getent` 由 nyc 的 resolved 发往其上游（nyc 抓包），lax 与 sha 的非 loopback 抓包没有该名称明文，阻断 nyc 的上游 DNS 时 lax 查询失败；配置已 CAS 恢复原内容、lab 已恢复 main 程序，私有证据 `.meshway/node-lab/x5/`。X5b 候选（`b5644c06` 程序）另做一次出口 resolver 不可用的真实触发：nyc 的 resolved 在 eth0 加入 Mesh 地址作 DNS 服务器后 0.5 秒内 nyc `meshway status` 的 `exit.dnsUnavailable=meshServer`、Server State 的 nyc `exitDns` 为 `unavailable`／`meshServer`，lax 的查询 SERVFAIL，nyc 自身的 MagicDNS 与系统解析不受影响；`resolvectl revert` 后恢复 `serving`、lax 再次解析。期间 dev Server 临时换为同数据目录的 X5b 构建（旧 Server 拒绝新增的 `exitDns` 字段），结束后换回 main 构建，配置以 rev 55 原 JSONC 源文 CAS 恢复（rev 60），证据 `.meshway/node-lab/x5/x5b/`。


#### exit6/off 行 {#exit6-off}

```sh
mise run validate:remote -- --ref <commit> --rows exit6/off
```

`exit/off` 的同一 Server／daemon／fixture，拓扑改为双栈：external 与 far 各取 `3ffe::/16` 内随机 /64（在 `2000::/3` 内且不在任何排除表中，是 Internet 分类地址；不用文档前缀），LAN 取随机 ULA /64（经显式 CIDR Access），router 与 E 在自己的 namespace 内开 IPv6 forwarding，服务另在 IPv6 上监听（`bindv6only`）；Mesh 控制与数据链路仍为 IPv4。E 的 `exitFamilies` 为 `["ipv4","ipv6"]`；R 另为 `exitFamilies` 默认的第二个出口（方案 `exit-ipv4`）。阶段编号沿用 [IPv6 出口设计](/v2/node/ipv6-exit-design) 的 `exit6/off` 表；第 9 阶段（出口前缀变化）属 I6-2，不运行；不跑 `exit/off` 的组件阶段（同一构建由 `exit/off` 覆盖）。

| 阶段 | 断言 |
| --- | --- |
| 1 能力 | E 的 acquire 有 `feature:exit-ipv4`／`feature:exit-ipv6` v1；IPC `exit.capability.families` 两族 `ready`、`installed`；E 的角色 `families` 两族、R 只有 IPv4；A 的投影 `eligibleExits` 带 E 的两族；A 的 egress journal 中 external 与 LAN IPv6 目的在 `managedIpv6`、不在 `sinkIpv6`，`ip -6 route get` 指向 Mesh TUN |
| 2 前置条件 | 一条 IPv4 出口长流在整个阶段不断；E 的 `all.forwarding=0` → 只有 IPv6 族 `ipv6ForwardDisabled`，恢复后回 `ready` 且角色不重装（同一 `installedRole`）；E 外部接口 `accept_ra=1` 加一条 `proto ra` 默认路由 → `ipv6RouterAdvertisementsIgnored`，撤去后回 `ready` |
| 3 包流 | A 的 IPv6 TCP、UDP echo、`ping -6` 到 external，TCP 到 ULA LAN；服务看到的来源是 E 的 external（LAN 为 E 的 LAN）IPv6 地址；E 有带 mark 的 IPv6 conntrack |
| 4 PMTU | far 链路 1280：64 KiB TCP echo 收敛，`ping -6 -M do` 失败且 A 收到 router 的 packet too big 1280、路由缓存 `mtu 1280`；E 外部接口调为 1300 时 E 宿主自己的 packet too big 到达 A；`traceroute -6 -I` 第 1 跳为 E、第 2 跳 router、第 3 跳目标 |
| 5 MSS | A–R 链路 1400（通道 `inner` 1286）：A 的 MSS 表有经出口的 IPv6 规则（`inner` − 60），IPv6 TCP echo 的 MSS 不超过它；链路 1380（`inner` < 1280）：A 的 IPv6 出口 `connect` 失败并收到本机 ICMPv6 1/0 |
| 6 撤回 | `exitEnabled` 撤回后 IPv6 长流终止、E 不再有带 mark 的 IPv6 conntrack、journal 的 quarantine 含 IPv6 回程 tuple；恢复后重装的角色从 journal 把 `quarantine6`／`echo_quarantine6` 装回，新 IPv6 流成功 |
| 7 只有 IPv4 的出口 | E 的 IPv6 族运行时不可用：A 对 IPv6 目的 `connect` 在 1 秒内以 `EACCES` 失败，同时 IPv4 `connect` 成功，`meshway ping` 判定 `exitUnavailable`；切到 R（只 IPv4）：A 的 IPv6 目的在 `sinkIpv6`，判定 `unsupportedFamily`，`connect` 立即被拒（宿主无 IPv6 路由时为 `ENETUNREACH`） |
| 8 DNS 与关联 | 名称规则经 E（fallback direct）：名称区的 AAAA 经 E resolver 原样返回，关联把该 IPv6 地址加入 A 的 `managedIpv6`，A 只用 IPv6 按名称连接经 E 成功；撤权与恢复 |
| 10 方案切换 | 切到 `mesh`：A 没有受管 IPv6，切换后 IPv6 出口长流不再有回复；切回 |

最后 `down` 恢复三端 namespace 基线（地址、两族路由与规则）、外来 sentinel 不变、没有出口表／带 mark 的 IPv4 conntrack，A 的物理接口抓包中没有这些 IPv6 目的的明文包。

首次运行暴露并修复的缺陷（同一提交序列）：IPv6 程序的回执被 decode 拒收（`ip6` 族）、nft 对 IPv6 版本半字节与 /128 前缀的列出形式、捕获表 `mesh6` 被当作静态集合（源端捕获冲突）、IPv6 地址分配取到池的 Subnet-Router anycast 地址（`::`，出口不对它发 ICMPv6 错误）、`NetmapDelta` 缺 `eligibleExits`（见 [changelog](/v2/changelog)）。隔离 NET_ADMIN 容器（arm64，Ubuntu 24.04 nft 1.0.9）另跑通 `exit_nft_high_mark_prefix_preserves_exact_program_ownership`（IPv4 与 IPv4＋IPv6 程序）、`managed_ipv6_capture_installs_reads_back_and_retires`、`exit_ipv6_program_rules_and_routes_install_and_leave_nothing`、`ctnetlink_exact_delete_reports_absent_and_keeps_a_replacement`（含 IPv6 UDP／TCP／ICMPv6）。

未验收：真实宿主的 IPv6 出口（it7、Planck 都没有全局 IPv6，见 [IPv6 出口设计 · 真机现状](/v2/node/ipv6-exit-design#real-hosts)）；出口前缀变化、临时地址与 `roam/exit6`（I6-2）；IPv6 的租约到期全撤与外部迟到回包被 quarantine 拒绝的显式注入（IPv4 由 `exit/off` 覆盖，同一实现）。

Planck 24 行矩阵 `20261017T213601Z-935b9b88`（`752673fe`，并行 8，墙钟 1,477 秒，同机无其他矩阵）全部通过；此前同一分支 `20261017T103344Z-bab11368`（`c798153a`）与 `20261017T211436Z-522aba47` 中 `exit/off`／`exit6/off` 创建网络失败（`all predefined address pools have been fully subnetted`），矩阵随后按 bridge 预算调度（[矩阵入口](/v2/development#validation-matrix)），前一次另有 `udp/resolv-conf`、`roam/flap`、`roam/exit` 期限失败，在后两次矩阵中通过。证据 `.meshway/validation/i61/`。

#### 穿透场景

```sh
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --traversal nat \
  --traversal-image-manifest /absolute/private/os/image-manifest.json
```

- 两个独立 IPv4 端口保持型 Linux conntrack SNAT；UDP 发布端点为空，STUN、互探及认证使用实际承载 socket。
- 内层双栈 UDP／TCP 强制沿 A→R→B，持续与纯接收业务跨越一个[持续窗口](#traffic-anchor)（70 秒，含一次续租与一次 60 秒租约截止）。
- `nat/off`（无退路、DNS off）中 A–R 是按需的 NAT–NAT 边：R 另有仅 Native TCP 的拨号器、A 另有不公告的 TCP listener（经 A 的 NAT 不可达），因此两端都能拨出、无一端常驻，只有 A 有经 R 的业务。R 不得 prepare 任何 A–R 尝试，只能经 `traversalRequests` 信令接受 A 的 offer，见[穿透信令](/v2/node/connectivity#traversal-signalling)；R–B 仍为 B→R 常驻边。
- `--traversal-fallback` 仅预授权 A→R 的 Native TCP 拨号，R→B 保持 Native UDP；TCP 入口初始不可达，业务持续 30 秒后开放该入口并阻断精确 A–R UDP 地址对；原 TCP socket、两个内层 TCP circuit 的 CID 与完整路径跨切换保留。
- State、双方接受的固定 offer、双侧抓包、conntrack、防火墙计数和应用回包共同构成证据。拓扑与固定 OS 资产见 `dev/traversal-fixture/README.md`。
