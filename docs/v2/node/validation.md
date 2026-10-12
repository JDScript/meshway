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

可执行行 ID 来自 `scripts/validate_matrix.py` 的 `ROWS`：`tun`、`packet`、`udp/off`、`tcp/off`、`vless/off`、`trojan/off`、`udp/resolv-conf`、`udp/resolved`、`nat/off`、`nat+fallback/off`、`nat/resolved`，顺序对应下表十一个隔离入口。常驻 lab 不属于一次性矩阵。统一运行、并发预算与远程证据位置见[矩阵入口](/v2/development#validation-matrix)。

默认矩阵另含 `exit/off`（真实 A→R→E IPv4 出口与单出口 DNS）与四个 `netmap/*` 生命周期行（见[netmap 生命周期行](#netmap-rows)）；出口详见[出口场景](#exit-scenario)。

| 入口 | 承载 | DNS | NAT | 覆盖行为 | 状态 |
| --- | --- | --- | --- | --- | --- |
| `test:tun:linux` | — | — | — | 真实 TUN 设备创建与双栈包 I/O | 当前 wire 已通过；`20261007T212324Z-904a137e` |
| `test:packet:linux` | Native TCP | off | — | A→R→B 双栈小包／满 MTU UDP、MTU+1 拒绝、daemon 多 MSS TCP、R 只转发受保护帧、原路径回复 | 当前 wire 已通过；`20261007T212330Z-f7e9a2e9` |
| `test:runtime:linux --transport native-udp --reported-endpoints` | Native UDP 本机上报端点（无 configured／STUN） | off | — | daemon 整组：下文[共同场景](#runtime-共同场景) | W5 全长通过；Planck `20261008T185416Z-08dce814`；历史静态端点：`20261008T033613Z-abbd4545` |
| `test:runtime:linux` | Native TCP | off | — | daemon 整组：共同场景 | W5 全长通过；Planck `20261008T185416Z-9a4a35f1` |
| `test:runtime:linux --transport vless` | VLESS TCP/TLS | off | — | 共同场景；仅 R listener、A/B 拨入，精确叶证书验证 | W5 全长通过；Planck `20261008T185416Z-b31bfda5` |
| `test:runtime:linux --transport trojan` | Trojan TCP/TLS | off | — | 共同场景；仅 R listener、A/B 拨入，精确叶证书验证 | 当前 wire 已通过（两次）；Planck `20261008T060730Z-58ae84d3`、`20261008T061612Z-c9edd384` |
| `test:runtime:linux` | Native UDP 静态端点 | resolv-conf | — | 共同场景＋[DNS 场景](#dns-场景) | 当前 wire 已通过（两次）；Planck `20261008T032749Z-3ba0ca12`、`20261008T050153Z-a1a8eebf` |
| `test:runtime:linux` | Native UDP 静态端点 | resolved | — | 共同场景＋DNS 场景 | V1 串行：当前 wire 已通过，`20261007T220446Z-5360cb6e`；Planck 旧候选未通过，本轮未重验，见[剩余边界](#remaining-boundaries) |
| `test:runtime:linux --transport native-udp --traversal nat` | Native UDP 协调穿透 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋[穿透场景](#穿透场景) | W5 全长通过；Planck `20261008T185416Z-cabbce4c` |
| `test:runtime:linux --transport native-udp --traversal nat --traversal-fallback` | Native UDP＋A–R Native TCP 退路 | off | 双 IPv4 端口保持型 SNAT | 共同场景＋穿透场景＋同路径 TCP 退路 | W5 短模式通过：`20261008T184559Z-c77e0683`；既有全长证据：`20261008T032749Z-4299bfde`、`20261008T050153Z-d30caa20` |
| `test:runtime:linux --traversal nat` | Native UDP 协调穿透 | resolved | 双 IPv4 端口保持型 SNAT | 整组组合 | 当前 wire 已通过（两次）；Planck `20261008T032749Z-d13bd2a9`、`20261008T050153Z-1978b05b` |
| [常驻三节点 lab](/v2/node/docker-lab) | Native UDP 显式端点 | resolved | — | 六向双栈系统解析／echo、路径诊断、关闭插桩的同路径 TCP 吞吐对照 | 未验收（当前整组）；2026-10-07 已有下述 L5 局部证据；共享 bridge 不构成 NAT 证明 |
| `validate:matrix --rows exit/off` | Native UDP；声明 Native TCP 能力 | 三端 resolved 后端（MagicDNS＋一个 `useWithExit` split） | IPv4 出口 masquerade | A→R→E；能力／安装、公网与 LAN、长流／仅接收、撤回、HTTP 中断内离线出口与租约到期全撤／恢复、journal；单出口 DNS（第 9 阶段）；域名出口与可信关联（第 10 阶段）；A 的 7,000 区间节点侧分割 | 当前 wire 已通过（含 DNS、域名出口阶段与节点侧分割）；R3＋X6 矩阵 `20261012T115405Z-61e14403` 行 `20261012T115835Z-1812d1b8`；X6 Planck 矩阵 `20261012T101704Z-ae64c486` 行 `20261012T102856Z-a05feedb`，见[出口场景](#exit-scenario) |

以下范围未验收：外层 IPv6 NAT、端点相关映射、映射轮换、NAT 与 resolv-conf 的组合、VLESS WS/TLS 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换、文件后端（resolv-conf）出口 resolver、resolved 代理 stub 缺失（systemd < 248 或 `DNSStubListener=no`）时的出口 resolver 不可用（仅组件测试）、`dns.overrideLocal` 出口 resolver（仅组件测试）、显式多出口 DNS plan、出口 IPv6、完整 TCP conntrack、动态 MTU／路径 MTU 发现及分片重组、userspace／SOCKS5。整机重启与 resolved 停机间隙不属于隔离矩阵，由系统服务安装单独实测：重启（含 Server 不可达）通过，systemd-resolved 重启后 5 秒内、停止再启动后 10 秒内恢复 Mesh 名称解析，见 [Linux 安装与系统服务](/v2/install#验收记录)；`udp/resolved` 行另在隔离 namespace 内重启 resolved 并要求 5 秒内再次解析 Mesh 名称。

常驻 lab 的 L5 原始证据目录为 `.meshway/perf/l5/20261007T162418Z-main-f39ba2b6-2cpu/`。该目录的两次 TCP 接收吞吐分别约 318／364 Mbps；协调者另报告空闲宿主复测 1,367 Mbps，独立报告路径待补，不能把该数值归入前述目录或用吞吐局部证据替代整组验收。

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
| `72144421`（R3） | 全量 16 行，Planck 矩阵 `20261012T105652Z-fcc0189b`（并行 8，墙钟 666 秒） | 16 行全部通过；`exit/off`（`20261012T110443Z-5d871fd5`，299.8 秒）A 由 7,000 区间 inline 集合计算并安装 7,002 个受管区间、7,040 条捕获表路由 |
| `879fff10`（R3＋X6 合并） | 全量 16 行，Planck 矩阵 `20261012T113016Z-e7abeae2` | 15 行通过；`exit/off`（`20261012T114014Z-f7d10cfa`）第 10 阶段通过，撤回恢复时 A 的内核监视器通知缓冲溢出（ENOBUFS，每批 256 条路由变更超出按缓冲计费的约 200 条），保护失效、恢复后 DNS 未回答；改为每 64 条等待后 `exit/off` 单行 `20261012T114921Z-806c1c28` 通过 |
| `c4018706`（R3＋X6） | 全量 16 行，Planck 矩阵 `20261012T115405Z-61e14403`（并行 8，墙钟 597 秒） | 16 行全部通过；`exit/off`（`20261012T115835Z-1812d1b8`，265.5 秒，含第 10 阶段） |
| `7cf62c52`（S3） | 全量 16 行，Planck 矩阵 `20261012T220402Z-33d41c74`（并行 8，墙钟 979 秒，开始负载 8.7–20.7） | 14 行通过，含 State 业务路径检查（A／R／B 各报告 A→R→B 及自身角色）；`exit/off`（`20261012T221422Z-665d0bf9`）第 10 阶段“first named connection did not leave through E”，`netmap/expiry`（`20261012T221429Z-4ea9cc44`）R 在租约到期时 TUN 尚未删除。两行复跑 `20261012T223042Z-07dc2864` 仍失败（`exit/off` 同一阶段；`netmap/expiry` 为 daemon-cleanup 测试 status 未收敛）；同一窗口以 main `1852b477` 复跑这两行 `20261012T224249Z-0dc077d7` 同样失败（`exit/off` 第 10 阶段“associated address did not follow its name rule”、`netmap/expiry` daemon-cleanup 未收敛），主机负载 12–24（其他租户矩阵并行），未归因于 S3 |
| `7facc522`（C3） | 全量 16 行，Planck 矩阵 `20261012T224334Z-469128d0`（并行 8，墙钟 1,625 秒，启动 load 12–26） | 16 行全部通过；`exit/off`（`20261012T230753Z-f4b5dbb0`，751 秒）含第 11 阶段方案切换。此前单行 `20261012T215508Z-04ee3535` 切到 `global:E` 时 A 的 DNS 安装拒绝新 netmap（第 11 阶段改为 `exit-alt`，见[剩余边界](#remaining-boundaries)）；`20261012T220808Z-43169c17` 在宿主 load 21 下接入阶段超出行期限，复跑 `20261012T222615Z-3a71f2cb` 通过 |

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

**方案切到 `global:<exit>` 与 `useWithExit` split resolver 的组合不可用**：`exit/off` 配置下 A 切到 `global:E` 后，Server 提交并下发了新 netmap，但 A 的 DNS 安装以 “DNS resolver plan is incomplete” 拒绝（split 解析器地址在 global 下归出口捕获），netmap 不能 applied，A 停在 `synchronizing` 直至切回（单行 `20261012T215508Z-04ee3535`）。属于 DNS 计划与出口动作的组合语义，未在 C3 修改。

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

入口收集能力／前置条件、saved／applied 与 netmap 租约、投影（A 的 egress 规则与 fallback、三端相同的外部 Access、E 的 exit role、R 无 role）、A 实际安装的分割（egress journal 的计划、nft `managed4` 元素数与 IPC `netmap.capture` 三者一致且不少于 7,000，split /32 不受管，external 与 LAN 目的受管）、capture、原始与 NAT tuple、nft／conntrack、服务日志、R 密文、E 阻断对照、持续／仅接收窗口、`exitEnabled` 撤回、HTTP 中断与租约到期、同 namespace journal 恢复证据。HTTP 中断同时结束在途 sync 长轮询：此后三端不再取得租约，已有 TCP 与每两秒新建的 UDP 流在离线期间继续经出口往返，直至路径上首个租约到期才结束；E 在自身租约到期时撤回 role，owned nft 表与 conntrack 精确清理，A 保留负向 egress 门（`managed4` 非空、无活动 selector）；恢复 HTTP 后无第二次 up 取得新租约并以新 circuit 建立新流。投影 ACK、projection 容量拒绝等内部边界另运行同源码的正式组件测试；组件通过不能代替独立进程行通过。所有阶段（1–11）、进程退出与精确清理通过后该次行才通过。

第 9 阶段（DNS）：A 的计划为 `viaExit`（路径 A→R→E），E 为 `serveExitResolver` 且只服务 A；A 的 `split` 解析器决定为 `direct`。A 的系统 `getaddrinfo` 必须得到 E 的系统 resolver（resolved，上游为 external 网段内的隔离 DNS authority）的回答：A、双栈 A＋AAAA（AAAA 原样返回）、90 条 A 记录（超出 Mesh UDP 1232 bytes，只能经 Mesh TCP 路由完成）；authority 记录的唯一提问者是 E 的 external 地址。A 自己的 MagicDNS 名称本地解析，未知 MagicDNS 名称本地拒绝，两者都不出现在任何上游；`split.test` 名称只从 A 的 split 地址到达 Resolver。A 的系统上游就是 Resolver，它对任何名称都会给出标记地址：E 上加 unreachable 路由阻断其 DNS authority 时 A 的查询失败，Resolver 不收到该名称。E 的出口 role 撤回与 A 的租约到期后查询失败，HTTP 中断后的租约内查询成功，撤回恢复与租约恢复后查询再次成功。抓包中 `exit-dns` 字节只出现在 E 的 external 接口，A、R 与 E 的其他接口没有。

第 10 阶段（域名出口，X6）：在第 9 阶段的交换检查之后临时改为“名称规则 `*.name.exit-dns.test` → Exit E via R、其余 direct、Access 为 `domain:name.exit-dns.test`”。A 安装的捕获（egress journal）不含名称区地址，关联期间恰含该服务地址；不经名称直接连服务地址按 IP 规则 direct（A 无路由、服务无访问）；名称的第一次 `getaddrinfo` 一次即得到服务地址（E 的回答被 A 持有至签名关联应用），按名称的 TCP 连接只以 E 的地址到达服务；控制器计数到关联任务完成，同名第二次连接不增加；关联期间不经名称连该地址也按名称规则经 E；按名称建立、每 2 秒往返一次的 TCP 与 connected UDP 流持续 100 秒、越过关联 60 秒 TTL 的到期：到期后不经名称新连该地址失败（按 IP 规则 direct，A 无直连路径），A 的捕获仍含该地址，持有的 TCP 每次往返成功、UDP 至多丢 1 次、外部服务只见 E；流结束后该地址退出 A 的捕获；撤销 `domain:` 授权后按名称连接失败；恢复原配置后进入第 11 阶段。

第 11 阶段（方案切换，C3）：A 的 assignment 允许 `exit`（默认）、`exit-alt`（与 `exit` 规则相同的另一方案）、`mesh` 与 `global:E`。`meshway exit --json` 由 A 的 netmap 给出四个方案，人读输出保存为 `a/exit-view.txt`。A 上每秒一次往返的 TCP 与 connected UDP 流经 E 建立后，`meshway exit exit-alt` 在 Server 提交、A 应用带该版本的 netmap 后返回（`applied=true`、本机版本等于提交版本）；切换后该 socket 的 TCP 与 UDP 往返全部失败（旧选择的流随新 selection 结束），新连接经 E 到达服务，A 的捕获仍为默认捕获。`meshway exit off` 后 A 的捕获无受管区间、无默认捕获，连外部服务失败且服务不见新的 E 访问（`a/exit-off.txt`）；`meshway exit exit` 恢复后进入第 5 阶段。`global:E` 只在列表中显示，不在此行切换：首次运行切到它时 A 的 DNS 安装以 “DNS resolver plan is incomplete” 拒绝该 netmap（`useWithExit` split 解析器在 global 下被出口捕获），A 停在 synchronizing，见[剩余边界](#remaining-boundaries)。

当前状态：**当前 wire 已通过（含 DNS、域名出口阶段、跨关联到期的持有流与 7,000 区间节点侧分割）**。F4a 审阅修复候选 `c90d91af` 之后（含持有流检查与数据面保留判定）：Planck 16 行矩阵 `20261012T195053Z-bc48b6cc` 全部通过，`exit/off` 行 `20261012T195514Z-0965a9f8` 425.4 秒；此前第一次带持有流检查的矩阵 `20261012T184725Z-20fb3fac` 在第 10 阶段失败（关联到期后已建立流每包按新判定被丢弃），证据 `.meshway/validation/f4a/`。R3 候选 `72144421`：Planck 矩阵 `20261012T105652Z-fcc0189b` 行 `20261012T110443Z-5d871fd5`，299.8 秒（A 安装 7,002 个受管区间、7,040 条捕获表路由）（合入 X6 前）。R3＋X6 合并后（候选 `c4018706`）：Planck 16 行矩阵 `20261012T115405Z-61e14403` 全部通过，`exit/off` 行 `20261012T115835Z-1812d1b8` 265.5 秒，第 10 阶段由 A 安装的捕获证明关联地址进入受管区间。此前：X6 候选 `cccc2594`：Planck 16 行矩阵 `20261012T101704Z-ae64c486` 全部通过（并行 8，墙钟 910 秒），`exit/off` 行 `20261012T102856Z-a05feedb` 445.0 秒，第一次名称解析 0.29 秒、一次任务完成、第二次连接零完成；本机短模式 `20261012T101111Z-bcc3f73b` 通过（另一次短模式 `20261012T100612Z-0c20b833` 在域名阶段之后的第 5 阶段 `docker exec cat` 失败，未复现）；证据 `.meshway/validation/x6/`。常驻 lab（X6 候选程序与 Server）：lax 以 `geosite:openai` → Exit nyc via sha、其余 direct，`curl` api.openai.com 的 SYN 只在 lax TUN 与 nyc 出口 TUN／eth0、example.com 只在 lax eth0；openai.com 与 `172-64-154-211.nip.io`（direct 名称规则）共享的地址连接被丢弃、openai.com 的另一地址经 nyc；配置以原 JSONC CAS 恢复，lab 回到 main 程序，证据 `.meshway/node-lab/x6/`。此前（X5）：Planck 矩阵 `20261011T195821Z-aa2222be`（候选 `e0449eae`）行 `20261011T200533Z-24e26301`，256.4 秒；此前 `20261011T143117Z-9dedf85e`（`94a0b680`）。此前不含 DNS 阶段的全长证据：Planck 单行 `20261010T160621Z-de9a2fae`、矩阵 `20261010T162437Z-4bbe5e35` 行 `20261010T162742Z-e9ed64a0`。IPv6 出口、源／Access／成员撤销逐项、切网、文件后端出口 resolver 与常驻 lab 整组仍未覆盖。常驻 lab 局部检查（不替代整组）：2026-10-11 以 X5 候选临时配置 lax 经 Exit→nyc via sha，lax 的 `dig`／`getent` 由 nyc 的 resolved 发往其上游（nyc 抓包），lax 与 sha 的非 loopback 抓包没有该名称明文，阻断 nyc 的上游 DNS 时 lax 查询失败；配置已 CAS 恢复原内容、lab 已恢复 main 程序，私有证据 `.meshway/node-lab/x5/`。X5b 候选（`b5644c06` 程序）另做一次出口 resolver 不可用的真实触发：nyc 的 resolved 在 eth0 加入 Mesh 地址作 DNS 服务器后 0.5 秒内 nyc `meshway status` 的 `exit.dnsUnavailable=meshServer`、Server State 的 nyc `exitDns` 为 `unavailable`／`meshServer`，lax 的查询 SERVFAIL，nyc 自身的 MagicDNS 与系统解析不受影响；`resolvectl revert` 后恢复 `serving`、lax 再次解析。期间 dev Server 临时换为同数据目录的 X5b 构建（旧 Server 拒绝新增的 `exitDns` 字段），结束后换回 main 构建，配置以 rev 55 原 JSONC 源文 CAS 恢复（rev 60），证据 `.meshway/node-lab/x5/x5b/`。


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
