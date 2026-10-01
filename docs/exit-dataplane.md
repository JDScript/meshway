# P1c · Linux IPv4 出口数据面

状态：独立 Linux 原型；controller / HTTP 协议尚未接入。本文的产品偏好是待确认提案，fixture 的显式选项不代表已经确认的产品默认值。出口授权独立于访问出口设备的 Mesh grant，也不从显式双向授权推导状态型回包。

## 安全边界与内部编译接口

`apps/agent/src/managed_dataplane/exit.rs` 提供内部 `ExitPlan`、`ExitRoute`、`ExitGrant`。类型不是 HTTP DTO，不修改 protocol：

- 每源节点表：规范 IPv4 prefix → 当前选中的完整 `source → mandatory via → exit` 路径，以及 `direct_prefixes`。**产品出口规则现为有序 first-match**（后续确认覆盖初始最长匹配定义）：controller/v3 编译器先做前序集合减法，消解 IP / rule-set 重叠，生成互不冲突的有效前缀，Agent 只对该等价输出做内核最长前缀匹配，不能把原始有序规则直接排序后安装。域名/IP跨条件重叠需要 P3c 可信解析上下文联合编译；被完全遮蔽规则由 controller 提示。Agent 不下载规则集。每节点输出上限 16384 IPv4前缀，超限明确失败，不能部分应用。
- **最新确认以路由方案取代默认出口候选模型**：`ProfileCatalog { profiles, default_profile, locked, automatic }`，每个 `RoutingProfile { id, routes, direct_prefixes, fallback }` 是controller已编译的“公共规则 + 方案有序规则 + 兜底”的有效表。`ProfileFallback`区分出口、direct、不接管；后两者在无具体匹配时均继续本地网络。公共规则排最前、在每个方案的等价表中保留。`select_profile`只接受允许方案ID，不携带规则、listener、NAT等配置。选择无需新policy revision，但创建新的packet/flow运行代际；锁定和可选范围由控制面决定。主备只在方案已授权的完整路径内切换，不按包随机挑出口。
- 每出口授权表：源节点身份、唯一 Mesh IPv4、完整正向路径、允许的外部前缀。回程路径明确编译为逆序路径，仅能传递状态型回包，不是一般反向 grant。中继同时持有两条精确路径，不能自行缩短。
- 表、端点公钥、承载配置、network、revision、lease 来自同一快照。P3a/P3c 必须在新 snapshot 的准备/原子激活流程接入；缓存旧出口表不能延长 lease。原型不在正常 Agent CLI 中启用，通过 ignored fixture 注入内部类型。
- `ExitPlan::validate` 在绑定 Runtime 前检查身份、地址、完整路径、MTU、重复前缀与本地投影。外部内层地址不得指向 Mesh pool、loopback、link-local、multicast、保留广播/零地址；源必须与认证源节点绑定。**统一出口职责**覆盖离开 Mesh 到目标节点本地 LAN 或互联网的流量，不另做 subnet router。管理员在 Web 开启“可作为出口”后，编译计划才包含出口职责；设备不通告、不配置。LAN 前缀访问授权独立于泛“互联网”授权，controller 必须联合投影为 `ExitGrant.prefixes`，不能从默认 /0 推导对未授权 LAN 的访问。

### 方案选择、取消与存量流

选择“全局经E”→“方案F”时停止旧packet pump，丢弃客户端旧flow state与TUN/delivery队列，双表准备并切换**整个方案的有效表**，包含具体出口、direct、兜底，完成后激活F的完整认证路径。**同revision选择保留Runtime、已认证承载与E2E缓存**：重启客户端Runtime会丢失反向接收key，而未重启的出口可能继续用原发送缓存，造成等待轮换才恢复。共享pump用RAII交还唯一receiver，在新准入激活前有界drain旧队列，不并行消费。策略revision不变，packet generation变化，lease剩余预算不重置；策略撤回/到期仍由Runtime所有者独立watchdog关闭承载、清空密码缓存，不能因receiver交还复活关闭的Runtime。

