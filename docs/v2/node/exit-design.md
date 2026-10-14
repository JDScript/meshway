# Exit／NAT 设计

> 首版（X1–X7：Linux IPv4 出口、单出口 DNS、可信域名关联、真实出口宿主前置条件）已实现。本页记录设计依据、owner 边界与限制；运行行为定义见[IPv4 出口角色与 NAT 投影](/v2/node/runtime#ipv4-出口角色与-nat-投影)与[数据面](/v2/node/dataplane)，验收状态以[验收范围](/v2/node/validation#exit-scenario)为唯一定义。

## 目标与建议

首版实现 Linux **IPv4 内核转发＋状态型 masquerade**，公网与显式授权的出口 LAN 使用同一准入、流生命周期和撤回机制。IPv6 在模型中明确表达，但首版出口执行拒绝；不能因没有 IPv6 NAT 而让受管 IPv6 从源端本地出网。

权限只有一个来源：Server 签名的 netmap。Control 验签并缓存它，Engine 由它计算本节点唯一的 `Desired`，Policy 在本节点投影上对首包给出 `Decision`（目标、服务、完整路径与出口 selection）；netmap 的 `exitRole` 只是资源职责，不是转发许可。Underlay 的唯一 Kernel owner 管理独立出口 TUN、nftables 私有表、路由、归属 journal 和精确 conntrack 清理；nft 变更经固定路径的 `nft` CLI 以类型化生成的批事务提交，conntrack 经内核 ctnetlink 直接读取与删除。dataplane 保持唯一业务流表、Noise、完整有序路径和最终输出检查。DNS 由现有 DNS owner 扩展为经 Mesh 的受许可 resolver 服务。

出口开关仍只有管理员 Web 配置 `nodes[id].exitEnabled`。节点收到职责后自动准备／安装／撤回所属转发资源，没有手工开出口的 CLI。宿主 `ip_forward` 是**检测前置条件**：Meshway 不修改宿主 sysctl；值不为 1 时 `exitCapability` 为 `unavailable`／`ipForwardDisabled`，本节点其余内容照常 applied，周期探测观测到 ready 后以同一内容安装角色（[出口宿主前置探测](/v2/node/runtime#出口宿主前置探测)）；宿主管理员如何预配置见 [Linux 安装 · 出口宿主](/v2/install#出口宿主)。自动安装 NAT／路由与自动修改全局 forwarding 是不同权限范围。

配置的唯一语义来源仍为 [routing.egress 与出口联合许可](/v2/configuration-reference/routing#出口联合许可)、[Access](/v2/configuration-reference/access#domains)、[出口 DNS](/v2/configuration-reference/dns#使用出口时)。下面定义实现建议与缺口，不另定义出口选择规则。

## 当前实现落点

| 位置 | 职责 |
| --- | --- |
| `crates/meshway-policy/src/compile/plan.rs`：`plan`、`external_route`、`route_of` | 按方案和 first-match 评估 exit、exitEnabled 和路径 |
| `crates/meshway-policy/src/compile/routing.rs`：`Routes::segment`、`through` | 出口前 Mesh 路段递归求解，保留原业务主体，拒绝重复节点和循环 |
| `crates/meshway-policy/src/compile/netmap/decide.rs`：`LocalCompiler::decide_with`、`Decision`、`egress_rule_for_name` | 节点在自己投影上对首包的唯一判定（目标、服务、完整路径、出口 selection） |
| `crates/meshway-policy/src/admission/netmap.rs`：`admit_handshake`、`admit_forward`、`admit_reply`、`admit_related`、`admit_transit`、`admit_dns` | 端点准入与中继判定；公网边界唯一实现为 `meshway_policy::is_internet_address` |
| `crates/meshway-engine/src/lib.rs`：`Desired` | 节点唯一授权状态：已验签内容、捕获边界、可信关联与保留地址 |
| `crates/meshway-configuration/src/netmap/mod.rs`：`ExitRole`、`DnsService`；`execution/exit.rs`：`EgressCapturePlan`、`ExitNat` | 出口职责与捕获计划；`ExitRole` 只是资源职责，不是转发许可 |
| `crates/meshway-dataplane/src/secure/exit.rs`：`exit_reply`、`fail_exit_projection`、`retire_exits`、`related_counters` | 出口流在唯一 SecureNode 流表中的投影、回复与退役 |
| `crates/meshway-underlay/src/linux/kernel/exit/` | 出口 TUN、nft 程序、ctnetlink conntrack、quarantine、journal、能力探测与宿主规则判定（`firewall/`：`judge.rs` 遍历判定，`nft.rs`／`iptables.rs` 解析） |
| `crates/meshway-underlay/src/linux/kernel/{mod,plan,capture/mod,journal,monitor}.rs`、`kernel/egress/` | 唯一 Kernel、私有 RPDB/table、正向 capture／负向 sink、源端 egress 门、资源监控；`plan::pools` 明确拒绝 `/0` |
| `crates/meshway-underlay/src/linux/protection.rs`：`Mark::transform`、`Shared::prepare` | SO_MARK 读改写／读回并保持外部 selector 语义；`kernel::allocate` 随机起点选择空闲单 bit，**不是固定 `0x40000000`** |
| `crates/meshway-underlay/src/linux/dns.rs`、`resolver_exchange.rs` | 前者仅 bootstrap resolver；后者消费密封 local resolver route，不能将 bootstrap 权限借给出口 DNS |
| `crates/meshway-dns/src/{selection,route,exit_route,association,owner/mod,dispatcher/mod,service/mod}.rs` | MagicDNS、useWithExit 选择、精确上游许可、出口 resolver 路由、可信关联、query ticket 与输出取消 |
| `crates/meshway-dns/src/system/linux/{mod,resolved,file/mod,storage}.rs` | namespace 预约、真实上游来源、系统 DNS 安装与恢复；不得绕过现有归属机制 |
| `apps/meshway-daemon/src/runtime/network/{worker,plane,reporting}.rs`、`exit/` | 装配：出口 driver、能力探测与上报；不持第二份流表或 nft owner |

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
  → 唯一 SecureNode：Policy `Decision`（源地址、目的／服务、完整路径）、唯一流、Noise
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
| Server | 配置投影为每节点 netmap 并按租约 ed25519 签名；可信 DNS 关联 overlay 签发 | 管理员开关、Access、身份；State 不是权限来源 |
| Policy | 节点投影上的首包 `Decision` 与 `admit_*` 准入 | first-match、source／target／profile／完整路径；Server 与节点共用同一实现 |
| Control | netmap／overlay 验签、缓存、租约、sync 与回执 | 只发布已验签内容；租约到期不靠 HTTP 返回即 fail closed |
| Engine | 由 netmap 与关联计算唯一 `Desired`，向各组件 `apply` | 本节点没有第二份授权状态 |
| dataplane | 唯一有界 FlowTable、协议状态、CID、原路径、guard | 原始已认证 Mesh 源与精确内层源、实际目的、服务、路径；回复必须已有正向流 |
| Underlay Kernel | 出口 TUN、nft、RPDB/table、NAT 资源投影、journal、能力探测 | 确认所属资源／标记／接口；提交和每次 I/O 的有效性；内核状态不生成业务资格 |
| DNS owner | 查询、响应、受控系统 resolver 适配、pending／TTL／取消 | 专用 DNS permit、查询名／来源／实际完整路径和输出截止；不是任意代理 |
| daemon | 装配、驱动有界队列、完成真实安装／上报 | 不持第二份流表、DNS 授权数据库或 nft owner |

选择**独立出口 TUN**而不复用普通本机 Mesh TUN：`Action::Deliver` 给本机服务与 `Action::Exit` 给内核转发有不同的入口和规则；共享设备会使“从 TUN 来”不足以区分两种职责。两设备仍归同一个 Host／Kernel，不增加 Host 实例。专用 TUN 不暴露 FD，不接受外来接口接管，不绑定出口公网地址；回程按远端 Mesh 源 `/32` 导向它。

### 内核规则与回程识别

1. 使用 `inet meshway_exit_<installation>` 过滤表与 `ip meshway_exit_nat_<installation>` NAT 表，名称／handle／规范规则摘要写 journal。不 flush ruleset，不接管宿主 FORWARD policy。私有 hook 对不属于本 owner 的流量 return；对本 owner 流量缺资格一律 drop。独立表 accept 不能覆盖其他表的 drop，宿主防火墙阻断应如实报不可用而非修改它；只有能匹配出口流量的宿主规则算阻断（限于其他接口的规则，如 Docker 网桥隔离，不算），判定见[出口宿主前置探测](/v2/node/runtime#出口宿主前置探测)。
2. 只有专用 TUN ingress、准入流的源节点精确 Mesh IPv4、获准外部目的／协议／端口及仍有效的 flow 投影能进入 FORWARD。公网/LAN 的区别由 Policy 许可表达，NAT 实现不分两条业务通路。禁止转入任一 Mesh 池、loopback、链路本地、组播、广播、出口本机地址和所属虚拟接口；出口本机服务须用 Mesh 服务许可。
3. 专用 TUN 的转发流在内核 conntrack 上标记可证明归属的 ct mark；该 mark 与 socket bypass mark 分开分配、保留外部 bit，不从网络包信任 mark。在 PREROUTING 路由前为已识别回复恢复专用回程路由标志；reply RPDB 只把逆 NAT 后的源节点地址导向出口 TUN。forward 方向仅凭专用 iif＋受管资格跳过客户端 capture 块、继续原宿主 RPDB，避免出口自己启用默认捕获时再次吞包。
4. `postrouting` 仅对所属 forward ct／资格集合做 IPv4 masquerade；公网与 LAN 均采用实际出接口地址。回包须 ct reply direction、关联原 tuple 和活 flow 投影；未关联新入站不允许进入 Mesh。DNS 和普通 Mesh TUN 流量不能被这条 NAT 规则接管。
5. 首版使用宿主默认 conntrack zone＋精确归属 mark／原 tuple／ct identity，不笼统设置一个“出口 zone”：回程包必须在 conntrack lookup 前知道 zone，不能在查到 ct mark 后才决定 zone。若环境无法证明原 tuple／mark 不冲突，安装拒绝。不以未经验证的 zone 分离宣称隔离成立。
6. conntrack 只拥有内核 NAT 映射。Underlay 的有界 kernel handle／tuple 投影完全由 dataplane 接纳、续期、退役事件驱动，不能自行从读状态或回包建立业务流；回复查询同一个 FlowTable 的 `LocalFlow`／CID 后再走正式准入。
7. 首包写入出口 TUN 后，owner 在至多 3 秒内等待其 owned conntrack 条目出现，以绑定精确清理用的 ct 身份；没有出现（首包在出口因 TTL 或 MTU 被丢弃、未经 FORWARD）则该流 `ConfirmFailed` 并退役。等待不占用任何队列：daemon 的出口 driver 与 Underlay owner 都把未决确认放在一旁按 10 ms 轮询，其他流的投影、续期、退役与包照常执行。

#### ICMP 错误 {#icmp-errors}

已投影流的 ICMP 错误是 conntrack RELATED 报文，在内核侧按原路回到出口 TUN，再由数据面按[被引用的流](/v2/node/dataplane#icmp-errors)准入；没有第二份白名单。IPv4 首版，受管 IPv6 仍 sink。

| 来源 | 内核路径 |
| --- | --- |
| 出口之外的路由器（time exceeded、destination unreachable、fragmentation needed） | masquerade 的 conntrack 把错误与其引用的包一起逆转换；`route` 链对 owned、reply 方向、state related、类型 3／11 且原 tuple 在 `flows`（echo 为被引用 request 的 sequence 在 `echo_flows`，按无选项的 20 字节被引用头读取）且不在 quarantine 的错误打出口 mark；一条 `fwmark ＋ ipproto icmp` 的 RPDB 规则在全部成对规则之后查回程表（只含已投影源的 `/32`，其他目的落空继续），`forward`／`final` 对同样条件放行至出口 TUN |
| 出口主机自己（TTL 在本机耗尽、本机出接口 MTU 不足） | 内核把错误挂到被丢弃 forward 包的 conntrack（RELATED、reply 方向）；该 ct 已 owned，或尚未确认（包死于 FORWARD 之前，未获 owned mark）。`local`（`type route hook output`）链对这两类且原 tuple 在 `flows`／`echo_flows`、不在 quarantine、未 closing 的错误打出口 mark 触发重路由到出口 TUN，`final` 同条件放行；源地址为内核按到源的路由所选（通常是出口的 Mesh 地址）。其他本机报文不受影响 |

出口 TUN 读取方（`Live::reply`）只按框架接收：发往该投影源、类型 3／11、被引用头为该投影的 forward 包；源地址不限，由数据面判定。未引用任何 conntrack 的伪造错误在内核即为 invalid，不会被路由进出口 TUN。

### 生存期、撤回与内核排队

只删 MASQUERADE 规则不足以终止已有 NAT：conntrack 保留映射；只删 daemon 流也不足以撤回已交给内核的正向工作。因此采用两层截止：

- dataplane 为已准入 flow 输出不透明的内核投影操作（flow incarnation、原 tuple、当前 deadline、scope），Underlay 只安装其镜像。nft 的有界 timed set 在 forward／回程交付门逐包检查，超时不得长于 flow idle 与 netmap 租约截止的较早者；新增／续期等待 ACK 后才 release 第一包。相对 timeout 从提交时剩余预算保守换算，排队／执行命令不重置原截止，ACK 后再次检查原 guard；时钟失效即关闭门。按批差量操作，不能每包启动 `nft`：daemon 的投影 driver 把紧随第一条到达的新增投影（相邻间隔不超过 3 ms，一轮至多 10 ms、64 条，例如一次 traceroute 的各探测）交给 Underlay 作为一轮（单独的新流因此至多多等 3 ms），表归属 dump 校验、日志落盘、nft 事务与回读 dump 每轮各一次；每条投影仍各自做归属与冲突检查（journal、owned conntrack、特殊地址）、各自在内核变更前写入 journal 记录、各自在回读中核对自己的元素后才 ACK，结果逐条回执，一条失败不改变其他条的结果（整轮事务失败时逐条重试）。续期、退役与 conntrack 确认不合轮。
- 正常撤回／方案切换先使原 guard 失效并停止新准入，再原子删除内核可转发投影、取消排队输出、退役旧 flow／cipher，精确删除其 ct mark＋original tuple 对应 conntrack，确认后投影记录离开历史，转换后的 reply tuple 进入有寿命的 quarantine（拒绝迟到回包；生命周期见 [IPv4 出口角色与 NAT 投影](/v2/node/runtime#ipv4-出口角色与-nat-投影)），同一原 tuple 此后可由新 incarnation 重新安装。删除失败保持 drop 门与 journal，不回收该 tuple，不回滚到旧许可。不能全局 flush conntrack。
- 续期只对仍活、在新租约下判定不变的同路径 flow 更新；**方案切换总是结束旧出口连接**。超过旧截止后不能通过新的 set timeout 或保留 conntrack 复活它。
- 崩溃后非持久 TUN 自动消失，timed set 到期、负向 capture 保留；重启先核对 boot／namespace／installation，再清除所属旧投影与 conntrack，以缓存的 netmap 重新握手。旧包不能因同 tuple 新建 flow 被接纳：journal 中的 quarantine 随恢复重新装入，覆盖延迟旧包窗口，容量耗尽拒绝新流。
- 必须明确最终边界：已在撤回前通过内核最终过滤并排入网卡的包不能被“追回”。nft 超时／drop ACK 证明后续穿越该门的包被拒绝，不等价于物理链路上此后零字节。若要求逐网卡发射时刻零旧包，需要额外 qdisc／egress 门设计，不能由普通 nft forward hook 冒称满足。

## NAT 技术选择与宿主条件

以下版本与维护情况于 2026-10-08 核对上游说明和 crates.io；不是已引入的 workspace 依赖。

| 选择 | 成熟度与约束 | 建议 |
| --- | --- | --- |
| [`nftnl` 0.9.4](https://crates.io/crates/nftnl/0.9.4) | Mullvad 的 libnftnl 安全封装，有实际 VPN 消费者，MIT／Apache-2.0；上游仍说明低层 rough edges／覆盖不完整。底层 `nftnl-sys` C FFI，需锁定 libnftnl／libmnl 与 musl 交叉构建链；safe API 不等于依赖无 unsafe | 长期进程内候选。引入时审计薄 native 边界，业务 crate 继续 forbid unsafe；不能为了调用缺失功能在 Underlay 业务代码放开 lint |
| [`rustables` 0.9.0](https://crates.io/crates/rustables/0.9.0) | nftnl-rs 分支，Rust 构造 netlink，GPL-3.0-or-later；上游仍明确试错式编码和 rough edges。不是仅因名字含 Rust 就证明无 unsafe／FFI 风险，覆盖与审计须逐项做 | 首版不选；减少 C 链成本不足以替代 ACK、dump、set timeout、conntrack 兼容证明 |
| 系统 `nft` CLI | netfilter 官方用户态，`nft -j -f -` 批事务，`nft -j list ...` 回读；无需 Meshway 自编 FFI，但有系统包／进程开销 | **首版推荐**。镜像／安装包锁定支持版本，固定可执行路径、结构化输入、无 shell 拼接、有界输出／超时／并发／join。取消后结果不明先读回 reconcile，不能假定事务未提交 |
| 用户态 NAT／netstack | 易将每次 socket I/O 绑 guard，跨平台潜力；需要 TCP／UDP／ICMP 转发、PMTU 与另一套资源集成，当前无正式 netstack | 不符合首版 Linux 内核转发目标，不作为权限不足的自动降级 |

conntrack 精确删除由 Underlay 唯一的 conntrack 模块经 ctnetlink（`NFNL_SUBSYS_CTNETLINK`）完成，不依赖 conntrack-tools：先按 IPv4 original tuple 与默认 zone `GET`，核对 id、mark、original／reply 与已绑定身份一致，再以 original tuple、默认 zone 与该 ct id `DELETE`（内核比较 id，同 tuple 的替换条目回答 `ENOENT` 而不被删除），最后 `GET` 回读不存在才算退役；删除的应答本身不是证据。请求总是携带 tuple，编码层不能表达 flush。回答在 64 KiB 单报文缓冲内有界解析；dump 中任一消息带 `NLM_F_DUMP_INTR`（dump 期间表已变化）时该 dump 为 `conntrackUnavailable`，不当作无匹配（已找到的条目仍算数），畸形、截断、zone／helper／offload 无法证明归属时分别为 `conntrackUnavailable`／`ownershipConflict`；内核 `EPERM`／`EACCES` 为 `netAdminMissing`。mark 位选择与安装探测使用内核 mark 过滤的 dump，返回项逐项复核。

安装探测至少包含 `/dev/net/tun`、实际 NET_ADMIN 操作、nft batch／set timeout／ct 支持、route/rule、ctnetlink mark 过滤读取和 `ip_forward`。`/proc/sys/net/ipv4/ip_forward` 为 0 返回 `ip_forward_disabled`；不可读／格式错返回不同原因 `forwarding_probe_failed`。不把 `uid=0` 或 cap bitmap 当真实成功。

还要检测实际 ingress／egress 的 `rp_filter`、`src_valid_mark` 与策略路由相容性：全局 `/0` capture 可能让 underlay 反向路径校验走错接口，**SO_MARK 本身不解决入站 rp_filter**。首版支持已证明的 loose/off 组合；strict 组合未经双向路由验证则报 `reversePathFilterConflict`。出口 TUN 不配 IPv4 地址，内核对无地址入接口的反向路径检查只在反向查找落到该接口时放行（否则 `EXDEV`，计入 `TcpExtIPReversePathFilter`）；因此每对的回程规则不带出口 mark，兼作 forward 包的反向查找，每个新投影在 ACK 前向内核取实际入路由判定，见 [IPv4 出口角色与 NAT 投影](/v2/node/runtime#ipv4-出口角色与-nat-投影)。不照搬 Tailscale 自动写 `src_valid_mark`，也不自动关闭宿主防火墙。出口接口／地址变化撤回受影响 NAT 映射并重探测，不把旧 masquerade 连续性强行保留。

## 客户端捕获、LAN 与切换

现有 socket mark＋own-mark goto→capture 块末尾的机制可以复用：Server HTTP／SSE、STUN、获准 carrier、明确 resolver socket 均继续按用途受保护，普通应用到同 IP 不获豁免。实际 mark 从 journal 读取，不能硬编码 `0x40000000`。已有 `meshAddressPools` 是地址所有权边界，不能把公网 `/0` 塞进它。

**egress capture plan** 由节点用 Policy 从自己 netmap 的规则与持有的 artifact 计算（R3；netmap 只带规则引用、fallback 与选择 generation），区分：受管 IPv4 区间／默认捕获、必须拒绝的 IPv6 区间，direct/unmanaged 为其补集。它只负责将包交 owner，并不授予整个范围的 Access；第一包仍由精确目的／协议／端口判定。字段、上限与实测见 [Routing · 执行捕获](/v2/configuration-reference/routing#执行捕获与精确许可)。

- 有序规则不能直接变成同序路由：Linux 最长前缀与业务 first-match 不同。Policy 把相交范围分割为语义等价、互斥且有界的区间；不能在更宽高优先级 exit 内插入更具体低优先级 direct 绕行。域名／IP 混合有冲突时按现行契约拒绝，不猜测。
- 受管区间是 owned nft 表的 interval 集合 `managed4`／`managed6`：未经活动客户端 TUN 的受管目的在 OUTPUT 立即拒绝（TCP reset／ICMP admin-prohibited），受管 IPv6 恒拒绝（Mesh IPv6 先由 `mesh6` 放行）。一条 IPv4 RPDB 选择器（非保护 mark、`iif lo`）查私有 egress table；活动时表中为受管区间的最小 CIDR 覆盖 `→客户端 TUN src Mesh 地址`，暂停／TUN 关闭时为空，包落到 main 后仍被集合拒绝，不允许继续 direct。导向不用 nft mark 重路由：那样不改连接已选的物理源地址，需源端 SNAT。
- 源端 LAN 例外只来自管理员有效方案中的明确 direct/unmanaged CIDR，不能自动把 RFC1918、当前连接网段或网关整个网段全放行；**出口 LAN** 则仍为 exit 动作＋显式 CIDR Access＋masquerade。两者不能混用一个 allow-LAN 开关。必要 DHCP／邻居发现等本机链路控制例外按最小协议范围建模，不授予 LAN 应用通路。
- 新的 `/0` 检查不能复用“所有相交外部 local 地址都冲突”的 Mesh 池检查；local-table 本机服务、loopback 和地址控制必须单独建模，外部路由优先级仍需核验。捕获不承诺阻止设备所有者访问本机进程；不能利用本机地址／broadcast 路由绕去远端 LAN。
- 切换顺序：先安装旧新范围并集的负向边界，暂停旧 flow／DNS 和投影，准备新表／TUN／resolver／权限，再切换活动 selector，读回确认全部必需组件后上报 applied。RPDB／DNS／nft 没有跨子系统事务，允许短暂失败，不允许短暂 direct。只能在显式新 direct/unmanaged 已生效或 down 时精确解除相应受管边界。
- 身份撤销、出口不可用、lease 过期不是显式 unmanaged。负向 `/0` 在期望 up 的 suspend／退出／同 namespace 重启期间保留，恢复控制联系仍走受保护 socket；旧实例／旧 DNS 回答／旧队列不换绑。

## DNS 路径与可信域名上下文

### 经 Mesh 的出口 resolver

**已交付（X5）。**行为定义见 [出口 resolver](/v2/node/dataplane#exit-resolver)，配置语义见 [DNS · 使用出口时](/v2/configuration-reference/dns#使用出口时)，验收见 [出口场景](/v2/node/validation#exit-scenario)。交付形态：

- **Ingress**：没有 Mesh IP 上的 DNS socket。查询是唯一 SecureNode 上 `Purpose::Dns` circuit 的加密请求／回复帧；出口收到不透明 `DnsRequest`（origin、完整路径、传输、内容引用、截止均来自已认证 circuit），交 DNS owner 的 `serve_authenticated`，回答交回同一 SecureNode 沿原路径返回。daemon 只驱动有界队列，DNS owner 不持 SecureNode／流表。
- **Purpose 与中继授权**：DNS circuit 的 grant 为 `(Dns, Mesh { exit })`，握手摘要的 selection 为 `DnsService` 摘要；中继以 `admit_dns` 在自己的投影上重算 `dns_service(origin)`，只按完整路径位置转发。服务仍按 revision 留在 netmap（`DnsPlan::ViaExit`／`ServeExitResolver`），没有 instance 或截止字段，截止来自 netmap 租约。
- **客户端 owner**：`MeshResolverRoute` 的 UDP 与 TCP 各自一条路由，共用原 ticket；出口能力事实非 ready 时不建 circuit、查询立即失败；任何失败都不落到源节点本机上游。
- **出口系统 resolver**：resolved 后端经 resolved 代理 stub（`127.0.0.54`），保留按链路／后缀选上游；会回到本节点锚点的名称回 SERVFAIL；resolved 服务器清单含本地 stub、锚点、Mesh 地址或经出口路由的地址时出口查询不可用。文件后端用安装前记录的原 nameserver。上游 I/O 使用仅出口查询可用的 served 快照、独立 UDP／TCP `LocalResolverRoute` 和受保护 socket。
- **wire／IPC／netmap**：均未改变（DNS circuit 帧已属 wire v6；`compilerVersion` 当时为 `meshway-netmap-1`）。

### 哪些查询走出口

优先级沿用现有定义：MagicDNS 本地精确记录／发现 → 保留 `useWithExit=true` 的最长后缀 split／default（该解析器地址在出口区间内时是源分割的 direct 例外，例外不是 Access 授权）→ 普通查询按方案的 DNS 判定。判定唯一实现为 `meshway_policy::compile::dns_resolution`，Server 编译 netmap、保存时校验与节点中继重建 `DnsService` 共用：

- 普通查询在解析前选定 resolver，不先向源端 DNS 泄漏问题再按答案选出口。只有名称规则需要先解析才能判定：IP 规则（含只含 IP 的规则集）与答案无关，不论经几个出口，普通查询都用本机系统 resolver（`noExit`）。
- 有经出口的名称规则或出口 fallback 时，它们共同的那一个出口（含有序 via）解析全部普通查询（`viaExit`）；名称不先在本机解析，因此不会泄漏。
- 名称规则／出口 fallback 指向不同出口或 via、或出口集合有主备时，需要“哪个名称交哪个出口”的显式 DNS plan，首版未实现：判定为 `exitPlanRequired`，Server 保存配置时即以 `dns_exit_plan_required` 拒绝，不下发到节点再 fail closed。
- IPv4 首版的出口解析通道可解析 A；AAAA 的存在不能变成 IPv6 路由豁免。已实现为保留真实 DNS 回答语义（AAAA 原样返回，不伪造 NXDOMAIN）；任何受管 IPv6 包由捕获和 permit 双重拒绝。只屏蔽 AAAA 不是防泄漏措施。

### 可信域名关联 {#trusted-dns-associations}

**已交付（X6）。**Noise 证明回答来自出口，不证明出口上报的域名／地址自然有权扩大准入；普通 State 或源端自报 hostname 都不能创建关联。关联只由下面的受控解析任务产生，配置语义见 [Routing · Rule match](/v2/configuration-reference/routing#rule-match) 与 [Access · 域名授权](/v2/configuration-reference/access#domains)。

**任务**由 Server 预先签发且常驻：出口 E 的已签 netmap 中 `ServeExitResolver { services }` 的每个 `DnsService` 就是一份任务，绑定 origin（`service.source`）、DNS plan（整份 service，含完整路径、源 selection `generation` 与出口 role）、出口 resolver（E）、revision 与内容（E 与源各自的 `NetmapRef`）以及期限（netmap 租约）。名称与 nonce 由每次完成填入：

| 步骤 | owner | 内容 |
| --- | --- | --- |
| 1 | 源 A 的 DNS owner | 普通查询照 X5 经 `Purpose::Dns` circuit 送 E（不改 wire、不另发请求） |
| 2 | E 的 DNS owner（served 路径） | 用出口系统 resolver 解析，回答**立即**沿原路径回 A；若 E 在自己的投影上 `egress_rule_for_name(A, name)` 命中（名称由某条规则的名称成员决定），且回答是 NOERROR 的 A／AAAA 并含公网地址、这些公网地址全部在至多 8 跳的 CNAME 链终点上且不超过 16 个，且 E 持有的关联里没有覆盖这些地址并剩余 ≥ 本次寿命一半的同名条目，就产生一份完成 |
| 3 | E 的 Control | `POST /agent-api/v1/dns-associations`（`DnsAssociationReport`：E 的 `netmap`、单次使用 `reportId`、`source`、`generation`、`name`、`recordType`、≤ 8 个 CNAME 链、1–16 个已排序公网地址、链与地址记录中最短 TTL、`resolvedAt`）；队列有界（256），满则丢弃该完成 |
| 4 | Server（`netmap::associations`） | 校验见下表；通过即在内存保存并唤醒 A 与 E 挂起的 `sync` |
| 5 | A／E 的 Control | `sync` 回答带 `associations: DnsAssociationsUpdate`（overlay＋签名），验签、绑定当前 netmap 后与 netmap 一起发布给 Engine |
| 6 | A 的 DNS owner | 第 1 步的回答在 overlay 已应用（Engine `Desired` 含覆盖其公网地址的同名关联、剩余 ≥ 5 s）后才交给本机客户端，TTL 截为关联剩余寿命减 2 s；至多等到该查询截止前 250 ms、不超过 5 s（`ASSOCIATION_WAIT`；查询截止为到达后 5 s），未到则回 SERVFAIL，只有该查询等待，客户端不会拿到名称规则无法判定的地址。名称规则决定的回答若含任何一份关联都无法完整覆盖的公网地址（超过 16 个、不在 CNAME 链终点、链成环或超过 8 跳）立即回 SERVFAIL，不截取其中一部分验证 |

Server 的完成校验（任一失败即拒绝，不部分接受）：

| 检查 | 失败 |
| --- | --- |
| 调用者 E 的当前 netmap 等于 `netmap`；`generation` 等于源当前 `DnsService.generation` | 409 `state_scope_changed` |
| 源当前 netmap 为 `ViaExit { service }`，`service.exit` 是调用者，且 E 当前 netmap 的 `ServeExitResolver` 含同一 service | 403 `permission_denied` |
| 源投影上（含全部 artifact）`egress_rule_for_name(source, name)` 命中 | 403 `permission_denied` |
| 名称为查询规范形式（ASCII 小写、无结尾点）；名称与每个 CNAME 均为 1–253 B ASCII 小写（与 overlay 逐条校验为同一函数 `DnsAssociation::validate`，通不过的条目不会进入任何一方的 `sync`）；CNAME ≤ 8；地址 1–16、已排序去重、与记录类型一致、全部 `is_internet_address`（私网、Mesh、链路本地等永不进入关联，防 DNS rebinding 借名称规则进入 LAN）；`resolvedAt` 不早于 30 s、不晚于 Server 时间 5 s | 400 `invalid_request` |
| `reportId` 未用过（进程内保留至其窗口结束） | 409 `request_id_conflict` |
| 接纳后源与出口各自 overlay 的未到期条目都不超过 4,096（同一条目两份副本一起交付，绝不只截断一方） | 429 `state_limit` |

**Overlay**（`meshway_configuration::netmap::DnsAssociationOverlay { netmap, associations }`）按**接收节点**签发：A 收到自己作为源的条目，E 收到它服务的各源的条目；`netmap` 是接收者当前 `NetmapRef`。每条 `DnsAssociation { source, generation, name, recordType, cnames, addresses, expiresAt }`，`expiresAt = resolvedAt + clamp(TTL, 30 s, 1 h)`；overlay 本身没有总截止，只在它绑定的 netmap 及其租约内可用。签名为 `dns_associations_transcript(network, node, overlay.netmap, overlay digest, keyId, keyGeneration)`，复用 netmap 签钥；轮换窗口内另附后继钥签名 `rotationSignature`，节点只验它钉住的那把钥。`sync` 请求以 `associations` 指明持有的 overlay digest：相同则回答 `null`（不唤醒、不重发），不同则回答完整 overlay（空 `associations` 即清除）。overlay 不改变 netmap serial，不写入 `netmap.json`：节点重启、Server 重启都从空开始（Server 重启后的第一次回答清除节点持有的 overlay）。

**界限**：每源 128 条（超出先逐出最早到期者）、每 overlay 4,096 条（接纳时对源与出口统一预算，超出即拒绝该完成）、每源每 (名称, 记录类型) 一条（新完成替换旧条目）；Server 最多记 65,536 个源。

**失效**：Server 在任一方 netmap 变化（新 serial）后、下一次交付前用新内容重验每条关联：service／generation／名称规则不再成立即删除——撤权（`domain:` 授权由节点每次准入检查）、出口停用、方案切换（selection 变化）、名称规则删除都在同一次 `sync` 回答中随新 netmap 生效；跨 source、profile、出口和 netmap 不复用。条目到期由 Engine 计时：`Engine::next_expiry` 取租约与最早关联到期的较早者，到期后以同一内容、更少关联重新应用。

**节点侧执行**：

- 判定：`LocalCompiler::decide_with(source, dst, …, &Associations)`。dst 有该源在当前 selection 下的关联名称时，按有效规则顺序 first-match，规则命中条件为“匹配该地址**或**匹配其任一名称”；各名称得到的动作必须相同，否则 `egressConflict`（同 IP 多名称动作不相容即不可执行）。服务为覆盖该地址的 CIDR／`autogroup:internet` 授权与覆盖其任一名称的 `domain:` 授权之并。没有关联的地址只按 IP 分割决定：名称成员不拥有任何地址（`capture` 分割不再因名称规则整体 `egressConflict`）。
- 捕获：关联地址是节点侧分割的一类输入。Engine 的 `Desired` 在本节点计算的捕获区间上加入本源的关联地址——出口（及冲突）地址并入 `managedIpv4`／`sinkIpv6` 区间，direct／unmanaged 地址移出受管区间（`LocalCompiler::associated_capture`）；selection identity 不变。关联到期即以新的 `Desired` 重新应用、移除该地址；但本节点仍存活、经出口建立的流（唯一流表 `SecureNode::exit_destinations`）的目的地址留在捕获中（`Desired::retaining`，Policy `retained_capture`；只保留先前边界已捕获、新边界释放的 IPv4 地址，不新增），这些流按原判定继续经出口与必经点，流退役后（节点每次唤醒检查，至多约 1 s）才移除；保留期间新到该地址的流按新判定，direct／unmanaged 包在 Mesh 接口上丢弃，绝不从物理网卡带着 Mesh 源出网。netmap 内容变化不保留（数据面按新内容重判全部流）。超出节点侧上限（[Routing · 节点侧上限](/v2/configuration-reference/routing#capture-limits)）时按分割的 fail closed 形态（全范围捕获、IPv6 sink、`routingLimit`）；Server 不再预先校验捕获大小。
- 中继与出口握手：`transit` 对 `(nodes, via)` 也接受“有名称成员且动作完全相同的规则＋源有 `domain:` 或 Internet 授权”，中继不需要关联；出口在自己的投影上以同一 overlay 为源的新流作判定；已建立的流（源、中继与出口各自的唯一流表）在同一内容（同一 `Desired` 编译器）下保留准入时的判定，关联到期不改变它，netmap 内容变化才重判。
- 关联变化只影响新流：同一内容、不同关联时 `SecureNode` 只换判定缓存，已建立的流和 circuit 不重判（其捕获见上一条的保留）；DNS owner 保留 scope 与已接纳查询。netmap 内容变化时照常以当时的关联重判全部流。

**往返与等待**：首次解析到可用 ＝ 一次 Mesh DNS 往返（含出口解析）＋ E→Server 一次 HTTP 往返＋ Server→A 挂起 `sync` 的回答（半个往返）＋ A 应用；A 的回答至多为此再等到查询截止前（≤ 5 s）。之后在关联寿命内：本机缓存命中（TTL 已截短）零往返；缓存未命中的再次查询仍经 E 解析，但 E 见到仍剩余 ≥ 一半寿命的覆盖条目即不再完成任务，A 见到覆盖条目即刻放行，零额外 Server 往返；同名第二次连接直接按已应用关联建流。

**限制**：域名授权仍是解析地址集合，关联期间对该地址的任何连接（不经名称也一样）都按名称规则判定，不保证同 IP 的应用身份隔离；未经该源 Mesh DNS 得到的地址（硬编码 IP、DoH、其他 resolver、忽略 TTL 的应用缓存）只按 IP 规则判定。只有经出口 resolver 的普通查询产生关联：MagicDNS、`useWithExit` split 与 `DnsPlan::NoExit` 的方案不产生，其名称规则只是不匹配任何地址。AAAA 真实回答也产生关联，但受管 IPv6 包仍由捕获 sink 与准入双重拒绝。多出口／依赖答案选出口的显式 DNS plan 仍未实现。

## 协议与模型

| 面 | 类型（所属 crate 为准，HTTP 经生成 OpenAPI／SDK 发布） |
| --- | --- |
| 出口职责 | netmap 的 `exitRole`：`meshway_configuration::netmap::ExitRole { generation, family, nat }`，只授权准备资源；实际 NAT 投影仍需活 flow 与 Policy 判定 |
| 源端捕获 | 节点由 netmap 规则与 artifact 计算 `Desired::capture`（`EgressCapturePlan`）；受管 IPv6 必须 sink |
| 业务判定 | Policy `Decision`（目标、服务、完整路径、出口 selection）；握手 transcript 绑路径授权摘要，wire v6 |
| 能力事实 | `NodeState.exitCapability`：`ExitCapability { instanceId, netmap, roleGeneration, supportedFamilies, installedFamilies, status, observedAt }`，`ExitStatus`／`ExitUnavailableReason`；没有观测时省略，不补 ready。报告失败不要求 applied，角色准备失败也不阻断本节点其余内容的 applied（[出口宿主前置探测](/v2/node/runtime#出口宿主前置探测)）。静态 IPv4 后端声明使用 acquire 的 `feature:exit-ipv4` version 1 |
| 出口 DNS | netmap 的 `DnsPlan::ViaExit`／`ServeExitResolver` 与 `DnsService`；`NodeState.exitDns`（[exitDns](/v2/state-reference#exitdns)）；关联见[可信域名关联](#trusted-dns-associations) |
| 判定失败 | `UnavailableReason::ExitUnavailable`／`UnsupportedFamily`／`EgressConflict` 等，由 Policy 在节点判定中给出 |

版本：`compilerVersion` 为 `meshway-netmap-3`，数据面 wire v6（含 DNS circuit 帧）；现有 HTTP 根不变。显式多出口 DNS plan 未建模，Server 保存时以 `dns_exit_plan_required` 拒绝。

## 安全边界与限制清单

- **开放代理：**出口 role、该源的 Policy 判定（具体目的／服务、完整实际 path）和有效租约缺一不可；本机 DNS／NAT 无公网 listener，未请求方向不能建立新流。
- **源伪造：**已认证 origin 绑定其精确 Mesh 地址，lax 不成为 origin；不接受 LAN 源、另一 Node 源、IPv4-mapped IPv6 绕分类或源端声称已校验的 metadata。
- **公网集合：**唯一 Policy 分类排除 RFC1918、ULA、link-local、组播、CGNAT `100.64.0.0/10`、loopback、unspecified、保留／广播等特殊范围；测试覆盖边界地址，不能用 `!is_private()` 近似。出口 LAN 必须明确 CIDR Access，不能因域名解析成私网就借 Internet grant 放行。
- **hairpin：**首版禁止出口本机／所属虚拟设备／Mesh 池作为外部转发目标，避免 local INPUT、递归 TUN 或 NAT 回卷。出口集合中源节点自己的位置对该源就是 direct（[Routing · Action](/v2/configuration-reference/routing#action)），不产生单节点出口路径。
- **状态回复：**conntrack 不证明 business flow；TCP／UDP／echo 仍经过唯一 FlowTable。相同 tuple 换 path、profile／selection 或迟到旧回复不能接管新流。
- **撤回：**路径节点、成员、Access、出口开关、方案或 netmap 租约变化均检查；旧内核投影、DNS ticket、密文／明文队列一起失效。禁用出口后宿主原本开启的 ip_forward 不改回，但 Meshway 转发资格与 NAT 全部撤回。
- **IPv6：**首版拒绝 IPv6 出口判定，客户端受管 IPv6 负向捕获，出口专用 TUN 的 IPv6 转发 drop；外层 IPv6 carrier 与内层 IPv6 出口能力分别表达。已授权 Mesh IPv6 可继续使用自己的路径。
- **MTU／协议：**首版继承现有 IPv4 无 options／分片、TCP／UDP／ICMP echo 范围；明确限制超 MTU UDP。源节点自己超过首跳通道内层上限的包由源数据面回 fragmentation needed，源与出口的 TCP SYN 按通道上限限制 MSS（[通道 MTU](/v2/node/dataplane#channel-mtu)）；出口对来自外部的超限回包（UDP 等）只丢弃，不向外部发送错误。ICMP 错误不当业务 reply：只作为被引用活流的 RELATED 报文按[ICMP 错误](#icmp-errors)放行（内嵌 tuple 校验、每流每秒 10 条），PMTU 发现由此收敛；不开放通用 RELATED helpers。禁止 NFT flowtable／硬件 offload 绕过逐包截止门。
- **资源与宿主：**每节点流上限、nft set／ct 投影、query 数、命令队列和 buffers 有界。未知表／相同名字的外部资源／mark 冲突停装；mark 归属检查须覆盖外部 nft／connmark 写入，不能只检查当前 RPDB selector。规则丢失监测、部分事务、NET_ADMIN 丢失均停止正向 I/O，保留可核验恢复证据。出口角色自身的失败只停止该角色并以同一内容重装，不撤本节点的 netmap、capture 或 DNS（[运行中的角色失败](/v2/node/runtime#ipv4-出口角色与-nat-投影)）。
- **投影容量：**活投影 128 个（`exit::program::CAPACITY`）同时是成对回程规则的槽位数：每个 (源, 远端) 对占 RPDB 两个优先级、ICMP RELATED 规则一个，共 257 个，位于捕获规则之前的 512 个保留优先级内；nft 集合与 journal 历史为 4096（`HISTORY`，含 quarantine）。128 对 echo 足够（每个 echo 流在回复后 2 秒结束），对多源业务出口偏小：一个浏览器页面即可并发数十条 TCP。提高活投影上限不受 nft（4096）限制，但成对规则需与投影上限解耦（按对计数，对数仍受保留优先级约束），且每轮安装的归属 dump 与整份 journal 落盘随投影数线性增长（128 投影＋325 quarantine 时 journal 约 190 KB，每轮写两次）；现状保持 128；后续工作包 X9：先把 journal 改为增量记录、测量每轮安装耗时，再把活投影提高到 1024（对数保持 ≤ 128）。按序号的 echo quarantine 另把全出口持续 echo 速率限制在约 13 次／秒（4096 ／ 300 秒）。

## 验收

组件 owner 与文件归属见[当前实现落点](#当前实现落点)。真实 daemon＋Linux 系统资源的整组验收为矩阵行 `exit/off`（A→R→E，含前置条件、撤回、租约到期、DNS、可信关联、方案切换、ICMP 错误与 PMTU、`meshway ping`），阶段定义、当前状态与证据只在[验收范围 · 出口场景](/v2/node/validation#exit-scenario)维护；真实出口宿主见[X7 真实出口宿主](/v2/node/validation#x7-exit-hosts)。fixture 复用正式 owner，不为测试写第二个签发者或第二套 NAT／准入。

## 未决产品问题

名称规则经多个出口时的显式 DNS plan 配置交互仍需单独提交产品方案；在其定版前保存即拒绝（`dns_exit_plan_required`），不提前发明可用字段。只有 IP 规则的方案不受限：多个出口照常可用，DNS 走本机。域名型出口规则已随 X6 在单出口 DNS 方案下生效。
