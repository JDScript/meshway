# 验收范围

本页是 Node 验收范围的唯一定义：哪些行构成整组验收、每行验证什么、门槛从何而来、当前状态、真机上已证明的事实、已修复问题的回归由谁守住，以及已登记未修的边界。其他页面只描述行为与限制并引用这里。入口命令、参数与报告位置见[开发服务、测试与环境](/v2/development#validation-matrix)，测试与 fixture 清单见[测试清单](/v2/tests)，合入门槛见[工程流程](/v2/process)。逐轮运行记录不写在本页：版本级结论在 [changelog](/v2/changelog)，原始报告在私有证据 `.meshway/validation/<工作包>/`、`.meshway/node-lab/<OPS>/`。

## 验收层次与口径 {#scope}

| 层次 | 证明方式 | 不能由什么代替 |
| --- | --- | --- |
| State | Server 收到并确认的节点报告（`paths`、候选、`exitCapability`、`exitDns`） | 候选非空、本机日志 |
| saved／applied | IPC `saved`／`applied` 与 netmap 租约，各自读回 | 对方的 saved、Server 的 serial |
| 认证通道 | Status 中已认证的 channel（ID、remote、acquisition、MTU），双侧抓包 | 拓扑允许、端点可达 |
| 真实包流 | OS socket 的 UDP／TCP／ICMP 回显、服务日志看到的来源、路由器与 nft／conntrack 计数 | 进程内 SecureNode fixture、组件测试 |

- **一行通过**同时要求：fixture `passed`、所有进程成功退出、本 run 精确清理（行内与远程 supervisor 独立复核容器／网络均无遗留），报告记录源码指纹与二进制摘要。
- **全长与短模式**：全长运行（`scope=acceptance`）使用[持续窗口](#traffic-anchor)；`--short`（`scope=diagnostic-short`）把持续／仅接收窗口缩到 30 秒、TCP 退路 5 秒后触发，未测到的项为 null。**只有全长行更新本页状态**，短模式只用于诊断与包内门槛。
- 库测试、单独内核测试、镜像构建成功、常驻 lab 的局部检查都不替代对应行的整组结论；组件测试只证明内部边界（ACK、截止、容量），不代替独立进程行。
- 缺测为失败或 null，不填零；重放、读取不制造样本。
- **Planck 规则**：Planck 是共享宿主。同时运行矩阵的工作包至多 2 个，开跑前 `docker ps` 确认；矩阵按 bridge 预算调度、`roam/*` 行逐个运行（每行 4–5 个 bridge）；不在 Planck 制造宿主级 RTNL 停顿（这类注入只在本机 lab 或隔离容器中做）。各行报告记录启动 load，高负载与低负载结果不加区分地合并无效。远程原件在 `.meshway/validation/matrix/<run>/`，CI 证据在 runner 的 `~/meshway-ci-evidence/validate/`。

| 状态 | 含义 |
| --- | --- |
| 通过 | 当前 wire（[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)）上最近一次全长全绿矩阵中该行通过 |
| 未通过 | 当前实现运行过该行，至少一个必需检查失败；失败签名必须登记在[剩余边界](#remaining-boundaries) |
| 未验收 | 没有整组证据，不能声明已支持（见[未验收范围](#not-covered)） |

## 默认矩阵 {#matrix}

`scripts/validate_matrix.py` 的 `ROWS` 定义 26 行，默认全部运行。**当前状态：26／26 通过**——Planck 矩阵 `20261023T225001Z-701f8fde`（`8199c8bd`，alpha.23 发布候选，并行 8，墙钟 1,196 秒，启动 load 4.8）一轮全部通过；私有证据 `.meshway/validation/remote/zj2631@ai4ce-planck.dyn.poly.edu-20261023T225001Z-701f8fde/`。下表“报告”列为该矩阵中的行报告 ID。

| 行 | 拓扑与承载 | 覆盖 | 门槛摘要 | 报告 | 定义 |
| --- | --- | --- | --- | --- | --- |
| `tun` | 单 namespace | 真实 TUN 创建与双栈包 I/O | 全部用例通过 | `20261023T225441Z-0447f15a` | [tun 与 packet](#tun-packet) |
| `packet` | A→R→B，Native TCP | 受保护包流、满 MTU／MTU+1、多 MSS TCP、R 只转发密文、原路径回复 | R 明文交付 0 | `20261023T225441Z-9370d139` | [tun 与 packet](#tun-packet) |
| `udp/off` | A→R→B，Native UDP 仅本机上报端点；A–R 另有其后的 Native TCP | 共同场景；listener 端口被占后自愈（U1）；UDP 阻断时退到 TCP、放开后回到 Native UDP（CM-1b） | 共同场景全部检查；放开后 ≤ 75 秒回到 `native_udp`、circuit 不变 | `20261023T225441Z-0b737d0b` | [共同场景](#runtime-共同场景) |
| `tcp/off` | A→R→B，Native TCP | 共同场景 | 同上 | `20261023T225441Z-9998b149` | [共同场景](#runtime-共同场景) |
| `vless/off` | 仅 R listener，VLESS TCP/TLS | 共同场景＋精确叶证书 | 同上 | `20261023T225441Z-1c4f46fc` | [共同场景](#runtime-共同场景) |
| `trojan/off` | 仅 R listener，Trojan TCP/TLS | 共同场景＋精确叶证书 | 同上 | `20261023T225441Z-e5cc1c2e` | [共同场景](#runtime-共同场景) |
| `udp/resolv-conf` | Native UDP 静态端点 | 共同场景＋文件代理 DNS | 同上＋DNS 场景 | `20261023T225442Z-b34b2def` | [DNS 场景](#dns-场景) |
| `udp/resolved` | Native UDP 静态端点 | 共同场景＋resolved 后端 | 同上＋resolved 重启 ≤ 5 秒再解析 | `20261023T225944Z-a6a813a4` | [DNS 场景](#dns-场景) |
| `jammy/resolv-conf` | Native UDP 静态端点，Ubuntu 22.04 DNS 镜像 | 共同场景＋文件代理 DNS；resolved 后端的明确不支持 | 同 `udp/resolv-conf`＋resolved 检查报 `probe` 版本要求；真实 daemon 60 秒内探测 ≤ 6 次 | `20261023T225945Z-aa0ece92` | [DNS 场景](#dns-场景) |
| `nat/off` | A、R 各在端口保持型 SNAT 后 | 共同场景＋协调穿透 | R 只接受 A 的 offer | `20261023T225947Z-46ecba2b` | [穿透场景](#穿透场景) |
| `nat+fallback/off` | 同上＋A–R Native TCP 退路 | 穿透后切到 TCP 退路 | 同一 socket、circuit 与路径跨切换保留 | `20261023T225948Z-76ee60dc` | [穿透场景](#穿透场景) |
| `nat/resolved` | 同 `nat/off` | 穿透＋resolved | 两场景全部检查 | `20261023T225949Z-1df5ab24` | [穿透场景](#穿透场景) |
| `nat/relay` | 5 节点，A–B 直连被路由器丢弃 | 新流经第三节点中继 | 提交后 ≤ 30 秒经 R2 回复 | `20261023T230501Z-c4518314` | [中继兜底行](#relay-row) |
| `nat/reconnect` | A 端点相关映射 ↔ 公网 R | NAT 映射丢失、Server 重启、两端 daemon 重启后打洞边自愈 | 每阶段 ≤ 60 秒 `direct` | `20261023T230500Z-f3b9224e` | [重启自愈行](#reconnect-row) |
| `exit/off` | A→R→E，IPv4 出口 | 出口 16 阶段：能力、NAT、DNS、域名关联、方案、ICMP／PMTU、auto、echo、吞吐 | 各阶段断言 | `20261023T225441Z-85dbe1a5` | [出口场景](#exit-scenario) |
| `exit6/off` | 同上，双栈，NAT66 | IPv6 出口 | 各阶段断言 | `20261023T230920Z-6b92ebe6` | [exit6/off 行](#exit6-off) |
| `roam/direct` | A 两个 SNAT 上联，公网 B，LAN C | 运行中换上联 | 首个回复 ≤ 3 秒、circuit 与 TCP 保持 | `20261023T230914Z-f5f96a5b` | [漫游行](#roam-row) |
| `roam/nat` | A、B 都在 NAT 后 | 漫游后重新穿透 | 首个回复 ≤ 5 秒、A 发起 | `20261023T230525Z-01d25ab6` | [roam/nat 行](#roam-nat-row) |
| `roam/relay` | 同上＋公网中继 R | 空窗内新流经中继 | 空窗新流 1 秒内经 R | `20261023T230750Z-9f91f605` | [roam/relay 行](#roam-relay-row) |
| `roam/flap` | A 两个上联 | 快速往返 | 反应数＝path 变化数、无 circuit 退役 | `20261023T231130Z-95db8d8b` | [roam/flap 行](#roam-flap-row) |
| `roam/exit` | 出口节点 A 漫游 | 出口能力重探、新流经新上联 | 5 秒内再 `ready` | `20261023T230646Z-41552a9d` | [roam/exit 行](#roam-exit-row) |
| `fault/throttle` | A→R→B，B 经出口 E，四端 resolved | 外部压力下自动恢复与空闲 CPU | 无 `blocked`、≤ 10 秒恢复、线程 < 5 % | `20261023T225441Z-96aa77b6` | [fault/throttle 行](#fault-row) |
| `netmap/offline` | 本机上报端点 | Server 不可达 3 分钟（多次租约内） | 一切照常 | `20261023T230750Z-77fc8f2b` | [netmap 生命周期行](#netmap-rows) |
| `netmap/revoke` | Native TCP | 端口、密钥、成员变化到达数据面 | ≤ 5 秒 | `20261023T230813Z-bc0c4ec0` | [netmap 生命周期行](#netmap-rows) |
| `netmap/expiry` | Native UDP | 越过租约后 fail closed 并恢复 | TUN ≤ 2 秒删除 | `20261023T230842Z-ad691e45` | [租约到期撤回](#lease-expiry-withdrawal) |
| `netmap/restart` | 本机上报端点 | Server 不可达时空闲重拨与 daemon 重启 | 只用最后已知事实重建 | `20261023T230914Z-8f8ae810` | [netmap 生命周期行](#netmap-rows) |

**新增行或更新状态**：新行在本表按 `ROWS` 顺序加一行，并在[行定义](#rows)下按同一模板（拓扑、阶段与断言、门槛来源、不覆盖）加一个小节；后续运行只更新本节开头的矩阵 ID 与“报告”列。行暴露的已修问题加到[已修复问题索引](#fixed)，未修的加到[剩余边界](#remaining-boundaries)，真机结论加到[真机证据](#real-hosts)；运行过程写 changelog 与私有证据。修复合入的工作包负责把对应的剩余边界条目移到已修复问题索引。

### 未验收范围 {#not-covered}

外层 IPv6 NAT；隔离 fixture 中的端点相关映射（只有[真机证据](#real-nat)）与两端都在端点相关映射 NAT 后；映射轮换；NAT 与 resolv-conf 的组合；VLESS WS/TLS 承载上的 daemon 包流、TLS／WS 材料轮换、多路径切换；文件后端（resolv-conf）出口 resolver；resolved 代理 stub 缺失（systemd < 248 或 `DNSStubListener=no`）与 `dns.overrideLocal` 时的出口 resolver（仅组件测试）；显式多出口 DNS plan；真实宿主 IPv6 出口；完整 TCP conntrack；中继到下一跳更窄时的 ICMP 回报（只知首跳通道上限，见[通道 MTU](/v2/node/dataplane#channel-mtu)）；出口对外部超限回包的错误回报；Mesh IPv6 packet too big 的真实包流（仅组件与 A→R→B 运行测试）与分片重组；userspace／SOCKS5。整机重启与 resolved 停机不属于隔离矩阵，由系统服务安装实测，见 [Linux 安装 · 验收记录](/v2/install#验收记录)。

## 行定义 {#rows}

### tun 与 packet {#tun-packet}

`test:tun:linux`：宿主交叉编译，容器只运行预编译测试；固定 digest 镜像、独立 network namespace，只授予 TUN device 与 `NET_ADMIN`。

`test:packet:linux`：Server、A、R、B 四个独立容器／namespace 与一个 internal bridge。Server 运行真实 Router／HTTPS／生成管理客户端；各 Node 独立持钥接入，经正式 Native TCP provider 连接，不持有管理 token 或 Server TLS 私钥。

- 双栈 UDP 覆盖小包与恰好填满默认内层 MTU 1,366 B 的包（IPv4 payload 1,338 B、IPv6 1,318 B）；同一 socket 发 MTU+1：返回 `EMSGSIZE`，或内核分片被正式包解析器拒绝、不到达 echo，随后满 MTU 仍成功（拒绝不停止 owner）。
- 同一入口运行一组 Native TCP／DNS off 的真实 daemon conformance：双栈 UDP 边界与同一 TCP socket 的双栈 16 KiB（跨多个 MSS）逐字节 echo。门槛来源：daemon 按已安装 TUN MTU 分配包池，固定 1,280 B 的池无法接纳 1,366 B 包，直接 SecureNode fixture 证明不了这一边界。
- 不覆盖：分片重组、动态路径 MTU 发现（见 [`exit/off` 第 12 阶段](#exit-scenario)）。

### runtime 共同场景

`udp/off`、`tcp/off`、`vless/off`、`trojan/off` 以及全部 DNS／NAT 行共用的 daemon 整组。宿主编译实际 `meshwayd`、`meshway` 与验证 actor；Node 经 CLI 上线并由真实 Server 批准，业务使用 OS UDP／TCP socket，包处理由 daemon 的正式组件执行。容器使用独立 IPv4／IPv6 internal bridge，不发布宿主端口，不用 host network 或 privileged。租约 60 秒（`--netmap-validity 60s`）。

- 关闭系统 DNS 的 Mesh-only 网络，强制 A→R→B 与原路径回复；State 中 A／R／B 各报告 A→R→B 与自身角色。
- 内层双栈 UDP／TCP，含满 MTU／MTU+1 UDP 与 16 KiB TCP echo；同一 socket 跨需求添加；至少一个[持续窗口](#traffic-anchor)的续租；独立 UDP 流停发后的入站活动续租。
- Access 撤销／恢复；中转取消 IPv6 地址后继续转发 IPv6 密文；拒绝路由保留；离线 `down` 清理。
- 控制中断恢复：关闭 Node HTTP 接纳直至实际 instance 到期，再恢复联系；无需再次 `up`，原身份、进程与 revision 保持，取得新 instance、重新 applied 并上报 State，以新 OS socket 恢复原路径包流，过期的旧 TCP／cipher 被拒绝。
- `meshway ping`：hostname／双栈 Mesh IP、direct 与有序 via、独立 ICMP 权限、用户与 Node 共享任务及独立取消、SIGINT、控制中断后的订阅终态、新 instance 下重新探测。
- 源、终点的新交付计数与中转 TUN 的零明文交付分别记录。
- 行前内核测试：同目的地址的普通／受保护 socket、旧池拒绝边界、journal 恢复、外部同优先级规则冲突、HTTPS bootstrap 在显式 down 前后的联系；`underlay-restart-a`／`-b` 两个 namespace 共享私有目录验证容器重启边界（同 boot 新 netns 归档原 journal 并重建；改写为自身 boot／device／inode 时因缺驻留证据而归档重分配，不是 Conflict）；`daemon-owner-conflict` 验证启动 Conflict、递增的 `retryAt` 与 owner 释放后无新 `up` 的自动恢复。

各行差异：`udp/off` 只用本机上报端点（无 configured／STUN），且 B 的 listener 端口在 daemon 首次绑定时被占：Status 报告占用者，再占 3 秒释放后无需重启即绑定、清空 `lastError`、端口归 daemon（U1）。`udp/off` 的 R 另有只上报本机端点的 Native TCP listener `upgrade-tcp`，只有 A 有 Native TCP 拨号器，A–R 的承载顺序为平台顺序（Native UDP 在前）。持续窗口之前的**承载回归**阶段（CM-1b）：A 在自己通往 R 的接口上用 `tc` clsact 丢弃全部外发 UDP（tc 不归 Node 所有、不受其审计），A 的 Status 中 R 须在 30 秒内变为 `native_tcp`，并在 TCP 上完成一次双栈 UDP／TCP echo；保持 5 秒（Native UDP 尝试在旁边失败、退避增长）后放开，**门槛：放开后 75 秒内（退避上限 60 秒＋一次握手与 Status 轮询）R 回到 `native_udp`**，随后 3 秒业务；A 的真实包跟踪中 TCP 期间与回到 UDP 后的 circuit CID 集合相同且非空、完整路径不变。证据 A 的 `carrier-upgrade.json`（各时刻、两个 endpoint、两个 CID 集合）。门槛来源：CM-1b C1（现场 planck↔jp 在一次暂态后停在 native_tcp）；修复前的代码在该阶段失败（`A–R did not reach NativeUdp within 75s`）。`vless/off`、`trojan/off` 只有 R 有 listener、A／B 拨入，精确叶证书验证；配置证据要求 A／B 无 listener、dialers 只含所选协议，R 的 dialers 为空。不覆盖：VLESS WS/TLS、材料轮换、多路径（见[未验收范围](#not-covered)）。

#### 持续窗口 {#traffic-anchor}

全长持续／仅接收窗口为 **70 秒**：长于一次 sync 续租周期（Server hold 50 秒＋至多 10 % 抖动＋余量），也长于一次 60 秒租约，因此同一 socket 必然跨过一次续租及其建立时所依据的租约截止；更长的窗口不增加新的状态转移。2026-10-10 前的报告使用 125 秒（来自已删除的 demand permit 期限）。

### DNS 场景

`udp/resolv-conf`、`udp/resolved`、`jammy/resolv-conf` 在共同场景上加：真实 `getaddrinfo`、短名称、可见性 REFUSED、分域 UDP／TCP、系统 UDP→TCP 截断重试、进程重启、长时包流、失效阻断及 DNS／接口恢复。测试 namespace 使用私有 mount namespace 与私有 D-Bus／resolver，不改写 Docker 的 resolver bind 源；固定 OS 资产见 `dev/dns-fixture/README.md`。`udp/resolved` 另在 namespace 内重启 resolved，要求 5 秒内再次解析 Mesh 名称（与[系统服务实测](/v2/install#systemd-resolved-重启与停止)的门槛相同）。

三行都在 Node 之前运行一次 resolved 后端检查（`dns-resolved-backend`，`resolved_backend_installs_beside_foreign_domains_or_names_its_systemd_requirement`）：在该行 DNS 镜像自己的 D-Bus／resolved 上用 Kernel 的真实锚点与生产 owner，另建一条带 DHCP 式搜索域 `tko.oraclevcn.com` 的外来链路。

| 镜像 | 断言 |
| --- | --- |
| Debian trixie（systemd 257；`udp/*` 行） | 兜底 `~.` 计划安装成功，`local_domains` 恰为该域与外来链路的序号和名称；外来链路加 `~a.corp.example`（计划的 split 路由之下）或 `~.` 时观察以 `foreign_domain` 失败，移除后重证并重装；无兜底的计划在外来 `~.` 下安装、观察成功且不列出；down 后 resolved 设置与开始时一致 |
| Ubuntu 22.04（systemd 249.11；`jammy/resolv-conf`） | 安装以 `Unsupported`／`probe` 失败，`expected` 写明 `systemd-resolved 253 or later, or stable 249.15…` 与 `see install.md: Ubuntu 22.04`，`actual` 以 `no query reached 127.0.0.1:` 开头；锚点设置还原，down 后 resolved 设置一致 |
| Ubuntu 22.04，真实 daemon（`dns-resolved-cadence`，`an_unusable_resolved_is_probed_with_a_growing_pause`） | 内嵌 Server、`tun` 模式、默认 resolved 后端接入后，Status 始终非 `blocked`，每次探测失败时为 `retrying`／`dns` 且详情含版本要求与指引；自首次失败起 60 秒内探测失败（每次一轮锚点设置改写）4–6 次，前 5 个间隔逐个增长 ≥ 0.5 秒，第 4 次失败时 `retryAt` 至少在 7.5 秒后；down 后 resolved 设置一致。门槛 6＝暂停 1、2、4、8、16 秒各加 5 秒探测，第 6 次约在 56 秒；固定 1 秒时约 11 次 |

`jammy/resolv-conf` 的 DNS 镜像由 `dev/dns-fixture/jammy.lock.json`（`ubuntu:22.04` digest、Ubuntu snapshot `20261020T000000Z` 的 jammy／jammy-security／jammy-updates 索引）离线构建，其余与 `udp/resolv-conf` 相同；它验收的是 [Ubuntu 22.04](/v2/install#ubuntu-2204) 的显式文件代理路径。门槛来源：D4（resolved 249 收不到锚点 loopback 服务器的查询，[真机证据](#dns2-hosts)）与 D5（其他链路的搜索域使兜底计划整体挂起）。

### 穿透场景

`nat/off`、`nat+fallback/off`、`nat/resolved`（`--traversal nat`）：

- 两个独立 IPv4 端口保持型 Linux conntrack SNAT；UDP 发布端点为空，STUN、互探与认证使用实际承载 socket。
- 内层双栈 UDP／TCP 强制 A→R→B，持续与仅接收业务跨一个持续窗口。
- `nat/off` 中 A–R 是按需的 NAT–NAT 边：R 另有仅 Native TCP 的拨号器、A 另有不公告的 TCP listener（经 A 的 NAT 不可达），两端都能拨出、无一端常驻，只有 A 有经 R 的业务。R 不得 prepare 任何 A–R 尝试，只能经 `traversalRequests` 接受 A 的 offer（[穿透信令](/v2/node/connectivity#traversal-signalling)）；R–B 为 B→R 常驻边。
- `--traversal-fallback` 只预授权 A→R 的 Native TCP 拨号，R→B 仍为 Native UDP；TCP 入口初始不可达，业务 30 秒后开放该入口并阻断精确 A–R UDP 地址对；原 TCP socket、两个内层 TCP circuit 的 CID 与完整路径跨切换保留；撤权前等两侧路由器的 traffic 快照会合，两侧各有同一条 ESTABLISHED 退路流。
- 证据：State、双方接受的固定 offer、双侧抓包、conntrack、防火墙计数与应用回包。拓扑见 `dev/traversal-fixture/README.md`。

### 中继兜底行 {#relay-row}

`nat/relay`（5 节点）在 `nat/off` 的拓扑（A、R 在 SNAT 后，B 公网）完成共同启动与持续流后：router-a 丢弃 A↔B、router-r 丢弃 R↔B 的全部报文，A 没有直连，R1 只保留与 A 的通道；两个公网中继此时才 enroll，节点 ID 较小的作 R3（与 A、B 都有拓扑、只与 B 常驻，因此与 A 无通道），另一个作 R2（与 A、B 都常驻）。State 显示 R2 与 A、B，R3 只与 B，R1 只与 A 有通道后，Controller 去掉 `A→B via R` 并允许 A–B 直连。Policy 候选为 `[A, B]` 与经 R1／R2／R3 的三条两跳路径，只有 R2 与两端都有通道。

| 检查 | 要求 |
| --- | --- |
| CLI ping | `meshway ping b` 的完整路径为 `[A, R2, B]` |
| 新流 | 新 UDP／TCP 5 元组完成回显，不加入 `[A, R1, B]` 上的旧 circuit（新流只加入排序位置不低于所选路径的 circuit） |
| 切换时间 | 提交新内容到首个中继回复 ≤ 30 秒 |
| status | A 的 peer B 为 `relay`、`via = [R2]`；结束时 R3 仍无与 A 的通道 |
| 节点 ID | `scenario-relay-roles.json` 中 R3 在 R2 之前（R1 与 R2 的先后随 enrollment，一并记录） |
| 路由器 | 各有两条 B IP 的 DROP 规则且 A→B 计数 > 0（A 确实尝试过直连），没有更宽的规则 |

不覆盖：经 `[A, R1, B]` 的旧流（R1 失去 B 后不再回复，不检查）；“直连不可能”由路由器丢包造成，不是两端端点相关映射＋端口相关过滤（该形态见[真机证据](#n3-real-network)）。

### 重启自愈行 {#reconnect-row}

`nat/reconnect` 在 `nat/off` 拓扑与持续流之后，把 A–R 变成“端点相关映射的 A ↔ 公网 R”：第一次断开时 router-a 对 A 的新 UDP 流改用随机端口映射（`SNAT --random-fully`），router-r 放行全部入站 UDP（DNAT 到 R）。A 的 STUN 映射对 R 无效，只有协调穿透加来源学习（R 从 A 的签名 INIT 学到实际端口）能连通。A–R 是按需边，无通道时 A 每 2 秒 `meshway ping r` 维持需求。

| 阶段 | 扰动 | 计时起点 |
| --- | --- | --- |
| `nat` | router-a 丢弃 A↔R UDP 直到 A 失去 R，再恢复并清空 A 的全部映射 | 恢复 |
| `server` | 真实 Server 原地重启（同库同钥、新 boot；instance、State、事实全部丢失），A、R 取得新 instance 且 State `current` 后同 `nat` 断开再恢复 | 恢复 |
| `restart-r` | 只重启 R 的 daemon | 重启指令 |
| `restart-a` | 只重启 A 的 daemon | 重启指令 |

门槛：每阶段 A 的 status 中 R 重新为 `direct`（新通道）且 ≤ 60 秒。来源：[重启后的恢复](/v2/node/connectivity#restart-recovery)不变量——任一方重启后一个 sync 周期加一个 State 周期（约 35 秒）内取得对方候选，需要打洞的边 60 秒内重新直连。路由器证据：随机映射与入站 DNAT 计数 > 0，两次断开的 DROP 计数 > 0，Server 阶段的恢复晚于 Server 重新服务，各阶段时间单调（`controller/scenario-reconnect-timings.json`、`traversal-packets.json` 的 `reconnect`）。

### 出口场景 {#exit-scenario}

`exit/off`（`--traversal exit`）使用真实 Server／CLI／daemon 与独立身份的 A、R、E：仅 E 启用出口，仅 A 使用 Exit→E via R 的方案。公网分类目标在完全隔离的 internal bridge 中（`11.77.x.0/29`），另有显式授权的精确 RFC1918 LAN；TEST-NET 或 LAN 成功不记为 Internet grant。三端运行隔离的私有 D-Bus／resolved：A 的系统上游是 Resolver 容器（也是 A 的 `useWithExit` split 解析器），E 的系统上游是 external 网段内的隔离 DNS authority；MagicDNS 开启；租约 60 秒。A 的方案以一个 7,000 个 `/20` 区间加 split /32 的 inline 集合规则把它们定为 direct：A 由 artifact 计算并安装约 7,000 个受管区间（egress journal 的计划、nft `managed4` 元素数与 IPC `netmap.capture` 三者一致且不少于 7,000，split /32 不受管，external 与 LAN 目的受管）。所有阶段、进程退出与精确清理通过才算通过。投影 ACK、容量拒绝、审计／清理／变更重试、dump 重读等内部边界由同源码的组件阶段（`dev/exit-fixture/components.py`）覆盖。

| 阶段 | 断言 |
| --- | --- |
| 1–8 基本出口 | 能力／前置条件、saved／applied 与租约、投影（A 的 egress 规则与 fallback、三端相同的外部 Access、E 的 exit role、R 无 role）、IPv6 默认 sink、公网与 LAN、原始与 NAT tuple、nft／conntrack、服务日志、R 只见密文、E 阻断对照、持续／仅接收窗口、`exitEnabled` 撤回、关闭 IPv4 转发时的投影拒绝、journal 恢复。HTTP 中断同时结束在途 sync：已有 TCP 与每 2 秒新建的 UDP 出口流继续往返，直到路径上首个租约到期才结束（节点持有的 peer 事实不早于租约过期）；E 在自身租约到期时撤回 role，owned nft 表与 conntrack 精确清理，A 保留负向 egress 门（`managed4` 非空、无活动 selector）；恢复 HTTP 后无第二次 `up` 取得新租约并以新 circuit 建立新流 |
| 2 前置条件 | E 在 `ip_forward=0` 下启动仍 running（applied＝saved、data plane ready），IPC 与 State 的 `exitCapability` 为 `unavailable`／`ipForwardDisabled` 且未安装角色；打开 forwarding 后不重启、不发新 netmap，周期探测观测到 ready 即以同一内容安装（`ready`、`installedFamilies=[ipv4]`、applied 不变） |
| 9 出口 DNS | A 的计划为 `viaExit`，E 为 `serveExitResolver` 且只服务 A，split 解析器为 `direct`。A 的 `getaddrinfo` 得到 E 的系统 resolver 的回答：A、A＋AAAA（AAAA 原样）、90 条 A 记录（超出 Mesh UDP 1,232 B，只能经 Mesh TCP）；authority 只见 E 的 external 地址；A 的 MagicDNS 名本地解析，未知 MagicDNS 名本地拒绝，均不出现在上游；`split.test` 只从 A 的 split 地址到达 Resolver；E 上加 unreachable 路由阻断 authority 时 A 查询失败且 Resolver 不收到该名；E 撤回与 A 租约到期后失败，租约内 HTTP 中断时成功，恢复后成功；`exit-dns` 字节只出现在 E 的 external 接口；A 的另一条链路设 DHCP 式搜索域 `tko.oraclevcn.com` 时 5 秒内 Status `localDnsDomains` 恰列出该域与链路名，期间每次采样 running／ready、`lastError` 为 null，经出口的名称仍只由 E 解析，链路删除后 5 秒内不再列出（[其他链路的域](/v2/node/dataplane#foreign-domains)，DNS-2 D5） |
| 10 域名出口 | 名称规则 `*.name.exit-dns.test` → Exit E via R、其余 direct、Access `domain:name.exit-dns.test`：A 的捕获在关联期间恰含服务地址；不经名称直连该地址按 IP 规则 direct（失败）；第一次 `getaddrinfo` 一次得到地址（E 的回答被 A 持有至签名关联应用），按名称的 TCP 只以 E 的地址到达服务；同名第二次连接不增加 Server 往返；按名称建立、每 2 秒往返的 TCP 与 connected UDP 持续 100 秒越过关联 60 秒 TTL：到期后不经名称新连失败，持有的 TCP 每次成功、UDP 至多丢 1 次、服务只见 E；流结束后地址退出捕获；撤销 `domain:` 后按名称失败 |
| 11 方案切换 | assignment 允许 `exit`、`exit-alt`、`mesh`、`global:E`：`meshway exit exit-alt` 在 A 应用带该版本的 netmap 后返回，切换后旧 socket 的往返全部失败、新连接经 E；`meshway exit off` 后无受管区间与默认捕获、外部服务不可达；`global:E` 下 A 保持 running，split 地址仍是 direct 例外、只由 A 自己问到 Resolver |
| 12 ICMP、PMTU、MSS | 另有只转发 IPv4 的 router 连接 external 与 `far` 网段（far 接口 MTU 1,280）。`traceroute -I` 第 1 跳 E、第 2 跳 router、第 3 跳目标，后续跳比第 1 跳多出 ≤ 20 ms（同轮到达的投影合并安装）；64 KiB TCP echo 经 fragmentation needed 收敛，A 路由缓存 `mtu 1280`；router 伪造 5 条引用不存在 tuple 与 30 条引用该流 NAT 后 tuple 的 time exceeded，A 只收到后者且在每流每秒 10 条预算内（≥ 1 条）。A–R 链路 MTU 1,400 时通道为 `1400/1286`、MSS 表限 1,246，经 E 的 64 KiB echo 不限 MSS 也完整返回，`ping -M do -s 1259` 收到 A 自己合成的 `Frag needed (mtu = 1286)`，`-s 1258` 成功 |
| 13 `meshway ping` 外部目标 | 公网 IP 经 E、direct 目标、域名经 E resolver 与关联、撤权后 denied |
| 14 自动方案 | [auto](/v2/configuration-reference/routing#auto) 为 US → `exit`、JP → `exit-alt`、fallback `mesh`，地区依次未声明、US、JP、FR：`{mesh, noTrustedRegion}`（无出口）→ `{exit, label, US}`（经 E，E 从自己的投影重算 A 的 selection）→ JP 时旧流往返全部失败、旧关联删除、新名称经 E → 手动 `exit` 覆盖后 `auto` 回到 JP → FR `{mesh, noRuleMatched}` → `--default` 清除后回到默认 `auto`（`requestedProfile` null、`version` 保留、`cleared=true`）；`routing.autoResolve` 活动每次解析变化恰一条、手动覆盖没有（`auto-profile.json`、`controller/auto-activities.json`） |
| 15 echo 寿命、容量与恢复 | ① A `ping -i 0.2` 60 秒加每 2 秒一次 traceroute：300 个回复全部收到，E 的投影峰值 ≤ 46，角色始终已安装；② E 两次删除该 echo 的 conntrack 条目（CT 换代），角色保持、无 `Exit owner stopping`；③ 删除 E 回程表中 A 的 `/32`：只撤角色（`Exit role failed; withdrawing it alone`）并 25 秒内重装，E 每次采样都 running／ready／applied；A 的 MagicDNS 全部成功，经 E 的普通名称只允许在中断窗内失败（窗以 E 日志的停止时刻起、重装后首份就绪能力被确认＋1 秒止，失败次数 ≤ ⌈窗长／0.5 s⌉＋1，窗外 100 %）（`echo-capacity.json` 的 `recovery.dnsOutage`） |
| 16 吞吐与队列压力 | A 对 E 的 Mesh 地址与经 E 对 external 各跑 iperf3 TCP 20 秒正反向，只记录不断言（矩阵为 debug 构建，只比较同行比例）；A 经 E 的一条 TCP 流 20 秒，同时每 0.2 秒 ping 同一服务（不断安装／退役投影）：每个 1 秒区间都有数据（≥ 0.1 Mbit/s）、echo 回复 ≥ 95 %、角色保持；通用抓包在本阶段前后分段。诊断（只记录不断言）：本阶段全程另抓 E 的出口 TUN 与 A 的 Mesh TUN（A 不含 Mesh iperf3，`-s 128`、每文件至多 600,000 包，约 86 MB 上限），A／E 每 1 秒采样 TUN 计数、Status 摘要（phase、数据面、角色与出口流数、peers），E 另采 conntrack 数与 `Exit queue full` 计数、A 另采 iperf3 socket（`ss -ti`），并按秒汇总三端日志事件；iperf3 超时时记录两端 socket、E 的 conntrack 与 Status（`exit-throughput.json` 的 `diagnostics`／`stuck`，`*/phase16-*`） |
| 17 已接受的入站连接 | A 选全量出口方案（`exit`）期间，Controller 经 A 的控制链路（`ca`，A 的捕获管理该地址）向 A 的 TCP echo 服务建立一条连接，每秒一次小包往返：捕获下 ≥ 30 秒、A 租约到期（负向门，A `applied` 为 null）后 ≥ 10 次、恢复后 ≥ 5 次，全程同一连接、每次往返都在 3 秒内应答、相邻样本间隔 ≤ 3 秒；同一时段 A 自己向 Controller 地址发起的新连接在捕获下失败（被捕获）、在负向门下 2.5 秒内被拒绝，A 经 E 到 external 的新流成功（[已接受的入站连接的回包](/v2/node/exit-design#inbound-replies)，X19，`inbound-replies.json`） |

门槛来源：第 15 阶段上限 46＝该负载能同时存活的投影数（每个 echo 序号一条流，回复后 2 秒、无回复 10 秒结束：5／秒 ×（2＋2）秒＋1，加 ⌈10 ÷ 2⌉ 轮 traceroute × 5 个探测），与往返时间无关、远低于容量 128；回复后不结束的 echo 仅 ping 即达 50。第 12 阶段 20 ms 区分“同轮安装”与“逐流一轮”（后者每跳约 +50 ms）。第 16 阶段 95 %／0.1 Mbit/s 区分“队列溢出只丢包”与“流被结束”。第 17 阶段 30 秒＝PROD-1 jp 现场 SSH 中断后到清除选择的时长量级（约 40 秒），每次往返 3 秒内应答区分“回包走 underlay”与“回包进出口后由 TCP 重传侥幸恢复”（被捕获的回包永不到达 Controller）；负向门下 2.5 秒区分本机 TCP reset 拒绝与 connect 超时。吞吐数值见[出口正向写路径](/v2/node/performance#exit-write-path)。

不覆盖：真实宿主 IPv6 出口（见 [exit6/off](#exit6-off)）、源／Access／成员撤销逐项、文件后端出口 resolver、常驻 lab 整组。

### exit6/off 行 {#exit6-off}

`exit/off` 的同一 Server／daemon／fixture，拓扑改为双栈：external 与 far 各取 `3ffe::/16` 内随机 /64（在 `2000::/3` 内且不在排除表中，属 Internet 分类；不用文档前缀），LAN 取随机 ULA /64（经显式 CIDR Access），router 与 E 在自己的 namespace 内开 IPv6 forwarding，服务另在 IPv6 上监听（`bindv6only`）；Mesh 控制与数据链路仍为 IPv4。E 的 `exitFamilies` 为 `["ipv4","ipv6"]`；R 为只有 IPv4 的第二个出口（方案 `exit-ipv4`）。阶段编号沿用 [IPv6 出口设计](/v2/node/ipv6-exit-design)；第 9 阶段（出口前缀变化）属 I6-2，不运行；不跑组件阶段（同一构建由 `exit/off` 覆盖）。

| 阶段 | 断言 |
| --- | --- |
| 1 能力 | E 的 acquire 有 `feature:exit-ipv4`／`feature:exit-ipv6` v1；IPC `exit.capability.families` 两族 `ready`、`installed`；E 的角色两族、R 只有 IPv4；A 的 `eligibleExits` 带 E 的两族；external 与 LAN IPv6 目的在 `managedIpv6`、不在 `sinkIpv6`，`ip -6 route get` 指向 Mesh TUN |
| 2 前置条件 | 一条 IPv4 出口长流全程不断；`all.forwarding=0` → 只有 IPv6 族 `ipv6ForwardDisabled`，恢复后回 `ready` 且不重装（同一 `installedRole`）；`accept_ra=1` 加 `proto ra` 默认路由 → `ipv6RouterAdvertisementsIgnored`，撤去后回 `ready` |
| 3 包流 | IPv6 TCP、UDP、`ping -6` 到 external，TCP 到 ULA LAN；服务看到 E 的 external（LAN 为 E 的 LAN）IPv6 地址；E 有带 mark 的 IPv6 conntrack |
| 4 PMTU | far 1,280：64 KiB TCP echo 收敛，`ping -6 -M do` 收到 router 的 packet too big 1,280、缓存 `mtu 1280`；E 外部接口 1,300 时 E 自己的 packet too big 到达 A；`traceroute -6 -I` 依次 E、router、目标 |
| 5 MSS | A–R 1,400（`inner` 1,286）：MSS 表有经出口的 IPv6 规则（`inner` − 60），IPv6 TCP 的 MSS 不超过它；A–R 1,380（`inner` < 1,280）：IPv6 出口 `connect` 失败并收到本机 ICMPv6 1/0 |
| 6 撤回 | `exitEnabled` 撤回后 IPv6 长流终止、无带 mark 的 IPv6 conntrack、journal 的 quarantine 含 IPv6 回程 tuple；恢复后从 journal 装回 `quarantine6`／`echo_quarantine6`，新流成功 |
| 7 只有 IPv4 的出口 | E 的 IPv6 族运行时不可用：IPv6 `connect` 1 秒内 `EACCES`、IPv4 照常，`meshway ping` 判定 `exitUnavailable`；切到 R：IPv6 目的在 `sinkIpv6`，判定 `unsupportedFamily`，`connect` 立即被拒 |
| 8 DNS 与关联 | 名称规则经 E：AAAA 经 E resolver 原样返回，关联把该 IPv6 地址加入 `managedIpv6`，只用 IPv6 按名称连接经 E 成功；撤权与恢复 |
| 10 方案切换 | 切到 `mesh` 后无受管 IPv6，IPv6 出口长流不再有回复；切回 |

结束时 `down` 恢复三端 namespace 基线（地址、两族路由与规则），外来 sentinel 不变，没有出口表与带 mark 的 conntrack，A 物理接口上没有这些 IPv6 目的的明文包。不覆盖：真实宿主 IPv6 出口（it7、Planck 都没有全局 IPv6，见[真机现状](/v2/node/ipv6-exit-design#real-hosts)）；出口前缀变化、临时地址与 `roam/exit6`（I6-2）；IPv6 的租约到期全撤与迟到回包被 quarantine 拒绝的显式注入（IPv4 由 `exit/off` 覆盖，同一实现）。

### 漫游行 {#roam-row}

`roam/direct`（`--traversal roam`）是本行自有的 6 容器拓扑：A 有两个上联（uplink1／uplink2，各经一个端口保持型 SNAT 路由器到 WAN；router-a1 另把 WAN 上的 UDP 35000 转发给 A）、一个与 C 共享的 LAN，以及只有 A 与 Controller 的 IPv6 网络 `srv6`（A 经它连 Server；STUN 与对端仍为 IPv4）；Controller、公网 B 与 C 在 WAN 上。topology 只允许 A–B 与 A–C，每条边只有一方能拨出（常驻边）；租约 1,800 秒，不让租约换代混入测量。A 对 B 持续 100 pps 带序号 UDP、10/s ping 与每 50 ms 一行的 TCP echo，对 C 同样（无关通道）。

| 阶段 | A–B 方向 | soft | hard | dual-stack |
| --- | --- | --- | --- | --- |
| `dialer` | A 拨号 B 的已发布 listener | 默认路由改到 router-a2，再删 uplink1 地址 | 断开 uplink2，接入 uplink1 并设默认路由 | 接回 uplink2 等 6 秒，只把 IPv4 默认路由改到 router-a2（链路、地址与到 Server 的 IPv6 路由不变） |
| `listener` | B 拨号 A 在 router-a1 发布的 listener | 同上 | 同上 | 同上 |

计时起点为新默认路由生效（A 的时钟）。每格观察 12 秒。门槛（D14，见[漫游设计](/v2/node/roaming-design)）：首个内层 UDP 回复 ≤ 3 秒、A 的业务 circuit 集合不变（State 的 `paths`）、TCP 未重置、A–C 通道 ID 不变；dual-stack 格另要求 A 1 秒内把变化分类为 path 并有一次 Manager 漫游反应。另记录 ping 与 TCP 的首个回复、双向丢包、最长中断、channel ID 与 remote、候选发布时刻与漫游 debug 事件。不覆盖：真实 sleep／wake、Wi-Fi 切换（真机见[漫游真机](#real-hosts)）。

### roam/nat 行 {#roam-nat-row}

`roam/nat`（`--netmap-scenario roam-nat`）在漫游行拓扑上把 B 放进自己的 NAT（router-b）；三台路由器都是端口保持型 SNAT、只放行本侧发起流的回包、无转发端口；A、B 的 Native UDP listener 都无发布端点，A–B 为常驻边，只能经协调穿透连通。两格：soft 与 hard（同漫游行；soft 结束后 router-b 删除为 router-a1 保留的回包，使 hard 也是 B 的 NAT 没见过的映射）。每格观察 25 秒。门槛：首个内层 UDP 回复 ≤ 5 秒（D14）、circuit 集合不变、TCP 未重置、A–C 不变、A 取得新通道、新穿透的首个 prepare 来自 A、B 在仍持有旧通道时旁接受（offer 方向可与旧通道相反）；另记重新获取时刻。TCP 最长中断不设门槛（空窗约 3 秒时为 3.2 或 6.4–6.7 秒，取决于内核 RTO 退避）。

### roam/relay 行 {#roam-relay-row}

`roam/relay`（`--netmap-scenario roam-relay`）在 `roam/nat` 拓扑上加公网中继 R（A、B 都拨它，A–R、R–B 常驻），无 `via`，Policy 候选为 `[A, B]` 与 `[A, R, B]`。A 每 250 ms 用新 5 元组发一个 UDP 报文（每个都是新流：各自绑定 20000 起的源端口，1,000 秒内不重复，在临时端口范围之外——内核选的临时端口关闭后会被随机复用，A 流表 60 秒空闲内的同一 5 元组是原流，见[漫游中的策略中继](/v2/node/connectivity#roaming)；发送失败的流按无回复计入）；A 失去对 B 的直连时启动 100 pps 的“中继流”，恢复直连 2 秒后启动 4 秒的“恢复后流”；R 的收包每 250 ms 采样。门槛（在 `roam/nat` 之外）：

- 旧直连流最长中断 ≤ 重新获取＋1 秒（已有流不迁移，空窗就是重新穿透时间，D7）。
- 空窗（失去直连 0.25 秒后至恢复前 0.25 秒，记为 `gapWindowS`）内每个新流 1 秒内经 R 得到回复，且中继流启动前 R 的收包速率 > 0；窗内没有新流时为不适用（`newFlowsThroughR: null`，不是通过）；未测到失去直连或恢复时刻为失败。
- 直连恢复后中继流仍经 R（R 收包 ≥ 150 pps）且最长中断 ≤ 1 秒；恢复后流得到回复且使 R 收包增加 < 50 pps。
- 扰动前的 circuit 全部仍在（空窗内新增一条经 R 的）。

### roam/flap 行 {#roam-flap-row}

`roam/flap`（`--netmap-scenario roam-flap`）用漫游行拓扑、A 拨号公网 B，两个上联始终接入：soft 在 0.4 秒间隔内切换默认路由 a2→a1；burst 在一条命令内连切三次（a2→a1→a2，远小于 250 ms 合并窗）；hard 以 uplink1 链路 down／up 加路由切换往返 5 次（0.4 秒间隔）。门槛：分类出的 path 变化数等于 Manager 漫游反应数（burst 恰为 1，其余 1 至切换次数）；State 报告 ≤ path 变化数 × 16＋1（每次 path 变化后 1.5 秒内至多 16 份，见[快速往返](/v2/node/connectivity#roaming)）；无 circuit 退役、集合不变；B 的通道最终指向 A 最后一次切换所用路由器；首个回复 ≤ 3 秒、TCP 未重置、A–C 不变。结束时 supervisor 等每个节点 `roam-down` 完成（至多 40 秒）后才停止 Controller。

### roam/exit 行 {#roam-exit-row}

`roam/exit`（`--netmap-scenario roam-exit`）用漫游行拓扑、A 拨号公网 B：A 开启 `exitEnabled`、`ip_forward=1`，是 B 的出口；B 的方案 `exit` 只有一条 IP 规则（WAN 上服务 T 的 /32 经 A，回退 direct，无 DNS 计划），Access 允许 B→T 的 UDP 42424／TCP 42425／ICMP。T 记录每个新 UDP 来源与每个接受的 TCP 对端。B 对 A 持续 Mesh 流量，A 对 C 同样；每格前 B 对 T 启动一组新出口流量，扰动同 `roam/direct` 的 soft、hard；12 秒后 B 对 T 新建一条 TCP 与一个 UDP 流。A 的 `exitCapability` 每 0.2 秒采样。门槛：B→A Mesh 首个 UDP 回复 ≤ 3 秒、B 的 circuit 集合不缩小、Mesh TCP 未重置、A–C 不变；能力先报 `probing`、5 秒内回到 `ready` 且仍装有 IPv4；本格的旧出口 TCP 连接结束（报错或新路由 3 秒后不再有回显）；新 TCP 与 UDP 流得到回复，T 看到的新 TCP 来源是 A 的新上联路由器；A 没有出口角色停止／失败记录；行结束时各节点 `down` 成功。

### fault/throttle 行 {#fault-row}

`fault/throttle`（`--traversal fault`）把现场手工做过的外部压力变成常规验收，持续证明两条不变量：暂态阻断由命名 owner 有界重证后以同一 netmap 自动恢复，期间 Status 为 `retrying`＋最早原因，只有已证实的不一致才是 `blocked`（[撤回期间的原因](/v2/node/runtime#ownership-end)）；事件循环没有电平分支，故障期间与恢复后空闲 CPU 有上限。拓扑是本行自有的一个 internal bridge：Controller（真实 Server）、A、R、B、E。A→B 经 R；B 的方案把一个文档地址经出口 E（E 的默认路由指向 bridge 上无人持有的下一跳，没有报文离开 bridge）；四个节点各在自己的 mount namespace 运行 systemd-resolved、resolved 后端与 MagicDNS；静态 Native UDP 端点、常驻边、租约 1,800 秒（Control 中断不越过租约）。A 以 5 pps 向 B 发 UDP 回显，各节点每 500 ms 记录 Status。注入全在 daemon 进程之外、只作用于本行资源：容器 CPU 限额（`docker update --cpus`）、namespace 监督进程重启 resolved（新进程与新 D-Bus owner）、Controller 自己 namespace 中的 nft 表；不改宿主路由／防火墙／sysctl，不制造宿主 RTNL 停顿。基线测一个 CPU 窗口，之后五段，每段在被施压节点恢复后测 60 秒 CPU 窗口。fixture 见 `dev/fault-fixture/README.md`。

| 阶段 | 注入 | 门槛（除共同项外） |
| --- | --- | --- |
| `cpu-long` | B 限额 0.02 核 90 秒 | 出现撤回；撤回期间 `lastError` 非空、出现 `retrying`；解除后 ≤ 10 秒 B running／ready 且 applied 为基线 serial；A 的回显 ≤ 10 秒恢复；之后不再 `suspended`；记录走 X12 变更重试、X13 `verify_unavailable` 还是只有 DNS（不作门槛） |
| `cpu-short` | B 限额 0.02 核 30 秒（DNS 先自行重证的顺序） | 同上；每次撤回 WARN 都有一条重装 INFO |
| `resolved-restart` | B 的 resolved 重启两次，间隔约 10 秒 | 重启后 ≤ 5 秒 A 的 Mesh 名再次回答；B ≤ 10 秒 running／ready、同一 serial；故障期间 CPU 上限 |
| `control-outage` | Controller 丢弃 B 的 TCP 8443 双向 100 秒（长于一次 sync 的 90 秒预算） | 丢包计数 > 0；B 每个样本 running／ready、同一 serial，出现 `lastError`／`syncError: network`、从不出现 `networkError`；A→B 回显最长间隔 ≤ 2 秒；恢复后 ≤ 70 秒（退避上限 15 秒＋最长 hold 54 秒）`sync: connected`、错误为 null；故障期间 CPU 上限 |
| `exit-audit` | E 限额 0.02 核 12 秒（其 nft 读取超出 2 秒预算） | E ≤ 10 秒 running／ready、≤ 30 秒（owner 的未验证上限）出口能力再次 ready 且已安装；A→B 最长间隔 ≤ 2 秒；记录是否出现审计 `unavailable` |

共同项：全程所有节点没有 `blocked`；撤回次数 ≤ 10 且等于重装次数；空闲窗口内 A 的回显最长间隔 ≤ 2 秒；四个 meshwayd 的 CPU 由 `/proc/<pid>/task/*/stat` 两次采样之差计算（缺测为失败）：**最忙线程 < 5 %、整进程 < 10 %**（单核百分比；本行为 debug 构建、每秒两次 Status 读取与 5 pps 业务流）；行结束时 applied 仍为基线 serial。门槛来源：10 秒恢复与 30 秒未验证上限为 owner 的重证节奏（[归属的结束与重证](/v2/node/runtime#ownership-end)）；撤回 ≤ 10 区分有界撤回与 X15 的约 37,000 次／秒循环；线程 5 % 区分 X18 的电平分支（一线程约 100 %）与 release 空闲 0.56 %，也是现场 soak 的门槛（[v0.1.0 门槛](/v2/process#v0-1-gate)）。门槛的唯一实现在 `dev/fault-fixture/evidence.py`，离线正负例 `scripts/test_fault_fixture.py`。判别力：同一 fixture 摘入 alpha.19（`a41791bf`，X18 之前）在 CPU 门槛失败，摘入 alpha.14／alpha.15 在撤回期间 `lastError` 为 null（H4 之前）失败（`.meshway/validation/f7/`）。

不覆盖：X12 变更重试与 X13 `verify_unavailable` 路径——B 的 egress capture 规则集小，限额下的撤回只经 DNS（`observe_deadline`），E 的 12 秒限额也只有 DNS 撤回；这两条路径由各自的隔离组件测试证明（见[已修复问题索引](#fixed)）。underlay 的测试专用工具故障不能从 daemon 进程外触发。

### netmap 生命周期行 {#netmap-rows}

| 行 | 场景 | 断言 |
| --- | --- | --- |
| `netmap/offline` | 本机上报端点（无 configured／STUN，拨号候选只来自 peer 事实）；关闭 Node HTTP 3 分钟（三个 60 秒租约期，期间续租与过期判断都发生） | 持续流不断，Status 不撤回；常驻 lab 另做 10 分钟离线复验 |
| `netmap/revoke` | Native TCP；依次撤一个 TCP 端口服务再恢复、B 经 daemon 轮换自己的密钥、撤销成员 | 每个新 serial 5 秒内到达数据面：被撤端口拒绝、恢复后可用；轮换后 A、R 的 circuit 重新握手且流量继续；被撤成员不可达 |
| `netmap/expiry` | 关闭 Node HTTP（503）越过 60 秒租约 | fail closed：业务阻断、拒绝路由保留；Status 去掉租约后 2 秒内 TUN 删除（记录 `scenario-expired-tun-removed-ms`）；恢复后自动重新 applied |
| `netmap/restart` | 本机上报端点；关闭 HTTP 后停业务 125 秒（越过 120 秒按需邻接空闲关闭与全部事实的新鲜度），再重启三个 daemon | 空闲关闭后 A 的 `meshway ping` 与新 5 元组经重拨成功（`scenario-idle-redial-*`）；重启后的邻接只能来自 `peer-facts.json` 的[最后已知事实](/v2/node/connectivity#last-known-facts) |

#### 租约到期撤回 {#lease-expiry-withdrawal}

Status 的 `netmap` 按查询时的时钟判定租约（`now < expiresAt`），到截止即为 null、`dataPlane` 为 `suspended`；Engine 在同一截止由单调计时器触发 `expire`，依次撤回 connectivity、dataplane、DNS、underlay 并删除 TUN，耗时数十毫秒。因此 Status 先于 TUN 删除变化，这是预期时序，不是“到期未撤”；行门槛据此为“Status 去掉租约后 2 秒内 TUN 删除”，之后照旧检查拒绝路由与业务阻断。截止到撤回完成之间，worker 在撤回前处理中的至多一批（32 包）不逐包检查租约，这一窗口未单独测量。

## 真机证据 {#real-hosts}

常驻环境：lab 三容器 lax／sha／nyc（OrbStack，经 Mac 的 Cloudflare WARP 出网）、`meshway-vm`（OrbStack machine，systemd）、us-lax-it7（x86_64 公网、无 NAT、只能拨出）、Planck 容器 `meshway-planck`（Docker masquerade 后）与开发 Server，见[常驻 lab](/v2/node/docker-lab)。每条为一句结论与最近一次证据；部分检查不替代整组。

| 能力 | 已证明的事实 | 证据 |
| --- | --- | --- |
| <a id="real-nat"></a>端点相关映射穿透 | it7（只能拨出）↔ VM（WARP：同一 socket 对不同目的不同外部端口）经来源学习 `direct native_udp`，双向 `meshway ping` 5/5；netcheck 在 VM 判 `endpoint-dependent`、在 it7 判 `endpoint-independent`；外层全部带 DF、0 分片 | N2a，2026-10-13，`.meshway/node-lab/n2a/` |
| <a id="n3-real-network"></a>keepalive 与不可直连形态 | lab↔VM 部署后无流量即直连；`* ↔ *` keepalive 六向数秒内 `direct` 且端点稳定；lab／VM ↔ Planck 容器双向不可直连（WARP 端点相关映射 × 端口相关过滤：lax 的 punch 到不了 Planck 网卡），设计上只能中继 | N3，`.meshway/node-lab/n3/` |
| <a id="n4-real-network"></a>经第三节点中继 | lax／sha／nyc／VM → Planck 各 10/10 `via relay us-lax-it7`，经 Mesh 的 HTTP 200；无关配置修订不重置候选退避（failures 1–9，暂停 1→256 秒） | N4，`.meshway/node-lab/n4/` |
| <a id="n5-real-network"></a>Server 与 daemon 重启自愈 | Server 更新后 ≤ 38 秒、`dev.py restart server` 后、只重启一端 daemon 后、`docker restart` 后均自动回到应直连的边 `direct`、其余 `relayIdle` | N5，`.meshway/node-lab/n5/` |
| <a id="m1-real-network"></a>通道 MTU 与 MSS | it7→VM 不限 MSS 的 TCP 由 0 变为 408／394 Mbit/s；it7 合成 `Frag needed (mtu = 1301)`；纯 IP 双出口六台 running；名称规则分布在两个出口时保存 422 `dns_exit_plan_required` | M1，`.meshway/node-lab/m1/` |
| <a id="x7-exit-hosts"></a>真实出口宿主 | Planck 容器（`rp_filter=2`）作出口：`ping` 4/4、`traceroute -I` 依次出口 Mesh 地址、Docker 网桥、目标，SMTP 握手成功；it7（Docker 28 iptables-nft）`exitEnabled` 时防火墙判定无冲突、外来 nft 对象前后不变。未完成：经 it7 出口的公网 HTTP，M3 后（ctnetlink）it7 出口能力复验 | X7，`.meshway/node-lab/x7/` |
| <a id="u1-startup-self-heal"></a>启动期自愈 | lab 八轮重建中 namespace inode 多次复用，每次按 cookie 归档旧记录、无 `Underlay startup failed`；listener `EADDRINUSE` 1 秒后重试绑定成功并清空 `lastError` | U1，`.meshway/node-lab/u1/` |
| <a id="d3-dns-health"></a>宿主 RTNL 停顿 | Planck 宿主 Wi-Fi（ath12k）每 8 h 24 min 的 PTK 重新协商使宿主 netlink 停顿约 5 秒；D3 之后节点 `observe_deadline` 后约 0.15 秒恢复、0.3 秒内重装，不再阻断至重启（4.11 秒停顿一次，249 份 Status 仅 1 份 `preparing`）；本机 lab 以拆除大量 dummy 接口复现 6–13 秒停顿均自动恢复 | D3，`.meshway/validation/d3/`、`.meshway/node-lab/ops10/d3-rekey/` |
| 漫游（lab） | lax 换 bridge（soft）ping 无中断；断开再接回（hard）sha／nyc 0.11 秒内经挑战跟随新来源，最长中断 ≤ 0.62 秒，channel ID 不变 | R1 `.meshway/validation/r1/lab/`；OPS13 |
| 漫游（VM） | 加第二地址／改 `src`／恢复三次都分类为 path、每次重新发布 STUN 观测；sha→VM 最长中断 0.31 秒，it7 跟随 NAT 映射往返 | R3 `.meshway/validation/r3/vm/`；OPS14 `.meshway/node-lab/ops14/r3-vm/` |
| <a id="dns2-hosts"></a>公网主机上的 resolved 后端 | kr（Ubuntu 22.04，systemd 249.11-0ubuntu3.23）resolved 对锚点链路上的探测域回答 “No appropriate name servers or networks”、探测 5 秒无回答（不是防火墙）；隔离复现：同一 resolved 把发往链路 `127.0.0.1:<port>` 的查询从未送达，Ubuntu 24.04（255）与 Debian 12（252.39）送达。jp（OCI）enp0s6 的 DHCP 搜索域 `tko.oraclevcn.com` 使全量出口方案 `foreign_domain` | PROD-1 `.meshway/prod/README.md`、`.meshway/prod/migrate/kr-dns-unsupported.txt`；DNS-2 `.meshway/validation/dns2/` |
| 出口 DNS 与域名出口（lab） | lax 经 Exit nyc via sha 时名称只经 nyc 的 resolved 发往上游，阻断上游时查询失败；`geosite:openai` 只经 nyc 出口；nyc resolved 把 Mesh 地址作 DNS 服务器后 0.5 秒内报 `exitDns: unavailable`、源 SERVFAIL、恢复后再解析 | X5／X5b／X6，`.meshway/node-lab/x5/`、`x6/` |
| 空闲 CPU | alpha.21 六台 meshwayd 空闲最忙线程（`top -H` 快照）：lab 1.0–2.0 %、VM 0.4–0.6 %、Planck 0.4 %、it7 3.8–7.6 %；X18 修复后 VM release 空闲 0.56 %。it7 按 60 秒口径：两个 worker 各 3.1–3.2 %、进程 7.0–7.3 %、`nft` 子进程另约 4.2 %，逐项归因见[空闲 CPU 口径](/v2/node/performance#idle-cpu)（剩余边界 P2） | OPS20 `.meshway/node-lab/ops20/cpu-idle*/`；X18；P2 `.meshway/validation/p2/` |
| 吞吐 | 常驻 lab 与 Planck 的分层吞吐、出口正向写路径与 WARP 路径限制见[分层性能测量](/v2/node/performance)与 [OrbStack 与 WARP](/v2/install#orbstack-warp) | performance.md |
| 系统服务 | 整机重启（含 Server 不可达）无需 `up` 恢复 running；resolved 重启 5 秒内、停止再启动 10 秒内恢复 Mesh 解析；Server 安装、升级、回退与备份 | [Linux 安装 · 验收记录](/v2/install#验收记录) |
| <a id="prod-soak"></a>prod 7 天 soak | 运行中：alpha.23，T0 2026-10-24T00:15Z（D1＝10-24，D7＝10-30），五台公网节点（it7、jp-tyo-arm64 物理机；kr-icn-arm64、planck、cn-hkg-dmit 容器）连同 planck 容器中的 Server，按[里程碑 C](/v2/process#v0-1-gate) 的日程注入故障，门槛与判定见同处；T0 前基线：10 条边 `meshway ping` 各 10/10，60 秒窗口最忙线程 ≤ 1.8 %（it7 进程 3.9 %） | 采集机 kr-icn-arm64 `~/meshway-soak/reports/`（日报与 `final.md`）；工具 `scripts/soak/` |

## 已修复问题索引 {#fixed}

每行一个已修复的产品缺陷；修复说明与不变量在对应 changelog 版本段，原始证据在 `.meshway/validation/<id>/`。fixture 自身的修正不列入。

| ID | 症状 | 根因 | 修复版本 | 回归守护 |
| --- | --- | --- | --- | --- |
| X5b | `exit/off` 中 E 的租约在持续窗口内到期 | sync 长轮询 hold 只看 hold 上限，可越过节点持有的租约 | alpha.3 | hold 以节点租约为界；所有 runtime 行的租约续租 |
| R3＋X6 | 撤回恢复后 A 的 DNS 不回答 | 每批 256 条路由变更溢出内核监视器通知缓冲（ENOBUFS），保护失效 | alpha.4 | 每 64 条等待；`exit/off` 7,000 区间分割 |
| F4a B3 | 关联到期后已建立的出口流被丢弃 | 已建立流每包按新判定重新准入 | alpha.5 | `exit/off` 第 10 阶段持有流 |
| E1 | 出口 PMTU 例外后 A 失败 | egress owner 把内核 PMTU 缓存例外当作外来路由 | alpha.5 | `exit/off` 第 8、12 阶段 |
| X7 | Planck 容器作出口 `ping` 0/3；it7 整机 `not applied` | 出口 TUN 无 IPv4 地址时 loose `rp_filter` 一律 `EXDEV`；Docker 限定网桥的 DROP 被判为防火墙冲突 | alpha.5 | `exit/off` 第 2 阶段；[真机](#x7-exit-hosts) |
| N4 | 中继选不到唯一可用的 it7；新流被固定在无回复的旧路径 | 候选路径截断到 4 条；新流加入排序更低路径上的旧 circuit | alpha.5 | `nat/relay` |
| N5 | Server 重启后需打洞的边全部 `unreachable`，只重启一端无效 | 穿透 socket 的 Underlay 授权仍指向旧 instance（`target_not_authorized`） | alpha.6 | `nat/reconnect` 的 `server` 阶段 |
| M1 | it7→VM 内层 TCP 0 bit/s；`global:E`＋split resolver 时 DNS 拒绝新 netmap | 无按通道 MSS clamp 与本机 Frag Needed；DNS plan 未计入 split 例外 | alpha.6 | `exit/off` 第 11、12 阶段 |
| U1 | 容器重建后 Underlay 永久冲突退避；listener `EADDRINUSE` 需重启 | 新 namespace 复用旧 nsfs inode 被当作同一 namespace；同端口改绑先 bind 后关旧 socket | alpha.7 | `udp/off` 端口占用检查；`manager::tests::netmap::rebind` |
| H1 | 经出口 traceroute 每跳 +50 ms、默认等待下超时 | 每个 echo 流各自一轮投影安装 | alpha.9 | `exit/off` 第 12 阶段 ≤ 20 ms |
| X8 | 出口 ping 丢包 68–74 %，随后整个数据面撤回 | echo 投影寿命约 57 秒占满 128；CT 换代后 `maintain` 冲突停 owner；出口角色失败被当作数据面失败 | alpha.10 | `exit/off` 第 15 阶段；`exit_recovery::netmap_exit_echo_reconnect_and_route_loss_stop_only_the_role` |
| X10 | 出口反向吞吐只有 Mesh 的 0.43，压力步断流 | 队列溢出结束流、内核操作阻塞已安装流 | alpha.11 | `exit/off` 第 16 阶段 |
| D3 | Planck DNS owner 阻断至重启（四次） | 宿主 RTNL 停顿中断锚点读取，永久结束锚点 guard | alpha.11 | `dns_anchor_survives_suspend_and_owner_restart_until_explicit_removal`；`nat/resolved` |
| P1 | HTTP 中断内新出口流在租约到期前被拒（`netmap_denied`） | peer 事实按 `observedAt`＋60 秒到期，hold 不为续期唤醒 | alpha.12 | `exit/off` 第 8 阶段 |
| <a id="x11-exit"></a>X11 | 出口审计一次读取失败即停角色，清理失败后 `ownershipConflict` 至重启 | 读取失败被当作不一致 | alpha.13 | `exit_audit_read_failures_are_unknown_and_a_failed_cleanup_completes_on_retry`、`netmap_exit_failed_cleanup_is_retried_and_the_role_installs_again` |
| <a id="x12-change"></a>X12 | 限额下 egress capture `pause` 失败结束宿主内核归属；本端换 boot 时对端空等 15 秒 | 变更失败被当作已知结果；本端换 boot 不退役 circuit | alpha.14 | `exit/off` 组件 `changes-egress`／`changes-exit`；`own_new_boot_ends_every_circuit_at_once_and_its_close_reaches_the_far_end` |
| R3 | 出口节点漫游后 `down` 超时 | 内核为同一原始 tuple 新建的 conntrack 条目被判为外来 | alpha.15 | `roam/exit` 行结束 `down` |
| <a id="x13-reproof"></a>X13 | 归属暂态结束后安装、控制中继、DNS 全部拒绝直到重启 | 整个 Host 共用的 conflict token 取消后无重证途径 | alpha.15 | `an_unavailable_ownership_end_is_proved_again_and_a_foreign_object_keeps_it_ended` |
| <a id="x15-livelock"></a>X15 | 限额解除后不自愈、meshwayd 约 110 % CPU | worker 的 `biased` select 中 DNS 失败与恢复两个电平分支轮流就绪，重证永远选不到 | alpha.16 | `a_recovered_dns_fault_and_an_ended_term_reach_the_reproof_without_spinning`；`fault/throttle` 撤回次数 |
| F6a B1 | 被删的 capture 规则不再重装 | 读表失败时重装通知已被消费 | alpha.17 | `roaming::a_repair_whose_read_fails_stays_owed_and_completes` |
| <a id="s4"></a>S4 | 升级后节点 CAS 连续 `version_conflict`；Planck `synchronizing`／`suspended` 而包照通 | Server 启动不重编译另一构建的 netmap；`Health::failed` 忽略确认前已重证的故障 | alpha.18 | `a_start_by_another_build_recompiles_and_the_nodes_next_sync_receives_it`、`a_fault_re_proved_before_the_plane_looked_is_one_failure_and_one_recovery` |
| <a id="h4"></a>H4 | 撤回期间 `lastError` 为 null、无重装日志、`observedAt` 停在故障前 | 撤回报告在清理结束后才发布；日志只在 DNS 恢复分支 | alpha.19 | `fault/throttle` 的 `retrying`＋原因与重装 INFO；`status::tests::*` |
| <a id="n6"></a>N6 | `nat/resolved` 恢复后 prepare 一直 `notAvailable` | Control 时钟先读单调后读墙钟，延迟被永久计入，节点时钟超前 Server 的 10 秒期限 | alpha.19 | Control 时钟单测；`nat/resolved` |
| <a id="x18-tracker-spin"></a>X18 | 每台 TUN 节点一个线程 100 % CPU | Status 跟踪任务克隆的 DNS guard 未 `mark_unchanged`，`changed()` 永远立即返回 | alpha.20 | `the_tracker_is_not_woken_by_a_dns_guard_send_it_has_seen`；`fault/throttle` CPU 门槛 |
| <a id="dns2-d4"></a>DNS-2 D4 | kr（Ubuntu 22.04）`DNS probe check failed (system DNS backend is unsupported) … no reply within 5 s`，原因不明 | systemd-resolved 249.11 把链路 DNS 查询的 socket 绑在锚点链路上（`SO_BINDTOIFINDEX`），发往 loopback bind 的查询到不了；systemd 5f9041af（253，稳定版 249.15／250.10／251.11／252.5）才对 localhost 免绑。锚点无法以两版都支持的方式成立 | 未发布（DNS-2） | 探测报明确的版本要求（`probe` 的 `expected`／`actual`）；`jammy/resolv-conf` 的 resolved 检查；[Ubuntu 22.04](/v2/install#ubuntu-2204) 显式文件代理 |
| <a id="dns2-d5"></a>DNS-2 D5 | jp（OCI）选 fallback＝exit 的方案时 `retrying · dns`（`foreign_domain`：enp0s6 的 DHCP 搜索域 `tko.oraclevcn.com`），整个包平面挂起 | `SystemDnsPlan::conflicts` 把计划的兜底 `~.` 当作覆盖一切名称，任何其他链路的域都判冲突 | 未发布（DNS-2） | `a_fallback_yields_to_specific_foreign_domains_and_explicit_domains_do_not`、`local_domains_follow_successful_observations_and_clear_with_a_fault`、`local_dns_domains_are_read_live_from_the_dns_owner`、`domains_resolved_by_the_local_network_are_listed`；`udp/resolved` 的 resolved 检查；`exit/off` 第 9 阶段 |
| <a id="x16-dump"></a>X16 | `exit/off` 第 15 阶段 echo 299／300（约每 10 轮一次） | 同一 netlink socket 上的并发 dump 被内核以 `EBUSY` 拒绝，出口投影安装失败、首包丢弃 | alpha.21 | `dumps_of_one_socket_are_taken_one_at_a_time`；`exit/off` 组件 `dumps-exit` |
| <a id="s5"></a>S5 | `compilerVersion` 升级后全部 State 503 | 旧编译器的 delta 基准按当前格式解码 | alpha.21 | `a_base_another_compiler_wrote_is_no_base_for_state_or_sync`、`a_start_after_a_compiler_upgrade_compiles_auto_again` |
| <a id="rc1-recovery"></a>RC-1 | 暂态安装失败约 20 秒后 `blocked`；已证实的 DNS／中继／出口不一致显示 `retrying` 并被自动越过；持久冲突需要重启；Status 读 worker 快照（G1–G13，[恢复语义](/v2/node/runtime#recovery)） | 各 owner 各写一份分类、期限与 Status 拼合 | 未发布（`rc1/recovery`） | `recovery::tests::*`、`kernel::tests::classification`、`lifecycle::tests::*`、`status::tests::a_transient_failure_retries_and_only_a_proven_one_blocks`；`fault/throttle` |
| R5 | `roam/exit` soft 格 `exitReadyAgain`：出口能力 6–7 秒才回 `ready` | 路径变化时在途的 State 报告以网络错误失败后按 5 秒重试 | 未发布（RC-1 M9） | `a_network_failure_right_after_a_path_change_is_retried_at_once_once`；`roam/exit` |
| G14 | `fault/throttle`：B 限额解除后 running，A→B 回显 5.3–9.5 秒后才恢复 | 中继对下一跳换 boot 时仍在握手的 circuit 留下死 CID，源端每 250 ms 的 INIT 重试被当作重放，直到源端约 10 秒的期限 | 未发布（RC-1） | `a_restarted_terminal_lets_a_handshaking_origin_retry_through_the_relay_at_once`；`fault/throttle` 包流 ≤ 0.3 秒 |
| G15 | `fault/throttle`：限额解除瞬间多一次约 150 ms 撤回／重装 | 重装的 hold 之后、install 之前观察失败的故障在成功的 install 之后仍在 | 未发布（RC-1） | `a_successful_installation_supersedes_an_earlier_fault`；`fault/throttle` `withdrawalsAtRestore` 0 |
| <a id="x17-starvation"></a>X17 | `exit/off` 第 16 阶段压力步开始时 A 的新出口流（echo、iperf3 SYN）0.9–8 秒无回复，15 轮中 4 次 ≥ 0.9 秒；严重时 iperf3 超时（X17b） | 刚结束的出口流的迟到帧在 A 被逐个拒绝，吃掉一次包处理的唯一 200 µs 配额（含其发送与 TUN 写），本机方向每轮 0 个；本机＋入向超过读取上限后 TUN 与载体数据不再读取，worker 睡到 1 秒节拍或下一批迟到帧 | 未发布（X17） | `each_direction_has_its_own_share_of_a_turn`、`a_turn_left_short_of_time_is_due_again_at_once`；`exit/off` 第 16 阶段 `Local packets held` 的 `oldest_local_ms` |
| <a id="f8b-server"></a>F8b D3-1 | Server 升级在 journal 丢失或查询失败时回退到旧库，而新 Server 已签发过租约 | 回退判据是 `journalctl \| grep` 的日志行 | 未发布（F8b） | `the_first_answer_is_preceded_by_the_durable_served_record`；VM 演练（`LogLevelMax` 丢日志，新脚本拒绝回退） |
| F8b D3-2 | 主密钥错配的 v8 库迁移后新旧版本都打不开 | 迁移先发布 schema 9 再在打开时检查主密钥 | 未发布（F8b） | `a_mismatched_master_key_publishes_nothing`、`the_outcome_of_an_attempt_is_read_from_the_directory`；VM 演练 |
| F8b D3-3 | 停止失败时升级回退与卸载仍替换程序／unit | `systemctl stop … \|\| true`；卸载只查 `active` | 未发布（F8b） | VM 演练（`RefuseManualStop`） |
| F8b D1-H1 | `--artifact` 校验后解包原路径 | 校验与解包读的不是同一份文件 | 未发布（F8b） | `install.sh` 只读一次进私有目录 |
| F8b D3-4 | 备份失败删除输出目录中他人的文件 | 先查不存在再 `open`（接管已有目录），失败时删全部文件 | 未发布（F8b） | `a_failed_backup_removes_only_what_it_wrote`、`a_new_directory_is_created_once_and_an_existing_name_is_never_adopted` |
| F8b D3-5 | 单个迁移步骤超过 120 秒即被启动超时杀掉 | 只按步骤续期 | 未发布（F8b） | `a_step_longer_than_the_budget_is_renewed_throughout` |
| F8b D3-6 | 损坏的库被服务管理器反复重启 | `SQLITE_CORRUPT`／`SQLITE_NOTADB` 归为暂态 | 未发布（F8b） | `damaged_database_pages_are_a_persistent_block`、`rejected_database_is_not_replaced_or_reset` |
| F8b D1-M2 | artifact 下载并发无上限，Server 内存随请求增长 | 无在途预算 | 未发布（F8b） | `downloads_are_bounded_per_node_in_number_and_in_bytes_until_released`、`provider_sets_are_verified_bound_by_digest_and_served_only_to_referencing_nodes` |
| F8b D1-M3 | 登录可无限速猜测口令 | 无失败退避 | 未发布（F8b） | `failed_logins_of_one_account_and_address_back_off`、`accounts::throttle::tests::*` |
| <a id="f8a-review"></a>F8a | REV-D2 审阅（Node 侧）：双栈出口清理重试与"CREATE 已提交、回执未存"的启动恢复判 Conflict 永久重试；IPC 任务退出后协调器忙循环；关闭的事实 watch 空转；jumbo 名额占满而总槽空闲时包调度 `wake_at=now` 忙循环；DNS 私有存储锁占用／I/O 判为已证实；DNS 准备失败启动显示 `retrying`、运行期 `blocked`；中继／listener `blocked.since` 每次读取为 now；dump 未收到 `NLMSG_DONE` 也当作完整；sd_notify 阻塞发送；listener 重试每次 WARN | 写死两张表；已关闭的分支未禁用；容量等待按总槽判断；分类在映射处丢失；Status 重建 owner 状态而非投影 | 未发布（F8a） | `dual_stack_exit_cleanup_retries_and_a_create_without_receipt_is_recovered`（`exit/off` 组件 `cleanup6`）、`a_stopped_ipc_service_ends_the_coordinator_with_its_failure`、`a_closed_publication_is_not_read_again_until_another_is_attached`、`a_head_waiting_for_jumbo_capacity_is_due_when_a_credit_returns`、`a_held_lock_is_retried_and_unsafe_storage_is_proven`、`a_failed_preparation_is_judged_and_published_as_a_running_owners_fault`、`owners_keep_when_their_failure_began_and_listeners_are_read_live`、`a_dump_without_its_end_is_a_failed_read_not_a_complete_one`、`a_full_notification_queue_does_not_block_the_sender_past_its_timeout`、`a_briefly_full_notification_queue_still_delivers`、`a_retried_bind_warns_once_per_kind_of_health`；`fault/throttle` |
| <a id="n7"></a>N7 | PROD-1：节点 `logout` 后改 `MESHWAY_SERVER_URL` 重启即 `blocked · storage`，`lastErrorDetail` 为 null，只能手工改名身份目录；`meshway-server --help` 的 `backup` 显示换钥说明、`rotate-netmap-key` 无说明 | 已注销的空身份记录仍按旧 origin 校验并以无原因的 `Record` 拒绝；Control 问题不带详情进入 Status；clap 说明注释错位 | 未发布（N7） | `a_left_node_moves_to_another_server_and_a_leaving_one_names_its_server`、`a_left_node_moves_to_another_server_and_a_leaving_one_says_why_not`（真实 meshwayd，两个 Server）、`corrupt_record_and_wrong_origin_never_regenerate_identity`、`help_describes_each_maintenance_command` |
| <a id="n8"></a>N8 | 身份目录锁被另一进程持有或读取失败时 daemon `blocked · storage`，原因消失后仍不恢复，需再次 `up` 或重启 | `Control::open` 的失败一律按运行期写入失败（`Recovery::Storage`）判定，没有重开周期 | 未发布（N8） | `control_open_retries_a_held_lock_and_a_failed_read_then_restores_the_cached_netmap`（真实 meshwayd：另一进程持锁、祖先目录 `EACCES`，Server 不可达时载入同一缓存 netmap）、`an_open_retries_a_held_lock_or_failed_read_and_blocks_a_proven_one` |
| <a id="u2-journal"></a>U2 | 宿主停顿后 lab 两台永久 `retrying · underlay`（每 10 秒 `private underlay journal is invalid or requires recovery`，数小时）、`lastErrorDetail` 为 null；同一 namespace 内 `down` 与重启同样被拒 | 重证后紧接的 DNS 撤回中 `suspend` 删除路由时 netlink 失败，base journal 留下 `pending: DeleteRoute`；本期归属未结束，此后每次 `ready()` 拒绝该 journal；启动／重证的 `Store::open` 也拒绝任何待定步骤 | 未发布（U2） | `a_suspend_interrupted_with_its_step_pending_is_reproved_and_installs_again`（`exit/off` 组件 `pending`）、`an_interrupted_step_is_completed_from_what_the_kernel_holds` |
| <a id="cm1-receive"></a>CM-1 B1 | P3 kr↔jp：两端网卡帧数相等、内核无丢弃，接收端写入 TUN 少约 2–3k 段，重传集中在慢启动与 owner 峰值 | 接收 owner 不看事件队列余量就读 socket，队列满时丢弃业务帧（只有 profile 计数），8 MiB 内核缓冲不起作用 | 未发布（CM-1） | `a_full_event_queue_keeps_business_frames_in_the_socket_until_the_consumer_makes_room`、`control_behind_a_stalled_consumer_is_answered_within_the_stall_bound`；[L3 前后对照](/v2/node/performance#cm1-receive-backpressure) |
| <a id="u2b-read-failure"></a>U2b | `meshway-vm` 宿主停顿后永久 `retrying · underlay`（每 5–10 秒 `kernel ownership or routing protection conflict; resources kept`，数小时）、`lastErrorDetail` 为 null；同一时段 lab 的 Status `Apply`、`Node sync failed; retrying {error: Contract}` | DNS 撤回中的 suspend 开头读取失败，TUN 照常关闭、内核随之删除其路由，base journal 仍记录 TUN 与路由且本期不结束，此后每次安装把"journal 有、内核无"的路由读成冲突；DNS 锚点 dump 失败的 `anchor_netlink` 判定本已为未知，只是 `error` 标为 `Conflict`。控制面：维护周期与 sync loop 并发认证，被对方替换上下文的一方得到 `Pending`，维护记 `Apply`、sync loop 记 `Contract` | 未发布（U2b） | `a_suspend_whose_read_fails_after_closing_the_tun_is_reproved_and_installs_again`（`exit/off` 组件 `pending-tun`）、`a_superseded_authentication_of_the_sync_loop_is_no_failure`、`a_local_failure_names_its_cause_and_a_server_answer_does_not` |
| <a id="u3-recovery-gaps"></a>U3 | AUD-U 审计（源码级，无现场复现）：一次 journal 写入失败后 base／出口 journal 的 Store 永久拒绝（本期不结束，此后每次安装、suspend、`down` 都以 `private underlay journal is invalid or requires recovery` 失败；出口审计把它读成已证实的改动、清理重试永远失败）；DNS 锚点创建在设备已建后读取失败即需新 namespace 或手工删除；egress 退役删表成功而回读失败后"有收据、无表"永久冲突；安装时读回的外来不一致、池覆盖锚点地址等以 `retrying` 无限重试 | 私有存储的 poisoning 写入者把未知的写入结果报成 `Journal`（已证实）且没有重开；锚点指纹只在内存，持久意图只有名称；退役没有持久的 retiring 意图；`Kernel::interrupted` 只在有待定步骤时结束本期，Node 层把所有 `Conflict` 当未知重试 | 未发布（U3） | `a_failed_journal_write_ends_the_term_and_is_read_back_by_the_reproof`、`a_failed_exit_journal_write_is_read_back_and_the_cleanup_completes`、`a_half_created_dns_anchor_is_reclaimed_by_its_fingerprint_and_created_again`、`a_retirement_whose_read_back_failed_completes_and_a_missing_table_otherwise_conflicts`、`only_a_proven_install_failure_ends_the_term_as_a_conflict`（`exit/off` 组件 `journal-unwritten`、`exit-journal`、`anchor-reclaim`、`egress-retire`、`install-conflict`）、`an_external_address_inside_a_mesh_pool_fails_closed_until_it_leaves`（池重叠不结束归属） |
| <a id="cm1b-carrier"></a>CM-1b C1 | 现场 planck↔jp（边顺序 `native_udp, native_tcp`、keepalive）在 jp 约 100 秒的包平面中断后停在 `native_tcp`，此后不再回到 Native UDP | 选定通道存在时拨号 owner 停止尝试所有其他候选；监听方拒绝替换健康流式通道的 UDP INIT；常驻边的流式通道不空闲关闭，没有 owner 让边回到更前的承载 | 未发布（CM-1b） | `manager::tests::netmap::upgrade::*`；`udp/off` 承载回归阶段 |
| <a id="u4-reinstall"></a>U4 | alpha.23 发布矩阵 `fault/throttle`：B 限额后 DNS `observe_deadline` 撤回，DNS owner 10 秒后已重证，包平面却至行结束未重装，`retrying · dns`、`retryAt` 为 null | 通道的共享发送端是 `tokio::sync::Mutex`：排队的锁等待者在持有者释放时即获得锁，不论之后是否再被 poll。包处理 owner 跨轮保留发送 future、重配置期间不 poll；获得锁的那个 future 一直持有它，而撤回所 join 的通道 owner 关闭时要锁同一发送端，`Manager::shutdown` 永不结束。`retryAt` 只投影 Control 与 DNS owner 的重试，DNS 重证后 worker 自己的重装时点不可见 | 未发布（U4） | `an_output_kept_unpolled_does_not_hold_the_sender_from_shutdown`、`an_owner_that_does_not_stop_is_aborted_after_its_grace`、`retry_at_is_the_planes_reinstall_once_the_dns_owner_re_proved_itself`；`fault/throttle` |
| <a id="x19-inbound"></a>X19 | PROD-1 jp（OCI 1:1 NAT、经公网 SSH 管理）选中 fallback 为出口的方案时入站 SSH 立即断开，直到从别处经 API 清除选择；包平面挂起时同样断 | 源 capture 的 RPDB 选择器与 OUTPUT 负向门只按目的判定：本机对已接受连接的回包（目的为对端公网地址）同样被导入出口 TUN，挂起时被负向门以 TCP reset 拒绝 | 未发布（X19） | `the_inbound_batch_adds_exactly_the_reply_chain_of_a_current_table`、`accepted_connections_keep_underlay_replies_through_capture_pause_and_upgrade`（`exit/off` 组件 `inbound-replies`）；`exit/off` 第 17 阶段 |

## 剩余边界 {#remaining-boundaries}

已登记、未修复或未定位的问题的唯一列表。每条出现新的失败签名时更新频率与证据；修复后移到[已修复问题索引](#fixed)。

| 跟踪 | 症状 | 频率 | 证据 |
| --- | --- | --- | --- |
| I6-2 | IPv6 出口前缀变化不退役旧流；出口不对外部超限回包合成 packet too big | 未实现（v0.1.0 后，已知限制） | [IPv6 出口设计](/v2/node/ipv6-exit-design) |
| 未定位 | `exit/off` 第 12 阶段 `A live UDP flow through the exit did not start`：20 个报文 0 回显，E 两次 `Exit forwarding refused by the kernel route decision`；单行复跑通过 | 一次 | `.meshway/validation/f7/`（`20261022T065619Z-34e5d00e`） |
| X17b | `exit/off` 第 16 阶段压力步 iperf3 50 秒未结束：[X17](#x17-starvation) 的停顿之后 E 结束了一条出口流（`Exit flow failed by its own projection`，当时的日志无原因与元组），iperf3 的连接随之中断；停顿已修复，此后的流结束带 `cause` 与 `tuple` | 两次（N6、P1，均在 X17 修复前）；X17 工作包 27 轮（修复前 15、后 12）未再出现 | `.meshway/validation/n6/`（行 `20261018T045847Z-d65d2124`）、`.meshway/validation/p1/`（`20261015T065357Z-b98749cc`）、`.meshway/validation/x17/` |
| OPS14 | 真机漫游 it7→VM 中断 0.61–0.83 秒（5/s ping 丢 2–3 个），高于 0.5 秒预期；sha→VM 与 VM→it7 达标 | 每次地址变化 | `.meshway/node-lab/ops14/r3-vm/` |
| P2 | it7 空闲 CPU 高于其他五台。已定性（[归因](/v2/node/performance#idle-cpu)）：真实转发 ≤ 0.02 %、中继边空闲无开销；主要是固定周期工作——DNS owner 每 250 ms 的 observe 4.7 %（两轮锚点证明＋约 23 次 D-Bus）、出口／egress 每秒审计 1.9 %（另 `nft` 子进程约 4.2 %），唤醒频率与 Planck 相同，没有忙轮询。其中一项是浪费：锚点证明（`kernel::anchor::inspect` → `rtnl::links`，Underlay）解码宿主每个 link 的完整 IFLA_LINKINFO／AF_SPEC，`netlink-packet-route` 对 bridge／TUN 逐属性预先格式化整段 payload，it7（Docker 网桥）上占 1.4 %，违反“周期证明的成本只取决于它比较的字段”；已由 RC-1 G16 修复（只完整解码 index／名称／别名命中的 link，证明语义不变），it7 现场复测待下一次部署；observe 的 250 ms 节拍与每轮两次证明、出口／egress 每秒审计在 v0.1.0 保持，审计改为进程内 netlink 读取延后至 v0.1.0 之后 | 每次空闲采样 | `.meshway/node-lab/ops20/cpu-idle/`、`cpu-idle-2/`、`.meshway/validation/p2/` |
| R1 | 一次 path 变化约 2 秒后 Manager apply 返回 `transport operation cancelled or timed out`（某 owner 未经请求结束），具体 owner 未定位；现由 WARN 的 `owner` 字段给出 | 两次（矩阵、lab），此后未见 | `.meshway/validation/r1/`（`20261015T172722Z-c3cc3381`） |
| 历史 | 未定位且此后未复现：`nat/resolved` 撤权阶段 DNS `Backend` 后 45 秒无首个回显（U1，`20261013T205702Z-9626e731`）；`exit/off` 第 10 阶段首个名称连接未经 E（H2，`20261014T202904Z-1f0503dc`）；高负载下的第 10 阶段同名第二次连接多一次往返与 LAN UDP 单包超时（F4a） | — | 对应工作包目录 |

设计上的边界（不是缺陷，行为定义在引用页）：

- **长于 30 秒的内核停顿 fail closed**：变更与回读失败共用 30 秒上限；停顿期间回读全部失败满 30 秒时出口审计停止角色、egress capture 以暂态阻断结束宿主内核归属，Host 每 1–8 秒重证，停顿结束后自动重装；控制中继监听失败与容量耗尽不结束归属；已证实的冲突每 60 秒只读重证，证据消失即恢复。见[归属的结束与重证](/v2/node/runtime#ownership-end)、[内核变更失败](/v2/node/runtime#kernel-change)。
- **Planck 宿主 Wi-Fi 周期停顿**是宿主问题，产品侧只保证自动恢复（[真机证据](#d3-dns-health)）。

## macOS 实验验收 {#macos}

[Apple Host](/v2/node/apple-design) 的 VM 验收行，不属于 `validate_matrix.py` 的 Linux 入口，也不在 v0.1.0 范围。VM 为 macOS 26.6.2 arm64，Linux peer 为 `9e4c040b`；外层 IPv4、Mesh-only capture；固定 VPN 配置经用户批准，更新保存不再请求确认、UUID 不变。原生 Rust 单测、Swift 编译／签名或 URLSession 证据不代替这些行。环境见 [macOS lab](/v2/node/macos-lab)，私有证据 `.meshway/macos-lab/{a3,b1,c1}/`。

| 行 | 必需场景 | 状态 |
| --- | --- | --- |
| `macos/upgrade` | 同 Team／bundle ID、递增 CFBundleVersion，经 `.replace` 激活 | 通过（0.1/2→0.1/10，无 GUI 再批准） |
| `macos/ipc` | App／probe → 签名认证 XPC → Rust IPC 13：Status、Up、Down、断连取消、有序 stop／join | 通过（0.1/7：36 条请求 ×1000 次 ping 后 SIGTERM 各客户端，预算未耗尽、provider 保持响应） |
| `macos/enrollment` | enrollment key 接入、验签与保存 netmap、applied／running／peer | 通过 |
| `macos/mesh` | utun ↔ Linux peer 双向 IPv4／IPv6 ping、TCP、UDP；provider underlay 绕行 | 通过（Native TCP 与 Native UDP 各 8 组合 SHA-256） |
| `macos/dns` | 系统 resolver 解析 Mesh 名、UDP／TCP DNS 与撤回 | 通过 |
| `macos/offline` | Server 停机，签名租约内通信 | 短租约（约 60 秒）通过；不代表 10 分钟离线长流 |
| `macos/lease` | 短租约到期后丢包且不落物理网络；Server 恢复后自动恢复 | 通过（Native UDP；en0 过滤抓包 0 包） |
| `macos/restart` | stop／startTunnel 后在 Server 离线时从 `netmap.json` 恢复并保持租约期限 | 通过（0.1/5） |
| `macos/exit` | 出口捕获、TCP／UDP NAT、DNS／关联、方案清除、撤回／到期不 direct、管理连接绕行 | IPv4 通过（0.1/8；40 次独立 SSH 全部成功）；IPv6 运行时不可用时回 ICMPv6 admin-prohibited，但 `nc` 仍等到 2 秒 connect 截止；实际 IPv6 出口转发未验收 |
| `macos/roam` | 物理接口中断与自动恢复，保持捕获和现有流 | 同接口／原地址恢复通过（同一 TCP 20/20、UDP 18/20）；未验证换 NAT、跨接口、重新穿透、中继空窗与真实 sleep／wake |
| `macos/merged-smoke` | A3＋B1＋B2 合并后的类型化 probe、默认本地网络排除与真实包流 | 通过（0.1/10，`c0aacd66`）；不替代 GUI 人工验收 |

不覆盖：外层 IPv6、IPv6 实际出口转发、完整 Linux 漫游矩阵、性能、正式 App UI／macOS CLI、Developer ID 分发。
