# 网络模式与本机运行

## 启动与接入

`meshwayd`／`meshway` 提供 Server 控制接入与 Linux TUN 运行装配。以下本地示例显式选择 `control-only`，假定已有当前版本 Server 位于 `127.0.0.1:8430`（示例值；开发服务固定 8420，见[开发服务](/v2/development#本机服务)），Console 位于 `127.0.0.1:3001` 并连接该 Server，且 Server 已配置对应的 [Console origin](/v2/server#运行参数)：

```sh
mise run build:node
install -d -m 700 .meshway
./target/debug/meshwayd \
  --data-dir "$PWD/.meshway/node" \
  --network-mode control-only \
  --server http://127.0.0.1:8430
```

另开终端操作同一 daemon：

```sh
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" up
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" status --watch
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" status --json
./target/debug/meshway --socket "$PWD/.meshway/node/control.sock" down
```

- 首次 `up` 原样显示 Server 返回的浏览器批准链接并等待批准。Server 未配置 Console origin 时，JSON 状态保留 `enrollmentUrl: null`，人类输出提示联系 Server 运维，状态仍为等待批准。
- 在 Console 登录并选择网络批准后，daemon 自动兑换身份、认证、取得实例、保存配置和发送 saved 回执。
- 此示例的 `controlReady`／`dataPlane=controlOnly` 表示控制流程就绪，`applied=null`，不安装网络。Linux TUN 使用 `--network-mode tun`（默认值），还须满足下节安装范围与设备／路由权限。

`meshwayd` 前台运行，SIGINT／SIGTERM 执行关闭与 join。进程退出保留期望状态：退出前为 up，重启后自动重新取得运行资格；显式 down 后重启继续保持下线。CLI 的 Ctrl-C 仅停止等待，不撤回已由 daemon 接纳的持久操作。

| 参数 | 行为 |
| --- | --- |
| daemon `--data-dir` | 私有状态目录，默认 `/var/lib/meshway`；父目录须存在，Unix socket 位于其 `control.sock` |
| daemon `--server`／`MESHWAY_SERVER_URL` | 必须明确指定 Server；远端要求 HTTPS，数字 loopback 可使用 HTTP |
| daemon `--hostname` | 首次接入建议名；省略取系统 hostname 的首个 DNS label，既有业务名称仍由 Server 管理 |
| daemon `--network-mode`／`MESHWAY_NETWORK_MODE` | 本机启动参数 `tun`（默认）或 `control-only`；不是云端业务字段，失败不自动回退 |
| daemon `--bootstrap-host NAME=IP` | 可重复，为 Server 或配置中 carrier 名称提供固定 bootstrap 映射 |
| daemon `--bootstrap-nameserver IP:PORT` | 可重复，指定受保护 bootstrap DNS 的真实上游；IPv6 使用 `[IP]:PORT` |
| daemon `--dns-backend` | `resolved`（默认）或显式 `resolv-conf`；本机安装后端，失败不自动切换 |
| daemon `--dns-bind` | loopback `IP:PORT`；resolved 默认 `127.0.0.1:5354`，文件代理默认 `127.0.0.1:53` 且要求端口 53 |
| daemon `--dns-anchor-address` | resolved 的独立本机 DNS 接口 IPv4 地址，默认 `198.18.255.254`；归属／地址冲突拒绝安装 |
| daemon `--dns-resolv-conf` | 仅用于文件代理，当前 daemon 仅接受 `/etc/resolv.conf`；文件必须满足[系统 DNS 归属约束](/v2/node/dataplane#系统-dns-安装) |
| CLI `--socket` | 默认 `/var/lib/meshway/control.sock`，可显式选择本机 daemon |
| CLI `--json` | 输出类型化公开状态；不包含 Node 私钥、兑换码或会话 token |
| CLI `up --enrollment-key` | 一次性接入密钥；支持字面值、`@file`、`-`／`@-`（stdin）。优先文件或 stdin，避免秘密进入 shell 历史与进程参数 |

默认目录适用于 Linux 系统服务；本地开发或其他 Unix 环境可像示例一样显式指定私有目录。安装参数不构成本机业务配置覆盖源。

daemon／CLI 共用[运行日志参数](/v2/logging)，日志位于 stderr；CLI 的 `--json` 状态结果位于 stdout。

本机多节点开发使用[三节点 Docker 环境](/v2/node/docker-lab)，统一投放预编译程序，并为每个 Node 保留独立状态卷。

## 安装范围与状态

当前 TUN 候选由 daemon 在唯一 [Engine](#engine) 后装配 `meshway-underlay::Host`、`meshway-connectivity::Manager`、`meshway-dataplane::secure::SecureNode` 与 DNS 查询 owner。TUN 源包在期望 netmap 上本地准入，不联系 Server。原路径回复查询 SecureNode 的唯一流表，不另建 daemon 反向流表。

| 范围 | 当前行为 |
| --- | --- |
| Linux Mesh-only | TUN capture、Native TCP／Native UDP 静态端点认证邻接、协调 UDP 穿透、双栈 UDP／TCP 多跳包流、持续续租和失效清理；验收状态见[验收范围](/v2/node/validation) |
| DNS | 已接入 loopback 服务、精确 resolver 需求、resolved／显式文件代理；所有必需组件安装成功才可 applied，后端约束见[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装) |
| Exit／本机 egress | daemon 按当前执行目标探测并安装 IPv4 出口角色、精确 NAT 投影和源端 capture／IPv6 sink；回包由同一 SecureNode 按原完整路径返回。角色、capture、承载及必需的负向 DNS 资源共同进入本机 applied 边界。出口 resolver 适配仍不可用，单独报告；完整多进程验收范围见[验收范围](/v2/node/validation) |
| 诊断 | `meshway ping` 与 Server observation tasks 共用获准测量器，见[路径诊断](/v2/node/diagnostics) |
| 承载 | Manager 共用获准的 stream、Native UDP 静态端点和协调 UDP 穿透；会话与交接见[邻接管理](/v2/node/transport#邻接管理)，NAT 范围见[连接获取](/v2/node/connectivity) |
| 其他平台／入口 | 非 Linux 的 tun 返回 unsupported；userspace（见[规划](#规划-userspace-networking)）、动态插件和切网尚未实现 |

仅验证 Mesh IP、无需系统 DNS 的场景，在 Server 的 [DNS 配置](/v2/configuration-reference/dns)中明确设置 `magicDNS=false`、`overrideLocal=false`，并保持 `servers`、`default`、`routes`、`searchDomains` 为空。此时不创建 DNS 服务／锚点，也不要求机器运行 resolved。存在 DNS 需求时须完成对应安装；本机后端选择不覆盖云端 DNS 业务配置。`control-only` 不安装系统 DNS。

### 出口宿主前置探测

Linux `Host::exit_probe` 在打开 Host 前执行，独占本进程的 Host 预约。探测读取 `ip_forward`、实际接口与临时出口 TUN 的 `rp_filter`／`src_valid_mark`，实际创建并关闭非持久 TUN，并通过固定路径 `/usr/sbin/nft` 的 JSON 原子事务检查 timed set、original conntrack tuple 与 ct mark 表达式；`/usr/sbin/conntrack` 执行有界的 IPv4 mark 过滤读取。探测不修改宿主 sysctl 或防火墙策略，临时 nft 表在同一事务中创建、删除，取消后的命令结果经 join 与表列表回读处理。

`ip_forward=0` 返回 `ipForwardDisabled`，读取失败或格式错误返回 `forwardingProbeFailed`；实际 TUN 管理操作无权限返回 `netAdminMissing`。有效 `rp_filter=max(all,interface)` 为 strict 时返回 `reversePathFilterConflict`，不会自动改为 loose/off。工具不可用分别返回 `nftUnavailable`／`conntrackUnavailable`。观测保留原始 `observedAt`，重试上报不得重新计时。

此前置探测成功仍为 `probing`，`supportedFamilies`／`installedFamilies` 为空。探测与下面的角色安装是不同操作，不能据此前置探测发布 `ready`／`applied`。宿主管理员预配置 forwarding 和提供工具／设备不等于完整出口功能已可用。

已打开的 Host 使用 `probe_exit(&mut self, instance: InstanceId, target: ConfigurationRef) -> ExitCapability`。它使用真实、处于 up 状态的外部接口，排除所属 Mesh／出口 TUN 和 DNS 锚点；临时设备的精确 index 注册到同一 monitor，创建／删除不会制造外部切网事件。探测检查实际 TUN 权限、转发／反向路径条件、完整 IPv4 nft 程序和 conntrack 读取；排除经 owner 回读确认的自有表后，无法证明兼容的宿主阻断规则／旧 iptables 表返回 `firewallConflict`，未知 mark 修改／offload 返回 `ownershipConflict`，不改写宿主规则。

live probe 成功返回 `ready` 和 `supportedFamilies=[ipv4]`，`installedFamilies` 仍为空；其含义是所报后端前置条件完成，不代替角色安装或业务连通证明。失败携带具体原因。返回值保留传入 instance／target 与本次真实 `observedAt`，X4 不需自行把 `probing` 改成 `ready`。

daemon 首次取得执行目标及每次向网络 worker 发布执行刷新时，若 `exitEnabled` 或 `exitRole` 存在，即调用 live probe。该 Future 的 monitor 预约不可跨线程移动，因此唯一网络 worker 在一个固定线程上驱动，并在停止时显式等待退出；Host、Control 和内核资源仍各只有一个 owner。探测失败也可上报 State；只有角色及其他必需组件实际完成当前目标安装后，才填充 `installedFamilies` 和 applied。

Control 在保存执行目标和签发快照时检查本地 `exitRole.instanceId` 等于当前实例、出口开关仍启用，以及 permit 内引用的本地角色与目标角色一致。`NodeState.exitCapability` 是独立观测：失败可在尚未 applied 时报告，新观测必须绑定当前 instance／精确 target；错绑在发送 HTTP 前拒绝。不确定交付的重试保留原内容、序号和 `observedAt`，不把旧探测换绑到新目标。能力报告不生成应用回执，也不替代必需组件的原子 applied 边界。出口 role 安装期间，网络 worker 在每个事件上检查该观测：超过 State 报告寿命（60 秒）的三分之一即重新真实探测，持续业务流量不推迟刷新。

### IPv4 出口角色与 NAT 投影

Linux `Host::install_exit_role(ExitRolePlan)` 消费当前执行快照中相同的出口职责，返回 `ExitRole`。资源仍归该 Host 的唯一 Kernel owner；角色使用独立非持久 TUN，不暴露 FD。crate 根导出 `ExitRole`、`ExitScope`、`ExitIncarnation`、`ExitSendGuard`、`Invalidated`、`OriginalTuple`、`ExitTransport`、`ProjectionAck`、`CtConfirmation`、`ExitReplyReader` 和 `ExitReply`。

| 操作 | 行为 |
| --- | --- |
| `install(incarnation, original_tuple, deadline, scope)` | 拒绝已有冲突 CT／tuple／incarnation；安装精确 timed set 资格并回读，返回 `ProjectionAck`，此后才允许提交首包 |
| `send(incarnation, packet, guard: impl ExitSendGuard)` | 每个排队包独立持有原 guard 与入队时的投影；每次 TUN 写 future 的 poll 前同步检查该 guard，再受原截止／撤回／tuple 约束；不使用最新 Live 投影替换排队包的权限 |
| `confirm(incarnation)` | 首包产生 CT 后，按 IPv4 original tuple＋归属 mark 检查，返回 `CtConfirmation::Confirmed`；有界等待不超过三秒及流截止，缺失或冲突时先关闭资格，再返回 `ConfirmFailed`；清理失败返回错误 |
| `renew(incarnation, deadline, scope)` | 仅在旧资格仍活、绑定相同且已确认 CT 身份仍一致时原子续期 timed set；不创建新的 CT，不复活过期投影 |
| `retire_flow(incarnation)` | 先关闭该资格和用户态输出；其它活 ICMP sequence 仍引用同一 CT 时保留共享映射，最后一个引用退役后按 family／original tuple／mark 精确删除并读回；释放不再共用的回程规则／路由 |
| `reply_reader()` | 单次取得独立回包 reader；`recv()` 返回 incarnation 与逆 NAT 包，排队期间继续保留原 guard；业务回复仍须通过唯一 dataplane 流表与原完整路径准入 |
| `retire()` | 停止正向 I/O，关闭全部资格、清理 CT／回程资源，join TUN owner 后移除所属 nft 表；失败保留 journal 和拒绝边界 |

`ExitScope` 携带生产者提供的授权／路径 binding 与不可逆撤回 token，不签发业务权限。原 tuple 仅支持 IPv4 TCP、UDP 和 ICMP echo；角色计划不能替代精确源地址、目的、服务和活 flow。投影当前最多 128 个活项，最多保留 4096 个 incarnation／tuple 历史及已知转换 tuple 的拒绝记录；容量耗尽只拒绝该新增流，已安装投影与角色不受影响。成功退役的原 tuple 在同一角色内也不复用，恢复后的旧转换 tuple 保留隔离。

incarnation 的类型为 `ExitIncarnation`：`new([u8; 32])`／`bytes() -> [u8; 32]` 原样保留全部 32 字节，不截断成 `u128`。`ExitSendGuard: Send + Sync + 'static` 由 Underlay 定义，唯一方法为 `check(&self) -> std::result::Result<(), Invalidated>`；X4 用 X2 原包的 `exit_scope()` 实现，Underlay 不依赖 dataplane。

`ExitTransport::IcmpEcho { identifier, sequence }` 按两字段独立投影。同 identifier 不同 sequence 可以共存并共享内核 CT；读回包也精确匹配 sequence，退役一个不会撤销另一个的资格。ICMP 使用独立 timed set；固定 nft 版本通过规则内的 32-bit 字读取、掩码和恢复实现 sequence 检查，未知输入 mark 拒绝，成功返回前恢复专用路由 bit，临时值不交给外部 hook／RPDB。这只实现内核资源共享，不增加业务流准入规则。

角色安装使用 `inet meshway_exit_<installation>` 与 `ip meshway_exit_nat_<installation>` 私有表。只有独立 TUN ingress、Mesh 源、精确活投影、NEW 状态且归属 bit 尚未设置时，FORWARD 才设置 CT mark；已归属 NEW／ESTABLISHED／RELATED 必须继续匹配投影，未归属 ESTABLISHED 不接管。NEW 重传仍受同一资格约束。PREROUTING 只准备专用路由 bit，回程要求 reply direction／原 tuple／归属 mark；最终 postrouting 再检查资格。原方向执行 masquerade，回程 RPDB 将逆 NAT 后的源节点 `/32` 导向出口 TUN。正向绕过客户端 capture 的规则同时匹配出口 iif 与资格生成的 mark。

Mesh 池、loopback、链路本地、组播、保留／广播及出口本机地址不能作为外部转发目的。IPv6 出口拒绝。宿主必须提供实际可用的 TUN／NET_ADMIN、固定路径 nft／conntrack、`ip_forward=1` 和已支持的 loose/off 反向路径校验；无法证明 mark 写入或 flow offload 不冲突时停装。**不要求或修改 `nf_conntrack_acct`**，计数不用于 CT 归属证明。宿主原有防火墙仍可拒绝包。

所有 nft 变更通过结构化 JSON 原子批提交；工具执行每次两秒预算，全局命令槽最多等待一秒，输入和各输出管道各 2 MiB，进程并发与请求／包队列均有界。timeout 使用秒，按原单调／墙钟截止减去预算保守取整，实际资格可提前到期。续期在异步检查后再次核验旧资格，ACK 不延长原 guard。取消或丢失响应后 join 并读回核验，不把工具被杀视作回滚。

私有 `underlay-exit.json` 记录 boot／namespace、基础与出口 installation、节点／网络／职责、TUN 名称与 index／MTU、mark／回程表、精确 route/rule、nft handle 与规范规则、投影截止／CT 身份／隔离 tuple，以及 pending／closing／removed。`exit-child.json` 在工具执行前记录 PID／启动时间／namespace，阻止仍存活或不可判定的旧工具与恢复竞争。同 namespace 重启先核对资源并关闭旧投影、精确清除 CT；跨 namespace 记录归档，不操作原 namespace。所有权冲突保留证据并停装。

出口 journal 当前为 v2，保存完整 incarnation 与 ICMP sequence。v1 只进入精确清理路径，不恢复活投影；旧 128-bit handle 完整保留用于追溯，旧记录没有 sequence 证据时保留较宽的转换 tuple 拒绝记录，不猜测 sequence 或放行重用。

撤回 ACK 证明后续穿越资格门的包被拒绝；此前已通过最终过滤、排入 TUN／网卡的包无法追回。源端默认 capture、IPv6 sink、切换、出口 DNS 与正常 daemon 的整体验收状态以[验收范围](/v2/node/validation)为准。

### daemon 出口装配与 applied

出口驱动 FIFO 最多持有 128 个操作／原始包包装；先排入 SecureNode 的 Install／Renew，再排入依赖该 ACK 的包。驱动使用完整 256-bit incarnation、原 tuple、原单调／UTC 截止和同一 Control clock；先完成 Host 投影，再完成 dataplane ACK，等待 `projection_ready`，将每包 `exit_scope` 保留至最终 TUN write poll。首包后的 CT 确认失败调用 `fail_exit_projection`，只关闭对应 flow；回程 reader 输入 `exit_reply`，依赖续期的回复等待 ACK 后才进入既有 carrier 发送路径。

驱动只记录已安装投影及 CT 是否确认，不持第二份业务流表。角色消失、出口关闭、实例替换或执行停止时，先取消原输出、调用 `retire_exits`，再排空退役事件；FIFO barrier 等待已交出的 ACK 释放命令预算，继续取出此前尚不能物化的 Retire。精确 CT 清理结果传给 RetirementAck，最后退役角色并等待驱动退出。原始已过期或已撤回的排队包不会换绑到续期后的投影。

客户端先通过 `install_egress_capture` 准备旧／新范围的负向边界，再安装当前快照并 `switch`；任一必需组件失败时 suspend 保留负向边界，显式 down 由同一 Host 精确清理。中继可安装外部 permit，但没有本地 role 就没有出口 writer。

`DnsEgress::ViaExit`／`ServeExitResolver` 的专用 DNS-purpose 适配尚未接通。daemon 不发布这些查询的执行 Scope；配置需要系统 DNS 时，已有 DNS owner 保持负向安装。客户端 `ViaExit`／`Unavailable` 计划即使关闭 MagicDNS／overrideLocal，也须安装负向 root DNS，不能恢复本地 fallback。状态中的 `exit.dnsUnavailable=dnsUnavailable` 与 IPv4 后端能力／角色安装分别显示，缺少 resolver 适配不否定业务 IP 出口；必需的负向 DNS 系统资源安装或健康检查失败仍阻断整体 applied。`exit/off` 不测试查询，但客户端仍须提供该负向安装所需的系统后端，不能据其结果宣称出口 DNS 可用。

### 源端 egress capture 与切换

`Host::install_egress_capture(EgressCapturePlan) -> Result<EgressCapture>` 复用当前客户端 TUN。它可以先于执行安装建立负向边界；尚无相同有效执行目标时返回非 active handle，不据此报告 applied。`Host::install` 在所需 capture 的负向边界准备后安装 TUN，并仅在当前快照与计划精确一致时激活正向捕获。中继的外部 permit 不要求本机 `exit_role`，也不授予本机 NAT writer。

| API | 行为 |
| --- | --- |
| `EgressCapture::switch(new_plan)` | 原子建立旧／新范围并集的负向 OUTPUT 门，暂停正向捕获，准备新 RPDB／table／路由，再切换活动 TUN selector 并回读；当前执行必须认可新计划，失败保留负向边界 |
| `EgressCapture::is_active()` | 计划已激活且当前执行仍有效；不代表业务许可或数据面已完成安装 |
| `EgressCapture::plan()` | 当前记录的类型化计划副本 |
| `EgressCapture::retire()` | 显式解除所属捕获；先暂停正向表，再精确移除 RPDB 与私有 OUTPUT 表，供明确 down／解除受管范围使用 |

独立的私有 egress routing table 使用 `0.0.0.0/0 → 客户端 TUN`；查表规则仅覆盖编译后的 `managedIPv4`，`directIPv4`／`unmanagedIPv4` 不借最长前缀覆盖较早规则。每个受管范围有正向表缺失时的 unreachable 回退，IPv6 受管范围为 sink，Mesh 路由保持优先。范围列表必须规范、互斥且总数不超过 4096；还受实际可用 RPDB 槽和有界 journal／命令大小约束。

捕获规则只匹配本机来源和非保护 mark。Server／peer 的受保护 socket 继续使用原宿主 RPDB，普通应用连接相同 IP 不获得豁免；出口 TUN 转发也不重新进入本机 capture。`inet meshway_egress_<installation>` 的 OUTPUT 门覆盖 local-table 广播等早于捕获查表的路径，防止其从外部接口直出。本机 local／loopback 服务保留；IPv6 ND 仅允许 type 133–137、hop limit 255、本机或未指定源的链路控制，保护新邻居解析，IPv6 TCP／UDP／echo 字面量不因此放行。

`underlay-egress.json` 保存 boot／namespace、随机 installation、mark／table、计划、当前门与待提交门、设备标识、nft 规范对象／handle、精确 route/rule 及 removed。自有规则／路由变化注册到同一 monitor，不制造外部切网事件。暂停、执行失效和同 namespace 重启保留负向范围；TUN 消失不会落回 main direct。恢复先关闭旧活动 selector 并清理旧正向资源，只有新的有效执行安装可以重新激活；显式 down 才解除所属负向边界。

IPC v5 的公开运行状态：

| 字段 | 含义 |
| --- | --- |
| `networkMode` | `tun`／`controlOnly`，本机选择 |
| `dataPlane` | `controlOnly`、`stopped`、`preparing`、`ready`、`suspended`、`failed`；同一有效实例内来自组件实际安装状态，publication 刷新不自行投影为 suspended。ready 不代替当前执行许可 |
| `authenticated`／`instance` | 当前有效会话与仍持有的有效实例；目标 guard 失效不清空它们。停止、fencing、身份失效、原 session／instance 截止仍使旧快照失效 |
| `instance.executionExpiresAt` | 当前快照执行截止；目标刷新导致旧 guard 失效时为 null，直到新的状态快照发布；不因此隐藏 instance 或延长旧输出许可 |
| `saved`／`applied` | 独立的必填可空目标引用；saved 来自持久保存，applied 来自对应实例／目标的实际成功安装结果 |
| `phase` | 完整枚举见下表；结合 dataPlane、当前资格和错误读取 |
| `lastError`／`retryAt` | 类型化故障与重试时点；安装问题可为 unsupportedMode、underlay、tun、dns、transport、apply |
| `eventStream` | 必填，`connected`（HTTP 已接纳）、`reconnecting`、`disconnected`；不代表 Node 授权或业务连通 |
| `lastEventAt` | 必填可空，本进程最近一次有效事件的接收时间；未收到为 null，保活／读取 status 不改写它 |
| `exit` | 可省略；包含 target-bound `capability`、已安装 role generation、`activeFlows` 和独立的 `dnsUnavailable`。未观测的计数保持 null，失去当前执行权限后不显示已安装角色／活流数 |

`phase` 的全部 9 个值：

| 值 | 含义 |
| --- | --- |
| `down` | 期望下线，已停止当前网络 |
| `starting` | 正在建立控制上下文 |
| `awaitingApproval` | 持钥接入等待浏览器批准 |
| `synchronizing` | 正在同步当前运行目标 |
| `controlReady` | 控制流程就绪；结合 `dataPlane` 判断安装 |
| `running` | 当前目标的数据面已就绪 |
| `retrying` | 故障后等待有界重试 |
| `blocked` | 需要操作员处理，后台不继续准入 |
| `stopping` | 正在停止与清理 |

`lastError` 使用 `Problem` 的全部 17 个值；无故障为 null：

| 值 | 含义 |
| --- | --- |
| `network` | 控制 HTTP 网络或响应交付故障 |
| `serverBusy` | Server 限流、暂态拒绝或不可用 |
| `credentials` | 会话／实例凭据拒绝、身份失权 |
| `enrollmentExpired` | 接入请求到期 |
| `enrollmentWithdrawn` | 接入批准被撤回 |
| `enrollmentCancelled` | 接入请求已取消 |
| `storage` | 私有记录、持久化或恢复失败 |
| `contract` | 类型契约、绑定或控制生命周期不匹配 |
| `configuration` | 本机 Server origin 等启动配置无效 |
| `clock` | 会话／实例期限失效，须重新取得资格 |
| `worker` | 所需运行任务异常退出 |
| `unsupportedMode` | 当前模式或执行能力不受支持 |
| `underlay` | 底层保护或资源准备失败 |
| `tun` | TUN 安装／运行失败 |
| `dns` | DNS 安装／运行失败 |
| `transport` | 承载安装／运行失败 |
| `apply` | 当前目标安装或回执失败，需要重新同步 |

IPC 封闭对象解码拒绝未知字段；当前 v5 支持 Ping 通道的 `localEndpoint` 来源，旧版本请求返回 `ErrorCode::Version`。CLI 与 daemon 同版本发布，不将缺字段补成已连接或零时间。

ready／applied 只表示该目标的本机安装结果，不表示 peer 已建立认证通道或应用已收回业务包。Node State 中的 listener／link 来自组件观测；路径、RTT、丢包缺测保持空。安装与报告顺序见 [Control ApplicationTicket](/v2/node/control#安装回执与运行报告)。

## 业务数据面

统一业务数据面采用 **L3 IPv4／IPv6 IP 包**。系统 TUN 由 OS socket／路由把受管流量送入 TUN，需要 TUN／路由权限及所需 DNS 资源；它不隐含内核模块、内核内加密或 eBPF。当前目标不包含 Ethernet 透明桥接、MAC 学习或二层广播域。

内层 IP 业务与外层承载独立：TCP／UDP 业务不据此强制选择同名 transport，实际承载由配置与当前执行许可约束。本机权限或 TUN 安装失败不能静默切换模式后报告原目标 applied。同一设备一次只启用一张网络。

## CLI 与 IPC

Linux 首版每台机器运行一个系统级 `meshwayd`，同一时间只启用一张网络。CLI 连接该服务；不为每个本机用户创建独立 daemon 或自动复制 Node 身份。当前使用 owner-only 目录与 `0600` socket，在解析请求前后核验内核提供的 peer UID，只允许 root 或 daemon 同 UID。更细的操作员授权与系统服务安装策略仍待实现。

| 命令／操作 | 行为／状态 |
| --- | --- |
| `meshway up` | 首次浏览器接入，或用保存的身份重新认证并取得运行资格 |
| `meshway up --enrollment-key @file` | 通过 Server 的一次性 Tag 接入密钥加入网络，不取得用户会话；既有身份不被接入密钥改绑 |
| `meshway rotate-key` | 同步返回已完成换钥的公开身份／状态或类型化失败；随后重新取得实例和运行资格，换钥成功不等于数据面已 applied |
| `meshway down` | 停止当前网络并终止相关资格；daemon 与私有身份保留 |
| `meshway ping <target>` | 获准的端到端 Mesh 探测；参数、授权与输出见[路径诊断](/v2/node/diagnostics) |
| `meshway status` | 区分进程、控制联系、saved／applied 和实际通道状态 |
| 切网 | 尚未实现；先停止旧网络、完成隔离清理，再运行新网络 |

CLI 通过 Unix domain socket 发命令、读结果；状态由 daemon／组件的唯一 owner 持有。`meshway_protocol::ipc` 是唯一的请求／响应定义，当前版本为 5，使用 JSON 与长度前缀，单条消息上限 64 KiB。`exit` 为新增可省略字段，保留版本 5；CLI 和 daemon 仍同版本发布。每个连接处理一个请求，Ping 可返回有序的多个事件；连接数、请求等待、写入和队列均有上限。`Up` 的 `enrollmentKey` 可省略；`Up`／`Down`／`RotateKey` 的 `expectedRevision` 由 CLI 读取后提交，换钥不会绕过本机 CAS。

推荐的无交互接入与换钥用法：

```sh
meshway up --enrollment-key @/absolute/private/enrollment-key
# 或由秘密提供者写入 stdin，输入结束后提交：
cat /absolute/private/enrollment-key | meshway up --enrollment-key -
meshway status --json
meshway rotate-key
```

接入密钥输入统一走 IPC 持久命令；daemon 不另提供重复的 `--enrollment-key-file` 启动源，避免启动参数与已接纳意图的恢复顺序冲突。换钥处理预算为 65 秒、CLI 等待 70 秒；超时／Ctrl-C 不取消已接纳操作，先通过 status 的公钥 generation 检查结果再决定是否重试。原始接入密钥、私钥和 token 不进入状态、错误分类或普通日志。

up/down 按本机期望状态 revision 执行 CAS，并保留最后一次变更的请求身份以恢复不确定结果。本机 revision 与 Server 配置 revision 分开。已接纳命令的持久化由协调器完成，CLI 断开不会撤回它；读取 status 不写入状态或生成测量。

daemon 独占目录锁，启动时记录 socket 的文件身份；只有身份匹配且确认无监听者的旧 socket 可清理。未知文件、符号链接、活跃 socket 和损坏的状态文件都保留并报错。

CLI 生命周期不承载 Node 身份或 lease，不收集账户密码、不取得浏览器会话。业务 listener／DNS／策略无本地覆盖源；获准路由方案切换仍是使用选择，具体入口待定。`down` 不等于云端永久删除。

## Engine

`meshway-engine::Engine` 是 Node 的唯一期望状态 owner。sync 验证并保存的 netmap 与租约经独立通道直接送达网络 owner（不经 Control 锁与维护调度），网络 owner 每收到一次调用一次 `reconfig(Some(&netmap))`；撤回时调用 `reconfig(None)`。Control 的维护 HTTP 按单个操作持锁，sync 回复最多等待一个进行中的操作。

| 输入 | Engine 行为 |
| --- | --- |
| 新内容（更大 serial） | 构造一次不可变 `Desired`（内容与共享 `LocalCompiler`），按 Host → 邻接 → SecureNode → DNS 顺序调用各组件幂等 `apply`。安全相关步骤（capture／TUN、负向 DNS、circuit 与流收窄、出口退役）失败时对全部组件应用空期望；只缺新增能力（listener／拨号 socket、邻接 Manager、出口角色、DNS 查询应答）时组件返回 `Degraded`：内容保持安装、对应能力缺失并上报，回执不报 applied，有界重试同内容 |
| 同内容、不更早到期的租约 | 只延长自身计时器，不调用任何组件；当前为 `Degraded` 时重新应用同内容。Server 时钟回拨造成更早的 `issuedAt` 不算重放 |
| 更早到期的同内容租约或回退引用 | 拒绝，当前期望不变 |
| 计时到期（`next_expiry`／`expire`） | 按 SecureNode → 邻接 → Host → DNS 顺序应用空期望；已到期内容须由新租约重新安装。单调计时在宿主挂起期间不走，网络 owner 每秒的例行唤醒另按墙钟检查到期 |

各组件 `apply`：

| 组件 | 签名 | 空期望 |
| --- | --- | --- |
| Host | `Host::install(Desired)`；同内容为空操作，同一节点新内容保留 TUN 与用途不变的描述符（用途只含声明与 peer 登记密钥）；进行中的连接、穿透与 DNS 交换不因内容变化中止，只有其注册用途在已安装内容上改变时才结束 | `Host::suspend`：结束全部操作与描述符，保留负向 capture 边界；exit 角色随之撤销并精确清理 CT |
| 邻接 | `Manager::apply(&Desired)`；保留 peer 密钥与承载顺序不变、仍准入且资源用途不变的已选通道与资源（含已建立的退路承载）；资源保留规则与 Host 共用同一实现 | `Manager::shutdown` |
| 数据面 | `SecureNode::apply(&Desired) -> Applied`；保留授权 digest 与路径密钥不变的 circuit、决策不变的流 | 丢弃 SecureNode：取消全部 circuit、流与排队输出 |
| DNS | `Attachment::install(Desired)`，经 `publish_scope` 发布查询范围 | `publish_scope(None)`：立即结束旧 ticket、路由与未发送回答 |
| 诊断 | `Owner::synchronize(&Desired, instance, tasks)` | `Owner::clear` |

已建立 circuit 在双向 90 秒无帧时结束；源端连续发送 15 秒未收到任何认证帧时结束并在下一个包重新握手，端点以 10 秒被动 keepalive 应答单向流。网络 owner 每轮把 Manager 的 `peer_boots()` 交给 `SecureNode::set_peer_boots`：相邻对端换了 boot（重启）时结束绑定在它上的 circuit，中继返回的 `Close` 由同一发送路径送往存活一侧；同一 boot 的承载更换（例如 UDP 失效后改用 TCP 退路）不结束 circuit。行为见[数据面](/v2/node/dataplane)。下一跳暂无承载时登记邻接需求，包在有界首包队列中等待承载恢复。

内容变化只丢弃已在旧内容上准入的输出；尚未准入的已接收帧（含握手帧）保留，由 SecureNode 按当前内容逐帧准入。

## 控制调度与恢复

Server HTTP 等待与本机命令处理独立。配置查询定时兜底、实例续租与控制报告通常每 20 秒；本节点实际执行目标／资格变化的事件立即唤醒既有控制周期，周期进行中的重复提示由单槽队列合并，已由 RPC 取得的同一配置快照提示不再触发刷新。真实安装完成、socket／邻接或业务依赖变化还会触发合并后的报告，最多每秒一次。仅观测时间变化不触发新事件，旧报告重试保留原序号／内容／时间，也不确认后来出现的新状态。故障采用有界退避，期限由控制组件独立校验。down 先持久保存本机下线意图，再停止当前资格并尽力释放远端实例；离线时也不能把旧 lease 展示为有效。当前 IPC 不承载业务包。

daemon 同时持有一条受保护的 Node SSE 消费任务。本节点目标、revision、实例、可见密钥或成员集合变化及需要重新同步的 reset 唤醒上述既有控制周期，重复提示合并；普通 peer State、观察投影和重连成功不触发 apply。定时刷新作为断线与通知遗漏的兜底。事件消费者不直接安装配置、续期或授权；关闭原因、重连预算与旧事件范围检查见[事件通知与刷新](/v2/node/control#事件通知与刷新)。已有待确认换钥／接入密钥记录在认证前由同一 Control owner 恢复，发起入口限制见[换钥与恢复](/v2/node/control#换钥与恢复)。

已知目标变更的通知、报告 scope 轮换或刚确认的新诊断目标优先执行刷新，排在旧 scope 的 traversal／结果网关请求之前；刷新成功后恢复有界网关调度。通知本身不赋予新目标权限。

期望上线时，网络失败、HTTP 408／429／5xx 按 1–30 秒有界退避自动重试。不可解码或超限响应会撤回旧运行上下文后重新取得资格；运行中的 session／instance 拒绝、lease 到期或实例更换会重新持钥认证、取得有效 instance 并上报 State，不要求再次执行 `up`。新鲜身份证明遭拒、Server（`controllerId`）／key／目标绑定错误、本机记录损坏仍进入 `blocked`；后台残留请求不能绕过该停止状态。

Linux 运行中的外部接口删除／重建、地址变更及路由变化由既有 Underlay netlink 监视器通知。变化关闭旧控制 HTTP 的底层连接，后续请求按当前路由重新连接并沿用上述有界重试；不要求重启 daemon、再次 `up` 或修改配置。本机端点变化经既有合并报告路径发布，刷新规则见[连接获取](/v2/node/connectivity#stun-与-server-协调)。只有 kernel 来源的 netlink 通知参与资源归属检查；用户态 multicast 消息被忽略，不能把普通接口重连变成永久保护冲突。当前 namespace 中真实的所属 TUN／DNS 锚点或保护规则冲突仍阻断执行。

Underlay 启动时当前 namespace 的资源归属或路由保护冲突仍拒绝准入，保留现有资源；状态为 `retrying`／`lastError=underlay`，以同一 1–30 秒有界退避发布 `retryAt` 并重新检查。冲突 owner 释放资源后无需再次 `up`。损坏的 Underlay journal／私有存储错误归为 `storage`，安装参数或 bootstrap resolver 配置错误归为 `configuration`，仍须修正后恢复。

控制失败日志包含实际 operation、错误类别、HTTP status、已解码的 problem code、Server request ID 和恢复决策；不记录响应原文或凭据。显式 `down` 的清理若失败，在外部冲突解决后重新执行 `down` 会触发新的清理尝试，同请求重放仍保持幂等。

`state_scope_changed` 表示报告在交付期间遇到正常 scope 轮换：丢弃当前报告，1 秒后刷新并重新采样，不进入 `retrying`、不设置 `lastError=apply`，保持当前 phase 与 SSE 消费任务。publication、execution lease、saved 前置条件或回执内容冲突按[回执分类](/v2/api/problems#node-配置回执冲突)退役精确回执，1 秒后刷新并重新 apply 当前目标。两者均不 reset instance 或主动 suspend／删除 TUN。实际执行截止、撤回和连续性交接检查独立生效；旧回执、输出与报告不换绑新目标。

自有负向规则、私有表路由以及 TUN 的创建／退役不会作为外部网络变化通知。TUN 关闭产生的延后地址／链路事件保留精确接口索引至事件排空，仅用于抑制自触发通知，不形成额外的资源归属或捕获豁免。suspended 期间的 Mesh `EHOSTUNREACH`／`ENETUNREACH` 是负向边界的实际结果，不触发控制连接重建或端点重枚举。

## 持久与重建

| 私有保存 | 重新建立 |
| --- | --- |
| Server 信任绑定、Node 密钥、待完成接入／换钥 | 短期会话、instance、执行 lease |
| 完整已保存目标与 digest | 实际安装结果、通道、打洞和包会话 |
| 所选网络、期望上线及有效使用选择 | 在线／可达性证明、计数基线 |
| 系统资源归属 journal | 已认证的资源状态与运行代际 |

缓存只作恢复基准，不复活旧授权。重启、释放或过期后使用新 instance；配置历史不能覆盖永久吊销。运行目录／IPC 地址是安装参数，不是另一份网络业务配置。

## 生命周期

```text
取得私有目录／IPC → 装配组件与 provider → 接受上线目标
  → 持钥接入与执行目标 → 校验并保存（saved）
  → 先准备底层保护，再安装组件 → applied／failed 与真实 State
```

宿主协调一个活动网络及有效执行上下文，组件拥有其任务、buffer、socket 和系统资源。具体运行库不重复编译网络策略；saved／applied／业务连通的含义见[配置应用](/v2/configuration-reference/deployment)。

撤回、期限和 down 独立于挂起的 HTTP 生效。停止准入后取消并等待任务退出；显式 down 清理可证明所属的内核资源与 TUN，期望 up 时的过期／断连／退出保留负向 capture 边界。资源生命周期及 bootstrap 同 IP 隔离见 [Linux underlay](/v2/node/dataplane#linux-underlay)。

崩溃恢复只处理有可验证归属的 journal，不恢复旧流。未知格式或不明归属不能作为删除宿主规则的依据。journal 记录写入时的内核 boot id 与网络 namespace（device／inode）：任一身份不同（整机、容器虚拟机重启或容器重建后挂载同一卷）即为过期记录，在分配新资源前原子改名为 `underlay.json.stale-<Unix 纳秒时间>` 并持久保留；INFO 日志记录旧 namespace 与归档名。按无 journal 在当前 namespace 重新分配，不进入旧 namespace 清理，也不把旧记录作为当前资源的归属证明。归档失败时不覆盖原文件或继续分配。同 boot、同 namespace 仍按原有逐项归属核验恢复，当前 namespace 的真实冲突使用上述有界退避。

SIGTERM／SIGINT 干净退出且期望仍为 up 时，先取消和 join 运行任务，经 `suspend` 删除本进程的正向路由、关闭非持久 TUN，并清空、持久保存对应 journal 记录；负向 capture 规则及独立 DNS 锚点继续保留以供同 namespace 重启恢复。因此 `Node stopped failed=false` 不表示 journal 应整份删除。异常退出保留的可核验资源仍走恢复检查；namespace 变化使用过期归档规则。

## 规划：userspace networking

以下为规划，不是可用功能，`--network-mode` 不接受 userspace：

- 无 TUN，应用通过本机 SOCKS5 接入，由用户态 netstack 处理流量，并接入同一 dataplane，共用身份、准入、端到端保护、完整路径与回复规则。
- 本机 SOCKS5 使用当前网络的业务权限、DNS 和路由，不能作为绕过准入的直拨通路；不包含应用内 socket SDK 或公网代理服务。
- 命令范围、监听地址／端口、本机访问认证、域名解析、UDP 关联生命周期、MTU／分片与出口能力尚待定版。
