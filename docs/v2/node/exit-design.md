# Exit／NAT 设计草案

> **未实现、未验收。** 本页是 X0 设计研究稿，不加入文档导航，不构成可用 API 或功能承诺。当前安装器仍拒绝出口目标；运行范围以[网络模式](/v2/node/runtime#安装范围与状态)和[验收范围](/v2/node/validation)为准。

## 目标与建议

首版实现 Linux **IPv4 内核转发＋状态型 masquerade**，公网与显式授权的出口 LAN 使用同一准入、流生命周期和撤回机制。IPv6 在模型中明确表达，但首版出口执行拒绝；不能因没有 IPv6 NAT 而让受管 IPv6 从源端本地出网。

推荐 Underlay 的现有唯一 Kernel owner 管理独立出口 TUN、nftables 私有表、路由、归属 journal 和精确 conntrack 清理；初版通过固定路径的 `nft` CLI 提交类型化生成的批事务。dataplane 保持唯一业务流表、Noise、完整有序路径和最终输出检查；Control 保持唯一执行快照签发者。DNS 由现有 DNS owner 扩展为经 Mesh 的受许可 resolver 服务。

出口开关仍只有管理员 Web 配置 `nodes[id].exitEnabled`。Agent 收到职责后自动准备／安装／撤回所属转发资源，不新增手工开出口的 CLI。宿主 `ip_forward` 是**检测前置条件**：本方案不修改宿主 sysctl；值不为 1 时明确报告 `exit_unavailable: ip_forward_disabled`，由系统安装文档说明宿主管理员如何预配置。自动安装 NAT／路由与自动修改全局 forwarding 是不同权限范围。

配置的唯一语义来源仍为 [routing.egress 与出口联合许可](/v2/configuration-reference/routing#出口联合许可)、[Access](/v2/configuration-reference/access#domains)、[出口 DNS](/v2/configuration-reference/dns#使用出口时)。下面定义实现建议与缺口，不另定义出口选择规则。

## 当前实现落点

研究基线为 Meshway `678591fe`；文件位置以该提交为准。

| 位置 | 已有行为／新增工作的边界 |
| --- | --- |
| `crates/meshway-policy/src/compile/plan.rs`：`plan`、`external_route`、`route_of` | 已按方案和 first-match 评估 exit、exitEnabled 和路径；规划结果不是运行能力证明 |
| `crates/meshway-policy/src/compile/routing.rs`：`Routes::segment`、`through` | 出口前 Mesh 路段递归求解，保留原业务主体，拒绝重复节点和循环 |
| `crates/meshway-policy/src/admission.rs`：`admitted_path`、`admitted_traffic`、`admit_forward`、`admit_reply` | 实际文件不在 `compile/` 下；已校验 sourceAddresses、精确 IP、协议／端口、完整 path、previous peer、出口开关与到期 |
| `crates/meshway-configuration/src/execution/mod.rs`：`PathPermit`、`TrustedDnsAssociation`、`DnsEgress` | 外部 permit 当前是精确地址／端口，不是整个公网的宽 grant；DNS 只有 noExit 或不可用证明 |
| `crates/meshway-dataplane/src/lib.rs`：`PendingAction::release`、`Action::Exit` | `Hop::Exit` 已能生成受 guard 约束的明文动作；动作枚举存在不代表宿主已转发 |
| `crates/meshway-underlay/src/linux/kernel/{mod,plan,capture/mod,journal,monitor}.rs` | 唯一 Kernel、私有 RPDB/table、正向 capture／负向 sink、资源监控；`plan::pools` 明确拒绝 `/0` |
| `crates/meshway-underlay/src/linux/protection.rs`：`Mark::transform`、`Shared::prepare` | SO_MARK 读改写／读回并保持外部 selector 语义；`kernel::allocate` 随机起点选择空闲单 bit，**不是固定 `0x40000000`** |
| `crates/meshway-underlay/src/linux/dns.rs`、`resolver_exchange.rs` | 前者仅 bootstrap resolver；后者消费密封 local resolver route，不能将 bootstrap 权限借给出口 DNS |
| `crates/meshway-dns/src/{selection,route,owner/mod,dispatcher/mod,service/mod}.rs` | MagicDNS、useWithExit 选择、精确上游许可、query ticket 与输出取消；现有 Service 只允许 loopback |
| `crates/meshway-dns/src/system/linux/{mod,resolved,file/mod,storage}.rs` | namespace 预约、真实上游来源、系统 DNS 安装与恢复；不得绕过现有归属机制 |
| `apps/meshway-daemon/src/runtime/network/{mod,worker,requirements}.rs` | `publish` 协调发布，worker 调用 `requirements::validate`；实际拒绝位于 `requirements.rs`：非 noExit DNS、exitEnabled、非 Mesh permit、非 DNS owner 的 localEgress 均阻断安装 |

没有独立的 `docs/v2/node/dns.md`；现行 DNS 组件定义在 [dataplane](/v2/node/dataplane#dns-查询-owner)。

## Tailscale 源码研究

上游依据为本地 `references/tailscale` checkout `ab2380a6312858ccf95451787f33acc9c9fca066`。本节路径相对该 checkout。历史路径 `wgengine/router/router_linux.go` 在此版本为 **`wgengine/router/osrouter/router_linux.go`**。这里的结论限于所读版本；Tailscale 托管控制面的 ACL 编译器不在此仓库内，不能从客户端源码声称验证了服务端全部授权逻辑。

### Linux 转发、NAT 与清理

| 文件／函数 | 关键结论 |
| --- | --- |
| `util/linuxfw/detector.go`：`detectFirewallMode`、`pickFirewallModeFromInstalledRules` | `TS_DEBUG_FIREWALL_MODE` 优先于 pref hint；此版本无偏好通常默认 iptables，缺二进制／无编译支持可选 nftables；显式 auto 按已有规则和能力选择，两者均使用或均空且 nft 可用时选 nft。Gokrazy／JetKVM 特例选 nft |
| `util/linuxfw/iptables_runner.go`：`AddChains`、`AddHooks`、`addBase4`、`addBase6`、`AddSNATRule` | `filter/ts-input`、`filter/ts-forward`、`nat/ts-postrouting` 经 INPUT／FORWARD／POSTROUTING jump 接入。TUN 入站转发打 subnet mark，masquerade 匹配该 mark，不是所有宿主出站都 NAT |
| `tsconst/linuxfw.go` 的 Linux mark 常量；`util/linuxfw/linuxfw.go` | subnet mark `0x40000`，bypass mark `0x80000`，mask `0xff0000`；两种 mark 用途不同 |
| `util/linuxfw/nftables_runner.go`：`AddChains`、`AddBase`、`AddSNATRule` | Go nftables/netlink 后端，批量 `conn.Flush()`。此提交在 ip／ip6 的常规 `filter`／`nat` 表建 FORWARD／INPUT／POSTROUTING 和 `ts-*` jump 链；不是仅有独立 `ts-filter`／`ts-nat` 表。`expr.Masq` 匹配 subnet mark；旧表名出现在兼容清理中 |
| `util/linuxfw/iptables.go`：`checkSupportsV6Filter`、`checkSupportsV6NAT`；nft runner `getTables` | IPv6 filtering／NAT 支持需检测，iptables NAT 表列表仅纳入可用 IPv6 NAT；nft 按 IPv6 能力建对应族的规则 |
| `cmd/tailscale/cli/up.go`；`ipn/ipnlocal/local.go`：`routerConfigLocked` | `--snat-subnet-routes` 默认 true，映射 `NoSNAT` 再到 `SNATSubnetRoutes`。与 advertise-exit-node 同时关闭会警告 Internet 流量可能不可用；并非仅 LAN 才受该开关影响 |
| Linux router：`Set`、`Close`、`setNetfilterModeLocked`、`delSNATRule` | Set 差量移除 SNAT／routes；Close 降接口、删 RPDB／routes、关闭 netfilter；runner 的 `DelHooks`／`DelBase`／`DelChains` 清理自有 jump／内容／链。删 NAT 规则本身不等于清除已有 conntrack |

`net/netutil/ip_forward.go::CheckIPForwarding` 读取全局和所需接口 forwarding，区分读失败与不可用 warning，提示 `https://tailscale.com/s/ip-forwarding`。`ipn/ipnlocal/local.go::CheckIPForwarding` 和 `health/health.go::SetIPForwardingCheck` 接入 CLI／运行 health，控制联系还报告 `IPForwardingBroken`。

**不能概括为 Tailscale 从不写 sysctl：**普通 Linux 出口主要检查并提示；Linux router 的 `Set` 在 Gokrazy 广告路由时调用 `enableIPForwarding` 写 IPv4／IPv6 forwarding，`cmd/containerboot/forwarding.go` 也准备容器 forwarding；此 checkout 的 router 还会写 `net.ipv4.conf.all.src_valid_mark=1` 处理 rp_filter。Meshway 推荐检测而不改宿主 sysctl，不能复制这些写入例外。

非 Linux 一句结论：`cmd/tailscaled/tailscaled.go::handleSubnetsInNetstack` 在 Windows、命令行 macOS 等平台默认通过用户态 netstack 转发／SNAT（GUI／其他平台有各自装配），并非复用 Linux nftables。

### 经出口的 DNS

- `ipn/ipnlocal/node_backend.go::dnsConfigForNetmap` 先保留 MagicDNS 域，再将普通查询默认 resolver 设为选中出口的 peerapi `/dns-query`；显式 `UseWithExitNode` default 优先，split 仅保留带该标记的 resolver。`ipn/ipnlocal/local.go::exitNodeCanProxyDNS` 根据 peer 服务能力产生 URL。研究中所称 `exitDNS` 对应此服务／resolver 路线，当前代码不是必须存在一个同名变量。
- `ipn/ipnlocal/peerapi.go::handleDNSQuery` 是 RFC 8484 格式的 **HTTP-over-WireGuard**，并非公网裸 HTTP／HTTPS；`WhoIs` 关联来访 peer，`isPeerAPIDNSAllowed` 执行准入。`offersExitNodeOrAppConnectorAndPeerHasAutogroupInternet` 在提供出口职责时，用当前 filter 对该源到 `0.0.0.0:53`（IPv6 `[2000::]:53`）的 TCP 许可作 Internet grant 探针；不能照搬成 Meshway 的公网地址分类。
- `net/dns/resolver/tsdns.go::HandlePeerDNSQuery` 在 Linux 读取 `stubResolverForOS()` 得到的系统 nameserver，通常尊重出口机器的 `/etc/resolv.conf`（当前仅取一个）；若指回 Tailscale service IP，直接用自身配置 defaults 避免环。Windows／Android 使用 Go 系统 resolver。macOS 文件不能完整表示分域，源码有明确限制。
- `net/dns/resolver/forwarder.go::forwardWithDestChan` 接受出口指定 resolver，`send`／`sendDoH`／`sendUDP`／`sendTCP` 执行有界转发。它不是给请求方一个任意上游代理权限。
- `--exit-node-allow-lan-access` 控制客户端 LAN 路由例外，**不是**选择出口 DNS 的开关；MagicDNS／split DNS 与 LAN 路由分别配置。

### 默认路由与原子性

`ipn/ipnlocal/local.go::routerConfigLocked` 把所选 peer 的 OS routes 交给 router；选 exit 时补齐 `0.0.0.0/0` 与 `::/0`，缺某族也保留黑洞式捕获以免泄漏。Linux router `addRoute` 放进 table **52**；`baseIPRules` 通常是 priority **5210 mark→main、5230 mark→default、5250 mark→unreachable、5270→52**，由受保护 underlay socket 的 bypass mark 避开默认路由，不依赖把每个 DERP／peer IP 全局放行。

`routerConfigLocked` 的 `LocalRoutes` 在 Linux 由 `addThrowRoute` 装为 table 52 的 throw：允许 LAN 时外部接口网段绕出该表；关闭时把这些网段显式捕获。其 internal VM 网络还有独立例外，不能当作 Meshway 的既定产品语义。

`Set` 持 mutex 并用 `cidrDiff` 先加后删 route/address（加失败不继续删旧项）、逐步改 netfilter，**不是 DNS＋WireGuard＋RPDB＋NAT 的一个内核原子事务**。nft 的单次 Flush 才有自身批次原子边界。Meshway 必须自行定义 fail-closed 跨组件切换，不能借 Tailscale 的 Set 宣称方案切换无泄漏或存量流已终止。

### 出口准入与回包

`wgengine/wgcfg/config.go::PeerConfig.AllowedIPs` 与 `wgengine/userspace.go` 的 peer 配置把加密 peer 绑定可发源地址；`wgengine/filter/filter.go::RunIn`／`runIn4`／`runIn6` 检查广告目的集合和控制面下发的源／目的／协议／端口 filter。`wgengine/filter/tailcfg.go::MatchesFromFilterRules`／`parseIPSet` 支持普通地址选择器和 `cap:<peer-node-capability>` 源匹配；filter 的 `CapTestFunc` 查受信节点能力，不能把包自报 `cap` 当授权。

本 checkout 未发现可直接替代上述 filter 的通用 **`exitNode` capability**；`tailcfg/nodecap` 的 suggest／auto-exit-node 是选择功能，`tailcfg/peercap` 是另一类应用 capability。Kubernetes CRD 的 `exitNode` 布尔字段也不是包准入 grant。`autogroup:internet` 在客户端表现为编译后的 filter 及前述 DNS 探针，服务端如何生成它不在本次可证源码范围内。

Linux masquerade 的 conntrack 自动将回包逆 NAT 为原 Mesh 源地址，内核 route→TUN 后由 WireGuard 的目的地址／AllowedIPs 找 peer；Tailscale 也有用户态 filter 的状态（例如 UDP LRU，TCP 非 SYN 放行逻辑），因此不是“全无显式状态”。该较宽状态判断不满足 Meshway 的严格流／原路径合同；Meshway 不能只凭 conntrack ESTABLISHED 发回，也不能在回复时重新求更短的路。

## 数据路径与唯一 owner

```text
sha 应用 IPv4
  → 源端 Underlay：有序 egress capture → 客户端 TUN
  → 唯一 SecureNode：精确需求／permit、源地址、唯一流、Noise
  → sha → lax → nyc：完整许可路径；lax 仅会话转发密文
  → nyc SecureNode：解密、真实包准入、Action::Exit（仍持原 guard）
  → nyc Underlay：出口专用 TUN 写入 → 内核 PREROUTING／FORWARD
  → nft 精确投影门＋POSTROUTING masquerade → nyc 公网或获准 LAN

远端状态型回复
  → nyc 内核 conntrack 逆 NAT → 专用回程路由 → 出口专用 TUN 读出
  → nyc 同一 SecureNode 唯一流表／CID：admit_reply
  → nyc → lax → sha：原完整路径反向、Noise 保护
  → sha 客户端 TUN 写入 → 原应用
```

| Owner | 持有内容 | 不可由其他组件代替的检查 |
| --- | --- | --- |
| Server／Policy | 联合许可编译、source／target／profile／完整路径、可信 DNS 依赖 | 管理员开关、Access、first-match、身份、路径；State 不是权限来源 |
| Control／authority | 当前 permit 所在快照、instance／publication、代际取消、三重 Deadline、安装回执 | 生产唯一签发；先撤销旧操作，过期不靠 HTTP 返回才生效 |
| dataplane | 唯一有界 FlowTable、协议状态、CID、原路径、guard | 原始已认证 Mesh 源与精确内层源、实际目的、服务、路径；回复必须已有正向流 |
| Underlay Kernel | 出口 TUN、nft、RPDB/table、NAT 资源投影、journal、能力探测 | 确认所属资源／标记／接口；提交和每次 I/O 的有效性；内核状态不生成业务资格 |
| DNS owner | 查询、响应、受控系统 resolver 适配、pending／TTL／取消 | 专用 DNS permit、查询名／来源／实际完整路径和输出截止；不是任意代理 |
| daemon | 装配、驱动有界队列、完成真实安装／上报 | 不持第二份流表、DNS 授权数据库或 nft owner |

选择**独立出口 TUN**而不复用普通本机 Mesh TUN：`Action::Deliver` 给本机服务与 `Action::Exit` 给内核转发有不同的入口和规则；共享设备会使“从 TUN 来”不足以区分两种职责。两设备仍归同一个 Host／Kernel，不增加 Host 实例。专用 TUN 不暴露 FD，不接受外来接口接管，不绑定出口公网地址；回程按远端 Mesh 源 `/32` 导向它。

### 内核规则与回程识别

1. 拟用 `inet meshway_exit_<installation>` 过滤表与 `ip meshway_exit_nat_<installation>` NAT 表，名称／handle／规范规则摘要写 journal。不 flush ruleset，不接管宿主 FORWARD policy。私有 hook 对不属于本 owner 的流量 return；对本 owner 流量缺资格一律 drop。独立表 accept 不能覆盖其他表的 drop，宿主防火墙阻断应如实报不可用而非修改它。
2. 只有专用 TUN ingress、已分配给 permit.source 的精确 IPv4、获准外部目的／协议／端口及仍有效的 flow 投影能进入 FORWARD。公网/LAN 的区别由 Policy 许可表达，NAT 实现不分两条业务通路。禁止转入任一 Mesh 池、loopback、链路本地、组播、广播、出口本机地址和所属虚拟接口；出口本机服务须用 Mesh 服务许可。
3. 专用 TUN 的转发流在内核 conntrack 上标记可证明归属的 ct mark；该 mark 与 socket bypass mark 分开分配、保留外部 bit，不从网络包信任 mark。在 PREROUTING 路由前为已识别回复恢复专用回程路由标志；reply RPDB 只把逆 NAT 后的源节点地址导向出口 TUN。forward 方向仅凭专用 iif＋受管资格跳过客户端 capture 块、继续原宿主 RPDB，避免出口自己启用默认捕获时再次吞包。
4. `postrouting` 仅对所属 forward ct／资格集合做 IPv4 masquerade；公网与 LAN 均采用实际出接口地址。回包须 ct reply direction、关联原 tuple 和活 flow 投影；未关联新入站不允许进入 Mesh。DNS 和普通 Mesh TUN 流量不能被这条 NAT 规则接管。
5. 首版使用宿主默认 conntrack zone＋精确归属 mark／原 tuple／ct identity，不笼统设置一个“出口 zone”：回程包必须在 conntrack lookup 前知道 zone，不能在查到 ct mark 后才决定 zone。若环境无法证明原 tuple／mark 不冲突，安装拒绝。实现阶段可另评估专用 namespace，但不以未经验证的 zone 分离宣称隔离成立。
6. conntrack 只拥有内核 NAT 映射。Underlay 的有界 kernel handle／tuple 投影完全由 dataplane 接纳、续期、退役事件驱动，不能自行从读状态或回包建立业务流；回复查询同一个 FlowTable 的 `LocalFlow`／CID 后再走正式准入。

### 生存期、撤回与内核排队

只删 MASQUERADE 规则不足以终止已有 NAT：conntrack 保留映射；只删 daemon 流也不足以撤回已交给内核的正向工作。建议采用两层截止：

- dataplane 为已准入 flow 输出不透明的内核投影操作（flow incarnation、原 tuple、当前 deadline、scope），Underlay 只安装其镜像。nft 的有界 timed set 在 forward／回程交付门逐包检查，超时不得长于 flow idle、permit、instance／execution 截止的交集；新增／续期等待 ACK 后才 release 第一包。相对 timeout 从提交时剩余预算保守换算，排队／执行命令不重置原截止，ACK 后再次检查原 guard；时钟失效即关闭门。按批差量操作，不能每包启动 `nft`。
- 正常撤回／方案切换先使原 guard 失效并停止新准入，再原子删除内核可转发投影、取消排队输出、退役旧 flow／cipher，精确删除其 ct mark＋original tuple 对应 conntrack，确认后回收 handle。删除失败保持 drop 门与 journal，拒绝 tuple／incarnation 复用，不回滚到旧许可。不能全局 `conntrack -F`。
- 续租只对仍活且 Control 认可直接连续性的同授权／同路径 flow 更新；**方案切换总是结束旧出口连接**。超过旧截止后不能通过新的 set timeout 或保留 conntrack 复活它。
- 崩溃后非持久 TUN 自动消失，timed set 到期、负向 capture 保留；重启先核对 boot／namespace／installation，再清除所属旧投影与 conntrack，重新签发和握手。旧包不能因同 tuple 新建 flow 被接纳；必要的 tuple 隔离／tombstone 需覆盖延迟旧包窗口，容量耗尽拒绝新流。
- 必须明确最终边界：已在撤回前通过内核最终过滤并排入网卡的包不能被“追回”。nft 超时／drop ACK 证明后续穿越该门的包被拒绝，不等价于物理链路上此后零字节。T2 同时测延迟 daemon 输出、内核队列和门关闭后发起的包，报告准确截止；若要求逐网卡发射时刻零旧包，需要额外 qdisc／egress 门设计，不能由普通 nft forward hook 冒称满足。

## NAT 技术选择与宿主条件

以下版本与维护情况于 2026-10-08 核对上游说明和 crates.io；不是已引入的 workspace 依赖。

| 选择 | 成熟度与约束 | 建议 |
| --- | --- | --- |
| [`nftnl` 0.9.4](https://crates.io/crates/nftnl/0.9.4) | Mullvad 的 libnftnl 安全封装，有实际 VPN 消费者，MIT／Apache-2.0；上游仍说明低层 rough edges／覆盖不完整。底层 `nftnl-sys` C FFI，需锁定 libnftnl／libmnl 与 musl 交叉构建链；safe API 不等于依赖无 unsafe | 长期进程内候选。引入时审计薄 native 边界，业务 crate 继续 forbid unsafe；不能为了调用缺失功能在 Underlay 业务代码放开 lint |
| [`rustables` 0.9.0](https://crates.io/crates/rustables/0.9.0) | nftnl-rs 分支，Rust 构造 netlink，GPL-3.0-or-later；上游仍明确试错式编码和 rough edges。不是仅因名字含 Rust 就证明无 unsafe／FFI 风险，覆盖与审计须逐项做 | 首版不选；减少 C 链成本不足以替代 ACK、dump、set timeout、conntrack 兼容证明 |
| 系统 `nft` CLI | netfilter 官方用户态，`nft -j -f -` 批事务，`nft -j list ...` 回读；无需 Meshway 自编 FFI，但有系统包／进程开销 | **首版推荐**。镜像／安装包锁定支持版本，固定可执行路径、结构化输入、无 shell 拼接、有界输出／超时／并发／join。取消后结果不明先读回 reconcile，不能假定事务未提交 |
| 用户态 NAT／netstack | 易将每次 socket I/O 绑 guard，跨平台潜力；需要 TCP／UDP／ICMP 转发、PMTU 与另一套资源集成，当前无正式 netstack | 不符合首版 Linux 内核转发目标，不作为权限不足的自动降级 |

初版 conntrack 精确删除也可使用固定路径 `conntrack` 工具（conntrack-tools），要求 family、original tuple、归属 mark/mask 和读回身份校验；内核不支持所需过滤／可验证删除时明确不可用。它不是 nft CLI 自带功能。X3 负责证明 tuple 重用竞态和工具取消后回读；不能只检查工具退出码。

安装探测至少包含 `/dev/net/tun`、实际 NET_ADMIN 操作、nft batch／set timeout／ct 支持、route/rule、可验证 conntrack 管理和 `ip_forward`。`/proc/sys/net/ipv4/ip_forward` 为 0 返回 `ip_forward_disabled`；不可读／格式错返回不同原因 `forwarding_probe_failed`。不把 `uid=0` 或 cap bitmap 当真实成功。

还要检测实际 ingress／egress 的 `rp_filter`、`src_valid_mark` 与策略路由相容性：全局 `/0` capture 可能让 underlay 反向路径校验走错接口，**SO_MARK 本身不解决入站 rp_filter**。首版支持已证明的 loose/off 组合；strict 组合未经双向路由验证则报 `reverse_path_filter_conflict`。不照搬 Tailscale 自动写 `src_valid_mark`，也不自动关闭宿主防火墙。出口接口／地址变化撤回受影响 NAT 映射并重探测，不把旧 masquerade 连续性强行保留。

## 客户端捕获、LAN 与切换

现有 socket mark＋own-mark goto→capture 块末尾的机制可以复用：Server HTTP／SSE、STUN、获准 carrier、明确 resolver socket 均继续按用途受保护，普通应用到同 IP 不获豁免。实际 mark 从 journal 读取，不能硬编码 `0x40000000`。已有 `meshAddressPools` 是地址所有权边界，不能把公网 `/0` 塞进它。

新增由 Server 编译的 **egress capture plan**，区分：受管 IPv4 前缀／默认捕获、明确 direct/unmanaged 例外、必须拒绝的 IPv6 范围，以及计划 generation。其 route 只负责将包交 owner，并不授予整个前缀的 Access；第一包仍触发精确目的／协议／端口 demand。

- 有序规则不能直接变成同序路由：Linux 最长前缀与业务 first-match 不同。Policy 必须把相交 CIDR 分割为语义等价、互斥且有界的集合；不能在更宽高优先级 exit 内插入更具体低优先级 direct 绕行。域名／IP 混合有冲突时按现行契约拒绝，不猜测。
- 私有 egress table 装 `0.0.0.0/0→客户端 TUN`；受管 IPv6 装明确 unreachable／drop（Mesh IPv6 仍先按 Mesh 专用路由处理）。正向表缺失／TUN 关闭时有覆盖同范围的负向规则，不允许继续落到 main default。
- 源端 LAN 例外只来自管理员有效方案中的明确 direct/unmanaged CIDR，不能自动把 RFC1918、当前连接网段或网关整个网段全放行；**出口 LAN** 则仍为 exit 动作＋显式 CIDR Access＋masquerade。两者不能混用一个 allow-LAN 开关。必要 DHCP／邻居发现等本机链路控制例外按最小协议范围建模，不授予 LAN 应用通路。
- 新的 `/0` 检查不能复用“所有相交外部 local 地址都冲突”的 Mesh 池检查；local-table 本机服务、loopback 和地址控制必须单独建模，外部路由优先级仍需核验。捕获不承诺阻止设备所有者访问本机进程；不能利用本机地址／broadcast 路由绕去远端 LAN。
- 切换顺序：先安装旧新范围并集的负向边界，暂停旧 flow／DNS 和投影，准备新表／TUN／resolver／权限，再切换活动 selector，读回确认全部必需组件后上报 applied。RPDB／DNS／nft 没有跨子系统事务，允许短暂失败，不允许短暂 direct。只能在显式新 direct/unmanaged 已生效或 down 时精确解除相应受管边界。
- 身份撤销、出口不可用、lease 过期不是显式 unmanaged。负向 `/0` 在期望 up 的 suspend／退出／同 namespace 重启期间保留，恢复控制联系仍走受保护 socket；旧实例／旧 DNS 回答／旧队列不换绑。

## DNS 路径与可信域名上下文

### 经 Mesh 的出口 resolver

采用与 peerapi DoH 相同的信任形态：客户端 DNS owner 通过**完整获准 Mesh 路径**访问出口 DNS owner，Noise 在源／出口间保护；不额外建设公网 DNS HTTP 端口。建议首版使用标准 UDP DNS＋独立 TCP fallback，监听出口 Mesh 地址的保留端点；端点地址／端口由职责计划发布，尚无固定公共 API。

仅监听 Mesh IP 不足以认证：同 namespace 进程可直连 socket。扩展现有 loopback Service 时，新增密封的 Mesh ingress 适配：输入由唯一 dataplane 在专用 DNS permit 下交付，携带 origin、实际完整 path、scope、流关联；回包也交同一 SecureNode。若选内核 socket 接入，则必须证明 socket 输入只可能来自相同准入入口，并保留不可伪造关联；不能用 `remoteAddr` 反查 Node 替代密码身份。daemon 只驱动适配，DNS owner 不另持 SecureNode／流表。

出口 DNS permit 是目的受限的内部用途：只能调用指定出口 resolver，不因授予 DNS 服务而开放访问该出口所有端口或转发任意 IP。绑定 source、exit、path、profile generation、查询范围／预算及 deadline；允许域名型 LAN 访问的主体也可有相应查询资格，不能复制 Tailscale 的 Internet-only 探针。

出口 DNS owner 调用**出口机器的系统 resolver**。优先实现可验证的 resolved 适配（保留系统按链路／后缀选上游，排除本机 Mesh stub 递归）；文件后端使用 owner 记录的真实系统上游。不能把 `/run/systemd/resolve/resolv.conf` 中的平面列表当成完整 split DNS 语义。首版仅支持已证明无环的真实上游／resolved 路线；未知本地 stub、回指 Meshway、自依赖出口 resolver 返回不可用，不选公共 DNS 补救。查询上游 I/O 有专门职责许可和 socket 保护，不借 bootstrap API；UDP→TCP 重试需要独立传输资格。

### 哪些查询走出口

优先级沿用现有定义：MagicDNS 本地精确记录／发现 → 保留 `useWithExit=true` 的最长后缀 split／default → 普通查询去 DNS plan 指定出口。例外 resolver 的 IP 路径独立获准，不能因保留 DNS 就直连 LAN。

- 首版建议支持 **单一可预先确定出口的 profile**，包括固定出口 default 和指向同一出口的 IP CIDR 规则；只要该 profile 使用 exit，普通查询在解析前就选该出口（明确例外除外）。这样不会先向源端 DNS 泄漏问题，再根据答案决定出口。
- 单纯 IP CIDR 不足以决定一个未知名称应走哪个出口，不能用“只有结果落在 exit 网段的查询才去 exit DNS”作为算法。多出口、依赖答案才能选出口、domain/IP 相互依赖的方案，须先有显式无环 DNS plan；首版不能确定时保留 `exitPlanRequired` 拒绝安装。
- IPv4 首版的出口解析通道可解析 A；AAAA 的存在不能变成 IPv6 路由豁免。建议保留真实 DNS 回答语义，不伪造 NXDOMAIN；任何受管 IPv6 包由捕获和 permit 双重拒绝。只屏蔽 AAAA 不是防泄漏措施。

### 可信关联不能只靠“走过出口 DNS”

Noise 证明回答来自出口，不证明出口上报的域名／地址自然有权扩大另一正常出口的准入。现行 [Access 域名合同](/v2/configuration-reference/access#domains)要求 **Server 受控解析／受信结果校验后发布**；普通 State 或源端自报 hostname 都不能创建 `TrustedDnsAssociation`。

完整域名实现建议增加独立受控解析任务：Server 为 origin／name／DNS plan／出口 resolver／revision／nonce／deadline 签发任务；指定出口 DNS owner 返回绑定的名称、完整受限 CNAME 链、A／AAAA、原 TTL 和解析来源。Server 检查任务只能完成一次、当前成员／实例／职责、配置和 scope、地址分类与 TTL 上界，并明确把管理员指定 resolver 的受控输出作为信任输入，再发布 source-scoped association 和精确业务 permit。任意未经委派的节点报告一律拒绝。TTL 取 CNAME／RR 最短寿命与任务／执行期限交集；截断、循环、不一致响应和过期 completion 不发布。

客户端正常 DNS 分流还需有界 association→IP 上下文，多个名称同 IP 的不相容动作拒绝／不可执行；不能把最近一次 DNS 查询当全机任意包的可信业务名。跨 source、profile、publication 或切换出口不复用缓存。即使如此域名授权仍是地址集合，不保证同 IP 的应用身份隔离。

**首版交付范围建议：**IPv4 IP／CIDR／Internet 许可＋单出口普通 DNS＋MagicDNS／明确 useWithExit 例外；域名型 Access／egress 必须等可信 producer 与上下文冲突处理一并完成才解锁，不能因 resolver 已通提前宣称支持。X6 可在该 IP 版之后交付，不阻碍 `exit/off`，但完整用户使用的单出口 DNS 属于首版 X5。

## 协议与模型建议

下列名称是草案字段，不是当前可提交 JSON；最终类型由所属 crate 定义并生成 OpenAPI／SDK。

| 面 | 建议变化／约束 |
| --- | --- |
| 配置 | 保留 `exitEnabled` 和既有 egress 规则。首版无 NAT-off／端口映射开关。显式多出口 DNS plan 另行建模，不能滥用 State 填配置 |
| 静态能力与 State | acquire capability 声明可支持的 exit 地址族／后端版本；State `exitCapability` 用 `notRequested / probing / ready / unavailable` 类型化状态并包含 supportedFamilies、实际 installedFamilies、instance／target 绑定和观测时点；`exitUnavailableReason` 在 unavailable 必填，其他状态不适用，未知／过期不补 ready |
| 失败原因 | 至少 netAdminUnavailable、ipForwardDisabled、forwardingProbeFailed、nftUnavailable、conntrackUnavailable、reversePathFilterConflict、firewallConflict、routeConflict、dnsUnavailable、unsupportedFamily、ownershipConflict。日志展示可映射为 `exit_unavailable: ip_forward_disabled`，HTTP 采用同一类型的规范 camelCase |
| 出口职责 | 新类型化 role plan，绑定 network／exit／instance／publication、IPv4 masquerade、允许的 capture／return 资源语义与 deadline；安装前即允许报告失败。不能先要求 ready 才发职责、又先要求职责才探测 ready 造成循环 |
| 业务 permit | 复用 `PathPermit` 的 demand、sourceAddresses、精确 target／service、profile、完整 paths、expiresAt、原路径回复。增加明确 exit 执行约束（地址族、NAT 模式、profile selection generation／角色绑定）；不重复复制一个更宽“出口 CIDR 许可”来替代精确业务许可 |
| 源端捕获 | `NodeConfiguration` 增加独立 egress capture plan 和不受管例外证明；source address family 缺失时仍保留拒绝边界。delta／digest／obsolete 解码同步更新 |
| DNS | `DnsEgress` 增加可执行单出口计划；内部 DNS service permit、系统 resolver 职责、查询 scope 与来源；可信解析任务／完成契约由 X6 再扩展。保留 noExit 的明确证明 |
| 安装／State 顺序 | 扩展 ApplyFailure 以传递实际 exit 原因；Control 接纳当前角色的失败事实，无需伪造 applied。Server 能据新鲜能力事实将需求判为不可用，但不能由 ready 自行增加授权；收到撤销或旧 lease 到期由本地即时失效 |
| 版本／生成链 | 编译产物语义与捕获／DNS 权限变化，建议从 `meshway-demand-5` 升至 **`meshway-demand-6`**（合入时由协调者分配，避免并行占号）；旧节点不能把不认识字段忽略后继续 applied。重生成 OpenAPI、Rust／TS 客户端、运行时 schema、文档契约，运行 drift 与消费者验证 |

纯 Exit IP 包仍可复用 wire v5 的加密载荷，但新增 permit 摘要必须包含全部新授权字段。若引入独立 DNS circuit purpose／新的帧字段，需另行升级 wire 并全路径协调升级，不能仅改 compilerVersion 冒充 wire 兼容。若 IPC 公共状态增加封闭字段／错误枚举，同步升级 IPC 和 CLI；现有 HTTP 根不改。

## 安全边界与限制清单

- **开放代理：**出口 role、单源 permit、具体目的／服务、完整实际 path 和活执行上下文缺一不可；本机 DNS／NAT 无公网 listener，未请求方向不能建立新流。
- **源伪造：**已认证 origin 绑定其精确 Mesh 地址，lax 不成为 origin；不接受 LAN 源、另一 Node 源、IPv4-mapped IPv6 绕分类或源端声称已校验的 metadata。
- **公网集合：**唯一 Policy 分类排除 RFC1918、ULA、link-local、组播、CGNAT `100.64.0.0/10`、loopback、unspecified、保留／广播等特殊范围；测试覆盖边界地址，不能用 `!is_private()` 近似。出口 LAN 必须明确 CIDR Access，不能因域名解析成私网就借 Internet grant 放行。
- **hairpin：**首版禁止出口本机／所属虚拟设备／Mesh 池作为外部转发目标，避免 local INPUT、递归 TUN 或 NAT 回卷。源节点自己作为出口的单节点路径当前不合法，明确拒绝；所需本机 direct 由配置表达。
- **状态回复：**conntrack 不证明 business flow；TCP／UDP／echo 仍经过唯一 FlowTable。相同 tuple 换 demand、path、profile／instance 或迟到旧回复不能接管新流。
- **撤回：**路径节点、成员、Access、出口开关、方案或执行 lease 变化均检查；旧内核投影、DNS ticket、密文／明文队列一起失效。禁用出口后宿主原本开启的 ip_forward 不改回，但 Meshway 转发资格与 NAT 全部撤回。
- **IPv6：**首版拒绝 exit IPv6 permit，客户端受管 IPv6 负向捕获，出口专用 TUN 的 IPv6 转发 drop；外层 IPv6 carrier 与内层 IPv6 出口能力分别表达。已授权 Mesh IPv6 可继续使用自己的路径。
- **MTU／协议：**首版继承现有 IPv4 无 options／分片、TCP／UDP／ICMP echo 范围；明确限制超 MTU UDP。TCP 需验证双向 MSS／PMTU，ICMP errors 不能直接当业务 reply。若实现 RELATED 错误，须校验内嵌原 tuple／流并另做 parser／state 测试，不开放通用 RELATED helpers。禁止 NFT flowtable／硬件 offload 绕过逐包截止门。
- **资源与宿主：**每节点流上限、nft set／ct 投影、query 数、命令队列和 buffers 有界。未知表／相同名字的外部资源／mark 冲突停装；mark 归属检查须覆盖外部 nft／connmark 写入，不能只检查当前 RPDB selector。规则丢失监测、部分事务、NET_ADMIN 丢失均停止正向 I/O，保留可核验恢复证据。

## 工作包与依赖

以下是待实现任务；“T2”指真实 daemon＋Linux 系统资源整组验收层。`exit/off` 等是**建议新增行**，当前 `validate_matrix.py` 尚不接受这些行名。所有包复用正式 owner，不为测试写第二个签发者或第二套 NAT／准入。

| 包 | 文件／crate 独占所有权 | 依赖与可并行范围 | 必需验证 |
| --- | --- | --- | --- |
| X1 契约／联合编译 | `meshway-model` State、`meshway-configuration` execution/delta/digest、`meshway-policy` compile/admission；Server 执行编译与状态接纳；`meshway-protocol` 类型。manifest／lock／生成链和版本统一由协调者管理 | 首先冻结 capture／role／permit／failure 接口，之后供 X2–X5 并行消费；同包内 Server 与模型改动统一整合 | first-match CIDR 分割、联合许可、地址族拒绝、Internet 边界、撤销不受旧引用阻挡；OpenAPI/drift、Rust/TS 消费者 |
| X2 dataplane 出口生命期 | `crates/meshway-dataplane/src/`（lib、flow、secure、carrier 等）及所属测试 | 依赖 X1；与 X3/X4/X5 并行，先约定 opaque kernel projection／reply ingress 接口 | 真 SecureNode 进程内 A→R→E，伪源／错路径／错 CID／错 tuple、纯接收续租、到期／切换、旧输出、projection failure 和容量 |
| X3 Underlay NAT owner | `crates/meshway-underlay/src/linux/kernel/` 新 exit 模块，journal／monitor／plan／capture；`linux/protection.rs` 和 Host 出口 API | 依赖 X1/X2 接口；与 X2/X4/X5 并行。X4 只消费 Host，不修改这些文件；新依赖交协调者 | 隔离 namespace：nft 原子批、route／mark、masquerade／逆 NAT、精确 ct 删除、严格 rp_filter 拒绝、crash/journal／外部冲突、无 NET_ADMIN／forwarding=0、timed set 到期 |
| X4 Control／daemon 集成 | `crates/meshway-control/`、必要 `meshway-authority/` 检查；`apps/meshway-daemon/src/runtime/network/{mod,worker,requirements,facts,reporting}.rs` 与包输出队列；IPC／CLI 状态接线由本包通过协调者整合 | 依赖 X1，等待 X2/X3 可执行接口后联调；与 X5 文件分界固定，DNS 子目录归 X5 | 实际职责自动安装、完整成功才 applied、失败可上报、HTTP 挂起时撤回、方案切换原子失败边界、无第二次 up 的恢复 |
| X5 单出口 DNS | `crates/meshway-dns/src/`；`meshway-underlay/src/linux/resolver_exchange.rs`／`linux/dns.rs` 的用途隔离适配；daemon `runtime/network/dns/` | 依赖 X1 的 DNS permit 和 X2 Mesh ingress；与 X2/X3/X4 并行，Host/kernel 改动只提接口给 X3 | 真实系统 resolver／MagicDNS／useWithExit、UDP→TCP、出口断开无本地 fallback、伪 socket 输入、过期回答、重启和 resolver 环；T2 `exit/resolved`，文件后端单独 `exit/resolv-conf` |
| X6 可信域名任务（后续） | Server 新受控解析任务模块、DNS owner 新任务 producer 模块、Policy 域名关联判定；共享类型／生成链经协调者；不并发修改 X1/X5 所属文件 | X1/X5 完成后；不阻碍 IPv4 IP 版，但域名功能必须等本包 | 授权任务一次完成、错 source／revision／nonce、TTL/CNAME、DNS rebinding、私网答案、同 IP 冲突、多出口无环 plan；新增独立域名 T2 场景 |
| X7 系统验收与 lab | `tools/meshway-runtime-fixture/`、`tools/meshway-packet-fixture/`、`scripts/verify_runtime.py`、`scripts/validate_matrix.py`、必要 `dev/` 锁定工具镜像和部署脚本；`docs/v2/node/validation.md` 证据更新 | X1 接口冻结后可准备 fixture；整组等 X2–X5；不修改产品 crate，失败回归交对应 owner | `exit/off` 为 IP 数据面门槛；DNS 行独立；nyc／sha→lax 真 lab，核对 PID/hash、完整路径、撤回与清理 |

X1 的 manifest／lock／生成物不能由其他工作包同时改写。Console 的 exit 能力／原因展示由用户转达给 Console 工作流，消费生成字段；X0 不联系或修改 Console 会话。

### T2 `exit/off` 建议合同

拓扑：隔离 Controller、A、R、E 和外部 IPv4 服务／E 的私有 LAN 服务；A→R→E，无 A→E 业务捷径。公网分类用隔离 fixture 明确分配的公网测试地址和不可通外网的路由环境，不把 RFC1918 echo 当 Internet 成功。每次 run 记录源码／产物 hash、namespace、接口、RPDB、nft 计数、ct tuple 与逐端 pcap。

1. 管理员打开 E.exitEnabled、允许源的 Internet 和精确 LAN 服务、配置 profile／via；真实 daemon 自动准备，saved/applied、实际资源与包流分别记录。
2. IPv4 TCP 多 MSS／UDP 小包与 MTU 边界／ICMP echo；公网与 LAN 同一 NAT，远端看到 E 出接口源；R 无明文，回包 E→R→A；125 秒持续与仅接收流跨旧需求截止。
3. 不获准源／端口／LAN、公网授权下 RFC1918／CGNAT、伪造内层源、重复 path／跳过 R、伪 reply、新反向 TCP／UDP、hairpin、IPv6 都拒绝。对端只看到许可流，drop 计数不可代替无包证据。
4. 逐项撤销 Access、exitEnabled、成员、路径、profile；旧 TCP／UDP／echo 停止，新流拒绝，nft 投影和 ct 精确消失；恢复后旧 cipher／tuple 输出不复活，新连接可通。
5. 控制 HTTP 中断跨 execution／instance 真到期，源保持负向捕获、出口停止转发；恢复不需第二次 up。分别阻塞用户态输出／命令 ACK，测取消与截止。
6. 容器内 forwarding=0、移除 NET_ADMIN、nft 不可用／宿主 drop、strict rp_filter 冲突、规则外改、nft 提交失败、daemon SIGKILL／同 namespace 重启、跨 namespace 旧 journal；如实报告原因，不能 applied 或 direct。改变 sysctl 仅限 fixture 自己的 namespace。
7. 默认 `/0` 下 Server/SSE/STUN/peer 保持真实通路；普通应用到相同 Server／peer IP 不享 bypass。LAN explicit direct 例外、first-match 重叠和 IPv6 sink 均检查。
8. 离线 down／撤回清理仅所属表／链／ct／路由，外部哨兵资源逐项不变。报告成功还须 fixture／daemon 正常结束和 supervisor 无遗留复核。

### 常驻 lab 验收步骤

1. 先用现有 `node:lab versions/status` 和真实 Server 接口核对 nyc／sha／lax 的身份、PID、hash、namespace、配置 revision；地区名只是节点标识。记录现有混合承载／DNS 配置，不运行会覆盖它的通用 configure。
2. X7 更新锁定 Ubuntu 工具镜像纳入所选 `nft`／conntrack 工具；image-build、Linux build、deploy 分开，部署后再次核对实际 `/proc/<daemon-pid>/exe` hash。不进入宿主虚拟机目录、不改 Docker daemon／宿主 sysctl。
3. 为实验保留私有原配置和 CAS revision。由管理员 Web 开 nyc.exitEnabled，给 sha 精确所需 Internet 服务授权，分配 profile，要求 sha→lax→nyc；出口 LAN 另选 nyc 可达的受控测试服务并显式授权 CIDR／服务。nyc 的 forwarding 前置条件由 lab 声明配置提供，Agent 不执行手工 sysctl。
4. 从 sha 用数字 IPv4 先验证 TCP／UDP／echo，再默认 IPv4 出口；在 nyc 出接口抓 NAT 前后 tuple、nft 计数、ct 记录和远端访问日志，在 lax 抓加密承载和零业务明文。三个容器可能共享相同公网地址，**仅 curl IP-check 相等不能证明经 nyc**；应结合 nyc 唯一实际发包与阻断 nyc 后业务失败证明。
5. 按 X5 验普通系统 DNS 确实由 nyc resolver 发出、MagicDNS 保持、useWithExit split 独立获准；断 nyc resolver 不向 sha 上游 fallback。再验 125 秒持续／纯接收、Web 关出口、撤权限、方案切换、实际 lease 到期、旧队列拒绝与重新开启恢复。
6. sha 直达 nyc 物理链路即使可用也不能绕 lax；阻断 lax 必经边后访问失败而非自动直达。测试 IPv6 字面量和 AAAA 结果不会经 sha 本地泄漏。读取 nyc DNS／路由／转发能力失败原因，不以路径检测计划代替真实执行。
7. 按保存的 revision／当前变化核对后 CAS 恢复实验配置，退出实验职责并验证资源撤回；保留身份和外部配置。报告原始证据独立私存，只有实际通过的 T2 行更新 validation 状态，lab 局部成功不替代矩阵。

## 未决产品问题

当前 **没有阻塞 IPv4 IP/CIDR 首版的用户决策**；已有原则足够推进 X1–X5。需实现阶段证明的 conntrack 删除竞态、nft/路由联动、rp_filter 与 resolver 无环性是工程门槛，不转交用户重新决定安全原则。

多出口的显式 DNS plan 配置交互、域名型授权解锁时点随 X6 单独提交产品方案；在其定版前沿用“缺少可信依赖不可执行”的既定边界，不提前发明可用字段。
