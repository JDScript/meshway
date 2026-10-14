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

`meshwayd` 前台运行，SIGINT／SIGTERM 执行关闭与 join。进程退出保留期望状态：退出前为 up，重启后自动重新取得运行资格；显式 down 后重启继续保持下线。CLI 的 Ctrl-C 仅停止等待，不撤回已由 daemon 接纳的持久操作。`up`／`down` 最多等待 30 秒，期间 `starting`／`synchronizing`／`stopping`／`retrying`／`leaving` 视为仍在收敛；期满仍为 `retrying`，或进入 `blocked`，以失败退出并显示最近问题。

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
| CLI `up --reenroll` | 以接入意图 `existing` 重新接入当前身份（同 Node ID 与密钥，须 Console 批准），用于错过签发钥轮换窗口后的恢复；与 `--enrollment-key` 互斥，见[保留身份的重新接入](/v2/node/control#保留身份的重新接入) |

默认目录适用于 Linux 系统服务；本地开发或其他 Unix 环境可像示例一样显式指定私有目录。安装参数不构成本机业务配置覆盖源。

### 系统服务

Linux 发行包以 systemd 服务 `meshwayd.service` 运行：`Type=notify`，daemon 在本机 IPC socket 绑定后发送 `READY=1`、开始停止时发送 `STOPPING=1`（`NOTIFY_SOCKET` 不存在时不发送，前台与容器运行不受影响）；状态目录 `/var/lib/meshway`、运行目录 `/run/meshway`（跨 daemon 重启保留），日志为 journald 中的 JSON。`meshway version` 输出程序版本、git 描述、IPC、circuit wire 与 netmap 编译契约版本，不联系 daemon。安装、unit 取值、升级、卸载与重启／resolver 间隙实测见 [Linux 安装与系统服务](/v2/install)。

daemon／CLI 共用[运行日志参数](/v2/logging)，日志位于 stderr；CLI 的 `--json` 状态结果位于 stdout。

本机多节点开发使用[三节点 Docker 环境](/v2/node/docker-lab)，统一投放预编译程序，并为每个 Node 保留独立状态卷。

## 安装范围与状态

当前 TUN 候选由 daemon 在唯一 [Engine](#engine) 后装配 `meshway-underlay::Host`、`meshway-connectivity::Manager`、`meshway-dataplane::secure::SecureNode` 与 DNS 查询 owner。TUN 源包在期望 netmap 上本地准入，不联系 Server。原路径回复查询 SecureNode 的唯一流表，不另建 daemon 反向流表。

| 范围 | 当前行为 |
| --- | --- |
| Linux Mesh-only | TUN capture、Native TCP／Native UDP 静态端点认证邻接、协调 UDP 穿透、双栈 UDP／TCP 多跳包流、持续续租和失效清理；验收状态见[验收范围](/v2/node/validation) |
| DNS | 已接入 loopback 服务、精确 resolver 需求、resolved／显式文件代理；所有必需组件安装成功才可 applied，后端约束见[系统 DNS 安装](/v2/node/dataplane#系统-dns-安装) |
| Exit／本机 egress | daemon 按已应用 netmap 探测并安装 IPv4 出口角色、精确 NAT 投影和源端 capture／IPv6 sink；回包由同一 SecureNode 按原完整路径返回。capture、承载及必需的负向 DNS 资源共同进入本机 applied 边界；出口角色是本节点为其他源提供的能力，其前置条件不满足或安装失败只报告 `exitCapability: unavailable`，不阻断 applied（见[出口宿主前置探测](#出口宿主前置探测)）。源节点 DNS 经出口 resolver 解析（[出口 resolver](/v2/node/dataplane#exit-resolver)），出口 resolver 不可用只对源 SERVFAIL 并单独报告 `exit.dnsUnavailable`；完整多进程验收范围见[验收范围](/v2/node/validation) |
| 诊断 | `meshway ping` 的 Mesh 目标与 Server observation tasks 共用获准测量器；公网目标的 echo 是经同一 SecureNode 的普通本地包，见[路径诊断](/v2/node/diagnostics) |
| 承载 | Manager 共用获准的 stream、Native UDP 静态端点和协调 UDP 穿透；会话与交接见[邻接管理](/v2/node/transport#邻接管理)，NAT 范围见[连接获取](/v2/node/connectivity) |
| 其他平台／入口 | 非 Linux 的 tun 返回 unsupported；userspace（见[规划](#规划-userspace-networking)）、动态插件和切网尚未实现 |

仅验证 Mesh IP、无需系统 DNS 的场景，在 Server 的 [DNS 配置](/v2/configuration-reference/dns)中明确设置 `magicDNS=false`、`overrideLocal=false`，并保持 `servers`、`default`、`routes`、`searchDomains` 为空。此时不创建 DNS 服务／锚点，也不要求机器运行 resolved。存在 DNS 需求时须完成对应安装；本机后端选择不覆盖云端 DNS 业务配置。`control-only` 不安装系统 DNS。

### 出口宿主前置探测

Linux `Host::exit_probe` 在打开 Host 前执行，独占本进程的 Host 预约。探测读取 `ip_forward`、实际接口与临时出口 TUN 的 `rp_filter`／`src_valid_mark`，实际创建并关闭非持久 TUN，并通过固定路径 `/usr/sbin/nft` 的 JSON 原子事务检查 timed set、original conntrack tuple 与 ct mark 表达式；经 ctnetlink 执行有界的 IPv4 mark 过滤 dump（内核按 mark 过滤，返回项仍逐项复核，不作身份或授权证据）；已加载 legacy iptables 表时以 `/usr/sbin/iptables-legacy-save` 只读其内容。探测不修改宿主 sysctl 或防火墙策略，临时 nft 表在同一事务中创建、删除，取消后的命令结果经 join 与表列表回读处理。

`ip_forward=0`，或新建 TUN 继承的 `forwarding`（来自 `conf/default/forwarding`，临时 TUN 上读取）为 0，返回 `ipForwardDisabled`；读取失败或格式错误返回 `forwardingProbeFailed`；实际 TUN 管理操作无权限返回 `netAdminMissing`。有效 `rp_filter=max(all,interface)` 为 strict 时返回 `reversePathFilterConflict`，不会自动改为 loose/off；loose（2）与 off（0）可用：出口 TUN 没有 IPv4 地址，内核对它的入包做反向路径检查时只在反向查找本身落到该 TUN 才放行，回程表的成对规则因此不带出口 mark，正是该反向查找（见[IPv4 出口角色与 NAT 投影](#ipv4-出口角色与-nat-投影)）。nft 不可用返回 `nftUnavailable`；ctnetlink 不可用（无 `nf_conntrack_netlink`、不支持 mark 过滤、回答畸形或超时）返回 `conntrackUnavailable`，内核以 `EPERM`／`EACCES` 拒绝返回 `netAdminMissing`。观测保留原始 `observedAt`，重试上报不得重新计时。

此前置探测成功仍为 `probing`，`supportedFamilies`／`installedFamilies` 为空。探测与下面的角色安装是不同操作，不能据此前置探测发布 `ready`／`applied`。宿主管理员预配置 forwarding 和提供工具／设备不等于完整出口功能已可用。

已打开的 Host 使用 `probe_exit(&mut self, instance: InstanceId, netmap: NetmapRef, role_generation: Digest) -> ExitCapability`。它使用真实、处于 up 状态的外部接口，排除所属 Mesh／出口 TUN 和 DNS 锚点；临时设备的精确 index 注册到同一 monitor，创建／删除不会制造外部切网事件。探测检查实际 TUN 权限、转发／反向路径条件、完整 IPv4 nft 程序和 ctnetlink 读取；排除经 owner 回读确认的自有表后，可能挡住出口流量的宿主规则返回 `firewallConflict`，未知 mark 修改／offload 返回 `ownershipConflict`，不改写宿主规则。

宿主规则只在**能匹配出口流量**时冲突。判定（`exit/firewall/` 的唯一实现）对 `ip`／`inet` 族的 nft 规则和已加载 legacy iptables 表的内容做同一遍历，考察的包为：从出口 TUN 进入与从出口接口进入的回包（prerouting 钩子，含 raw／mangle／nat 等各优先级）、经出口 TUN 转发到出口接口及其回包（forward、postrouting 钩子；出口接口为主路由表中通往 Internet 分类地址的 IPv4 路由的接口，即默认路由与直连公网网段，只通往私网的路由如 Docker 网桥不在内）、本机发往出口 TUN 的 ICMP 错误（output 钩子）。安装前的探测把出口 TUN 视为新建的内核 `tun<N>`（名字不同于任何现有接口）；已安装角色的周期探测按实际出口 TUN 名精确匹配。

| 规则 | 判定 |
| --- | --- |
| `drop`／`reject`／`queue`／`tproxy`／`dup`／`fwd`，legacy 的 `DROP`／`REJECT`／`NFQUEUE`／`QUEUE`／`TPROXY`／`TEE` | 其匹配可能命中上述任一包即冲突；经 `jump`／`goto`（legacy 为自定义链目标）可达的链同样遍历 |
| 接口约束（`iifname`／`oifname`／`iif`／`oif`，legacy `-i`／`-o`，含 `!`、集合与 `*`／`+` 通配） | 按实际接口判定：只匹配其他接口（如 Docker 的 `br-*`／`docker0`）的规则不命中出口流量；`tun*` 命中出口 TUN |
| `ct state` 只含 `invalid`／`untracked`，`meta nfproto ipv6` | 不命中（出口流量为 IPv4 且被跟踪） |
| 目的地址（`ip daddr`，legacy `-d`，含前缀与集合） | 转发方向的目的是 Internet 分类地址，回包方向是出口接口地址或 Mesh 地址池：与之不相交（如 Docker 为发布容器加的 raw `-d <容器> ! -i <网桥> -j DROP`）即不命中 |
| 其他匹配（源地址、端口、`limit`、xt match 等） | 视为可能命中 |
| prerouting／forward／output／postrouting 基链 `policy drop`（legacy 内建链 `DROP`） | 出口流量不能确定在此前被同链 `accept` 时冲突 |

因此 Docker（`DOCKER-ISOLATION-STAGE-*`、`DOCKER` 链只丢弃进出其网桥的包）与出口共存，Docker 把 `FORWARD` 置为 `policy drop`（Docker 自己打开 `ip_forward` 时的默认）、ufw 默认 forward drop、firewalld 区域转发拒绝和不限接口的 drop 仍冲突。input 钩子与 netdev／bridge／arp 族不在判定范围；它们若丢弃出口流量，表现为下面的逐流路由判定拒绝或 conntrack 确认失败。遍历按（链, 包）记忆结果，共享子链不会成倍展开；一次判定最多评估 2^20 条规则，超过即 `firewallConflict`（无法判定）。

live probe 成功返回 `ready` 和 `supportedFamilies=[ipv4]`，`installedFamilies` 仍为空；其含义是所报后端前置条件完成，不代替角色安装或业务连通证明。失败携带具体原因。返回值保留传入的 instance、`netmap` 与 `roleGeneration` 及本次真实 `observedAt`，调用方不自行把 `probing` 改成 `ready`。

已应用的 netmap 带本节点 `exitRole` 时，网络 worker 在应用后调用 live probe。该 Future 的 monitor 预约不可跨线程移动，因此唯一网络 worker 在一个固定线程上驱动，并在停止时显式等待退出；Host、Control 和内核资源仍各只有一个 owner。出口前置条件不满足或角色安装失败时，内容照常 applied：`exitCapability` 为 `unavailable` 及具体原因并上报 State，IPC Status 显示 `Exit unavailable (<原因>)`，peer 与本节点作为源的流量不受影响，也不进入 apply 重试。能力探测按下文周期重新真实探测；观测为 `ready` 而当前内容的角色尚未安装时，网络 worker 以同一内容重新 apply（`Engine::reapply`），由此安装角色。实例在内容应用之后才取得（启动时从保存的 netmap 恢复）时，取得实例即探测一次。只有角色实际安装后才填充 `installedFamilies`。

**运行中的角色失败只撤该角色**：已安装角色的 Host owner 因自身内核操作失败（`maintain`／`retire`／审计）或其自有对象在 owner 之外被删除（见下文 monitor 分类）而停止时，出口 driver 报告 `ended`，网络 worker 只结束出口 runtime：SecureNode 退役全部出口流、各投影随 CT 精确清理，日志 `Exit role failed; withdrawing it alone`；netmap 保持 applied，capture、DNS、邻接、控制中继与本节点作为源的流量不变。随后立即真实探测一次：`ready` 即以同一内容 `Engine::reapply` 重装角色；清理失败（资源保留）时探测报 `ownershipConflict`，该角色在下次进程启动的 journal 恢复之前保持不可用。被保留的残留（关闭的门、已消失的出口 TUN 上带 `IIF_DETACHED` 的规则）仍记在出口 journal 与 monitor 期望中，不使 Host 的 capture 校验或新内容安装失败；撤回一个仍在服务的角色失败仍是失败。

出口职责只来自 Server 签名的 netmap：`exitRole` 绑定内容（`generation`），不绑定 instance，重新取得 instance 保留已安装的角色。`NodeState.exitCapability` 是独立观测：失败可在尚未 applied 时报告，观测绑定当前 instance、持有的 `netmap` 与其 `roleGeneration`；Control 在发送 HTTP 前拒绝 instance 或 netmap 已变化的观测（`StaleReport`，丢弃该报告而不改绑）。不确定交付的重试保留原内容、序号和 `observedAt`，不把旧探测换绑到新 netmap。能力报告不生成应用回执，也不替代必需组件的原子 applied 边界。netmap 带出口职责期间，网络 worker 在每个事件上检查该观测：超过 State 报告寿命（60 秒）的三分之一即重新真实探测，持续业务流量不推迟刷新。

### IPv4 出口角色与 NAT 投影

Linux `Host::install_netmap_exit_role(&ExitRole)` 消费已应用 netmap 中本节点的出口职责（`meshway_configuration::netmap::ExitRole`），返回 Underlay 的 `ExitRole` handle。资源仍归该 Host 的唯一 Kernel owner；角色使用独立非持久 TUN，不暴露 FD。crate 根导出 `ExitRole`、`ExitScope`、`ExitCutoff`、`ExitIncarnation`、`ExitSendGuard`、`Invalidated`、`OriginalTuple`、`ExitTransport`、`ProjectionAck`、`CtConfirmation`、`ExitReplyReader` 和 `ExitReply`。

| 操作 | 行为 |
| --- | --- |
| `install(incarnation, original_tuple, deadline, scope)` | 拒绝已有冲突 CT／tuple／incarnation；安装回程路由与成对规则后，向内核取该对 forward 包的实际入路由判定（入接口为出口 TUN、带出口 mark，不发包），不是单播转发路由即拒绝该流，并在日志中指明原因（`rp_filter`、无转发路由、martian）；安装精确 timed set 资格并回读，返回 `ProjectionAck`，此后才允许提交首包 |
| `send(incarnation, packet, guard: impl ExitSendGuard)` | 每个排队包独立持有原 guard 与入队时的投影；每次 TUN 写 future 的 poll 前同步检查该 guard，再受原截止／撤回／tuple 约束；不使用最新 Live 投影替换排队包的权限 |
| `confirm(incarnation)` | 首包产生 CT 后，按 IPv4 original tuple＋归属 mark 检查，返回 `CtConfirmation::Confirmed`；有界等待不超过三秒及流截止，缺失或冲突时先关闭资格，再返回 `ConfirmFailed`；清理失败返回错误 |
| `renew(incarnation, deadline, scope)` | 仅在旧资格仍活、绑定相同且已确认 CT 身份仍一致时原子续期 timed set；不创建新的 CT，不复活过期投影 |
| `retire_flow(incarnation)` | 先关闭该资格和用户态输出；其它活 ICMP sequence 仍引用同一 CT 时保留共享映射，最后一个引用退役后按 family／original tuple／mark 精确删除并读回；释放不再共用的回程规则／路由 |
| `reply_reader()` | 单次取得独立回包 reader；`recv()` 返回 incarnation 与逆 NAT 包，排队期间继续保留原 guard；业务回复仍须通过唯一 dataplane 流表与原完整路径准入 |
| `retire()` | 停止正向 I/O，关闭全部资格、清理 CT／回程资源，join TUN owner 后移除所属 nft 表；失败保留 journal 和拒绝边界 |

`ExitScope` 携带 dataplane 提供的授权／路径 binding 与不可逆撤回 token，不签发业务权限。原 tuple 仅支持 IPv4 TCP、UDP 和 ICMP echo；角色不能替代精确源地址、目的、服务和活 flow。投影最多 128 个活项；未退役投影记录与 quarantine 合计最多 4096 项，容量耗尽只拒绝该新增流，已安装投影与角色不受影响。每个 ICMP echo 请求是一条流（键为 `(id, sequence)`）：等待回复或其 ICMP 错误至多 10 秒，收到回复后 2 秒结束（每一跳的流表相同，见[数据面](/v2/node/dataplane)），出口投影随之退役；一个 5 次／秒的 `ping` 因此只占约 10 个投影，traceroute 的探测各占至多 10 秒。按序号的 quarantine 使持续 echo 速率另受 4096 ／ 300 秒 ≈ 13 次／秒（全出口合计）的约束。

**quarantine 生命周期**：流退役时其 CT 精确删除，转换后的 reply tuple 进入 quarantine（nft `quarantine`／`echo_quarantine` 集合，同时写入 journal），拒绝该 tuple 的迟到回包；投影记录随之离开历史，同一原 tuple 此后可由新的 incarnation 重新安装。quarantine 条目至少保留 300 秒（长于被删 NAT 流迟到回包的 conntrack 超时），维护时清除超过 900 秒者；新增流遇到容量上限时先清除已满 300 秒的条目，仍满则拒绝该流，不逐出未满 300 秒的条目。journal v1 恢复的转换 tuple 记入单独的 legacy quarantine，不按寿命清除。

**echo 连接的 CT 换代**：内核对同一原 tuple 只有一条 conntrack；同一 echo id 的各序号投影共享它。该条目在仍有序号投影存活时结束（ICMP conntrack 超时 30 秒，或被删除），后一序号的包新建一条（新 id）。这不是归属冲突：退役记录旧条目的投影时，若当前条目由同一连接另一条未退役投影持有或尚待确认，则旧条目已证明不存在，该投影按自己记录的 reply tuple 与序号进入 quarantine，不删除当前条目；安装新序号时同样只要求当前条目可归属于该连接的某条投影。当前条目不能归属任何投影时仍为冲突（owner 停止，按上文只撤角色）。

**monitor 对出口自有对象的分类**：Underlay monitor 观察到出口自有对象（出口回程表内的路由、出口规则，含已脱离 TUN 的规则）在 owner 之外被删除时，只停止出口角色：owner 退役全部投影并精确清理（已不存在的路由视为已删），随后按上文重装；Host 的 capture、DNS 锚点与保护不受影响。出口表内出现外来路由、出现未知规则等无法归属的变化仍为整个 Host 的保护冲突（fail closed）。冲突日志给出原因与 `table`／`interface`／`prefix`。

incarnation 的类型为 `ExitIncarnation`：`new([u8; 32])`／`bytes() -> [u8; 32]` 原样保留全部 32 字节，不截断成 `u128`。`ExitSendGuard: Send + Sync + 'static` 由 Underlay 定义，唯一方法为 `check(&self) -> std::result::Result<(), Invalidated>`；X4 用 X2 原包的 `exit_scope()` 实现，Underlay 不依赖 dataplane。

`ExitTransport::IcmpEcho { identifier, sequence }` 按两字段独立投影。同 identifier 不同 sequence 可以共存并共享内核 CT；读回包也精确匹配 sequence，退役一个不会撤销另一个的资格。ICMP 使用独立 timed set；固定 nft 版本通过规则内的 32-bit 字读取、掩码和恢复实现 sequence 检查，未知输入 mark 拒绝，成功返回前恢复专用路由 bit，临时值不交给外部 hook／RPDB。这只实现内核资源共享，不增加业务流准入规则。

角色安装使用 `inet meshway_exit_<installation>` 与 `ip meshway_exit_nat_<installation>` 私有表。只有独立 TUN ingress、Mesh 源、精确活投影、NEW 状态且归属 bit 尚未设置时，FORWARD 才设置 CT mark；已归属 NEW／ESTABLISHED／RELATED 必须继续匹配投影，未归属 ESTABLISHED 不接管。NEW 重传仍受同一资格约束。PREROUTING 只准备专用路由 bit，回程要求 reply direction／原 tuple／归属 mark；最终 postrouting 再检查资格。原方向执行 masquerade，回程 RPDB 将逆 NAT 后的源节点 `/32` 导向出口 TUN。正向绕过客户端 capture 的规则同时匹配出口 iif 与资格生成的 mark。每对（远端、源）的回程规则 `from <远端>/32 to <源>/32 lookup <回程表>` 不带 mark，同时是内核对该对 forward 包的反向路径查找；带 mark 的配对 `unreachable` 规则在表中无路由时终止回程查找。不带 mark 而走到出口 TUN 的包没有归属 mark，被 forward／final 链的 `oif` 规则丢弃。

Mesh 池、loopback、链路本地、组播、保留／广播及出口本机地址不能作为外部转发目的。IPv6 出口拒绝。宿主必须提供实际可用的 TUN／NET_ADMIN、固定路径 nft、内核 ctnetlink、`ip_forward=1` 和已支持的 loose/off 反向路径校验（宿主前提见 [Linux 安装 · 出口宿主](/v2/install#出口宿主)）；无法证明 mark 写入或 flow offload 不冲突时停装。**不要求或修改 `nf_conntrack_acct`**，计数不用于 CT 归属证明。宿主原有防火墙仍可拒绝包。

所有 nft 变更通过结构化 JSON 原子批提交；工具执行每次两秒预算，全局命令槽最多等待一秒，输入和各输出管道各 8 MiB（容纳 2 × 16,384 个区间的暂存捕获并集及其回读），进程并发与请求／包队列均有界。timeout 使用秒，按原单调／墙钟截止减去预算保守取整，实际资格可提前到期。续期在异步检查后再次核验旧资格，ACK 不延长原 guard。取消或丢失响应后 join 并读回核验，不把工具被杀视作回滚。

私有 `underlay-exit.json` 记录 boot／namespace、基础与出口 installation、节点／网络／职责、TUN 名称与 index／MTU、mark／回程表、精确 route/rule、nft handle 与规范规则、投影截止／CT 身份／隔离 tuple，以及 pending／closing／removed。`exit-child.json` 在工具执行前记录 PID／启动时间／network 与 PID namespace，阻止仍存活或不可判定的旧工具与恢复竞争：另一个 network namespace（boot、inode 或 cookie 不同，见[生命周期](#生命周期)）的记录归档，同一 network namespace 而 PID namespace 不同时无法证明工具已退出，仍为冲突。同 namespace 重启先核对资源并关闭旧投影、精确清除 CT；跨 namespace 记录归档，不操作原 namespace。所有权冲突保留证据并停装。

出口 journal 当前为 v2，保存完整 incarnation 与 ICMP sequence。v1 只进入精确清理路径，不恢复活投影；旧 128-bit handle 完整保留用于追溯，旧记录没有 sequence 证据时保留较宽的转换 tuple 拒绝记录，不猜测 sequence 或放行重用。

撤回 ACK 证明后续穿越资格门的包被拒绝；此前已通过最终过滤、排入 TUN／网卡的包无法追回。源端默认 capture、IPv6 sink、切换、出口 DNS 与正常 daemon 的整体验收状态以[验收范围](/v2/node/validation)为准。

### daemon 出口装配与 applied

出口驱动 FIFO 最多持有 128 个操作／原始包包装；先排入 SecureNode 的 Install／Renew，再排入依赖该 ACK 的包。驱动使用完整 256-bit incarnation、原 tuple、原单调／UTC 截止和同一 Control clock；先完成 Host 投影，再完成 dataplane ACK，等待 `projection_ready`，将每包 `exit_scope` 保留至最终 TUN write poll。首包后的 CT 确认失败调用 `fail_exit_projection`，只关闭对应 flow；回程 reader 输入 `exit_reply`，依赖续期的回复等待 ACK 后才进入既有 carrier 发送路径。

驱动只记录已安装投影及 CT 是否确认，不持第二份业务流表。角色消失、出口关闭、实例替换或执行停止时，先取消原输出、调用 `retire_exits`，再排空退役事件；FIFO barrier 等待已交出的 ACK 释放命令预算，继续取出此前尚不能物化的 Retire。精确 CT 清理结果传给 RetirementAck，最后退役角色并等待驱动退出。原始已过期或已撤回的排队包不会换绑到续期后的投影。

客户端先通过 `install_egress_capture` 准备旧／新范围的负向边界，再安装当前 `Desired` 并 `switch`；任一必需组件失败时 suspend 保留负向边界，显式 down 由同一 Host 精确清理。中继以 `admit_transit` 转发经出口的 circuit，但没有本地 role 就没有出口 writer。

`DnsPlan::ViaExit`／`ServeExitResolver` 由 daemon 的 DNS 装配驱动：源节点的 Mesh 查询进入唯一包处理 owner 的有界队列（最多 128 个），由它在 SecureNode 上建立 DNS circuit 并收发；出口收到的认证请求交给同一 DNS owner 的 `serve_authenticated`，回答再交回 SecureNode。DNS owner 不持 SecureNode 或流表，组件边界见 [出口 resolver](/v2/node/dataplane#exit-resolver)。只要计划不是 `noExit`，即使关闭 MagicDNS／overrideLocal 也启动系统 DNS 安装：源节点 `ViaExit`／`Unavailable` 安装 root 捕获，不能恢复本地 fallback；出口节点需要它的 served 系统快照。出口快照不可用只使出口查询 SERVFAIL 并报告原因，不否定业务 IP 出口，也不使本节点 DNS 安装失败；必需的 DNS 系统资源安装或健康检查失败仍阻断整体 applied。

### 源端 egress capture 与切换

`Host::install_egress_capture(EgressCapturePlan) -> Result<EgressCapture>` 复用当前客户端 TUN。它可以先于执行安装建立负向边界；尚无相同的已应用内容时返回非 active handle，不据此报告 applied。`Host::install` 在所需 capture 的负向边界准备后安装 TUN，并仅在当前 `Desired` 的捕获与计划精确一致时激活正向捕获。中继转发经出口的流不要求本机 `exitRole`，也不获得本机 NAT writer。

| API | 行为 |
| --- | --- |
| `EgressCapture::switch(new_plan)` | 原子建立旧／新范围并集的负向 OUTPUT 门，暂停正向捕获，准备新 RPDB／table／路由，再切换活动 TUN selector 并回读；当前执行必须认可新计划，失败保留负向边界 |
| `EgressCapture::is_active()` | 计划已激活且当前执行仍有效；不代表业务许可或数据面已完成安装 |
| `EgressCapture::plan()` | 当前记录的类型化计划副本 |
| `EgressCapture::retire()` | 显式解除所属捕获；先暂停正向表，再精确移除 RPDB 与私有 OUTPUT 表，供明确 down／解除受管范围使用 |

计划由本节点从 netmap 规则与 artifact 计算（`Desired::capture`）：受管 IPv4 与 sink IPv6 为有序、互斥、不相邻的区间，作为 OUTPUT 门 `managed4`／`managed6` 的 interval 元素；未经活动客户端 TUN 的受管目的立即拒绝（TCP reset／ICMP admin-prohibited），IPv6 受管目的恒拒绝，Mesh 路由保持优先。一条 IPv4 RPDB 选择器查独立的私有 egress routing table，活动时表中为受管区间的最小 CIDR 覆盖 `→ 客户端 TUN src Mesh 地址`，暂停时为空。上限 16,384 个区间、32,768 条路由，见 [Routing · 节点侧上限](/v2/configuration-reference/routing#capture-limits)。

捕获规则只匹配本机来源和非保护 mark。Server／peer 的受保护 socket 继续使用原宿主 RPDB，普通应用连接相同 IP 不获得豁免；出口 TUN 转发也不重新进入本机 capture。`inet meshway_egress_<installation>` 的 OUTPUT 门覆盖 local-table 广播等早于捕获查表的路径，防止其从外部接口直出。本机 local／loopback 服务保留；IPv6 ND 仅允许 type 133–137、hop limit 255、本机或未指定源的链路控制，保护新邻居解析，IPv6 TCP／UDP／echo 字面量不因此放行。

`underlay-egress.json`（v2）保存 boot／namespace、随机 installation、mark／table、计划、当前门与待提交门（区间与设备）、nft 规范对象／handle、选择器规则及 removed；已 down（removed）或属于另一次启动的记录不论版本直接归档，同一次启动内仍持有资源的其他版本记录在解析内容前即以专用错误（含文件名与版本，提示先用写它的旧版本 `meshway down` 或重启主机再升级）阻止启动；捕获表路由由门推出，恢复时与表内实际路由精确对账。自有规则／路由变化注册到同一 monitor（路由每 64 条等待其观测，不使通知缓冲溢出），不制造外部切网事件。暂停、执行失效和同 namespace 重启保留负向范围；TUN 消失不会落回 main direct。恢复先关闭旧活动 selector 并清理旧正向资源，只有新的有效执行安装可以重新激活；显式 down 才解除所属负向边界。

IPC 12 的公开运行状态（`meshway status --json` 原样输出）：

| 字段 | 含义 |
| --- | --- |
| `networkMode` | `tun`／`controlOnly`，本机选择 |
| `dataPlane` | `controlOnly`、`stopped`、`preparing`、`ready`、`suspended`、`failed`；来自组件实际安装状态，netmap 刷新不自行投影为 suspended。ready 不代替当前执行许可 |
| `authenticated`／`instance` | 当前有效会话与仍持有的有效实例（`instanceId`／`generation`／`expiresAt`）；instance 只绑定 State 与回执，不是数据面期限（数据面只受 `netmap.expiresAt` 约束）。停止、fencing、身份失效、session／instance 截止使其失效 |
| `saved`／`applied` | 独立的必填可空目标引用；saved 来自持久保存，applied 来自对应实例／目标的实际成功安装结果 |
| `phase` | 完整枚举见下表；结合 dataPlane、当前资格和错误读取 |
| `lastError`／`retryAt` | 类型化故障与重试时点；安装问题可为 unsupportedMode、underlay、tun、dns、transport、apply |
| `lastErrorDetail` | 必填可空：故障组件给出具体原因时的一行说明，目前为暂未绑定的 listener：bind 错误、占用该端口的进程（本 daemon 自己标注 `this daemon`）、端口在本机临时端口范围内时的提示，以 `retrying` 结尾（例如 `listener lab-udp cannot bind 0.0.0.0:35000: Address in use (os error 98); port 35000 is inside this host's ephemeral port range 32768-60999; retrying`，行为见[连接获取 · listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)）；其他情况为 null。daemon 日志的同一告警带相同文本 |
| `sync` | 必填，`connected`（最近一次 sync 成功、下一次由 Server 挂起）、`reconnecting`、`disconnected`；不代表 Node 授权或业务连通 |
| `lastSyncAt`／`syncError` | 必填可空：本进程最近一次返回已验签 netmap 的 sync 时间；sync 重试期间的失败类别 |
| `netmap` | 必填可空：当前 netmap 引用与签名租约 `issuedAt`／`expiresAt`，以及 `missingArtifacts`（引用但尚未持有的规则集 digest；命名它们的规则不可用，见[规则集 · 下发](/v2/configuration-reference/rule-sets#下发)）、`artifactProblem`（缺失时最后一次的 `download`／`integrity`／`storage`／`budget`，否则 null）与 `capture`（本节点从 netmap 与 artifact 计算并安装的出口捕获：`managedIpv4`／`sinkIpv6` 区间数、捕获表 `routes` 数，以及未安装分割而改为完整捕获时的 `unavailable` 原因，否则 null；本节点无出口捕获时整项为 null，计算见 [Routing · 节点侧上限](/v2/configuration-reference/routing#capture-limits)）、`routing`（本节点在该 netmap 下的路由方案选择 `RoutingSelectionState`，字段见 [Routing · 当前选择的状态](/v2/configuration-reference/routing#selection-state)；无法由 netmap 求出时为 null）；无有效租约为 null |
| `node` | 必填可空：已安装 netmap 中的本节点 `hostname` 与 Mesh `addresses`；未安装（down、control-only、准备中）为 null |
| `peers` | 必填：已安装 netmap 中可见的其他节点（与 `ping` 解析名称的集合相同），按 hostname 排序；未安装为空。字段见下表 |
| `sockets` | 必填：已安装 netmap 中本节点已绑定的 Native UDP listener 与拨号候选 socket（按地址族、地址排序），其后是绑定正在重试的 listener（按 key）；未安装为空。每项 `resource`（`listener`／`dial` 资源）、`boundAddress`、`receiveBuffer`／`sendBuffer`（内核读回的实际字节数，Linux 为授予值的两倍）、`receiveRequested`／`sendRequested`（配置值，0 时为默认值）与 `unavailable`；读回不足以容纳请求（Linux 上小于请求的两倍，判定唯一见 [UDP 批量接口](/v2/node/transport#udp-批量接口)）表示被 `rmem_max`／`wmem_max` 截断，`meshway status` 标注 `limited`。重试中的 listener 只有 `resource` 与 `unavailable`（最近一次失败，例如 `listener lab-udp cannot bind 0.0.0.0:35000: Address in use (os error 98)`），其余为 null，`meshway status` 显示 `unavailable`；重试绑定成功后同样读回缓冲。规则见 [UDP 批量接口](/v2/node/transport#udp-批量接口) |
| `exit` | 可省略；包含 target-bound `capability`、已安装 role generation、`activeFlows` 和独立的 `dnsUnavailable`（出口 resolver 不能为源解析的原因，与 State [`exitDns`](/v2/state-reference#exitdns) 同源；可用或不服务时为 null）。未观测的计数保持 null，失去当前执行权限后不显示已安装角色／活流数 |

`peers[]` 只投影本节点自己的事实，不生成测量或授权：

| 字段 | 含义 |
| --- | --- |
| `nodeId`／`hostname`／`addresses` | 投影中的配置值 |
| `state` | `direct`：有选定的认证通道；`relay`：有经其他节点的已建立 circuit（本节点为端点）；`relayIdle`：需要邻接但无通道、也无 circuit，而新 circuit 会经中继：排序第一的中继路径首跳有通道、其上每个中继在最后已知事实中都与下一节点有通道（`meshway status` 的 STATE 列显示 `relay (idle)`，见[中继兜底](/v2/node/connectivity#relay-fallback)）；`offline`：该 peer 的最后已知事实缺失或已过 `PeerFacts.expiresAt` 且以上皆无；`unreachable`：需要邻接但无通道、circuit 或可用中继，即确实没有路径；`idle`：其余（含非相邻且当前无 circuit） |
| `transport` | 必填可空：`direct` 为选定通道的承载，`relay`／`relayIdle` 为首跳通道的承载；其他为 null |
| `endpoint` | 必填可空：`direct` 选定通道的实际远端地址；其他为 null |
| `mtu` | 必填可空：`direct` 的 Native UDP 选定通道的 `path`（外层路径 MTU，IP 字节）与 `inner`（一个 Data 帧能承载的最大内层 IP 包）；搜索完成前、其他承载或未确认时为 null，定义见[外层路径 MTU](/v2/node/connectivity#path-mtu) |
| `via` | `relay`／`relayIdle` 时按路径顺序的中转 Node ID；其他为空。到同一 peer 有多条已建立 circuit 时取新流实际会加入的那条（与首包同一路径排序 `SecureNode::choose_path`），没有可加入的才取其中最短的一条 |
| `rtt` | 必填可空：到该 peer 最近一次成功 `ping`／观察任务测量的 `durationUs` 与 `observedAt`；没有测量为 null，不补零；down、撤回或租约结束时清空 |
| `idlePath` | 可省略：该 peer 既无选定通道也无已建立 circuit 时（`state` 为 `relayIdle`／`idle`／`unreachable`／`offline`），新 circuit 此刻会走的路径，取值只能是该 state 允许的：`relayIdle` 为 `via` 所列的那条中继路径（`SecureNode::relay_route`）；`unreachable`／`offline` 没有可走的路径，总是 `none`；`idle` 为与首包同一选择 `SecureNode::idle_route`（`ranked` 候选顺序中第一条首跳有通道且未暂停的，否则排序第一的；不登记邻接需求）。`route` 为 `direct`（直连路径，`via` 为空）、`relay`（`via` 为按路径顺序的中转 Node ID）或 `none`（没有新 circuit 可走的路径，或已安装内容不给本节点到该 peer 的路径）。有通道或 circuit 时省略（实际路径见 `endpoint`／`via`）。`meshway status` 的 ENDPOINT 列显示 `direct (idle)`／`relay (idle) via <R>`／`no path` |

`phase` 的全部 10 个值：

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
| `leaving` | 持久注销意图等待 Server 确认：数据面已停，按 1–15 秒退避重试 `leave`，期间不重新接入（见[自助注销](/v2/configuration-reference/nodes#node-self-leave)） |

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

IPC 封闭对象解码拒绝未知字段；旧版本请求返回 `ErrorCode::Version`。CLI 与 daemon 同版本发布，不将缺字段补成已连接或零时间。

ready／applied 只表示该目标的本机安装结果，不表示 peer 已建立认证通道或应用已收回业务包。Node State 中的 listener／link 来自组件观测；路径、RTT、丢包缺测保持空。安装与报告顺序见 [Control ApplicationTicket](/v2/node/control#安装回执与运行报告)。

## 业务数据面

统一业务数据面采用 **L3 IPv4／IPv6 IP 包**。系统 TUN 由 OS socket／路由把受管流量送入 TUN，需要 TUN／路由权限及所需 DNS 资源；它不隐含内核模块、内核内加密或 eBPF。当前目标不包含 Ethernet 透明桥接、MAC 学习或二层广播域。

内层 IP 业务与外层承载独立：TCP／UDP 业务不据此强制选择同名 transport，实际承载由配置与当前执行许可约束。本机权限或 TUN 安装失败不能静默切换模式后报告原目标 applied。同一设备一次只启用一张网络。

## CLI 与 IPC

Linux 首版每台机器运行一个系统级 `meshwayd`，同一时间只启用一张网络。CLI 连接该服务；不为每个本机用户创建独立 daemon 或自动复制 Node 身份。当前使用 owner-only 目录与 `0600` socket，在解析请求前后核验内核提供的 peer UID，只允许 root 或 daemon 同 UID。系统服务安装见 [Linux 安装与系统服务](/v2/install)；更细的操作员授权仍待实现。

| 命令／操作 | 行为／状态 |
| --- | --- |
| `meshway up` | 首次浏览器接入，或用保存的身份重新认证并取得运行资格 |
| `meshway up --enrollment-key @file` | 通过 Server 的一次性 Tag 接入密钥加入网络，不取得用户会话；既有身份不被接入密钥改绑 |
| `meshway rotate-key` | 同步返回已完成换钥的公开身份／状态或类型化失败；随后重新取得实例和运行资格，换钥成功不等于数据面已 applied |
| `meshway down` | 停止当前网络并终止相关资格；daemon 与私有身份保留 |
| `meshway logout` | 持久保存注销意图后请 Server 永久删除本节点（[自助注销](/v2/configuration-reference/nodes#node-self-leave)），确认后下线并清除本机身份；没有回答时保持 `leaving` 并在后台重试，Server 明确拒绝时撤回意图、本机不变 |
| `meshway ping <target>` | 先打印本节点的本地判定，再探测：Mesh 节点为获准的端到端 Mesh 探测，公网 IP／域名为经出口路径或本机直出的 ICMP echo；参数、授权与输出见[路径诊断](/v2/node/diagnostics)与[Mesh 外目标](/v2/node/diagnostics#external) |
| `meshway status [--watch]` | 本节点摘要与 peers 表；`--watch` 每秒刷新（TTY 原地重绘，否则逐份输出） |
| `meshway netcheck` | 一次有界的本机网络诊断，见[路径诊断 · netcheck](/v2/node/diagnostics#netcheck) |
| `meshway exit [profile \| --default]` | 无参数：显示本机 netmap 中的方案选择；带方案：请 Server 切换并等待本机应用，`off` 即 `mesh`；`--default`：请 Server 清除持久选择、回到默认方案。见[出口方案](#exit-profile) |
| `meshway version` | 程序与协议版本；不连接 daemon |
| 切网 | 尚未实现；先停止旧网络、完成隔离清理，再运行新网络 |

CLI 通过 Unix domain socket 发命令、读结果；状态由 daemon／组件的唯一 owner 持有。`meshway_protocol::ipc` 是唯一的请求／响应定义，当前版本为 12，使用 JSON 与长度前缀，单条消息上限 64 KiB；CLI 和 daemon 同版本发布。每个连接处理一个请求，Ping 可返回有序的多个事件，Netcheck 返回一个报告，SelectProfile 与 ClearProfile 返回 Server 提交的选择（`Selection`）；连接数、请求等待、写入和队列均有上限。`Up` 的 `enrollmentKey` 可省略；`Up`／`Down`／`RotateKey`／`Logout` 的 `expectedRevision` 由 CLI 读取后提交，换钥与注销不会绕过本机 CAS。

### CLI 输出与退出码

所有命令都有 `--json`：输出一个类型化 JSON 文档（`ping` 为每行一个事件），字段即上文 IPC 类型，不含秘密。人读输出只在 stdout 为 TTY 且未设置 `NO_COLOR` 时使用颜色与表格线；管道、文件或 `NO_COLOR` 下为不含控制字符的对齐纯文本。宽度按显示宽度（含 CJK）计算。读端关闭（如 `meshway status | head`）时静默以 0 退出。

```text
meshway-sha  100.64.0.2  fd89:98a2:2060::1  running
  Server     https://meshway-dev-api.jdscript.dev/
  Sync       connected · last sync 24s ago
  Netmap     revision 60 · serial 68 · lease 23h 59m left
  Data plane ready · mode tun
  Exit       none
  Identity   node dbb6fcbb-… · network 8998a220-… · key generation 1

PEER         MESH IPV4   STATE   TRANSPORT   ENDPOINT            MTU        RTT
meshway-lax  100.64.0.1  direct  native_udp  192.168.97.3:52242  1500/1386  0.90 ms
meshway-nyc  100.64.0.6  relay   native_udp  via meshway-lax                2.00 ms
```

`status --json` 中对应的 peer：

```json
{"nodeId":"3740cacb-68ff-4fe8-9559-2733b5b1ad76","hostname":"meshway-nyc",
 "addresses":{"ipv4":"100.64.0.6","ipv6":"fd89:98a2:2060::5"},"state":"relay",
 "transport":"native_udp","endpoint":null,"mtu":null,"via":["21ee69e3-493e-40f8-b635-19ec4689fac9"],
 "rtt":{"durationUs":2079,"observedAt":"2026-10-11T20:38:37.572291623Z"}}
```

| 命令 | 退出码 |
| --- | --- |
| `up`／`down` | 0：已收敛，或仍在 `starting`／`synchronizing`／`stopping`／`retrying`（意图已保存）；1：`blocked`、意图被并发改变或 IPC 错误 |
| `logout` | 0：Server 已删除且本机已清除；1：`leaving`（意图已保存，后台继续）、Server 明确拒绝（本机不变）、本机无身份或 IPC 错误 |
| `ping` | 0：至少一次成功回复；1：全部失败／超时或操作错误；130：Ctrl-C |
| `exit` | 无参数：0 已输出，1 无 netmap 或 IPC 错误。带方案：0 为 Server 已提交且本机已应用，或已提交但 30 秒内未应用（stderr 警告，状态见 `status --watch`）；1 为不在允许范围（列出允许的方案）、已锁定、未就绪（未接入、无 netmap／instance）、Server 不可达或拒绝、选择被他处更新（`selectionConflict`，取得新 netmap 后重试） |
| `status`／`netcheck`／`rotate-key`／`version` | 0：已读取并输出（netcheck 的 Server 不可达或无数据面也是 0，报告本身说明）；1：IPC 或操作错误 |
| 参数错误 | 2 |

推荐的无交互接入与换钥用法：

```sh
meshway up --enrollment-key @/absolute/private/enrollment-key
# 或由秘密提供者写入 stdin，输入结束后提交：
cat /absolute/private/enrollment-key | meshway up --enrollment-key -
meshway status --json
meshway rotate-key
```

### 出口方案 {#exit-profile}

`meshway exit` 显示本机已持有 netmap 中的方案选择（`NetmapStatus.routing`，与 Server 对同一配置的回答相同，离线可读）：生效方案、请求的方案与版本、默认方案与是否锁定、`auto` 的解析依据，以及每个允许方案的类型与一句摘要（规则数、出口节点、未命中时的动作；节点名取自本机 peers，未知时为 Node ID）。

```text
Exit profile exit
  Requested  none · following the default
  Default    exit · not locked

PROFILE                                      KIND    STATE             SUMMARY
exit                                         custom  applied, default  2 rules · exits e · otherwise exit e via r
exit-alt                                     custom                    2 rules · exits e · otherwise exit e via r
mesh                                         mesh                      Mesh only; other external traffic unmanaged
global:e3210ff3-d7a6-4112-931b-0af816fddb47  global                    all external traffic: exit e
```

`meshway exit <profile>` 经 IPC `SelectProfile` 交给 Control：Control 以持有 netmap 中本节点的 `selectionVersions` 为 CAS 基准调用 `selectNodeRoutingProfile`，Server 提交后重新编译 netmap；CLI 等到本机持有的 netmap 带上该版本（TUN 模式另需该 netmap 已 applied）才返回，最多 30 秒。`off` 即 `mesh`。本机不保存“想要的方案”：Server 的回答只用于本次输出，生效状态只来自 netmap。切换总是新的选择版本，经旧方案出口的已有流随新 netmap 结束（[netmap 设计 · revision 变化时的已有流](/v2/node/netmap-design#revision-变化时的已有流)）。

`meshway exit --default` 经 IPC `ClearProfile` 以同一 CAS 基准调用 `clearNodeRoutingProfile`（[选择的入口](/v2/configuration-reference/routing#选择的入口)）：Server 清除持久选择后，节点回到 assignment 的默认方案（默认为 `auto` 时即按地区解析）。清除后的选择没有版本，CLI 等到本机 netmap 的选择版本不再是清除前持有的版本（版本只增）才返回；没有选择时立即返回。输出与无参数时相同（`Requested  none · following the default`），末尾一行 ``Persistent selection cleared; following the default `<profile>`.``。不能与方案参数同用。

`--json` 无参数时输出 `RoutingSelectionState`；切换或清除时输出 `{"committed": <Server 的回答>, "applied": <是否已应用>, "local": <本机 netmap 的选择或 null>, "cleared": <是否为清除>}`。IPC 错误码：`profileNotAllowed`（不在允许范围）、`profileLocked`（锁定在默认方案）、`selectionConflict`（选择已被 Console 等更新，本机尚未取得新 netmap）、`notReady`（未接入、无 netmap 或 instance）、`network`／`serverBusy`／`credentials`。Server 拒绝不结束 session 或 instance。

接入密钥输入统一走 IPC 持久命令；daemon 不另提供重复的 `--enrollment-key-file` 启动源，避免启动参数与已接纳意图的恢复顺序冲突。换钥处理预算为 65 秒、CLI 等待 70 秒；超时／Ctrl-C 不取消已接纳操作，先通过 status 的公钥 generation 检查结果再决定是否重试。原始接入密钥、私钥和 token 不进入状态、错误分类或普通日志。

up/down 按本机期望状态 revision 执行 CAS，并保留最后一次变更的请求身份以恢复不确定结果。本机 revision 与 Server 配置 revision 分开。已接纳命令的持久化由协调器完成，CLI 断开不会撤回它；读取 status 不写入状态或生成测量。

daemon 独占目录锁，启动时记录 socket 的文件身份；只有身份匹配且确认无监听者的旧 socket 可清理。未知文件、符号链接、活跃 socket 和损坏的状态文件都保留并报错；状态文件缺失时的目录判定见[数据目录与启动检查](#数据目录与启动检查)。

CLI 生命周期不承载 Node 身份或 lease，不收集账户密码、不取得浏览器会话。业务 listener／DNS／策略无本地覆盖源；路由方案切换是使用选择，由 Server 持有并决定，入口见[出口方案](#exit-profile)。`down` 不等于云端永久删除；永久删除本节点用 `logout`。

## Engine

`meshway-engine::Engine` 是 Node 的唯一期望状态 owner。sync 验证并保存的 netmap 与租约经独立通道直接送达网络 owner（不经 Control 锁与维护调度），网络 owner 每收到一次调用一次 `reconfig(Some(&netmap))`；撤回时调用 `reconfig(None)`。Control 的维护 HTTP 按单个操作持锁，sync 回复最多等待一个进行中的操作。

| 输入 | Engine 行为 |
| --- | --- |
| 新内容（更大 serial） | 构造一次不可变 `Desired`（内容与共享 `LocalCompiler`），按 Host → 邻接 → SecureNode → DNS 顺序调用各组件幂等 `apply`。安全相关步骤（capture／TUN、负向 DNS、circuit 与流收窄、出口退役）失败时对全部组件应用空期望；只缺新增能力（listener／拨号 socket、邻接 Manager、出口角色、DNS 查询应答）时组件返回 `Degraded`：内容保持安装、对应能力缺失并上报，回执不报 applied，有界重试同内容；listener 的 bind 由 Manager 自行重试，其可用性变化（变为不可用或重试后绑定）时以同一内容再 apply 一次，完整 apply 清除先前 Degraded 的 `lastError` |
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

Server HTTP 等待与本机命令处理独立。netmap 与 peer 事实只经 Control 的 `sync_loop` 取得：单一长轮询 `POST /agent-api/v1/sync` 由 Server 挂起至有变化，超时回 unchanged 即兜底刷新，挂起不越过节点持有的租约（至少余 15 秒）；每份验签通过的 netmap 交给网络 owner 的 [Engine](#engine)，撤回发布空 netmap。退避与撤回分类见 [Netmap 发布](/v2/node/control#netmap-发布)。`sync_loop` 只在准备与应用回答时持有 Control 锁；本节其余维护（实例续租与控制报告通常每 20 秒，观察任务领取每 2 秒）在同一 Control 上另行调度，sync 回答不排在它们之后。真实安装完成、socket／邻接或业务依赖变化还会触发合并后的报告，最多每秒一次。仅观测时间变化不触发新事件，旧报告重试保留原序号／内容／时间，也不确认后来出现的新状态。故障采用有界退避，期限由控制组件独立校验。down 先持久保存本机下线意图，再停止当前资格并尽力释放远端实例；离线时也不能把旧 lease 展示为有效。当前 IPC 不承载业务包。

Node 不消费 SSE 事件流（用户 SSE 保留给 Console）。已有待确认换钥／接入密钥记录在认证前由同一 Control owner 恢复，发起入口限制见[换钥与恢复](/v2/node/control#换钥与恢复)。

报告 scope 轮换或刚确认的新诊断目标优先执行刷新，排在旧 scope 的 traversal／结果网关请求之前；刷新成功后恢复有界网关调度。刷新本身不赋予新目标权限。

期望上线时，网络失败、HTTP 408／429／5xx、无可解码 body 的任何状态及 400 等其他已解码拒绝按 1–15 秒有界退避自动重试，签名租约内数据面持续运行。不可解码或超限响应会撤回旧运行上下文后重新取得资格；运行中的 session／instance 拒绝、lease 到期或实例更换会重新持钥认证、取得有效 instance 并上报 State，不要求再次执行 `up`。已解码的身份拒绝、Server（`controllerId`）／key／目标绑定错误进入 `blocked`：前者同时撤回数据面，后者让已持有的 netmap 运行到租约到期；本机记录损坏或写入结果不确定进入 `blocked` 并撤回数据面。分类定义见 [Node 控制组件](/v2/node/control#netmap-发布)；后台残留请求不能绕过该停止状态。

Linux 运行中的外部接口删除／重建、地址变更及路由变化由既有 Underlay netlink 监视器通知；未 up 的接口的创建／删除（例如 VPN 守护进程反复重建的空闲 `ppp0`）不承载任何路径，不算变化，已 up 接口的停用另以其地址／路由删除通知。变化关闭旧控制 HTTP 的底层连接，后续请求按当前路由重新连接并沿用上述有界重试；受保护控制中继按 Happy Eyeballs（RFC 8305）连接 Server 解析出的地址：地址族交替（从解析结果的首个地址族开始），250 ms 未完成即并发尝试下一个，任一失败立即尝试下一个，先连上者胜出，因此无路由或黑洞的 IPv6 不拖住 IPv4；中继连接在双向 120 秒无字节时关闭（不按连接年龄截断，挂起的 sync 长轮询不被中继自己的时钟切断）；不要求重启 daemon、再次 `up` 或修改配置。本机端点变化经既有合并报告路径发布，刷新规则见[连接获取](/v2/node/connectivity#stun-与-server-协调)。只有 kernel 来源的 netlink 通知参与资源归属检查；用户态 multicast 消息被忽略，不能把普通接口重连变成永久保护冲突。当前 namespace 中真实的所属 TUN／DNS 锚点或保护规则冲突仍阻断执行。

Underlay 启动时当前 namespace 的资源归属或路由保护冲突仍拒绝准入，保留现有资源；状态为 `retrying`／`lastError=underlay`，以同一 1–30 秒有界退避发布 `retryAt` 并重新检查。冲突 owner 释放资源后无需再次 `up`。损坏的 Underlay journal／私有存储错误归为 `storage`，安装参数或 bootstrap resolver 配置错误归为 `configuration`，仍须修正后恢复。

控制失败日志包含实际 operation、错误类别、HTTP status、已解码的 problem code、Server request ID 和恢复决策；不记录响应原文或凭据。显式 `down` 的清理若失败，在外部冲突解决后重新执行 `down` 会触发新的清理尝试，同请求重放仍保持幂等。

`state_scope_changed` 表示报告在交付期间遇到正常 scope 轮换：丢弃当前报告，1 秒后刷新并重新采样，不进入 `retrying`、不设置 `lastError=apply`，保持当前 phase，不影响 sync 长轮询。netmap 引用、saved 前置条件或回执内容冲突按[回执分类](/v2/api/problems#node-配置回执冲突)退役精确回执，1 秒后刷新并重新 apply 当前目标。两者均不 reset instance 或主动 suspend／删除 TUN。实际执行截止、撤回和连续性交接检查独立生效；旧回执、输出与报告不换绑新目标。

自有负向规则、私有表路由以及 TUN 的创建／退役不会作为外部网络变化通知。TUN 关闭产生的延后地址／链路事件保留精确接口索引至事件排空，仅用于抑制自触发通知，不形成额外的资源归属或捕获豁免。suspended 期间的 Mesh `EHOSTUNREACH`／`ENETUNREACH` 是负向边界的实际结果，不触发控制连接重建或端点重枚举。

## 持久与重建

| 私有保存 | 重新建立 |
| --- | --- |
| Server 信任绑定、Node 密钥、待完成接入／换钥 | 短期会话、instance、执行 lease |
| 完整已保存目标与 digest | 实际安装结果、通道、打洞和包会话 |
| 所选网络、期望上线及有效使用选择 | 在线／可达性证明、计数基线 |
| 系统资源归属 journal | 已认证的资源状态与运行代际 |

缓存只作恢复基准，不复活旧授权。重启、释放或过期后使用新 instance；配置历史不能覆盖永久吊销。运行目录／IPC 地址是安装参数，不是另一份网络业务配置。

### 数据目录与启动检查

`--data-dir` 内每个文件只有一个 owner；daemon 装配这些组件，并在状态文件缺失时据此判定目录归属：

| 文件 | owner | 用途 |
| --- | --- | --- |
| `daemon.lock`、`daemon.json`、`control.sock` | daemon | 目录锁、本机期望状态（revision／desiredUp／最近请求／socket 身份）、本机 IPC |
| `identity/` | Control（`meshway-control`） | `node-control.lock`、`control.json`（身份信封）、`clock.json`、`netmap.json`、`peer-facts.json`；目录内规则由 Control 自行检查 |
| `underlay.lock`、`underlay.json`、`underlay-egress.json`、`underlay-exit.json`、`exit-child.json` 及各自的 `*.stale-<Unix 纳秒>` 归档、`underlay-bootstrap.json`、`underlay-bootstrap.sock` | Underlay（`meshway-underlay`） | 内核资源 journal、出口／egress 账本、bootstrap 代理记录与 socket |
| `.write-<32 位十六进制>` | 私有存储 | 原子替换被中断后留下的 staging 文件，不被读取 |

fail closed 针对的是未知归属或不一致的本地存储。`daemon.json` 存在时按其内容恢复；缺失时按下表判定，通过则以 `revision=1`、`desiredUp=false` 的新状态写入 `daemon.json`，以未接入状态启动，并在 WARN 日志中列出目录里已有的自有文件。不通过时不写任何文件，`Node state is missing from a directory with unowned files.` 退出：

| 目录形态 | 结果 |
| --- | --- |
| 空目录、只有 `daemon.lock`（含 `install.sh --uninstall --purge` 后重装） | 启动，未接入 |
| lock＋`underlay.json`（空或记录了资源）＋`underlay.json.stale-*`／其他 Underlay 文件 | 启动，未接入；`tun` 模式下由 Underlay 按[既有规则](#持久与重建)回收 journal：上一 boot／其他 namespace 的归档，同 namespace 且驻留的资源先核验再在未接入的 down 中删除，空 journal 无事可做。`control-only` 不打开 Underlay，journal 原样保留 |
| 残留 `control.sock`／`underlay-bootstrap.sock` | 启动；无监听者的旧 socket 按 socket 身份规则回收，活跃 socket 仍报错退出 |
| `logout` 后的 `identity/`（空身份 `control.json`、`clock.json`、lock）＋journal | 启动，未接入；身份目录由 Control 载入为无身份记录 |
| 以上之外的任何名称、符号链接、类型不符（如目录名为 `underlay.json`、普通文件名为 `identity`） | 拒绝，目录原样保留 |

自己留下的 journal 不是拒绝启动的理由：它存在的目的就是让下一次启动回收上一实例的资源。手工删除身份文件后 systemd 不再陷入每 5 秒重启的循环；重新接入仍需显式 `up`。

## 生命周期

```text
取得私有目录／IPC → 装配组件与 provider → 接受上线目标
  → 持钥接入与执行目标 → 校验并保存（saved）
  → 先准备底层保护，再安装组件 → applied／failed 与真实 State
```

宿主协调一个活动网络及有效执行上下文，组件拥有其任务、buffer、socket 和系统资源。具体运行库不重复编译网络策略；saved／applied／业务连通的含义见[配置应用](/v2/configuration-reference/deployment)。

撤回、期限和 down 独立于挂起的 HTTP 生效。停止准入后取消并等待任务退出；显式 down 清理可证明所属的内核资源与 TUN，期望 up 时的过期／断连／退出保留负向 capture 边界。资源生命周期及 bootstrap 同 IP 隔离见 [Linux underlay](/v2/node/dataplane#linux-underlay)。

崩溃恢复只处理有可验证归属的 journal，不恢复旧流。未知格式或不明归属不能作为删除宿主规则的依据。

Underlay 的每份记录（`underlay.json`、`underlay-exit.json`、`underlay-egress.json`、`exit-child.json`）都带同一个 network namespace 身份，由 `meshway-underlay` 的唯一实现读取与比较。nsfs 的 device／inode 只在 namespace 存活期间唯一：容器重建（`docker compose up` 换容器、`docker stop`／`start`）后新 namespace 常拿到同一个 inode。namespace cookie（`SO_NETNS_COOKIE`，Linux 5.7 起）在一次 boot 内单调分配、从不复用，因此身份由以下部分组成：

| 部分 | 内容 | 不同即 |
| --- | --- | --- |
| 写入时的内核与 namespace | `/proc/sys/kernel/random/boot_id`、`/proc/self/ns/net` 的 device／inode，以及 namespace cookie（字段 `namespace_cookie`；`exit-child.json` 为 `net_cookie`） | 过期（整机、容器虚拟机重启、容器重建后挂载同一卷）；记录与当前都有 cookie 时，inode 相同而 cookie 不同即为被复用的 inode，同样过期 |
| 驻留证据 | journal 记录的、只能存在于写入它的 namespace 中的对象：精确字节的 RPDB 规则（随机私有 table 与随机优先级区间）或 DNS 锚点（alias 含随机 installation） | journal 记录了对象而当前 namespace 一个也没有：inode 被新 namespace 复用，同样过期 |

没有 cookie 的记录（本版本之前写入）或不支持 cookie 的内核仍只按 boot 与 device／inode 判定，再由驻留证据区分复用的 inode；`exit-child.json` 没有驻留证据，这种组合下 inode 复用仍可能使启动停在下述冲突退避，直到它按新规则重写（升级后第一次工具执行即重写）。cookie 由 `meshway-netns` 读取，这是 workspace 唯一经审计的 `unsafe`（一次 `getsockopt`）；比较规则也只有 `meshway_netns::same` 一份。系统 DNS 的 namespace 预约（`/run/meshway/dns-*/owner.json`，`namespace.cookie`）按同一规则：同一 inode 而 cookie 与当前不同的预约属于已不存在的 namespace，它与旁边的后端记录（`resolv-conf.json`、`resolved.json`）一起归档为 `<名>.stale-<Unix 纳秒时间>`（每种保留最新 3 份，与 Underlay journal 同一上限 `STALE_ARCHIVES`），当前 namespace 接管；`down` 时的只读检查把这样的预约视为没有需要撤销的内容。同一 namespace（同 cookie）的另一个 data 目录所有者，或仍持有该 namespace 锁的活进程，是真冲突。本版本之前写入的无 cookie 预约按 inode 规则：同一 inode 被接管并改写为带 cookie。

过期记录在分配新资源前原子改名为 `underlay.json.stale-<Unix 纳秒时间>` 并持久保留；INFO 日志记录旧 namespace（含 cookie）、原因（`changed_namespace`：boot、inode 或 cookie 不同；`reused_namespace_inode`：无 cookie 而驻留证据缺失）与归档名。按无 journal 在当前 namespace 重新分配，不进入旧 namespace 清理，也不把旧记录作为当前资源的归属证明。归档失败时不覆盖原文件或继续分配。身份相同且驻留证据存在时仍按原有逐项归属核验恢复：只缺部分对象、或出现不属于 journal 的同区间对象，仍是真实冲突并使用上述有界退避。没有记录任何对象的 journal 不需要驻留证据，直接按当前 namespace 重新分配。

SIGTERM／SIGINT 干净退出且期望仍为 up 时，先取消和 join 运行任务，经 `suspend` 删除本进程的正向路由、关闭非持久 TUN，并清空、持久保存对应 journal 记录；负向 capture 规则及独立 DNS 锚点继续保留以供同 namespace 重启恢复。因此 `Node stopped failed=false` 不表示 journal 应整份删除。异常退出保留的可核验资源仍走恢复检查；namespace 变化使用过期归档规则。

## 规划：userspace networking

以下为规划，不是可用功能，`--network-mode` 不接受 userspace：

- 无 TUN，应用通过本机 SOCKS5 接入，由用户态 netstack 处理流量，并接入同一 dataplane，共用身份、准入、端到端保护、完整路径与回复规则。
- 本机 SOCKS5 使用当前网络的业务权限、DNS 和路由，不能作为绕过准入的直拨通路；不包含应用内 socket SDK 或公网代理服务。
- 命令范围、监听地址／端口、本机访问认证、域名解析、UDP 关联生命周期、MTU／分片与出口能力尚待定版。