旧E的NAT映射不能迁移到F；旧TCP/UDP被截断，应用需建新连接。原型在客户端截断旧流，旧出口conntrack可保留至内核timeout，但回包因不匹配当前出口和flow代际被拒；P3c可向旧出口下发源级清理，不能用本地选择提升出口授权或延长lease。纯本地direct流不由packet pump转发，不迁移NAT。

`locked=true`拒绝与当前锁定方案不同的请求（包括切换“仅Mesh”）；同值选择不扩大权限。未授权ID拒绝且现有代际保持运行。“不使用出口”表达为选择内置“仅Mesh”方案：它仍执行公共规则，其余恢复main本地网络；不能删除公共规则。现阶段为Agent内部 `select_profile(id)` 与ignored fixture请求，不新增设备配置入口；P3c同UID私有IPC/CLI只承载“切换到哪个允许方案”。内置“全局经X”由controller为每个允许出口生成，不是Agent自建授权；所有方案分配时必须校验grants，未授权方案不能进入catalog。

### 自动方案与换网

`AutomaticProfile`携带控制面下发的地区→允许方案映射、当前已解析生效方案、无法定位的兜底方案。Agent只上报自身真实carrier STUN观测（公网IP、时间、有效期、socket/网络代际及observation关联），**不使用controller HTTP连接来源IP、不持有GeoIP数据**。controller校验观测新鲜度与所属节点/network，完成地区判断，在当前已授权catalog内解析目标，回传与观测关联的resolved profile；Agent调用可信 `resolve_automatic` 更新运行代际。自动选择本身仍是用户选中的“auto”，锁定auto不阻止controller更新其生效方案。

换网时观测先失效，重新STUN→上传→controller地区解析→下发→表切换。时延由STUN超时/重试、sync/推送周期与应用时间组成，不承诺零时延或固定地区切换速度；P3c应记录观测到生效时间与原因。旧观测不得维持旧地区推断：无有效观测、解析失败或过期时走已下发兜底方案；它也必须授权，可能是仅Mesh、指定出口或direct。controller断连同时导致lease到期时优先停止整个旧数据面并保持fail-closed，不能借auto兜底绕过lease。原型模拟controller收到位置变化后给出JP→方案F、未知→仅Mesh，未实现GeoIP下载/真实地区HTTP回路。安全上不得接受本地IPC提交地区、prefix或resolved table。

### direct 与大规则集

`direct_prefixes` 在独立出口表安装 `throw`：匹配后退出该表，继续 main，不进入 TUN；默认出口不能覆盖它。内核只在**已按有序规则消解的有效结果**上执行最长匹配。Mesh pool有独立 throw，继续已有 Mesh sink，不被默认出口接管。保护的承载 /32 仍优先于业务表。

批量 `ip -4 -batch -` 安装routes / unreachable / throw，每批一个有界子进程，不为8k条CIDR启动8k个进程；独立表先准备，再发布policy lookup，异常保留拒绝。明确controller取消所有接管时先删捕获rule，再按自己protocol flush该表；用户选择仅Mesh则替换为公共规则有效表。`ClientRoutes::replace`实现双缓冲表：**先停止旧I/O（代际fence）**，备用表准备完整捕获与direct，发布更高优先级lookup，退休旧lookup与旧表，再将新lookup恢复正常优先级，drain后激活新pump；默认捕获存在性与IPv6拒绝同步更新。packet generation层面原子，没有并行新旧准入；Netlink没有多route/rule事务，切换期间可能短暂丢包，不能声称所有Linux syscall是一个内核事务。任一步失败都不激活新旧pump，捕获保留；尚未清理的表须由可信所有权恢复处理。首次准备不接管前的流量不代表已启用VPN；applied仅在完成安装与lease检查后发布。

