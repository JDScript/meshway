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

默认矩阵另含 `exit/off`（真实 A→R→E IPv4 出口与单出口 DNS）、`nat/relay`（直连不可达时经第三节点中继，见[中继兜底行](#relay-row)）、`nat/reconnect`（Server 与 daemon 重启后打洞边自愈，见[重启自愈行](#reconnect-row)）与四个 `netmap/*` 生命周期行（见[netmap 生命周期行](#netmap-rows)）；出口详见[出口场景](#exit-scenario)。

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
| `validate:matrix --rows exit/off` | Native UDP；声明 Native TCP 能力 | 三端 resolved 后端（MagicDNS＋一个 `useWithExit` split） | IPv4 出口 masquerade | A→R→E；能力／安装、公网与 LAN、长流／仅接收、撤回、HTTP 中断内离线出口与租约到期全撤／恢复、journal；单出口 DNS（第 9 阶段）；域名出口与可信关联（第 10 阶段）；方案切换（第 11 阶段）；ICMP 错误与 PMTU（第 12 阶段）；`meshway ping` Mesh 外目标：公网 IP 经 E、direct、域名经 E resolver 与关联、撤权后 denied（第 13 阶段）；A 的 7,000 区间节点侧分割 | 当前 wire 已通过（含 DNS、域名出口阶段与节点侧分割）；R3＋X6 矩阵 `20261012T115405Z-61e14403` 行 `20261012T115835Z-1812d1b8`；X6 Planck 矩阵 `20261012T101704Z-ae64c486` 行 `20261012T102856Z-a05feedb`，见[出口场景](#exit-scenario) |

以下范围未验收：外层 IPv6 NAT、隔离 fixture 中的端点相关映射（仅有下述[实网证据](#real-nat)）、两端都在端点相关映射 NAT 后、映射轮换、NAT 与 resolv-conf 的组合、VLESS WS/TLS 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换、文件后端（resolv-conf）出口 resolver、resolved 代理 stub 缺失（systemd < 248 或 `DNSStubListener=no`）时的出口 resolver 不可用（仅组件测试）、`dns.overrideLocal` 出口 resolver（仅组件测试）、显式多出口 DNS plan、出口 IPv6、完整 TCP conntrack、数据面按通道 MTU 合成 Frag Needed／限制 MSS（通道路径 MTU 已发现并报告，见[外层路径 MTU](/v2/node/connectivity#path-mtu)）及分片重组、userspace／SOCKS5。整机重启与 resolved 停机间隙不属于隔离矩阵，由系统服务安装单独实测：重启（含 Server 不可达）通过，systemd-resolved 重启后 5 秒内、停止再启动后 10 秒内恢复 Mesh 名称解析，见 [Linux 安装与系统服务](/v2/install#验收记录)；`udp/resolved` 行另在隔离 namespace 内重启 resolved 并要求 5 秒内再次解析 Mesh 名称。

常驻 lab 的 L5 原始证据目录为 `.meshway/perf/l5/20261007T162418Z-main-f39ba2b6-2cpu/`。该目录的两次 TCP 接收吞吐分别约 318／364 Mbps；协调者另报告空闲宿主复测 1,367 Mbps，独立报告路径待补，不能把该数值归入前述目录或用吞吐局部证据替代整组验收。

### 实网 NAT 证据 {#real-nat}

2026-10-13，候选 `f1aa2250`：us-lax-it7（x86_64 公网、无 NAT、无 listener，只能拨出）与 meshway-vm（arm64，经 macOS Cloudflare WARP 出网：同一 socket 对不同目的得到不同外部端口，即端点相关映射）。证据 `.meshway/node-lab/n2a/`。

| 项 | 之前（main `1852b477`） | 之后 |
| --- | --- | --- |
| 邻接 | `unreachable`，双向 `meshway ping` 超时；it7 的 INIT 发往 STUN 候选端口，被 WARP 过滤 | `direct native_udp`，双向 `meshway ping` 5/5；it7 从 VM 的签名 punch 学到来源端口（候选 `:54017`，实际 `:53572`）。同时与两个 WARP 后的 lab 容器（main 版本，仅作响应方）直连 |
| 外层分片 | it7→VM 超过 1,357 B 的报文本机分片后全部丢失 | 全部带 DF，抓包 0 个分片 |
| 通道 MTU | 无 | it7→VM `1444/1330`（内层 DF ping 外层 1,444 B 5/5、1,445 B 0/5），VM→it7 `1500/1386` |
| TCP 吞吐（iperf3 20 s） | 无通道 | VM→it7 6.4 Mbit/s（62 次重传）；it7→VM 0（内层 TCP 1,366 B 超过 1,330，被外层丢弃；数据面 MSS 限制属后续工作）；`-M 1250` 时 it7→VM 101 Mbit/s。裸路径：VM→it7 117 Mbit/s、it7→VM 679 Mbit/s |

开发 Server 发布第二个 STUN 观测点后（2026-10-13 00:54），netcheck 在 VM 判定 `endpoint-dependent`、在 it7 判定 `endpoint-independent`；it7 换装中继修复后 30 分钟内 sync 0 次 `Network` 失败（此前每次失败都与中继 300 秒连接寿命到期精确对应）。

VM→it7 方向 Mesh TCP 远低于裸路径、UDP iperf 30 Mbit/s 时丢包 0.6%，原因未定位，不归入本证据的结论。

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

未完成：it7 安装 `conntrack` 前不能作为出口，lax 经 it7 出口的 `curl ifconfig.me` 未执行。

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
| `5183bf6e`（X7） | 全量 17 行，Planck 矩阵 `20261013T092932Z-0d781dd4`（并行 8，墙钟 778 秒，启动 load 1.2–2.9）；同产品行为的 `df591c3f` 矩阵 `20261013T090318Z-539d8762` 同样 17 行通过 | 17 行全部通过（`exit/off` `20261013T093730Z-4df28179` 504.3 秒，第 2 阶段 E 在 `ip_forward=0` 下 applied、打开 forwarding 后约 21 秒自动安装角色）；此前 `bede97a0` 矩阵 `20261013T071339Z-d4bdcc87` 16 行通过，`exit/off` 组件测试的投影目的 `8.8.8.8` 在无默认路由的隔离 namespace 中被新的逐流路由判定拒绝（测试改用 fixture 的外部服务并补拒绝断言），单行 `20261013T085053Z-f5b51954` 通过；证据 `.meshway/validation/x7/` |

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

入口收集能力／前置条件、saved／applied 与 netmap 租约、投影（A 的 egress 规则与 fallback、三端相同的外部 Access、E 的 exit role、R 无 role）、A 实际安装的分割（egress journal 的计划、nft `managed4` 元素数与 IPC `netmap.capture` 三者一致且不少于 7,000，split /32 不受管，external 与 LAN 目的受管）、capture、原始与 NAT tuple、nft／conntrack、服务日志、R 密文、E 阻断对照、持续／仅接收窗口、`exitEnabled` 撤回、HTTP 中断与租约到期、同 namespace journal 恢复证据。HTTP 中断同时结束在途 sync 长轮询：此后三端不再取得租约，已有 TCP 与每两秒新建的 UDP 流在离线期间继续经出口往返，直至路径上首个租约到期才结束；E 在自身租约到期时撤回 role，owned nft 表与 conntrack 精确清理，A 保留负向 egress 门（`managed4` 非空、无活动 selector）；恢复 HTTP 后无第二次 up 取得新租约并以新 circuit 建立新流。投影 ACK、projection 容量拒绝等内部边界另运行同源码的正式组件测试；组件通过不能代替独立进程行通过。所有阶段（1–12）、进程退出与精确清理通过后该次行才通过。

第 2 阶段（出口前置条件，X7）：E 在 `ip_forward=0` 下启动，必须照常 running（applied 等于 saved、data plane ready），IPC 与 Server State 的 `exitCapability` 同为 `unavailable`／`ipForwardDisabled` 且没有安装角色；fixture 在 E 的 namespace 打开 forwarding 后不重启 daemon、不发新 netmap，E 的周期能力探测观测到 ready 后以同一内容安装角色（第 1、3 阶段：`ready`、`installedFamilies=[ipv4]`、applied 不变）。

第 9 阶段（DNS）：A 的计划为 `viaExit`（路径 A→R→E），E 为 `serveExitResolver` 且只服务 A；A 的 `split` 解析器决定为 `direct`。A 的系统 `getaddrinfo` 必须得到 E 的系统 resolver（resolved，上游为 external 网段内的隔离 DNS authority）的回答：A、双栈 A＋AAAA（AAAA 原样返回）、90 条 A 记录（超出 Mesh UDP 1232 bytes，只能经 Mesh TCP 路由完成）；authority 记录的唯一提问者是 E 的 external 地址。A 自己的 MagicDNS 名称本地解析，未知 MagicDNS 名称本地拒绝，两者都不出现在任何上游；`split.test` 名称只从 A 的 split 地址到达 Resolver。A 的系统上游就是 Resolver，它对任何名称都会给出标记地址：E 上加 unreachable 路由阻断其 DNS authority 时 A 的查询失败，Resolver 不收到该名称。E 的出口 role 撤回与 A 的租约到期后查询失败，HTTP 中断后的租约内查询成功，撤回恢复与租约恢复后查询再次成功。抓包中 `exit-dns` 字节只出现在 E 的 external 接口，A、R 与 E 的其他接口没有。

第 10 阶段（域名出口，X6）：在第 9 阶段的交换检查之后临时改为“名称规则 `*.name.exit-dns.test` → Exit E via R、其余 direct、Access 为 `domain:name.exit-dns.test`”。A 安装的捕获（egress journal）不含名称区地址，关联期间恰含该服务地址；不经名称直接连服务地址按 IP 规则 direct（A 无路由、服务无访问）；名称的第一次 `getaddrinfo` 一次即得到服务地址（E 的回答被 A 持有至签名关联应用），按名称的 TCP 连接只以 E 的地址到达服务；控制器计数到关联任务完成，同名第二次连接不增加；关联期间不经名称连该地址也按名称规则经 E；按名称建立、每 2 秒往返一次的 TCP 与 connected UDP 流持续 100 秒、越过关联 60 秒 TTL 的到期：到期后不经名称新连该地址失败（按 IP 规则 direct，A 无直连路径），A 的捕获仍含该地址，持有的 TCP 每次往返成功、UDP 至多丢 1 次、外部服务只见 E；流结束后该地址退出 A 的捕获；撤销 `domain:` 授权后按名称连接失败；恢复原配置后进入第 11 阶段。

第 11 阶段（方案切换，C3）：A 的 assignment 允许 `exit`（默认）、`exit-alt`（与 `exit` 规则相同的另一方案）、`mesh` 与 `global:E`。`meshway exit --json` 由 A 的 netmap 给出四个方案，人读输出保存为 `a/exit-view.txt`。A 上每秒一次往返的 TCP 与 connected UDP 流经 E 建立后，`meshway exit exit-alt` 在 Server 提交、A 应用带该版本的 netmap 后返回（`applied=true`、本机版本等于提交版本）；切换后该 socket 的 TCP 与 UDP 往返全部失败（旧选择的流随新 selection 结束），新连接经 E 到达服务，A 的捕获仍为默认捕获。`meshway exit off` 后 A 的捕获无受管区间、无默认捕获，连外部服务失败且服务不见新的 E 访问（`a/exit-off.txt`）；`meshway exit exit` 恢复后进入第 5 阶段。`global:E` 只在列表中显示，不在此行切换：首次运行切到它时 A 的 DNS 安装以 “DNS resolver plan is incomplete” 拒绝该 netmap（`useWithExit` split 解析器在 global 下被出口捕获），A 停在 synchronizing，见[剩余边界](#remaining-boundaries)。

第 12 阶段（ICMP 错误，E1）：拓扑另有 router 容器（只开 IPv4 转发，无 Meshway 进程），连接 `external` 网段与第二个公网分类网段 `far`；external 服务的 far 地址经 E → router → far 到达，router 的 far 接口 MTU 为 1280（其余 1500，服务自己的接口不变，因此 TCP 对端通告的 MSS 不限制 A）。E 上临时加一条 far 网段经 router 的路由，阶段结束删除。A 的原始 ICMP socket 记录 A 收到的全部 ICMPv4 错误。`traceroute -I`（每跳 1 个、默认并发探测、等待 2 秒）必须给出第 1 跳为 E 的地址（E 本机 time exceeded）、第 2 跳为 router、第 3 跳为目标；64 KiB 的 TCP echo 经 router 的 fragmentation needed 收敛并完整返回，A 的路由缓存为 `mtu 1280`，`ping -M do -s 1300` 失败、`-s 1252` 成功，A 记录到来自 router 的 3/4、MTU 1280 错误；A 保持一条到 external 服务的 UDP 流时，router 伪造 5 条引用不存在 tuple 的 time exceeded（E 的 conntrack 无关联，内核丢弃）与 30 条引用该流 NAT 后 tuple 的错误：A 只收到后者，且在每流每秒 10 条的预算之内（至少 1 条）。数据面的引用匹配、来源规则与限速另由 `meshway-dataplane` 的流表、报文与 A→R→E 运行测试覆盖；伪造错误被数据面拒绝的计数（`related_counters`）不经 IPC 暴露，行内以 A 的接收结果为准。

当前状态：**当前 wire 已通过（含 DNS、域名出口阶段、跨关联到期的持有流、ICMP 错误与 PMTU 和 7,000 区间节点侧分割）**。E1 候选 `a8bd7eb3`：`exit/off` 行 `20261013T011243Z-62bc2868` 441.5 秒，ICMP 阶段（现为第 12 阶段）traceroute 依次为 E 的 Mesh 地址、router、目标，64 KiB TCP echo 经 router 的 10 条 fragmentation needed（MTU 1280）收敛（MSS 1228，A 缓存 `mtu 1280`），30 条注入错误到达 10 条（每流每秒预算），引用不存在流的 5 条没有到达；此前没有缓存例外修复的候选在第 8 阶段失败（A 的 egress owner 把该 PMTU 例外当作外来路由），证据 `.meshway/validation/e1/`。常驻 lab（E1 程序与 Server）：lax 以 `geoip:cn` → Exit sha（方案与公共规则两种形态），`traceroute -I 1.2.4.8` 显示 sha 的 Mesh 地址、docker 网关与其后各公网跳直至目标；公共规则形态下 sha 自身的 netmap 规则动作为 `direct`、DNS `serveExitResolver`、经 eth0 直出，路径检测 sha → `direct`、lax → Exit sha；真实路径约 1,356 字节的 MTU 在 sha 直出时也无 fragmentation needed（黑洞，与 Meshway 无关），lax 经 sha 的行为相同；每个新 echo 流需一次出口投影安装，经出口的 echo RTT 比 sha 直出高约 120–150 ms，默认等待 2–3 秒的 traceroute 在约 12 跳后超时（`-w 6` 可完整到达）；配置按 E1 键 CAS 移除恢复，证据 `.meshway/node-lab/e1/`。F4a 审阅修复候选 `c90d91af` 之后（含持有流检查与数据面保留判定）：Planck 16 行矩阵 `20261012T195053Z-bc48b6cc` 全部通过，`exit/off` 行 `20261012T195514Z-0965a9f8` 425.4 秒；此前第一次带持有流检查的矩阵 `20261012T184725Z-20fb3fac` 在第 10 阶段失败（关联到期后已建立流每包按新判定被丢弃），证据 `.meshway/validation/f4a/`。R3 候选 `72144421`：Planck 矩阵 `20261012T105652Z-fcc0189b` 行 `20261012T110443Z-5d871fd5`，299.8 秒（A 安装 7,002 个受管区间、7,040 条捕获表路由）（合入 X6 前）。R3＋X6 合并后（候选 `c4018706`）：Planck 16 行矩阵 `20261012T115405Z-61e14403` 全部通过，`exit/off` 行 `20261012T115835Z-1812d1b8` 265.5 秒，第 10 阶段由 A 安装的捕获证明关联地址进入受管区间。此前：X6 候选 `cccc2594`：Planck 16 行矩阵 `20261012T101704Z-ae64c486` 全部通过（并行 8，墙钟 910 秒），`exit/off` 行 `20261012T102856Z-a05feedb` 445.0 秒，第一次名称解析 0.29 秒、一次任务完成、第二次连接零完成；本机短模式 `20261012T101111Z-bcc3f73b` 通过（另一次短模式 `20261012T100612Z-0c20b833` 在域名阶段之后的第 5 阶段 `docker exec cat` 失败，未复现）；证据 `.meshway/validation/x6/`。常驻 lab（X6 候选程序与 Server）：lax 以 `geosite:openai` → Exit nyc via sha、其余 direct，`curl` api.openai.com 的 SYN 只在 lax TUN 与 nyc 出口 TUN／eth0、example.com 只在 lax eth0；openai.com 与 `172-64-154-211.nip.io`（direct 名称规则）共享的地址连接被丢弃、openai.com 的另一地址经 nyc；配置以原 JSONC CAS 恢复，lab 回到 main 程序，证据 `.meshway/node-lab/x6/`。此前（X5）：Planck 矩阵 `20261011T195821Z-aa2222be`（候选 `e0449eae`）行 `20261011T200533Z-24e26301`，256.4 秒；此前 `20261011T143117Z-9dedf85e`（`94a0b680`）。此前不含 DNS 阶段的全长证据：Planck 单行 `20261010T160621Z-de9a2fae`、矩阵 `20261010T162437Z-4bbe5e35` 行 `20261010T162742Z-e9ed64a0`。IPv6 出口、源／Access／成员撤销逐项、切网、文件后端出口 resolver 与常驻 lab 整组仍未覆盖。常驻 lab 局部检查（不替代整组）：2026-10-11 以 X5 候选临时配置 lax 经 Exit→nyc via sha，lax 的 `dig`／`getent` 由 nyc 的 resolved 发往其上游（nyc 抓包），lax 与 sha 的非 loopback 抓包没有该名称明文，阻断 nyc 的上游 DNS 时 lax 查询失败；配置已 CAS 恢复原内容、lab 已恢复 main 程序，私有证据 `.meshway/node-lab/x5/`。X5b 候选（`b5644c06` 程序）另做一次出口 resolver 不可用的真实触发：nyc 的 resolved 在 eth0 加入 Mesh 地址作 DNS 服务器后 0.5 秒内 nyc `meshway status` 的 `exit.dnsUnavailable=meshServer`、Server State 的 nyc `exitDns` 为 `unavailable`／`meshServer`，lax 的查询 SERVFAIL，nyc 自身的 MagicDNS 与系统解析不受影响；`resolvectl revert` 后恢复 `serving`、lax 再次解析。期间 dev Server 临时换为同数据目录的 X5b 构建（旧 Server 拒绝新增的 `exitDns` 字段），结束后换回 main 构建，配置以 rev 55 原 JSONC 源文 CAS 恢复（rev 60），证据 `.meshway/node-lab/x5/x5b/`。


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
