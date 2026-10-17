# IPv6 出口设计

> **I6-1 已实现**（行为定义已写回 [Exit 设计 · IPv6 出口](/v2/node/exit-design#ipv6)，验收见 [exit6/off](/v2/node/validation#exit6-off)）；I6-2（前缀变化退役、出口合成 packet too big）未实现。下文保留设计依据与拍板记录，“现状”一节描述的是 I6-1 之前。
>
> **原设计稿说明：**现行出口只支持 IPv4（[Exit 设计](/v2/node/exit-design)）：受管 IPv6 由源端捕获 sink、出口判定返回 `unsupportedFamily`、出口 TUN 的 IPv6 一律 drop。本页给出 IPv6 出口的方案、推荐与需用户拍板的问题；定稿后行为定义写回 [Exit 设计](/v2/node/exit-design)、[数据面](/v2/node/dataplane)、[Routing](/v2/configuration-reference/routing) 与 [DNS](/v2/configuration-reference/dns)，验收状态只在[验收范围](/v2/node/validation)维护。

## 现状 {#baseline}

下表是 `f58def1b`（alpha.18）中 IPv4 固定之处，也是 IPv6 出口要改的全部位置。

| 层 | 位置 | 现状 |
| --- | --- | --- |
| 配置／netmap | `meshway_configuration::netmap::ExitRole { generation, family, nat }` | 单一 `family`，Server 只发 `ipv4`／`masquerade`；角色需 inventory 声明 `feature:exit-ipv4` v1 |
| 能力事实 | `meshway_model::exit::ExitCapability { supportedFamilies, installedFamilies, status }` | 族列表已存在，但 `status` 只有一个：无法表达“IPv4 ready、IPv6 unavailable(原因)”。Server（`state/exit.rs`）要求 ready 必含 `ipv4`，installed 只能是角色那一个族 |
| 出口选择 | `KnownPeers::ready_exits`（`meshway-control`）→ `First::choose_exit` | 只认 `installedFamilies` 含 `ipv4` 的 peer |
| 判定 | `LocalCompiler::decide_with`；`compile/plan.rs`；dataplane `outbound.rs::new_route` | 目的或源为 IPv6 的出口判定一律 `UnsupportedFamily`；源无 Mesh IPv4 也是 |
| 源端捕获 | `compile/capture.rs`；underlay `kernel/egress/` | 分割已覆盖两族（`ipv6` 区间存在），出口动作的 IPv6 区间装为 `sinkIpv6`（OUTPUT reject）；捕获 RPDB 选择器与私有捕获表只有 IPv4 |
| 公网分类 | `meshway_policy::internet_ranges` | **已双栈**：`2000::/3` 去掉 `2001::/23`、`2001:db8::/32`、`2002::/16`、`3fff::/20`；`autogroup:internet` 因此已经授权 IPv6 公网目的，只是执行侧 sink |
| 数据面流 | `flow/exit.rs::ExitState::new`；`exit::ExitTuple { source: Ipv4Addr, … }` | 非 IPv4 tuple 返回 `Error::Flow` |
| 出口 nft | `kernel/exit/program.rs` | `inet` 过滤表＋`ip` NAT 表；集合元素类型 `ipv4_addr`；四条链首条规则 `nfproto != ipv4 → drop`；ICMP 错误只认 `icmp type {3, 11}`；禁止目的为 IPv4 字面前缀；`masquerade` 只在 `ip` 表 |
| conntrack | `kernel/exit/conntrack/netlink.rs` | nfgenmsg 固定 `AF_INET`，tuple 只解 4 字节地址 |
| RPDB／回程 | `kernel/exit/routing.rs` | 成对规则与回程 `/32` 路由只用 `AddressFamily::Inet` |
| 前置探测 | `kernel/exit/probe.rs`、`prerequisites.rs` | 读 `net/ipv4/ip_forward`、`ipv4/conf/*/rp_filter`；ctnetlink dump 为 IPv4 |
| 宿主防火墙判定 | `kernel/exit/firewall/` | 只遍历 `ip`／`inet` 族；`meta nfproto ipv6` 判为不命中；不解析 `ip6` 族与 `ip6tables-legacy-save` |
| MSS clamp | `kernel/egress/mss.rs` | Mesh 对端已有 `peer6`（上限 −60）；“经出口”的 `other` 规则只匹配 `nfproto ipv4`；出口 TUN 入向只查 `peer4` |
| 合成错误 | dataplane `packet/icmp.rs` | 源端 Packet Too Big 已有；`inner < 1280` 的通道不承载 IPv6（`Ipv6Unavailable`，本机回 ICMPv6 no route） |
| DNS | `meshway-dns`；关联 `DnsAssociation { recordType, addresses }` | AAAA 原样返回；AAAA 关联已按 (名称, 记录类型) 独立成条，但其地址并入 `sinkIpv6`；`Desired::retaining` 只保留 IPv4 |
| 验收 | `exit/off`、`dev/exit-fixture/` | 第 4 阶段断言 A 的 IPv6 全范围 sink；fixture 的 external／far／lan 均为 IPv4 |

## 1. 地址与 NAT 模型 {#nat-model}

### 选项

| 方案 | 做法 | conntrack 精确清理 | 两阶段 ACK／quarantine | ICMPv6 RELATED | 出口宿主前缀变化 |
| --- | --- | --- | --- | --- | --- |
| (a) NAT66 masquerade | 与 IPv4 对称：出口 TUN 转发到出口接口时 `masquerade` 为出接口的全局地址（内核按 RFC 6724 选源） | 同一实现，tuple 换为 16 字节地址（`CTA_IP_V6_SRC/DST`，nfgenmsg `AF_INET6`）；`GET`→核对 id／mark→按 id `DELETE`→回读不存在 | 不变：flows／quarantine 集合按族各一份，reply tuple 是转换后的 `(外部, 出口地址)`，迟到回包同样被拒 | 内核 `nf_nat` 对 ICMPv6 错误做与 IPv4 相同的 RELATED 逆转换（外层与被引用头） | 出口地址失效即旧流失效，与 IPv4 `roam/exit` 相同（见[第 6 节](#roaming)） |
| (b) 无 NAT，路由前缀 | 给每个源分配出口前缀内的地址：出口宿主需要一个**路由到它**的前缀（/56 委派或静态路由的 /64），或在同一 /64 上逐地址做 NDP proxy；源的内层地址或由源 TUN 直接持有该全局地址，或由出口做 1:1 NPTv6 | 仍可跟踪（无转换时 reply = original 反转），清理不变 | 不变，但 quarantine 不再保护“地址被另一源复用”——需要地址分配本身的冷却期 | 最简单（无转换） | 前缀变化使所有源的分配地址同时作废，需要重新分配并通知源；源 TUN 持有全局地址时还要改源端地址与路由 |
| (c) 第一版只做 (a) | (a) 落地，(b) 留作后续产品选项 | 同 (a) | 同 (a) | 同 (a) | 同 (a) |

(b) 的额外代价：

- **宿主前提更强**：常见 VPS 只给一个 on-link /64（it7 当前连这个也没有，见[真机现状](#real-hosts)）；NDP proxy 需要改宿主 `proxy_ndp` sysctl 与邻居代理表，违反“检测而不修改宿主”的现行原则；路由前缀需要上游配合。
- **隐私与身份**：每个源在 Internet 上可区分，出口后的地址等于 Mesh 节点的长期标识；NAT66 下所有源共享出口地址，与 IPv4 出口的可观察性相同。
- **分配状态**：地址分配是新的、跨 Server／出口的状态，必须有唯一 owner（Server 签发进 netmap，还是出口本地分配）、冷却期与撤回；没有它就会出现“新源拿到旧源刚释放的地址、接收旧连接的迟到包”。
- **入站**：(b) 天然允许外部主动连源，必须另有入站准入（现行出口“未请求方向不能建立新流”），否则就是开放入站。

### 推荐：(c)

第一版只做 NAT66 masquerade：与 IPv4 共用同一投影、ACK、quarantine、精确清理与 RELATED 机制，只是地址族不同；没有新的分配状态，也不改变“出口只为已准入正向流服务”的边界。(b) 只在用户需要“源有独立公网 IPv6”或入站时另立设计。

NAT66 的已知代价如实记录，不规避：破坏端到端地址（IPv6 原本的卖点），但与 IPv4 出口语义一致；出口地址选择交给内核源地址选择，可能选到临时（privacy）地址，见[第 6 节](#roaming)。

### 待用户决定

- **Q1**：第一版采用 NAT66 masquerade，路由前缀／NPTv6（每源独立地址）不做。推荐：同意。

## 2. 出口资格与宿主前提 {#host}

### 宿主前提与探测项

IPv6 与 IPv4 共用 TUN、NET_ADMIN、固定路径 `nft`、ctnetlink、mark 选择与防火墙判定；以下为 IPv6 新增或不同的项。Meshway 仍只检测、不修改宿主 sysctl 与防火墙。

| 项 | 要求 | 探测与失败原因（新增原因加 `*`） |
| --- | --- | --- |
| 转发 | `net.ipv6.conf.all.forwarding = 1`（IPv6 转发只看 `all`；新建 TUN 继承 `default`，与 IPv4 一样在临时 TUN 上读取） | 为 0：`ipv6ForwardDisabled`*；读失败：`forwardingProbeFailed` |
| 路由器通告 | 打开 forwarding 后，`accept_ra = 1` 的接口**不再处理 RA**：靠 SLAAC／RA 得到的默认路由与地址会在寿命到期后消失，宿主之后静默失去 IPv6 | 出口接口的 IPv6 默认路由 `proto ra`（或地址来自 RA）而该接口 `accept_ra ≠ 2`：`ipv6RouterAdvertisementsIgnored`*。宿主管理员应设 `accept_ra=2` 或静态配置；Meshway 不代改 |
| 出口地址与路由 | 出口接口有全局单播 IPv6 地址（`is_internet_address` 为真、非 tentative／dadfailed），主表有通往 Internet 分类 IPv6 地址的路由（默认路由或直连公网前缀） | 缺：`ipv6NoEgress`* |
| NAT66 | `ip6` 族 `nat` 链与 `masquerade`（`nf_nat`／`nft_masq`，Linux ≥ 5.1 无需独立 `nf_nat_ipv6`） | 完整 IPv6 nft 程序的探测事务失败：`nftUnavailable` |
| conntrack | ctnetlink 的 `AF_INET6` mark 过滤 dump | `conntrackUnavailable` |
| 反向路径 | IPv6 **没有** `rp_filter` sysctl；回程仍靠 RPDB 成对规则（源 `/128`→出口 TUN）。netfilter 的 rpfilter（firewalld 默认的 `IPv6_rpfilter`：`fib saddr . mark . iif oif missing drop`；`ip6tables -t raw -m rpfilter`）是宿主规则，由防火墙判定处理 | 见下行 |
| 宿主防火墙 | 判定遍历扩展到 `ip6` 族与 `ip6tables-legacy-save`，考察包按族分别构造：IPv6 出口接口取主表通往 Internet 分类 IPv6 地址的路由（可与 IPv4 出口接口不同）；Docker（≥ 27 默认启用 `ip6tables`）的 `ip6 filter FORWARD policy drop`、`DOCKER-USER`、raw `-d <容器 v6> ! -i <网桥> -j DROP` 按 B8 的方向化目的判定同样适用；`meta nfproto` 判定按考察包的族取值；`fib … oif missing` 形式的 rpfilter 判为满足（每个投影 ACK 前已以 `RTM_GETROUTE iif <出口 TUN> from <源>` 证明反向查找落到出口 TUN），其他 `fib` 表达式视为可能命中 | `firewallConflict`（按族报告） |
| ICMPv6 限速 | `net.ipv6.icmp.ratemask` 默认不限 Packet Too Big（type 2），出口宿主自己发出的 PTB 不被限速；其他错误受 `net.ipv6.icmp.ratelimit` 约束 | 不探测；只影响 time exceeded 的计数 |

出口 TUN 不配全局 IPv6 地址；内核为 `ARPHRD_NONE` 设备自动生成的链路本地地址保留（不影响转发，不作为错误源）。

### 能力按族报告

`ExitCapability` 现在只有一个 `status`，无法表达“IPv4 ready、IPv6 不可用及原因”。

| 方案 | 形态 | 评价 |
| --- | --- | --- |
| (a) 保留形态 | `status` 表示“任一族 ready”，另加 `familyUnavailable: {ipv6: reason}` | 改动小，但 `status` 的含义变得模糊，Server 校验（ready 必含 ipv4）要改成族相关的特例 |
| (b) 按族条目 | `families: [{ family, status, installed }]` 取代 `supportedFamilies`／`installedFamilies`／`status`；共享前提（NET_ADMIN、nft、ctnetlink、所有权冲突）失败时两族同一原因 | 每族的状态与原因各自成立，源端选择与路径检测直接按族读取；需要 IPC 与 State 契约版本变化 |
| (c) 两个角色 | 每族一个 `ExitRole`、一个 capability | owner、表、journal 各两份，违反“唯一出口 owner” |

**推荐 (b)**。`PeerExitFacts.installed_families` 已是按族列表，只改源端解读：`ready_exits(family)` 按流的族选出口。Server 校验改为：每族最多一条、`installed` 必须是角色声明的族、`ready` 的族须有对应 acquire 能力（`feature:exit-ipv4` v1、新增 `feature:exit-ipv6` v1）。

### 角色与配置

`ExitRole.family` 改为 `families`（有序去重，`nat` 仍是 `masquerade`），`generation` 覆盖族集合。**族集合变化即角色 generation 变化**：现有规则下已安装角色的全部投影 Retire，即管理员为一个出口打开 IPv6 时，它上面正在跑的 IPv4 出口连接会断一次。这是一次性的配置变更后果，第一版接受；要避免它需要按族的 generation，复杂度不值得。

角色是否包含 IPv6，有两种来源：

| 方案 | 含义 |
| --- | --- |
| (i) 能力即开 | `exitEnabled` 覆盖两族；inventory 声明 `feature:exit-ipv6` 就带上 ipv6 |
| (ii) 管理员按族开启 | 新字段 `nodes.<id>.exitFamilies`（默认 `["ipv4"]`），`exitEnabled` 仍是总开关 |

`autogroup:internet` 已经覆盖 IPv6 公网，(i) 会让升级后的现有网络在没有任何配置变化的情况下开始经出口转发 IPv6——实际出网的流量范围扩大了，而且出口宿主打开 IPv6 forwarding 还有上面的 RA 风险。**推荐 (ii)**：默认不变；Server 只在 `exitFamilies` 含 `ipv6` 且 inventory 声明 `feature:exit-ipv6` 时把 ipv6 放进角色。

### 待用户决定

- **Q2**：出口能力按族报告（`families: [{family, status, installed}]`），IPC 与 State 契约随之换版本。推荐：同意。
- **Q3**：IPv6 出口需管理员按节点显式开启（`nodes.<id>.exitFamilies`，默认仅 `ipv4`），而不是有能力即开。推荐：显式开启。
- **Q4**：`accept_ra ≠ 2` 而默认路由来自 RA 时，IPv6 族报 `ipv6RouterAdvertisementsIgnored` 不可用（而不是只告警）。推荐：不可用——它预示宿主稍后会失去 IPv6，接受它等于让出口在未来某刻静默失效。

## 3. 策略与投影 {#policy}

### 分割与上限

分割本身已双栈（`EgressPartition` 按 IPv4／IPv6 各自 first-match 切分，`geoip:*` 与显式 CIDR 的 v6 部分已进入 `ipv6` 区间）。变化在**装什么**：

| 区间 | 现在 | IPv6 出口后 |
| --- | --- | --- |
| 出口动作的 IPv4 | `managedIpv4`（进 TUN） | 不变 |
| 出口动作的 IPv6，且动作的出口集合中有配置上可承载 IPv6 的出口 | `sinkIpv6`（OUTPUT reject） | `managedIpv6`（进 TUN，src 为源的 Mesh IPv6）|
| 出口动作的 IPv6，出口集合在配置上都不承载 IPv6；或源没有 Mesh IPv6 地址 | `sinkIpv6` | 仍 `sinkIpv6`（静态 fail closed，最快的拒绝） |
| direct／unmanaged | 补集 | 不变 |

“配置上可承载 IPv6”是静态的：出口在全量配置中 `exitEnabled` 且 `exitFamilies` 含 `ipv6`。节点不能从自己的投影推断它，需要投影带上按族的出口资格：

| 方案 | 形态 |
| --- | --- |
| (a) `eligibleExits` 按族 | `eligibleExits: [{ node, families }]`（或并列 `eligibleExits6`），仍由 Server 按全量配置给出 |
| (b) `EgressPlan` 的每条规则带族 | `rules[].families`，由 Server 预先算好每条规则对每族的可执行性 |

**推荐 (a)**：族资格是出口节点的属性，不是规则的属性；放在 `eligibleExits` 里，分割、判定、DNS plan 与路径检测都经同一 `matching::executed` 读取，`EgressPlan` 不需要带族。`selection`（`egress.generation`）不变——它是选择身份，族资格变化经 netmap 内容变化生效。

出口集合含源自己时（[Action](/v2/configuration-reference/routing#action)）按族各自求：`[other, self]` 中 other 不承载 IPv6，则该源的 IPv6 目的按 direct，IPv4 目的经 other。这与现行“自己之前的出口按配置可用性判定”一致，只是可用性按族。

上限：

| 上限 | 方案 | 推荐 |
| --- | --- | --- |
| 捕获区间 16,384 | 按族各 16,384，或两族合计 | **合计**（`managedIpv4 + managedIpv6 + sinkIpv6`）：上限来自一次 nft 事务与读回的字节量（16,384 个 IPv6 区间约 1.6 MB），按族各算会让最坏批次翻倍、越过已测得的 ENOBUFS 边界 |
| 捕获路由 32,768 | 现在只计 IPv4 最小 CIDR 覆盖 | **两族合计**：上限来自内核监视器通知与 netlink 读取，IPv6 路由同样占用；IPv6 捕获表为独立的私有表与一条 IPv6 RPDB 选择器 |

`geoip:cn` 直连方案的 IPv6 补集规模尚无实测；定稿前用 `mise run perf:capture` 加 IPv6 区间测一次，若合计逼近上限再议。

### 运行时只有 IPv4 的出口

捕获是静态的，出口的 IPv6 族在运行时可能不可用（forwarding 被关、地址消失、防火墙）。此时 `managedIpv6` 中的包已进入源 TUN，由首包判定处理：

| 方案 | 行为 | 评价 |
| --- | --- | --- |
| (a) fail closed＋快速拒绝 | 选择按族：`choose_exit(nodes, ipv6)` 取第一个 IPv6 就绪的出口（含最后已知就绪）；没有则判定 `exitUnavailable`，dataplane 向本机 TUN 合成 ICMPv6 destination unreachable **code 1（administratively prohibited）**，与现有本机合成错误同一路径、同一限额（每秒 32 条） | 无泄漏；Linux 把 code 1 映射为 `EACCES` 硬错误，`connect` 立即失败，应用按地址列表退回 IPv4 |
| (b) fail closed＋静默丢弃 | 同上但不合成错误 | 无泄漏，但不支持 Happy Eyeballs 的应用要等连接超时 |
| (c) direct | 出口没有 IPv6 时让 IPv6 从源本地发出 | **拒绝**：运行时就绪不是 direct 的输入（现行[Action](/v2/configuration-reference/routing#action)），这正是要防的泄漏 |

**推荐 (a)**。按族选择意味着同一方案下 IPv4 与 IPv6 可能经不同出口（主出口只有 IPv4 就绪时，IPv6 经备用出口）；两者都在管理员允许的有序集合内，不扩大授权，但出口地址的地区可能不同，列为 Q6。

判定原因：静态不可承载（角色不含 ipv6、源无 Mesh IPv6）仍为 `unsupportedFamily`；运行时不就绪为 `exitUnavailable`，路径检测的 `targets[].exitUnavailableReason` 给出该族的具体原因（如 `ipv6ForwardDisabled`）。不另设 `exitFamilyUnavailable`：运行时与静态的区分已经由这两个原因表达，族由目的地址自明。

### 禁止的外部目的

`is_exit_destination` 的绝对排除需补 IPv6 字面范围，并在出口 nft `route` 链与之对应：`::/128`、`::1/128`、`::ffff:0:0/96`（IPv4-mapped 不是 IPv4 出口的别名）、`64:ff9b::/96` 与 `64:ff9b:1::/48`（NAT64 前缀不经出口转成 IPv4）、`fe80::/10`、`fc00::/7` 中的 Mesh 池、`ff00::/8`，以及 `fib daddr type {local, multicast}`。ULA 只有显式 CIDR Access（出口 LAN）可达，与 RFC1918 一致。

### 待用户决定

- **Q5**：出口 IPv6 族运行时不可用时，IPv6 目的 fail closed 并向本机合成 ICMPv6 administratively prohibited（不 direct、不静默丢弃）。推荐：同意。
- **Q6**：同一方案内按族选择出口（IPv6 可能经备用出口、IPv4 经主出口）。推荐：允许；若要求两族同出口，则主出口无 IPv6 时 IPv6 直接拒绝，备用出口不用于 IPv6。
- **Q7**：出口集合含源自身时，族资格按族各自求（`[other, self]` 中 other 不承载 IPv6 时，IPv6 对该源为 direct）。推荐：同意，与现行可用性规则同构。

## 4. DNS {#dns}

DNS plan（`dns_resolution` 的 `noExit`／`viaExit`／`exitPlanRequired`）与地址族无关：查询经 `Purpose::Dns` circuit 传递，A 与 AAAA 同一 resolver。变化在回答之后。

### AAAA 与泄漏

| 方案 | 行为 | 安全性 | 体验 |
| --- | --- | --- | --- |
| (a) AAAA 原样返回，包层 fail closed | 现行语义保留；IPv6 目的按第 3 节捕获与判定 | 防泄漏完全由捕获＋准入保证：出口规则覆盖的 IPv6 目的要么 `managedIpv6` 要么 `sinkIpv6`，都不会从物理网卡发出 | 配合第 3 节 (a) 的快速拒绝，应用立即退回 A 记录 |
| (b) 源端按出口能力过滤 AAAA | 源 DNS owner 在所选出口 IPv6 不就绪时把 AAAA 回答换成 NODATA | 不增加安全性（硬编码 IPv6、DoH、缓存仍要靠包层）；且让 DNS 回答依赖运行时事实，回答与之后的判定可能不一致 | 不合成错误时更快；有快速拒绝时无差别 |
| (c) 出口 resolver 按自身能力过滤 | 出口在自己 IPv6 不就绪时对 AAAA 回 NODATA | 同 (b)；而且解析出口与承载 IPv6 的出口可以不是同一个（IP 规则可经别的出口） | 同 (b) |

**推荐 (a)**：过滤 AAAA 从来不是防泄漏措施（现行文档已如此规定），泄漏边界只能在捕获与准入；快速拒绝已解决体验问题。(b)／(c) 只在发现有不处理 `EACCES`、只用第一个地址的应用时作为体验补丁另议，且不得被描述为安全机制。

### 可信关联

关联已按 (名称, 记录类型) 各一条，AAAA 回答产生独立的 AAAA 关联，界限（每源 128 条、每 overlay 4,096 条）两族共享，不需要改协议。变化：

- 捕获：名称规则决定为出口的 AAAA 关联地址并入 `managedIpv6`（出口集合在配置上不承载 IPv6 时仍并入 `sinkIpv6`）；决定为 direct／unmanaged 的移出受管区间。
- 保留：`Desired::retaining`／`retained_capture` 扩展到 IPv6——关联到期后，经出口仍存活的 IPv6 流的目的地址留在捕获中直到流退役。
- 等待：源 DNS owner 现在对 AAAA 回答同样等待关联生效（行为不变；之前它等来的关联只会让地址进 sink）。
- 同名 A 与 AAAA 的动作由同一名称规则决定，必然相同；不产生新的 `egressConflict` 来源。

### 待用户决定

- **Q8**：AAAA 不过滤，泄漏边界只在捕获与准入（配合 Q5 的快速拒绝）。推荐：同意。

## 5. 数据面 {#dataplane}

| 项 | 设计 |
| --- | --- |
| 流键 | `ExitTuple` 的地址改为 `IpAddr`（源与目的同族）；echo 仍按 `(identifier, sequence)`。Underlay `OriginalTuple` 同样改为按族；flows／quarantine／echo 集合按族各一份（nft concat 不能混合族），容量 `HISTORY` 4,096 为两族合计（journal 计数一份） |
| 准入 | `admit_forward`／`admit_reply`／`admit_related` 本身已不区分族；去掉 `outbound.rs`、`decide.rs`、`plan.rs` 中 IPv6 即 `UnsupportedFamily` 的分支，改为第 3 节的族资格；源的 `sourceAddresses` 绑定 Mesh IPv6 |
| ICMPv6 业务 | echo request 128／reply 129 作为 `IcmpEcho` 流，与 IPv4 echo 同一寿命规则 |
| ICMPv6 RELATED | 出口 nft 的错误类型改为按族：IPv4 `icmp type {3, 11}`，IPv6 `icmpv6 type {1, 2, 3}`（destination unreachable、packet too big、time exceeded；与 dataplane 能解析的集合一致，不放行 parameter problem）。路由器错误经 `route` 链＋`fwmark ＋ ipproto ipv6-icmp` 的 IPv6 RPDB 规则回到出口 TUN；出口宿主自己的错误（hop limit 耗尽、出接口 MTU 不足：IPv6 转发在 FORWARD 钩子之前检查 MTU，ct 尚未 confirmed）经 `local` 链的同一“owned 或 unconfirmed”条件重路由 |
| 出口 PMTU（外部方向） | 出口之外路由器或出口宿主发回的 PTB 经 RELATED 逆 NAT 回到源，源内核据此降 PMTU。IPv6 路由器不分片，这是 IPv6 下 PMTU 的唯一途径：组件测试必须覆盖“宿主自身 PTB 引用 unconfirmed ct”在 IPv6 上同样成立 |
| 源端 PMTU | 现有本机 PTB 合成（`inner` 上限）对出口流同样适用；`inner < 1280` 的通道不承载 IPv6：经这样一跳的 IPv6 出口流同样丢弃并回 ICMPv6 no route，不尝试经该通道出口 |
| 出口回包超限 | 现行“出口对外部超限回包只丢弃”在 IPv6 下更伤（无分片退路，UDP 回包大于出口首跳 `inner` 永远到不了）。可行做法：出口在出口 TUN 上以源 Mesh 地址的名义写入一条 PTB（引用逆 NAT 后的回包），内核按 RELATED 原方向 NAT 改写外层源与被引用头后发给外部发送方。列为后续切片 I6-2，第一版与 IPv4 同一限制 |
| MSS clamp | `meshway_mss_*` 表已在 `inet` 族、已有 `peer6`；补两处：经出口的 `other` 规则增加 `nfproto ipv6` 一条（上限 −60），出口 TUN 入向增加 `ip6 saddr` 查 `@peer6` |
| RPDB／回程 | 成对规则与回程路由增加 IPv6 版本（源 `/128`）。IPv4 与 IPv6 的规则优先级是各自独立的列表，按族各自占用保留优先级：IPv6 对数上限同 IPv4（128） |
| nft 表 | 过滤表已是 `inet`，在同一表内加 IPv6 集合与规则；NAT 表新增 `ip6 meshway_exit_nat6_<installation>`，角色含 ipv6 时创建，journal 记录其名称／handle，`tables()` 按角色的族返回。备选是把 NAT 改成单个 `inet` nat 表（Linux ≥ 5.2），但会改变已安装角色的表集合与 journal 恢复路径，第一版不做 |

## 6. 漫游与前缀变化 {#roaming}

与 R3 `roam/exit` 同一原则：出口接口或地址变化时能力先回 `probing`，重探测后 `ready`；用旧地址 masquerade 的流结束，新流用新地址。IPv6 的差别在地址生命周期：

| 情形 | 内核行为 | 设计 |
| --- | --- | --- |
| 前缀变化（PD 续租换前缀、迁移到新网络） | 旧前缀地址先变 deprecated，到 `valid_lft` 才删除（可能数小时到数天）；masquerade 新连接会选 preferred 地址，旧 ct 继续用 deprecated 地址，而上游可能已不再路由旧前缀，旧流挂到空闲超时 | 出口 owner 订阅出口接口的 IPv6 地址事件：**某前缀不再有任何 preferred 全局地址**即视为该前缀离开，能力回 `probing`，退役 reply tuple 目的落在该前缀内的投影（从 ctnetlink 读到的 reply tuple 判定），不等内核删除 |
| 临时（privacy）地址轮换 | 每天生成新临时地址、旧的变 deprecated，同前缀仍有 preferred 地址 | 噪声（与[漫游设计](/v2/node/roaming-design)对 R1 的分类一致）：不探测、不退役；旧临时地址真正删除时内核删除使用它的 masquerade ct，owner 按现有“内核已删除 confirmed 条目”路径结束该流 |
| 地址删除 | 内核删除使用该地址的 masquerade ct | 同 IPv4 |
| IPv6 默认路由消失 | 新流无路可走 | 该族 `ipv6NoEgress`，IPv4 族不受影响 |

长连接经出口时，临时地址轮换不会切断它；只有前缀离开或地址删除才会。若宿主 `use_tempaddr = 2`，出口流量多数会用临时地址出网——这是宿主自己的隐私选择，Meshway 不改写。

### 待用户决定

- **Q9**：前缀离开（该前缀不再有 preferred 地址）即主动退役绑在该前缀上的 IPv6 出口流，而临时地址轮换不算变化。推荐：同意；替代方案是只等内核删除地址，旧流可能挂数小时。

## 7. 验收 {#validation}

### 矩阵行

| 方案 | 评价 |
| --- | --- |
| `exit/off` 加阶段 | `exit/off` 已有 16 个阶段、约 800 秒，再加会让一次失败的定位更难，且 IPv4 行的第 4 阶段（A 的 IPv6 全范围 sink）需要保留作为“未开启 IPv6 时不变”的证明 |
| 新行 `exit6/off` | 复用同一 fixture 与 controller，只换拓扑参数与阶段集；与 `exit/off` 可并行 |

**推荐新行 `exit6/off`**；`exit/off` 不变（E 的 `exitFamilies` 保持默认 `["ipv4"]`，第 4 阶段继续断言 IPv6 sink），兼作“默认不开 IPv6”的回归证明。

### fixture 拓扑

在 `dev/exit-fixture/` 现有拓扑上，`external`、`far`、`lan` 改为双栈 Docker 内部网络（`docker network create --internal --ipv6`，`scripts/verify_runtime.py` 已在 Planck 上这样建 IPv6 网络）：

- `external`／`far` 的 IPv6 子网取自 `3ffe::/16` 内随机 /64：该前缀（原 6bone，RFC 3701 已收回）在 `2000::/3` 内且不在排除表中，因此为 Internet 分类地址，与 IPv4 用 `11.77.x.0/29` 同理；不使用 `2001:db8::/32`（文档前缀，不是 Internet 分类，许可的 `ExitDestinationScope::Internet` 断言会失败）。
- `lan` 用随机 ULA /64，Access 以显式 CIDR 授权（出口 LAN）。
- router 容器双栈并在自己的 namespace 内 `net.ipv6.conf.all.forwarding=1`（容器 `--sysctl`，不动宿主），far 侧 MTU 1280。
- E 的 external 链路上 IPv6 为静态地址（不依赖 RA），因此 `ipv6RouterAdvertisementsIgnored` 用一个单独阶段在 E 的 namespace 内模拟（`accept_ra=1`＋一条 `proto ra` 默认路由）。
- Mesh 控制／数据链路保持 IPv4（外层族与内层出口族无关）。

### `exit6/off` 阶段

| 阶段 | 断言 |
| --- | --- |
| 1 能力 | E 的 acquire 声明 `feature:exit-ipv6` v1；`families` 两族 ready、installed；A 的 `capture` 有 `managedIpv6`，`sinkIpv6` 不含出口区间 |
| 2 前置条件 | E 的 namespace `net.ipv6.conf.all.forwarding=0` → IPv6 族 `ipv6ForwardDisabled`、IPv4 族仍 ready 且已有 IPv4 出口流不断；恢复后 IPv6 回 ready、以同一内容重装；`accept_ra` 模拟得 `ipv6RouterAdvertisementsIgnored` |
| 3 包流 | A→R→E 的 IPv6 TCP、UDP、ICMPv6 echo 到 external 服务；服务看到的来源是 E 的 external IPv6 地址（masquerade）；A 物理接口上没有明文 IPv6 业务包（pcap） |
| 4 PMTU | 经 far（MTU 1280）的 IPv6 TCP 收敛、`ping -6 -M do` 报 PTB 1280；E 的 external 出接口 MTU 调低后由 E 宿主自己发出 PTB 同样到达 A；`traceroute -6` 每跳 time exceeded 均回到 A；窄通道（`inner` < 1280）时 A 本机回 no route 且不经该通道 |
| 5 MSS | A 的 IPv6 SYN 与 E 出口 TUN 入向 SYN 被限到首跳上限 −60 |
| 6 撤回与 quarantine | 撤权／停用出口后 IPv6 出口流立即终止，ctnetlink 回读 IPv6 条目不存在，外部迟到回包被 quarantine 拒绝；租约到期同样全撤；journal 恢复后 IPv6 quarantine 重新装入 |
| 7 只有 IPv4 的出口 | 运行时关闭 E 的 IPv6 族：A 对 IPv6 目的 `connect` 立即失败（`EACCES`），A 物理接口上无该目的的包；`curl` 对双栈名称经 IPv4 成功；路径检测给出 `exitUnavailable` 与 `ipv6ForwardDisabled`。另配一个 `exitFamilies: ["ipv4"]` 的出口 E2：A 的捕获对 E2 区间为 `sinkIpv6`，判定 `unsupportedFamily` |
| 8 DNS 与关联 | 名称规则经 E 的方案下，AAAA 经 E resolver 原样返回并产生 AAAA 关联，该 IPv6 地址进入 `managedIpv6`、连接经 E；关联到期后存活流保留、新流按新判定 |
| 9 前缀变化 | E 的 external 换到另一 `3ffe` /64（旧地址设为 deprecated 后删除）：能力 `probing`→`ready`，绑旧前缀的流结束，新流来源为新地址；添加并废弃一个临时地址不引起探测 |
| 10 方案切换 | 切到 `mesh` 结束全部 IPv6 出口流，A 的 IPv6 出口区间回到 direct／unmanaged 的补集 |

组件阶段：IPv6 版本的 nft 程序事务、ctnetlink `AF_INET6` GET／DELETE／回读、quarantine、unconfirmed ct 上的宿主 PTB、防火墙判定的 `ip6` 族与 Docker `ip6tables` 规则集样例、firewalld `IPv6_rpfilter` 样例判为不冲突。

### 真机现状（2026-10-16 只读）{#real-hosts}

| 机器 | IPv6 | 结论 |
| --- | --- | --- |
| us-lax-it7 | `ip -6 addr show scope global` 只有 Meshway TUN 上的 Mesh ULA（`fd79:5c1e:e325::4/128`）；主表与其他表都没有 IPv6 默认路由；`net.ipv6.conf.all.forwarding=1`、`all.accept_ra=2`；内核 6.8、nftables 1.0.9、`nf_conntrack_netlink` 已加载 | 没有全局 IPv6，不能作 IPv6 出口 |
| Planck 宿主 | 全局地址只有 CloudflareWARP 上两个 `2606:4700:cf1::/48` 内的 `/128`（一个 deprecated），WARP 表 65743 只有到 Cloudflare 前缀的路由，无 IPv6 默认路由；Docker 29.8.2 | 不能作 IPv6 出口；可承载隔离 fixture 的 `--internal --ipv6` 网络（`roam/direct` 已在用） |
| meshway-planck 容器 | 只有 Mesh ULA（`fd79:5c1e:e325::5/128`），所在 Docker 网络未开 IPv6 | 不能作 IPv6 出口 |

真机 IPv6 出口验收（X7 的 IPv6 对应项）需要一台有原生全局 IPv6（至少一个 on-link /64 与 IPv6 默认路由）的 Linux 宿主。

### 待用户决定

- **Q10**：验收用新行 `exit6/off`，`exit/off` 不变。推荐：同意。
- **Q11**：真机 IPv6 出口验收需要新增一台有原生 IPv6 的宿主（it7、Planck 都没有）；没有它时 I6 只能以隔离矩阵行交付。推荐：提供一台（任一带 /64 的 VPS），否则在文档中把真机 IPv6 出口列为未验收。

## 8. 切片 {#slices}

### I6-1（最小可交付，已实现）

| 组件 | 内容 |
| --- | --- |
| Configuration／Server | `nodes.<id>.exitFamilies`（默认 `["ipv4"]`）；`ExitRole.families`；`feature:exit-ipv6` v1；投影 `eligibleExits` 按族；`compilerVersion` 升到 `meshway-netmap-4`；Server 对 `ExitCapability` 的按族校验 |
| 契约 | `ExitCapability.families`；IPC 版本加一（Status 的 exit 部分）；新增 `ExitUnavailableReason`：`ipv6ForwardDisabled`、`ipv6NoEgress`、`ipv6RouterAdvertisementsIgnored`；重生成 OpenAPI／SDK 并过 drift |
| Policy | 按族的出口资格进入 `matching::executed`；分割装 `managedIpv6`；`is_exit_destination` 的 IPv6 排除；判定去掉 IPv6 即 `unsupportedFamily` 的分支；关联的 IPv6 捕获与保留 |
| Control／dataplane | `ready_exits(family)`；`ExitTuple` 按族；族不就绪时合成 ICMPv6 administratively prohibited |
| Underlay | 源端 IPv6 捕获选择器与私有捕获表；出口 IPv6 集合与规则、`ip6` NAT 表、ctnetlink `AF_INET6`、IPv6 RPDB 成对规则与回程、ICMPv6 RELATED（路由器与宿主自身）、按族前置探测、防火墙判定的 `ip6` 族与 `ip6tables-legacy-save`、MSS clamp 两处补齐 |
| DNS | 无协议变化；AAAA 关联进入 `managedIpv6` |
| 验收 | 矩阵行 `exit6/off` 第 1–8、10 阶段与组件阶段；`exit/off` 原样通过；`mise run perf:capture` 加 IPv6 区间测合计上限 |

### 后续

| 切片 | 内容 |
| --- | --- |
| I6-2 | `exit6/off` 第 9 阶段（前缀变化、临时地址）与 `roam/exit6`；出口对外部超限回包合成 PTB（IPv4 同时受益）；真机 IPv6 出口宿主验收 |
| I6-3 | 路由前缀／NPTv6（源独立公网 IPv6）与入站准入，需单独产品设计（Q1 的另一支） |
| 不在本设计 | NAT64／DNS64（`64:ff9b::/96` 始终禁止经出口）、IPv6 扩展头与分片（数据面现行范围不变）、userspace／netstack 出口 |

## 待用户决定（汇总） {#decisions}

| # | 问题 | 推荐 |
| --- | --- | --- |
| Q1 | 第一版地址模型 | NAT66 masquerade；路由前缀／NPTv6 不做 |
| Q2 | 能力报告形态 | 按族 `families: [{family, status, installed}]`，IPC／State 换版本 |
| Q3 | IPv6 出口的开启方式 | 管理员按节点 `exitFamilies` 显式开启，默认仅 `ipv4` |
| Q4 | RA 被 forwarding 关闭的宿主 | IPv6 族 `ipv6RouterAdvertisementsIgnored` 不可用 |
| Q5 | 运行时 IPv6 族不可用 | fail closed＋本机 ICMPv6 administratively prohibited |
| Q6 | 两族是否必须同一出口 | 允许按族选择（IPv6 可经备用出口） |
| Q7 | 出口集合含源自身时 | 族资格按族各自求 |
| Q8 | AAAA 是否过滤 | 不过滤，泄漏边界只在捕获与准入 |
| Q9 | 出口宿主前缀变化 | 前缀离开即退役其上的流；临时地址轮换不算变化 |
| Q10 | 验收组织 | 新行 `exit6/off`，`exit/off` 不变 |
| Q11 | 真机 IPv6 出口宿主 | 需新增一台有原生 IPv6 的宿主 |