后续可由controller聚合相同目标的CIDR（不能越过first-match边界），平台侧使用 radix lookup；nft interval sets可用于需要更多规模/条件的selector，不以逐条抖动为默认方案。验收记录1000个有效direct前缀的安装、代际原子替换、撤回耗时与全量正确性；原型用户态lookup尚为有界线性扫描，不声称已完成8k级吞吐优化。

## 客户端路由、承载与生命周期

Linux 使用自己拥有的独立 policy routing 表及规则，而非替换系统默认路由。每个出口前缀安装 TUN 路由以及优先级较低的 `unreachable` 同前缀兜底；`0.0.0.0/0` 也如此。policy lookup 在 main 之前，故 main 中更具体的 LAN 路由不会隐式绕过默认出口；未匹配前缀的表查询失败后继续 main，仍走本地网络。Mesh pool 原有持久 route sink 保持不变。

捕获rule限定 `iif lo`（Linux表示本机生成的流量）；同一个节点可以执行自己的路由方案并为其他节点提供出口，解密后进入TUN的转发包不能被该节点自己的默认方案再卷入TUN。职责来自计划，不按设备类型划分出口/客户端类别。

必须在捕获路由生效**之前**固定 peer endpoint、STUN、controller 与 bootstrap DNS 的物理路由。原型接收显式的 IPv4 underlay 地址集合，逐一从 `ip -j route get` 捕获 gateway / dev / source，在独立 bypass 表固定 /32 并安装更高优先级 lookup。无路由、地址落在 Mesh pool、表/规则已占用、IPv6 underlay 或无法确定所有权均拒绝激活。动态 endpoint 必须先固定新路由再发布新候选；DNS 重解析不能在出口捕获之后使用未经保护的候选。

原型的 /32 例外对该 IP 的所有业务端口生效，这是明确限制：生产应为 controller/STUN/承载 sockets 设置统一 SO_MARK，通过 fwmark 例外恢复原路由，限制为受保护进程/socket；不能声称当前 /32 例外是完全无泄漏的全流量 VPN。fixture 验证默认出口下承载仍可达且不会进入 TUN。

本地网络访问默认提案参照 Tailscale：默认出口不允许直接 LAN；显式 allow-LAN 时只对发现并批准的 on-link 前缀添加更高优先级 main lookup，绝不自动豁免全部 RFC1918。本原型只支持显式 bypass 地址，不提供生产 allow-LAN 开关。loopback 仍由 Linux local table 处理。

| 事件 | 原型行为及理由 |
| --- | --- |
| 仅撤销互联网授权、路径失败、配置无效 | 停止 I/O，撤销 NAT 的准许集合，清理该代际 conntrack；客户端捕获与 unreachable 保留。撤销权限不等于允许通过本地网络绕过管理意图。 |
| lease 到期、controller 长请求、异常停机/SIGKILL | 客户端保留 policy rule 与 unreachable；TUN 消失也不回落 main。出口残留防火墙阻止回程落到物理默认路由，非持久 TUN 的消失让残留 NAT 无法继续 Mesh 转发。 |
| 已认证的策略更新明确删除出口路由/用户明确关闭出口 | 协调者停止旧 I/O、出口清理状态后，显式执行 `restore_local` 删除自己拥有的捕获规则/表；被删除前缀恢复本地。此动作区别于异常撤回；原型提供显式 API，未来 controller 必须表达是否确实取消接管。 |

失败关闭优先避免强制出口的意外绕过；会牺牲异常期间本地出网可用性。是否提供用户可选 fail-open，须另行确认。初次安装中途失败也保留已安装的拒绝兜底，不删除为成功安装的 fail-closed 资源；失败报告不得变成 applied。原型每个 namespace 只允许一套出口，拒绝采用旧表；恢复进程需要可信持久所有权记录和旧代际清理（P3c），不能仅凭固定名字认领。

## 出口节点：NAT、状态回包与撤回

出口接收经过 Noise E2E 验证的 `ReceivedPacket`，再校验 exact path / source identity / source IPv4 / authorized destination prefix。未授权源、伪造源、Mesh 目的和特殊地址在写入 TUN 前拒绝。出口只允许 TCP、UDP、ICMP echo（完整、无 option/fragment IPv4）；其他协议/ICMP errors 当前丢弃，不宣传完整 PMTU 支持。

内层包写入 TUN 后 Linux 负责路由与 conntrack。独立 nftables table 的 prerouting 将 TUN 发来的已授权包打上代际 conntrack mark；forward 只允许 TUN→指定外网接口且源/prefix 在授权表中，禁止从 TUN 访问出口本机（input drop）。postrouting 对该 mark 的已授权源做 masquerade，外网只能看到出口外网地址。返回方向须同时满足外网接口、TUN 接口、当前 mark、`ct state established`；不允许外网主动发起 Mesh 连接。除此之外，任何涉及 TUN 或 Mesh 目的的转发都 drop，避免非 Mesh 源把出口当作普通路由器。

IPv4 forwarding、NAT与路由启用条件仅是收到可信出口职责计划。Agent 先安装拒绝边界，再尝试将当前 network namespace 的 `ip_forward` 自动设为1并读回；原值为0时正常撤回恢复0。缺权限、read-only sysctl、nft/conntrack工具缺失、操作超时或资源冲突返回明确错误，不能发出 active证明。Docker的 /proc/sys 通常只读，fixture使用namespace `--sysctl net.ipv4.ip_forward=1` 提供执行环境；生产函数仍含自动启用路径。P3c须把内部失败接入控制面applied/failed报告，不把saved ACK当可用出口。不触碰其他 nft 表或清空全局 conntrack。代际 mark 来自专用显式范围，原型只允许该 namespace 中未占用的一套 table，保留拒绝边界；生产需持久化 namespace/network所有权及mark分配。

LAN与互联网统一默认 masquerade（与 Tailscale subnet router默认SNAT一致）：目的主机无需安装返回Mesh pool的路由，避免泄露用户Mesh地址并统一状态回包边界。非SNAT子网路由不是本阶段选项，未来只有显式需求及回程路由/授权设计才增加。出口的指定外网/局域网接口仅允许已授权TUN转发，其他物理源不能把E当开放路由器。

撤回顺序：先原子替换 nft table 为纯拒绝边界，再 `conntrack -D --mark <generation>` 清理；因此删除 conntrack 失败也不能继续旧流。仅删除授权源的五元组但保留 `established accept` 会允许抢跑重建，禁止这种顺序。既有 TCP 流被**数据面截断**，不是保证向两端发送 RST；应用可按自己的 TCP 超时退出。新流同样失败。退出成功状态必须包括清理结果，拒绝 nft 更新失败不能报告撤回完成。

conntrack负责NAT、TCP状态、UDP timeout、ICMP echo association；不是用户身份授权，不保证应用层会话合法。NEW TCP必须是初始SYN（不能用内核tcp_loose的ACK重建绕过），UDP与ICMP echo request可创建状态；返回只允许ESTABLISHED，不启用helper/RELATED，减少FTP等扩权。拒绝fragments/options。mark是当前namespace内部边界，非跨主机可信标识；启动时拒绝已被占用的mark，不删除未认领状态。timeout继承内核设置；本原型不修改其sysctl，容量边界需后续设计。

SIGKILL 无法执行用户态 conntrack 删除；非持久 TUN 立即消失，保留的 forward Mesh drop 阻止存量回程与新流穿过。该等价截断必须实测，不把 conntrack entries 消失当唯一判据。重新激活必须清除旧 conntrack/旧标记，不沿用存量 NAT 映射。精确清理所有权后才允许另建 TUN。

## 客户端回包准入：需要流状态

只检查“分配出口 + 外部源 IP 匹配 prefix”不够：可信出口仍可能发起 unsolicited UDP/TCP，且同一目的有更具体规则分配别的出口。因此客户端必须要求：

1. 回程认证源为当前所选出口，完整逆序 path 一致；目的为本节点绑定 Mesh IPv4。
2. 按回包外部源 IP 重做最长匹配，选中的正向路径必须与该出口一致，不能用宽泛默认规则覆盖更具体规则。
3. 有本代际客户端先发的五元组/ICMP echo identifier 状态，且回包不是 TCP 初始 SYN / ICMP 请求。状态有界（4096 entries）、闲置 120 秒过期；只在 TCP 初始 SYN、UDP、ICMP echo request 建立。TCP seq/window 由本机协议栈校验；此表不是完整 TCP 状态机。容量满拒绝新流，不无界增长；状态不跨 revision/lease/出口切换保留。

出口内核 conntrack 是第一层；客户端流表是第二层。明确回程授权不允许 E 发起一般 E→C 业务。普通 Mesh 包仍由已有 `Ipv4PacketPlan` 处理，出口表不扩展其授权。

## DNS 与 P3c 域名上下文

本原型不修改 resolver，不声称 DNS 无泄漏。默认 IPv4 出口只能捕获走普通 IPv4 路由的 DNS；loopback stub、Docker `127.0.0.11`、系统 resolver 的独立接口/路由、保护的 bootstrap IP、IPv6 DNS、应用自选 DoH 都需独立处理。DNS 查询成功不授予互联网访问权限。尤其 Docker embedded DNS 由 daemon 转发，不能用容器默认路由证明 DNS 走出口。

P3c 参照 Tailscale exit DNS：本地可信 stub 把外部查询通过当前选中出口的认证服务转发；退出/lease 到期阻断这些查询，不回退物理 resolver；split upstream 是否 use-with-exit 显式表达。controller/peer 域名 bootstrap 走接管前的独立上游与受保护 socket，并有严格边界；拒绝自身递归。是否启用 OS override 与平台安装方式单独确认。

域名 suffix 按 DNS 标签边界匹配。参考 app connector 的 DNS discovery → IP route，不猜测每个 IP 包的域名。未来内部 `DomainExitBinding` 需携带 network/revision、规范 qname、CNAME chain、A/AAAA、TTL 到期、受信任解析来源、匹配 rule 与候选完整路径；由同一代际协调者投影为有 TTL 的 /32（IPv6 后续 /128）`ExitRoute`。共享 IP 的不同域名不可由普通 L3 包区分，须明确共享 IP 被一起路由或使用后续 Fake-IP/应用代理，不能承诺逐域名隔离。IP literal、非受管 DoH、已有 DNS 缓存与 TTL 更新/冲突是 P3c 必验项。本阶段不实现解析上下文。

### 三种设备侧域名上下文方案（供确认）

| 方案 | 上下文产生与优势 | 边界/成本 |
| --- | --- | --- |
| 受管 DNS映射（参考 Tailscale app connector） | OS resolver接入可信本地stub；解析的qname/CNAME/A/AAAA/TTL连同revision形成 `DomainExitBinding`，规则来源及授权仍由controller管理；真实IP保留，改动最小，适用于L3与任意传输。可以让选定出口解析以匹配地域结果。 | 共享IP将多个域名一起路由；应用自选DoH、缓存、IP literal不可推断域名。与其他IP规则重叠时必须按first-match联合编译，TTL变更撤回旧映射，不扩大授权。 |
| Fake-IP（参考 sing-box / mihomo FakeIP） | 本地stub从私有保留池返回合成IP，把每个连接对应qname与规则固定；TUN解包时恢复真实目的，回包反向还原，避开共享IP歧义，保持域名优先级。 | 必须增加DNS持久映射、碰撞/TTL/重启恢复、地址池隔离、应用兼容、DNSSEC与IP literal边界；合成目的不允许直接被出口当公网IP转发。需要本机NAT/用户态转译与flow管理，不能只有DNS造地址。 |
| 流量嗅探（参考 sniff） | 在首个业务数据解析HTTP Host、TLS ClientHello SNI、可解析QUIC信息，补充未经过本地DNS的域名；可辅助诊断。 | 初始SYN已需要选路；要等待域名必须先拦截/缓冲连接，普通L3转发不能事后无损改出口；ECH、加密QUIC、非HTTP/TLS与IP literal均不可见。嗅探是提示，不是认证授权，不能因看不到域名绕过规则或选择direct。 |

**推荐 P3c 先做受管DNS映射 + 出口DNS**，沿用app connector的真实IP模型，显式展示共享IP限制和外部DoH边界；用户要求严格区分共享IP域名时再选择Fake-IP完整转译，不同时引入不受控多套映射。sniff只作可选补充，不作为首版唯一策略依据。推荐尚待用户确认；本阶段三者均不实现，Agent不下载rule-set，设备只产生受约束的解析上下文并使用controller已下发的规则/候选。

## IPv6

本阶段仅 IPv4；TUN 禁 IPv6并拒绝 IPv6包。默认出口必须额外安装持久 `unreachable ::/0` 的 policy table/rule，保留必要 IPv6 underlay 需要 future socket mark；原型拒绝 IPv6 underlay，不静默让 IPv6 Internet 走本地。非默认 IPv4 分流不声称接管无对应 IPv6规则的域名；P3c 域名策略有 AAAA 时在 IPv6实现前须明确阻断/抑制，不能由 happy-eyeballs 绕过。

后续真实 IPv6地址/前缀、完整包与扩展头准入、ICMPv6/PMTU、IPv6 forwarding 与无 NAT 或 NAT66选择、双栈 conntrack、撤回和跨平台路由一起验收；AAAA 元数据不代表出口双栈已完成。

## 需用户确认的产品问题

1. 异常期始终 fail-closed 的产品偏好；原型已按此实现，明确取消选择与授权撤回的恢复边界见上文。
2. 默认出口 LAN access 默认关闭、通过direct排除开放本地LAN的体验；“互联网”授权是否排除出口LAN/保留地址的具体地址分类。
3. 出口 DNS / OS override、共享域名 IP 的路由语义与 IPv6不可用时的域名行为。

已确认而不再列为问题：公共规则优先，用户从允许范围切换路由方案（管理员可锁定），仅Mesh/全局经X/自动为内置方案；Web管理员开启出口能力，设备不通告；统一出口覆盖LAN与互联网；direct排除；规则集由controller管理下载编译；出口规则有序first-match，设备只执行有效展平表。

## 依据与验收

2026-10-01 实际阅读官方 [Exit nodes](https://tailscale.com/kb/1103/exit-nodes)（默认路由、opt-in、LAN默认关闭、forwarding、互联网授权）、[App connectors](https://tailscale.com/kb/1281/app-connectors)（DoH发现、共享IP限制）、[DNS](https://tailscale.com/kb/1054/dns)（exit resolver与use-with-exit）。有序中转与管理员集中路由是 Meshway 扩展，不把 Tailscale via tag 当作 ordered path。

域名上下文对照实际阅读：[sing-box FakeIP DNS server](https://sing-box.sagernet.org/configuration/dns/server/fakeip/)（legacy FakeIP配置已删除，不能复制旧格式）、[Protocol sniff](https://sing-box.sagernet.org/configuration/route/sniff/)、[mihomo DNS](https://wiki.metacubex.one/config/dns/)（fake-ip/redir-host、上游bootstrap、fake-ip-filter）、[mihomo sniff](https://wiki.metacubex.one/config/sniff/)（HTTP/TLS/QUIC、force-dns-mapping与override-destination边界）。

隔离验收入口见 `experiments/exit-dataplane/README.md`。结果 JSON 包含 `status`、每项真实命令证据与 `cleanup_errors`；吞吐/RTT仅为同条件可复现基线，不衡量包装增量。
