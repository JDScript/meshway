# Linux socket 级系统保护通路

## Namespace owner 接线合同（本轮切片）

**审查后的修复候选**：冻结 `337c334 + 987cbc2` 的 Request changes 报告和旧 artifact
保留。下述整组 operation、netns singleton 与 scope rollover 是后续修复，须交原
reviewer 复核；旧 `2186c3ba6d71` 证据不用于声称关闭 F1/F2。

`underlay::LinuxUnderlay` 是正常 Agent 可调用的内部准备入口；不接受设备提供的
mark、priority、table 或 nameserver 参数。主循环接线由 P3a owner 完成，本切片
本身不代表正常 Agent 已自动启用保护或完成真实 Controller/TUN 组网闭环。

```text
LinuxUnderlay::prepare(Preparation {
  state_directory: 已创建的私有 Agent 身份状态目录,
  identity: 已接入的真实节点身份,
  network: 当前唯一活动网络,
}) -> namespace owner
owner.system_dns() -> 当前系统解析 snapshot
owner.controller_channel(已绑定 controller URL) -> 独立 controller 退休 owner
owner.socket_generation() -> 独立 carrier PreparedGeneration
owner.retain_mesh_sink(真实 Mesh 前缀) -> block 内专用 reject-only 表（也覆盖转发包）
owner.capture(规范 Mesh/业务前缀 + 已拥有的 TUN 绑定) -> 自有表内 sink/capture
owner.withdraw_tun() -> 仅撤回 active TUN route，保留 unreachable sink
owner.activate_tun(相同前缀 + 新的真实 TUN binding) -> sink 不变的故障关闭重接
owner.relinquish() -> 明确取消全部接管；要求所有 socket capability/FD wrapper 已释放
```

接线顺序：在开始任何 capture 前准备 owner、DNS 与 controller channel，并将
controller client 注入所有 HTTP 消费者；再把独立 carrier generation 安装到现有
`Manager::install_prepared_underlay`，最后发布自有捕获。变更 endpoint/DNS 时先准备
新 channel/generation，再取消旧观测、withdraw/join 旧 carrier、切换。业务 lease
退休仍由 RootPacketGeneration 执行；`withdraw_tun` 不能代替 packet fence。
controller 恢复 owner 不属于 carrier lease，不能随 carrier 一起销毁或改回默认 client。
`CapturePrefix::tun` 接受不可手造 name/index 的 `TunBinding`；现有 `Tun` 新增
`underlay_binding()` 从自己持有的真实 `AsyncDevice` 创建该证明。
`Tun::open_with_underlay(options, &PacketPlan, &mut owner)` 是新增的正式 TUN 准备入口：
复用真实非持久 TUN 创建/MTU/地址逻辑，先保留 owned Mesh sink，随后安装自有表中的
TUN route；不执行 ambient DNS，也不写 main route 或目标 `/32` pin。P3a owner 将
旧 `Tun::open` 调用换成这个接口，并在 packet fence/join 后调用 `withdraw_tun`。
旧入口的行为仍保留，仅以新入口接线时才收敛到正式 route owner。

该 factory 同样接受既有 typed `Purpose::Stun` / `AuthenticationProbe`，其必要 DNS
经 `BootstrapDns(Stun / AuthenticationProbe)`，保护准备不只覆盖 TCP/VLESS。
首选复用实际 UDP carrier FD 的 STUN；仅 TCP/VLESS 或没有 UDP carrier 时，后续
若启用独立 probe socket，须独立记录其 socket/source/namespace/generation，不能
冒充 listener 映射或 peer 候选。本轮不启用默认 STUN，不新增 controller 观察点
或 GeoIP。Docker embedded DNS 及隔离容器双栈 socket 证据不证明公网嵌套 NAT
可达、打洞、换网实测或设备地区；全 UDP 失败仍应走后续已定义的方案兜底，不
从 controller 来源 IP / 隧道出口 IP推造地区，多源/双栈冲突须保留原观测边界。

本轮 namespace owner 保守支持标准 local/main/default RPDB，以及位于自有 block
之后、受既有 validator 支持的系统/VPN规则。分配双栈共同空闲的连续五个 priority、
未被 RPDB mark mask 使用的一个 bit，以及无 routes、无 rule 引用、无 rt_tables
alias 预留的两张捕获数字表及一张 Mesh reject-only 数字表；UR1 的 numeric identity /
block 外表引用拒绝保持有效。专用 Mesh lookup 位于 NOP 之前，不加 `iif lo`，
因此转发到 Mesh 的包在 TUN 停止后也不能漏回默认路由；该表在每次 socket preparation
中验证只含非 default 的 unreachable 前缀，不能含 TUN、gateway 或出口 default。
普通本地 profile capture 仍保留 `iif lo`，不会把非 Mesh 转发卷入本地出口方案。
未知 selector、先于 capture 的非 local lookup、mark 使用冲突、rt_tables drop-in、
netfilter mark 使用或 legacy netfilter 状态均报告失败；不删 foreign 条目。
运行环境必须提供 iproute2、nft 和 legacy-save 审计命令及 namespace NET_ADMIN。

所有权以当前 netns 的 dev/inode、boot ID、节点/网络绑定和完整 numeric kernel
属性写入 `underlay.json`。state 和 namespace 的锁文件保持 0600，目录 0700；
journal 使用同目录随机临时文件、fsync、rename、目录 fsync。namespace lock 位于
固定命名 Linux abstract AF_UNIX datagram socket `meshway.underlay.rpdb-owner.v1`，
属于 netns 而非 `/run` mount；不同私有 mount/statepath 的 Agent 也不能同时分配；capability/退休后
仍持 FD 的 wrapper 会继续持有锁。正常退出和 crash 均保留 committed sink/guard
与 journal；恢复核对实际资源，不复活旧 socket/packet generation。无法区分的
半提交（kernel mutation 和持久证据之间 crash）明确保留 pending 并拒绝 capability，
要求可靠 ownership reconciliation，不凭形状 adopt/delete。活事务可确认未发生
mutation 的安装失败按完整属性逆序 rollback；rollback 失败不返回可用 capability。
已经有持久证据但未完成 guard 的 partial reservation 会核对全部资源并精确 rollback，
返回明确的 retry 错误；下次正常 prepare 重新分配，不能在半配置下继续运行。

Journal v2 在首个 mutation 前持久化**完整 operation intent**：baseline、完整期望
resource set、Install/Rollback phase；单条资源写回后的 `pending=None` 不再代表
整组完成。每次资源完成至下一条 pending 之前，operation 始终保留。完整期望集合、
双栈 guard、双族 capture lookup、Mesh sink/lookup 关联和真实 kernel evidence
全部核验后才能清除 operation 并发布 capability。已证明 ownership 的 committed
prefix 可由 `recover_operation()` 精确续做/收敛；ambiguous 单条 pending 仍拒绝。
消失/换身份的 TUN 不重建、不恢复旧 packet/socket generation。旧 v1 row-only 的
缺 lookup / 单族 capture / orphan sink 明确拒绝，不凭 shape 修补未知 ownership。
`retain_mesh_sink` 的幂等返回也经过完整关联及实际 lookup 核验。

DNS 新入口 `SystemDnsSnapshot::from_system` 保留 `hosts: files dns` 下的 `/etc/hosts`
优先级，并在真实 Docker 环境中保留唯一 `127.0.0.11:53` embedded resolver。
Docker 网络别名的 A/AAAA 查询仍使用 marked UDP/TCP 工厂，而不是 getaddrinfo 或
手工传入 fixture DNS。plain 系统/VPN ordered upstream 仍受支持；只解析绝对名称，
不扩展 search suffix。systemd-resolved、split-scope、其他 loopback daemon、NSS
扩展、scoped IPv6、非已支持 resolver options 均明确拒绝，不展平或换公共 DNS。

P1c 的既有 main sink、`/32` pin 尚未迁移：当前 owner 不按形状认领或删除旧资源。
P3a 必须先接上新保护、确认既有资源的真实 ownership，再移除确实 owned 的 pin；
没有可靠 ownership 的 legacy 条目报告冲突。TUN 在主表发布 Mesh route 的旧入口
也必须迁到此 owner 的 capture block，不能在两套捕获长期共存时声称保护已上线。
Mesh 防回落使用 block 内专用 reject-only lookup，不依赖普通 `iif lo` 的 profile
capture，因此原系统/VPN lookup 消耗转发包之前就会拒绝已知 Mesh 前缀。新增表是
既有 guard/goto/NOP 模型的受限扩展；既有 capture selector 校验仍保持不变。

内核 RPDB 多条规则并非原子事务；外部管理员并发改变 routing/netfilter 的即时
fence 不由这个准备合同证明。准备、capture、恢复和 capability 交付均重新核验；
持续环境监控与普通主循环切换编排仍是协调者接线边界。下文旧验收记录保持冻结。

### 本轮实证与 P3a 接线边界

#### F1/F2 修复候选实证（旧证据保留）

`python3 apps/agent/tests/underlay_fixture/review.py` 的新 run
`mw-underlay-review-fix-127fb9bfd8cd`：`status=ok`、`cleanup_errors=[]`，25 个进程级
case exit 均为 0。result SHA256
`370436df98ff3618ec4c7a50699d0582f1da88238b979f3d84c323d7cd6a2157`，镜像
`sha256:078314788750a13fd2b1acab207c1dcb3e7c6e51a153fdb87940a94a58265c53`。
材料在批准 TMP `meshway-underlay-artifacts/mw-underlay-review-fix-127fb9bfd8cd/`。

- 冻结 A/B/C 的三个 row-only 不完整状态在真实 FIB 中重建，全部拒绝 capability、
  idempotent sink success 或 activate success；原 TMP 状态模型及日志未改。
- Mesh 两个资源、双栈 capture 四个资源的每个 committed-row 边界取消 future，
  `pending=None/operation=Some` 时 capability 被拒；同 owner 重试可靠续做并幂等成功。
- 初始 reservation 四个、Mesh 两个、双栈 capture 四个提交边界，逐点独立进程直接
  exit；新进程核对完整 intent/实际 ownership 后完成整组，仅完整拓扑可用。实际
  local lookup、Mesh forwarding `iif eth0` 及 IPv4/IPv6 capture 分界均校验。
- 同一 netns inode `4026532827`，不同 mount inode `4026532607/4026532618`，两容器
  都有独立私有 `/run` 和 state。retired factory 仍持有 abstract singleton 时，
  contender 得到 `AddrInUse`，没有 journal 或 kernel mutation；释放最后引用后可恢复。
- 测试持久 volume 不在 `/run`。旧容器 namespace 销毁后，新容器保留原始 journal、
  身份和配置文件；旧 scope 被私有归档，新 namespace 仅新建空闲 resources，身份 /
  配置字节未变。prior-boot 分支另外用 metadata 模拟验证，不冒称宿主 reboot 实测。

同一镜像重新执行原 fixture：
`python3 apps/agent/tests/underlay_fixture/run.py --skip-build --image meshway-underlay-review-fix:local`。
run `mw-rust-underlay-57c317201245` 为 `ok/cleanup_errors=[]`，result SHA256
`3186cce4e038839b1140ca91c942e782d17aee2e8efb3b8681aa55fb049388b0`；新 owner 20 次
protected pre-network calls / 2 ordinary connect / 0 ordinary bind，原 production /
UR2 / Native / Manager audits 保持 137 / 76 / 76 / 6。正常 TUN 创建、停止后的本地 /
转发拒绝、新 TUN 重接、controller/carrier 分离和双栈物理源继续通过。

host 独立 `target/underlay-review-fix-host`、`CARGO_BUILD_JOBS=2`：underlay lib 7 tests
及 all-features/all-targets Clippy 通过；Linux image 同样执行 Agent lib/tests Clippy。
两个早期 Clippy 构建失败 `fc447ffd8421/cc7009cd100a` 及首个通过 run `e8184b05fd0a`
保持原样。当前仍是待原 reviewer 复核的候选，未改 state/Manager/P3a，未生产启用。

Linux run `mw-rust-underlay-2186c3ba6d71`：`status=ok`、`cleanup_errors=[]`，UTC
`08:21:34.297782` → `08:22:54.820843`。镜像
`sha256:d05119b42c8362bf649052cc1ea1d07a8afe46dca7dc93ad8596083b3c6ce1ad`。
批准 TMP `meshway-underlay-artifacts/mw-rust-underlay-2186c3ba6d71/result.json` 的 SHA256
为 `802630b5a880b2fbf9a4c067c6a714f51cf9774892087495c5cc474d1e4ac7a6`。
run 记录基线 `4d106e0`、实际 diff digest、所有 underlay 与 TUN 源文件 digest；
image 内源文件逐字节核对。原 namespace 反例和此前失败 run 均保留。

- 自动选择真实双栈 RPDB reservation，无手工预装本轮 rules；验证 namespace / state
  排他锁、空表 alias 预留、未知 selector / foreign 保留、alias 后占用冲突、live
  部分安装精确回滚、完整 journal 预算在 mutation 前拒绝、已提交 partial prepare
  回滚重试，以及 ambiguous pending 拒绝。capability / 已退休 wrapper 继续持有锁。
- 新 `ControllerChannel` 经真实 Docker embedded DNS 发起生成 API 请求，HTTP 401
  status-first 有界处理；carrier 退休后 controller 仍可请求。`/etc/hosts` 优先、
  Docker 网络别名 A/AAAA、typed STUN bootstrap DNS / 独立 UDP probe purpose 均通过。
  新 `syscalls-activation.log` 有 **20 次** protected pre-network calls；只有预期的
  **2 次**同地址 ordinary TCP connect，真实 TUN 只捕获这两个双栈 SYN，无 protected
  DNS / TCP / UDP 泄漏。独立 probe 没有发布 listener 映射、peer 候选或地区。
- 两个**不同进程**执行 crash prepare / recover：第一个直接 `process::exit(0)`
  绕过 Rust Drop，第二个从私有 durable journal 与真实 FIB 恢复，旧 sink 保留，
  新 protected capability 可用。旧 socket / packet generation 不复活。
- 独立 NET_ADMIN + TUN namespace 调用 `Tun::open_with_underlay`，真实 lookup 指向
  `mwowner0` / table `10000`；withdraw / TUN 消失后本地与转发 Mesh lookup 均拒绝，
  新 `mwowner1` 可重接。controller TCP 的真实物理源为 `172.30.247.12` 和
  `fd74:6d77:247::12`。该 namespace 的实际 IPv6 状态为 `all=0/default=1/eth0=0`；
  default disabled inheritance 是现有 IPv4 TUN 的安全前提。仅设置 namespace 的
  `net.ipv6.conf.default.disable_ipv6=1`，没有宿主 sysctl、host network 或 privileged。
- 同次原 production / UR1 / UR2 / Native / Manager / permission-denied 均重新通过：
  protected pre-network audits 分别 **137 / 76 / 76 / 6**，原 listener 与普通同 IP
  对照保留。Native 的零 payload 关闭端口 RST 仍按原严格条件校验。

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/underlay-activation-host cargo test --locked -p meshway-agent --all-features --lib underlay:: -- --test-threads=1
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/underlay-activation-host cargo clippy --locked -p meshway-agent --all-features --all-targets -- -D warnings
python3 -m unittest discover -s apps/agent/tests/underlay_fixture -p 'test_*.py'
python3 apps/agent/tests/underlay_fixture/run.py --image meshway-underlay-activation:local
```

Host targeted underlay tests、fmt、all-targets Clippy 与 syscall auditor 已通过；Linux
image 构建执行 Agent all-features lib/tests Clippy。额外全量 Agent lib run 为
138 passed / 1 failed：既有 latency history 的 `received_at >= measured_at` 断言，
该项隔离复测通过；失败日志保留，未修改并行 owner 的测试或将其记成全量通过。

旧 run 真正验证的恢复范围为**同 boot / 同 netns 的进程恢复**。修复候选对相同
身份/网络的 boot 或 netns 变化另走**旧 scope 归档 + 新 scope 初始化**：先取得
新 netns singleton 和持久 state lock，审计实际新 namespace、预检分配与双栈 mark
权限，再把原始 journal 原子归档到私有 `underlay-scopes/scope-*.json` 并 fsync，
随后仅从新 namespace 的真实空闲资源初始化。归档保留旧 operation/pending/evidence，
不恢复旧权限，不按旧形状删除/认领新资源，不修改节点凭据与已保存配置。
身份/网络不匹配仍拒绝；旧 holder 仍持有共享 state lock 时也拒绝。该迁移不把
journal 放临时 `/run` 来丢弃历史。没有正常主循环及真机 reboot 验收前不宣称完整
生产恢复；真实容器新 netns 验证与 prior-boot metadata 分支验证分别记录。
只读 `/proc/sys` 下没有可靠 disabled inheritance 的 TUN 环境同样失败关闭。
真实物理换网、nested NAT、公网打洞、外部并发 RPDB / nft 监控以及 systemd-resolved
split scope 仍需后续环境适配与主循环联测。

P3a 的共享文件接点：Agent 持有 owner 和独立 channel，把 `channel.client()` 注入
所有 enrollment / heartbeat / material / DNS 同步 HTTP 消费者；在捕获之前把
`owner.socket_generation()` 与 `owner.system_dns()` 安装到已有 Manager 接点，
随后采用新 `Tun::open_with_underlay`。withdraw 时先 RootPacketGeneration fence
与 carrier join，再 `withdraw_tun` / drop TUN。正常 Agent 此轮尚无这些调用点；
正式已部署 Web、Apple、协议 / 生成器、根 manifest / lock 和 `state` / Manager 均未改。

状态：**内核机制证明 + Agent owning 工厂 / HTTP / bootstrap DNS + 托管 Native Runtime 接线**。托管 session 已统一注入 Native UDP/TCP 和 VLESS 外层 socket；内部 `Manager::install_prepared_underlay` 持有准备好的退休 owner，并在换代前 withdraw/join。Agent 日常构造仍使用明确 unmarked 的 `SystemUnderlay`；生产 RPDB owner/capture 尚未激活，P1c 目标 IP `/32` bypass 仍是已知原型限制。fixture 不代表生产已部署，不证明 VLESS/WebSocket、真实公网 NAT、GeoIP 或完整出口闭环已经完成。下方冻结 handoff / 旧实测是历史记录；最新接线边界见本文末节。

## 推荐：只跳过自有连续捕获规则

采用 `SO_MARK` + **正向 mark 匹配的 RPDB `goto`**，而不是“mark → main”或复制整个 underlay 路由表：

```text
socket: SO_MARK = preserved_foreign_bits | reserved_meshway_bits
IPv4/IPv6 自有连续 rule block（数值仅为 fixture）：
  priority 18000 fwmark <mark>/<mask> iif lo goto 18003
  priority 18001 iif lo lookup <当前自有捕获表>
  priority 18002 iif lo lookup <切换自有捕获表，仅切换期间存在>
  priority 18003 nop  # 必须存在的稳定跳转目标，继续原有系统 RPDB
  # > 18003 继续原有系统 RPDB；不增加强制 main lookup
```

受保护本地 socket 跳过**仅由 Meshway 拥有的连续捕获 block**，继续执行运行环境**原有 RPDB**。原有 main、VPN policy tables、企业网关、接口和防火墙没有被跳过或清空。普通 socket（包括同一进程、同一 IP、同一端口）仍匹配捕获 rule；转发 lookup 不匹配 `iif lo`，不能被节点自己的本地方案捕获。所有 Meshway 捕获入口（Mesh sink、默认出口、具体前缀、备用表与切换临时 rule）必须放在受保护跳转覆盖的自有 block 内；只保护出口 rule 不足以防止其他自有捕获入口循环。

**不能写 `not fwmark … iif lo lookup …` 来表达两个独立条件。** Linux RPDB 的 `not` 反转整条 selector 的匹配，而不是仅反转 fwmark；这会错误匹配非 `lo` 转发流量。fixture 包含 IPv4/IPv6 的真实 `route get … iif eth0` 反例与修正后的 lookup。若只使用单独 `not fwmark` 而没有 `iif lo`，同样不能证明出口节点转发不会卷入本地方案。

这不是目的地址白名单，不使用 UID/进程豁免，不给 AAAA 安装无条件 bypass。mark 只属于内部明确用途：controller HTTP、STUN/认证建连 probe、carrier 外层，以及这些对象的 bootstrap DNS。内层业务仍执行 grants 与完整有序 via；protected factory 不向业务路由的 `direct` 开放。mark 不传递跨主机身份或访问授权。

从 P1c 迁移时须在新 socket 保护准备完成后退休原型自有 `/32` 全目的 bypass lookup/table；两者长期共存仍会放行同地址普通业务。保留故障关闭的业务 sink，并实测普通 socket 捕获后才报告迁移完成。

实验的 `0x4d570000/0xffff0000`、连续 priorities `18000–18003`、tables `18401/18402` 仅属于隔离 fixture，不是生产固定分配。mark 不是对具有 `CAP_NET_ADMIN` / `CAP_NET_RAW` 的恶意本地进程的安全隔离；不能承诺这类进程无法自行设置相同 mark。

### 原有 VPN 与优先级

实验保留一条不同低位 mark 的系统 rule / table；受保护 socket 保留低位，真实 STUN 返回该表选择的源 IP，而不是 main 的源 IP。这验证 **RPDB 兼容机制**，不是验收所有 VPN 实现。生产必须观察实际系统 rules / marks / route tables / netfilter mark 改写，确认自己的捕获插入位置与现有 VPN 共存。遇到未知 selector、mark bits/表/priority owner 冲突或不确定的 nft/iptables 改写，应拒绝激活并提供诊断，不能强制抢占、清空规则或绑定物理接口绕过 VPN。

如果既有系统策略在 Meshway 捕获之前就消耗了普通业务查找，不能宣称业务已被 Meshway 捕获。须明确识别冲突，协商集成顺序或失败关闭；不要删除其他 VPN rule 来“修复”。规则改变不是多对象原子内核事务，外部管理员同时修改 RPDB 是正式实现需要监控/重检的边界。

## 建议的内部接口

放在已有 Agent 内部模块，不新增产品 crate，不复制 HTTP DTO：

```text
UnderlayRouteGuard::prepare(namespace, owned_reservation, routing_environment)
  -> PreparedUnderlayGeneration
ProtectedSocketFactory(generation)
  .udp(purpose, family, bind)
  .tcp(purpose, family, local_bind)
  .accepted_tcp(listener_generation, accepted_fd)
  .bootstrap_resolver(purpose, upstream_configuration)
```

- reservation 持久绑定 namespace/network 与所有者；记录完整自有 rule/route 属性而非只凭固定名字认领。要求连续 priority block 内无 foreign rule，不得让 goto 跳过任何既有系统 rule；block 最后保留自有稳定 NOP 目标。**Linux goto 需要 exact-priority 目标存在**，不能只填一个没有 rule 的“下一优先级”；unresolved goto 会 fall-through 到捕获。先安装 NOP，再 guard，核对 resolved 后再发布 capture；退休时先停止相关 socket/移除 capture，再 guard，最后 NOP。系统在 block 前的规则仍正常执行。空表也可能已由其他服务预留，内核“暂无 routes”不等于无 owner。
- 工厂创建 unconnected FD，校验已有 mark 的所有权，保留非自有 bits，设置并读回 `SO_MARK`，然后才 bind/connect/send；设置失败关闭 FD，绝不降级为 unmarked。
- 只接受内部 enum 用途，不接受来自 Web/IPC/业务 policy 的任意“请保护这个 socket”。接收合法下一跳仍须原有认证与连接授权。
- 代际持有取消 token、受保护 socket/HTTP pool/resolver 任务、观测关联与到期；不能仅在调用 `connect` 时临时设置一个全局状态。
- 先验证双栈保护，再发布捕获/applied。能力或权限不足是 failed，不是 saved ACK 或可达候选。Linux 设置 `SO_MARK` 需要 namespace 内 `CAP_NET_ADMIN`，或 Linux 5.17+ 的 `CAP_NET_RAW`；安装 RPDB / TUN 仍需 `CAP_NET_ADMIN`。实验不需要 `NET_RAW`，降权子进程真实证明无权限时拒绝。

## 现有代码的接入位置与缺口

### controller HTTP / reqwest / DNS

生成 `apps/agent/src/generated/client.rs::HttpClient::new` 的原 reqwest 默认行为保持兼容；生成器的 checked `transport.rs` adapter 增加 `HttpTransport` / `with_transport`，所有 14 个操作均在生成器中核对并注入，同一份生成 URL、bearer、JSON body、status-first 与 bounded response 解码继续使用。注入失败绝不调用原 reqwest fallback；生成文件只通过生成器更新。

本次核对本地依赖 **reqwest 0.13.5**：`ClientBuilder::interface` 是接口绑定，可能绕过其他 VPN，不是上述机制；`connector_layer` 包装连接服务，并不提供可靠的 pre-connect socket mark hook。实际实现选择维护中的 hyper 1 HTTP/1 connection API + tokio-rustls：`SocketFactory::tcp` 先创建 / 标记 / 读回 / connect，之后 TLS 和 HTTP 才获得 `ProtectedTcp`。未 vendor 或 patch reqwest。每个 IPv4/IPv6 候选单独走工厂，2 秒候选超时，整体请求 / 响应 10 秒期限；当前为顺序候选，不声称实现 happy-eyeballs。上层重试也只能重新进入同一个注入 transport。TLS 使用系统 trust roots、hostname 验证与 HTTP/1.1 ALPN；不关闭证书验证。

`ControllerHttp` 绑定 controller origin，拒绝 origin 变更和 proxy 环境变量；不跟随 redirect，不支持显式代理、HTTP/2、QUIC 或 DoH。这是有意收敛的 HTTP/1 实现，无连接池：每个请求独占一个已标记连接，response stream 持有唯一 driver 任务，body 完成 / 丢弃 / 错误 / 超时均 abort driver 并关闭 TCP。取消请求 future 也会 abort driver；不能靠只 drop client handle 宣称退休。生成客户端在注入 transport 时仍创建 reqwest request builder，但从不执行该 builder 的 `send`，它不会另开连接。

`BootstrapResolver` 的 UDP 与 truncated response TCP fallback 都走同一代际 factory，socket 用途为 `BootstrapDns(Controller / Carrier / AuthenticationProbe / Stun)`；不接受业务 DNS 请求入口。查询随机 transaction ID，校验来源、question、响应类型与 opcode，仅采用对应 owner/CNAME 链的 A/AAAA，缓存按用途、name、type 和该 resolver 的 generation 隔离，并受真实最小 TTL 限制。缓存最多 256 项，不缓存失败 / NODATA，不刷新旧代际时间；每个上游 3 秒 timeout，按系统 snapshot 的顺序尝试。任一家族 transport 错误不会伪装成 NODATA 或触发 getaddrinfo。

支持范围为内部 routing owner 提供的 **plain ordered system/VPN DNS snapshot**，保留其 server 顺序与原 RPDB，不选公共 resolver、不绑定物理接口。`from_plain_system` 只读取 plain `/etc/resolv.conf`，拒绝 loopback stub 和 systemd-resolved 拓扑。systemd-resolved / 其它 daemon、split-DNS、per-link/scoped IPv6 upstream、proxy / DoH / DoT 的完整系统语义尚无适配；NSS / mDNS / `/etc/hosts` override 也尚未适配，不宣称等价于完整系统 resolver。调用者不能把这些组合压成全局 nameserver 列表后声称保持系统分流。遇到已知或无法证明为 plain topology、或者目标依赖尚未适配的本地名称 override 的环境，正式 routing owner 必须拒绝激活，不静默用 DNS 替代该 override 或 unmarked fallback。绝对 hostname 不采用 search suffix；bootstrap 不是业务名称解析。不能把系统 `getaddrinfo` / `tokio::net::lookup_host` 自动等同于 protected DNS。

### UDP listener / outgoing / STUN

Native UDP 的 listener/outgoing socket 创建处改走工厂；`socket2`（或 `tokio::net::TcpSocket` 对应路径）在交给 tokio 前设置 mark。STUN 与认证 probe 复用 carrier 的实际 FD，保留 listener/outgoing 资源关联，不另造 fake identity。UDP listener 的所有 sendto/reply 都用相同 mark；connected UDP 的首次 connect 前也要设置。探测结果必须校验 observer、STUN cookie/transaction、socket generation、underlay generation、network/revision/lease 与过期时间；失败不能输出 mapped endpoint 或地区。此 fixture nonce echo 只检验保护路径，**不是生产 authenticated probe 验收**。

### TCP/TLS / VLESS / WebSocket

当前 `apps/agent/src/managed_connections/session.rs` 中 `TcpStream::connect` 之前须改由工厂创建/连接，得到 protected TCP 后再套 TLS、WebSocket、VLESS 握手。协议承载配置与逻辑路径不用因为 mark 改变。fixture 只使用 plain HTTP TCP 与验证证书的 TLS 连接，没有 VLESS/WS 握手。

TCP listener 在 listen 前保护，Linux accepted socket 通常继承 listener mark，但生产必须读回/验证继承；已经发出的 SYN-ACK 不能靠 accept 后补标记召回。缺保护时不要发布该 listener 可用。正式验收应覆盖 incoming/outgoing、WS upgrade、TLS SNI/证书失败、连接池与 reconnect，不能把 outgoing TCP 成功推广为监听可达。

## 换网、profile 与生命周期

1. 换网/endpoint/DNS 变更先准备新保护代际与新候选解析；准备失败不能偷偷使用 TUN 返回的观测。
2. 撤销旧观测新鲜资格，取消旧 probe/resolver/pool，停止并关闭旧 socket。
3. 激活新 socket；真实 STUN 完成后才发布绑定新代际的观测。旧结果、重报或配置续租不刷新时间。

公共规则与 profile 只改业务表；系统 guard 跳转与保护资格不属于可排序 rule。双表切换的临时 lookup 必须处于 guard 覆盖的自有 block。实验通过两张捕获表（同样保留前缀和 default）模拟此边界；没有编译真实公共规则、验证 grants 或证明产品 profile 功能完成。

关闭 socket 是未发送数据的 fence，不等于撤回已写入内核、已发到网络、已到达服务端或已进入对端队列的字节。正式数据面 R3 packet-generation fence 由另一个任务处理；本实验不重写 runtime/pump。HTTP pool/resolver 后台任务若仍持有 FD，单纯丢弃一个 client handle 也不能证明退休完成。

保护路由与业务 sink **独立生命周期**：控制断连/lease 撤回停止旧业务 I/O、保持自有 unreachable sink；必要 controller 恢复通道可按独立授权的系统生命周期保留。保护失败必须停止相关 socket 与候选，不能移除业务 sink 并回落底层。TUN 退出后普通受捕获流量仍 unreachable，保护 socket 仍通过原 RPDB；确认取消接管才精确移除自有捕获 rule/routes。只清理本所有者资源，不能 flush main/所有 rules/其他 marks/tables。恢复旧代际需可信持久 ownership；fixture 整个 namespace 最终销毁，不模拟 crash recovery 已实现。

## 实验入口与正式集成还需验收

可重复命令、每项证据与实测结果见 `experiments/underlay-routing/README.md` 和 `RESULTS.md`。仅独立 Docker namespace，IPv6 检查是 **双栈 underlay**，不是 IPv6 overlay。

协调者后续需：统一所有自有捕获入口；合并主循环接线与 socket 类型适配；真实 Native UDP/STUN/认证探测、VLESS+WS；ownership 持久恢复与外部 RPDB / nft/iptables mark 改写监控；系统 split-DNS adapter；换网时并发取消与 R3 fence 联测。CI 可加入 host-only tests，再在有 `/dev/net/tun` 的 Linux Docker runner 上运行生产代码 fixture；本分支不编辑根 CI。

## 可复用生产切片与接线 handoff

- `generation.rs`：`MarkReservation` 验证与保留 foreign bits；`PreparedGeneration::prepare` 核对双栈真实 RPDB 的连续 guard / capture / stable NOP，拒绝 duplicate / foreign / unknown selectors、保留 mask 冲突和 block 外捕获表引用；先验证双栈 SO_MARK 权限再返回工厂。这里只核对 routing owner **已经安装**的可信 reservation，不凭名字认领规则，不安装 / 删除 RPDB，不实现 ownership 恢复；外部并发改写监控和 nft/iptables 审计仍必须由 owner 完成。
- `socket.rs`：crate-private typed purpose，UDP bind / connect / connected send / sendto，TCP optional local bind / pre-connect 与 listener pre-listen，accept 只验证继承而不事后修补。每个 application write / UDP send 都核验 mark；全代际 Mutex 与 I/O poll 共享退休 fence，取消唤醒等待任务，TCP shutdown。没有公开 raw FD、into_inner 或普通业务 `direct` 调用入口。不支持平台时明确 `Unsupported`，不降级。
- `PreparedGeneration` 是唯一退休 owner，drop / explicit retire 都使 clone factory、resolver、HTTP 失效；FD wrapper 若被调用者继续持有，其 FD 可能仍保留，但 TCP 已 shutdown、application I/O 全部 fenced，后台 HTTP driver 随取消 / response drop 结束。该边界不能撤回已发送字节。不得把“仍持 FD 但退休”描述成 FD 已全部关闭。
- `controller.rs::ControllerChannel` 持有独立 controller generation 与注入 transport。业务 lease / sink 退休不得连带撤回必要 controller 恢复通道。换 controller endpoint / DNS snapshot / network 时由 owner 先 prepare 新 channel，然后替换并退休旧 channel；更换业务 profile 不重新分配 underlay 资格。
- `docs/underlay-integration.patch` 是 **未应用** 的最小 state HTTP 构造接线：所有生产 `HttpClient::new` 位置通过 `Agent::controller_client`，保留 enrollment token / node credential / response size 差异；独立 DNS service 的后台 Sync 保存注入 client，不能重新创建普通 client。验证应用命令 `git apply --check --unidiff-zero docs/underlay-integration.patch`；已在隔离的 HEAD archive 应用 candidate + handoff 后通过 Agent all-features/all-targets Clippy `-D warnings`，材料目录 `meshway-underlay-integration-a7sbmdfw`。P2a 稳定后协调者再合，并完成 channel 安装门槛：捕获发布之前必须已安装 prepared channel，退休后不得设回 None 或使用默认 reqwest 重试。独立 DNS service 在 controller generation 更换时须同步 restage，否则它保留的是已退休 client，并将持续失败关闭。patch 不实现 routing owner，也不触碰 P1c 主循环。
- Carrier handoff：冻结 P2a `56da5ad` 后采用下方 `underlay-p2a.patch` 的统一 owning hook，不再直接替换或重写 session 握手。`peer_addr` / `local_addr` / `set_nodelay` 由 `ProtectedTcp` 提供。Native UDP/STUN 保留同一 `ProtectedUdp`，探测仅借用该 FD 的受限 send/recv，不另造 socket。接收协议与认证仍由 P2a owner 管理；补充 hook patch 未应用。

### 依赖与复现

未改变 root manifest；Agent 增加的直接依赖都已在原 lock 中存在，只需 root lock 的 package dependency list / tokio-util feature 闭包更新，候选单列 `docs/underlay-lock-candidate.patch` 供协调者维护。新依赖均使用安全 API；workspace `unsafe_code = forbid` 保持生效。

```sh
# 独立 worktree 初次复现，由协调者先应用 lock candidate。
git apply docs/underlay-lock-candidate.patch
CARGO_TARGET_DIR=target/underlay-host cargo test -p meshway-agent --all-features --lib underlay::
python3 apps/agent/tests/underlay_fixture/run.py
```

新 fixture 编译 **实际 Agent 单元测试二进制**，独立 Docker 双栈 namespace 中安装原 guard/goto+stable NOP 与 default TUN sink，验证真实 HTTP / TLS / DNS UDP+TCP fallback、证书失败、generated status-first、listener SYN-ACK / accept、foreign bits / 原 VPN table、代际退休 / DNS 与 HTTP 取消、权限不足、同进程同 IP/端口普通 TCP / UDP 被捕获且没有 unmarked retry。完整 syscall log 和 observer 事件在批准 TMP `meshway-underlay-artifacts/mw-rust-underlay-*/`。runner 使用同一 performance lock / 继承 FD，最长 2400 秒包含等锁 / build / run；不使用 host network / privileged 或宿主配置。fixture RPDB 精确清理只属于 namespace 测试，不代表生产 crash recovery。

### 2026-10-02 生产代码 fixture 实测

最终 run `mw-rust-underlay-0f2e5cad741b`：`status=ok`，Rust 行为与真实降权测试 exit 均为 0，`cleanup_errors=[]`。镜像 `sha256:f0354bda2845e317f6daae3d0fef98079632fc7611d6b5e64bb7596ab7acae07`；运行时间 UTC `00:28:57.510445` → `00:30:19.634958`（含 image build / Linux Clippy）。

syscall audit 实际验证 **137 次** protected bind/connect/listen 前的 mark + readback；**4 个** listener 均 premark，其中双栈 loopback listener 的 wrapper 被刻意保留跨退休，退休后真实 connect 均 `ConnectionRefused`、accept 返回取消，证明不是仅丢弃一个 owner handle。8 次普通 connect 包括 6 次同远端 HTTP/TLS/DNS capture 对照与 2 次退休 listener 反向验证；4 次普通 UDP bind/send 对照。真实 TUN 只出现预期的 10 个远端普通 socket，protected HTTP、TLS、DNS fallback、listener reply 及重试没有进入 capture。

原始 `result.json`、`observer.jsonl`、`syscalls.log` 保留于批准 TMP `meshway-underlay-artifacts/mw-rust-underlay-0f2e5cad741b/`。首轮自签 CA 被错误当作 end-entity 的验收失败 `ae888cc93713`、缺 Linux Clippy component 的 build 失败 `082e46e8fea3` 都保留为失败记录，不计作通过；最终使用独立 CA 签发的 serverAuth leaf，未放宽 Rust TLS 校验。先前完整路径通过但尚无退休 listener / 完整 syscall audit 的 `166881604810` 也保留。

已通过 host `cargo fmt --check`、workspace all-features/all-targets Clippy、Web check、SDK typecheck + 12 个行为 tests、4 个 experiments 的 `tsc --noEmit` 和 protocol drift check；Linux Docker build 另外执行 Agent all-features/lib/tests Clippy `-D warnings`。Host syscall auditor 2 个 tests 验证缺 readback / SO_MARK EPERM 会被拒绝。

最终全消费者 Rust 检查也已通过：在同一 performance lock 下串行执行 `cargo test --locked --workspace --all-features --no-fail-fast -- --test-threads=1`，含 Agent、Apple core、Controller、dataplane、policy、protocol 与各 transport / doctests。此前并行构建时 dataplane 洪泛恢复测试曾得到 Timeout；当前分支和 `9c7d922` 隔离复测都通过，最终串行全套的 dataplane 70 tests 均通过。没有修改该测试或 dataplane 代码来规避失败。

## 冻结 P2a `56da5ad` 的补充整合

`docs/underlay-p2a.patch` 是 **未应用** 的补充 patch，顺序为本分支代码 → P2a `56da5ad` → state HTTP `underlay-integration.patch` → `git apply --recount --unidiff-zero docs/underlay-p2a.patch`。根 manifest / lock 由协调者合并，保留 P2a 的 `transport-runtime`、managed feature 脱离 lab，以及 v10 / 17 个生成操作；不替换其 schema 或模型。

- `managed_connections::sockets` 增加 crate-private visibility；保留 `Protection::prepare/accepted` 的默认行为，新增 owning `listen/dial` 与 bootstrap `resolve` seam。默认 `SystemUnderlay` 仍是 **unmarked**，不因为存在这个 trait / patch 就具有 SO_MARK。
- raw `TcpSocket/TcpStream` 不能携带代际应用写 fence。protected 分支通过 `OuterListener/OuterTcp` 直接持有已有 `ProtectedListener/ProtectedTcp`，不维护重复 FD 的强引用 ledger，不向 session 交付 `into_inner` raw socket。TLS / WS / VLESS 原握手逻辑只接收这个通用 AsyncRead/Write stream，没有重写协议。
- 新 `underlay::ManagedProtection` 构造只接受已准备的 `SocketFactory` 与内部 `SystemDnsSnapshot`。listener / candidate / peer ID 在 P2a 的原 `StreamContext` 保持关联，不作为 bypass 条件。protected adapter 对旧 raw `prepare/accepted` 路径返回 Unsupported，避免无 ownership 的 FD 逃逸；实际统一 helper 使用 owned seam。
- `Manager::new_with_protection` 与 async `install_socket_protection` 是内部注入点，后者先 withdraw / join 旧 session，再替换 factory；withdraw 保留既有业务 sink。`resolve_targets` protected 分支调用同一 generation 的 bootstrap resolver，不再对 carrier 域名调用 unmarked `lookup_host`；legacy SystemUnderlay 的默认行为保留。
- P2a 新材料能力 HTTP 调用也通过 `Agent::controller_client`；生成器的 `secret_debug.rs` 与 `secret_debug::adapt` 原样保留，transport adapter 独立作用于 client。补充生成使用 P2a OpenAPI，17 个操作全部通过 checked transport adapter。
- 此冻结 handoff 当时 **Native UDP / core Native TCP 的 Runtime socket 注入未接线**；已由本文末节的后续切片接入。guard/capture allocator、ownership/恢复、系统 DNS 非 plain 拓扑和 sink 迁移门槛仍由正式 routing owner 完成。

组合检查入口 `python3 apps/agent/tests/underlay_fixture/combine.py --p2a 56da5ad`：在批准 TMP 的独立 archive 组合代码 / manifest candidates / 两个 handoff patch，保留 P2a secret Debug 保护并重生成客户端，检查 Agent all-features/all-targets Clippy、默认无 features、仅 managed-connections 三种组合。它不会修改 P2a 冻结 checkout、当前分支源码或协调者 lock，不执行 Docker，也不把组合编译当作新的数据面实测。

最终组合材料：批准 TMP `meshway-underlay-p2a-r2fyii3r/result.json`，`p2a_commit=56da5ad`、`status=ok`，5 个命令 exit 全部为 0。原 `0f2e5cad741b` namespace 证据对应独立切片 `8ad2729`；组合阶段只重生成 / 编译 / Clippy，没有重新运行 Docker 或扩大其握手 / 数据面验收声明。

## UR1 / UR2 审查后修复（原证据保持冻结）

UR1：`PreparedGeneration::prepare` 改为 `ip -N -j -4/-6 rule show`，table 的判断使用数值身份而非 rt_tables 显示 alias。validator 同步识别标准 local=255 / main=254 / default=253 / unspec=0 的数字、数字字符串与旧 canonical 名称；其它无法解释的名称明确拒绝，不再跳过。备用表尚无 block 内引用 / guard-only 仍可作为 prepared staging，但任何已存在的 block 外自有表引用都必须拒绝。未知 selector、外部 mark mask 冲突与 VPN table 检查保留。

原 Astra 复现目录与 target 未修改。`python3 apps/agent/tests/underlay_fixture/review_regression.py --stage ur1` 在新的批准 TMP 副本以实际新源码回归原反例，9 tests 通过；命名 / 数字备用表外引用均拒绝，两个合法 staging 场景仍接受。该阶段刻意保留 UR2 的原坏结果断言，不能声称 DNS 已修复。材料 `meshway-underlay-review-fixed-n41wioum/`。新增 Linux alias case 的实测结果在后续 UR2 联合 fixture 中另行记录，旧 137-call 证据不用于证明此修复。

UR2：合法 SERVFAIL 现在记录真实错误并尝试同一个 `SystemDnsSnapshot` 内的下一台 protected 上游。该列表只能表达同一已确认 system/VPN 解析 scope 的有序替代，不能把 split-DNS 的不同 scope 展平；没有 ambient / public / unmarked resolver fallback。REFUSED 作为策略拒绝终局，不用备用规避；NXDOMAIN 与其它非重试服务响应也终局，不改写为 NODATA 或成功。所有已配置上游失败时返回最后一次实际服务 / 传输错误，原每上游 3 秒与 generation 取消仍适用。

`review_regression.py --stage all` 在新副本以 byte-for-byte 新 DNS 源码运行原控制流反例，9 tests 通过：主 SERVFAIL / 备用成功均有 A/AAAA 查询，全部失败仍报 Server Failure，NXDOMAIN / REFUSED 不查询备用；generation 取消无备用发送，两台 silent 同 scope 上游在 6 秒虚拟期限（加 timer-wheel tick）终止。材料 `meshway-underlay-review-fixed-k_d08hv0/`。这是 host 内存 double 的控制流证据，不替代真实 Linux DNS 与 SO_MARK 联测。

本次没有修改 HTTP DTO / 生成器、P2a WS packet fence 或 client error 脱敏 adapter，也没有重跑 P2a 组合。继续保持 SocketGeneration != PacketGeneration：underlay 退休不能替代业务 revision 的共享 packet fence。默认 SystemUnderlay 未激活 protected path；RPDB owner / 生产 activation、Native Runtime 接线与非 plain DNS 适配仍是前述明确残余。

新增 Linux 联合验收 `mw-rust-underlay-a8cbbee43ee1`：`status=ok`、`cleanup_errors=[]`，镜像 `sha256:ba39286df41a5bef127eef38637c898e4e4f497a2e00a857a45ee9805f5fb93c`。原 `0f2e5cad741b` result 未改。新 result SHA256 为 `7ceec2895016af93b1c528cd60e7e4dbcfa913f51c3d6f16c387d1676b743d8e`，材料在批准 TMP `meshway-underlay-artifacts/mw-rust-underlay-a8cbbee43ee1/`。

- UR1 双栈真实 namespace 中把 18402 命名为 `mw_capture_two`，向其加入 default TUN route 和仅 block 外 19001 rule；18001/18401 capture 保持、18002 未安装。实际新 `PreparedGeneration::prepare` 两次均以 owned capture table outside block 错误拒绝，exit 0。result 同时保存 `ip -j` 与 `ip -N -j`：前者显示 alias，后者显示 18402，以及 local=255/main=254/default=253 数字身份。新增 rule/route 按完整属性移除，alias 文件只在容器 tmpfs 内、随 namespace 销毁。
- UR2 observer 的 12 个双栈 case 全部通过：每族主 SERVFAIL 的 A/AAAA 各一次，健康已配置备用 UDP 各一次；截断的备用响应另走 protected TCP；全部失败确实查询两台且返回服务失败；NXDOMAIN / REFUSED / generation 取消的备用查询数均为 0。独立 `syscalls-ur2.log` 验证 **76 次** protected bind/connect 前 mark/readback、0 ordinary connect/bind。
- 同一次新 run 重新运行既有完整 production_paths / permission_denied：双栈 HTTP/TLS/DNS、listener 退休、VPN table / foreign bits、普通同 IP capture 均通过，新的基本路径 syscall audit 为 137 次，4 listeners premark；真实 TUN 仍仅出现预期 10 个远端普通 socket，无新增 backup / fallback leakage。该 137 次是此次新执行，不借用旧 run 证明新增 case。

复现命令：`python3 apps/agent/tests/underlay_fixture/review_regression.py --stage all`；Linux 用 `python3 apps/agent/tests/underlay_fixture/run.py --image meshway-rust-underlay-ur-fixes:local`，runner 仍持同一 performance.lock 到精确 cleanup。构建与 host 检查均 `CARGO_BUILD_JOBS=2`，新 target 为 `target/underlay-ur-fixes` / `target/underlay-ur-repro`，旧 target / Astra 原副本保持。root lock 只临时使用原已交付 candidate，候选内容未变，结束恢复协调者维护边界。

针对修复的 host 检查：Agent `--all-features --lib underlay::` 5 tests 通过；Agent `--all-features --all-targets` Clippy `-D warnings` 通过；fmt check 与 syscall auditor 2 tests 通过。Linux image 构建同样执行 Agent lib/tests Clippy。公共契约、生成器和产品接线未改变，本轮没有把旧全消费者 / P2a 组合结果冒充新增复测。

## Native owning Runtime 接线切片

- `meshway_dataplane::sockets` 提供克制的 owning `NativeSocketFactory`、UDP/listener/TCP I/O trait 与 typed listener/dial metadata。无 raw FD、`into_inner` 或业务 `direct` 接口；原有 owned buffer、Batch、队列上限、身份、Noise/E2E、Access 与路径校验保持原边界。`Runtime::start` 兼容原 system sockets，`start_with_socket_factory` 注入失败不重试默认工厂。UDP receive buffer 在 bind 前保留原配置语义。
- `Session::start_with_packet` 将同一 `Protection` 用于外部 VLESS、core native bind/listen/dial；`resolve_targets` 也经该对象解析。`ManagedProtection` 只使用已 prepared factory 与 plain DNS snapshot：Native UDP/TCP hostname 在管理层由同一 bootstrap resolver 解析成数值候选，每个地址重新创建 owning socket；Runtime factory 只接受 typed `SocketAddr`，不另开 DNS。TLS/WS/VLESS 握手仅换 owning 外层 stream，没有重写协议。
- `ProtectedUdp::writable/try_send_to` 使 STUN 的既有 readiness/try-send 路径也在同一个实际 carrier FD 上执行 mark 回读与退休 fence。其余 send/recv 同样持有保护代际。accepted TCP 继续验证完整继承 mark；listener 使用 `SO_REUSEADDR` 支持换代重绑，未启用 `SO_REUSEPORT`。
- `Manager::install_prepared_underlay` 先 withdraw/join 旧 session，保留业务 sink，然后安装 `ManagedProtection` 并持有唯一 `PreparedGeneration` owner；替换或 Manager drop 使旧保护代际退休。session watchdog 独立监听保护取消、业务 withdraw 与 lease；保护退休不继续报告 Applied，也不能通过一次成功保存 ACK 续命。controller 恢复通道的 owner 仍独立，业务 Manager 不持有它。
- Runtime 另有独立 `SocketGeneration`：每次 native I/O poll/try-send 和退休共享 fence，等待 read/accept/connect 会被唤醒；`replace_native_socket_factory` 先退休/join native engines，再在原 listener 地址/端口上重绑，失败保持 native 关闭，可用新 capability 在**同一 Runtime**恢复。它不恢复 PacketGeneration、不重新授权、不更换节点身份；保留 Runtime 的 E2E 缓存。初次 `:0` 分配的 listener 端口在该 Runtime 重绑时固定；outgoing UDP 是新 socket，可获得新端口。旧 STUN 请求按不可复用的内部 socket-generation identity 校验，不能因 ordinal/地址恰好相同而接受旧结果。
- 退休只 fence 未发出的应用 I/O；不能撤回已经进入内核/网络的字节。被外部调用者保留的 wrapper 可能仍持 FD，不能描述成全部 FD 已关闭。native engine join 后 listener 已丢弃；core TCP channel 和认证任务被取消/错误唤醒后丢弃。可用 listener 列表描述初次 bind，失败/退休状态须结合 manager observation，而非把固定地址清单当作持续可达证明。

### 激活门槛与剩余项

正常托管 Runtime 现在确实走统一注入 seam；**正常 Agent 默认仍 unmarked**。`install_prepared_underlay` 是 routing owner 的内部接点，并不是 CLI flag 或新本地配置入口。正式主循环尚无生产 reservation allocator、持久 ownership/恢复、外部 RPDB/nft mark 监控、全自有 capture block 发布或 `/32` pin/sink 迁移事务；它也尚未自动调用 `PreparedGeneration::prepare` / 安装 prepared controller channel。没有准备并验证这些资源之前，不能声称系统保护或全 IP 接管已激活。

后续 owner 必须先 prepare 双栈 guard/capability 和 bootstrap scope，在自有 capture 发布前安装 controller channel 与业务 Manager capability；换网/endpoint/DNS scope 更新先准备新代际，取消旧观测并 withdraw/join，再安装/重建或显式 Runtime rebind。保护失败保留 sink，禁止将 Manager 改回默认 system factory续跑。完整系统 split-DNS/NSS/local overrides、真实公网 NAT 打洞、自动建连主备策略及正式出口捕获仍是后续主线。本切片不安装/删除宿主路由、DNS、防火墙，不改 HTTP 契约/生成器/根 manifests。

### 验证入口

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/native-underlay cargo test --locked -p meshway-dataplane native_sockets -- --test-threads=1
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/native-underlay cargo test --locked -p meshway-agent -p meshway-dataplane --all-features --lib -- --test-threads=1
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/native-underlay cargo clippy --locked -p meshway-agent -p meshway-dataplane --all-features --all-targets -- -D warnings
python3 apps/agent/tests/underlay_fixture/run.py --image meshway-native-underlay:local
```

Host native tests是实际 loopback sockets 与注入失败测试，不能证明 SO_MARK。Linux runner 在同一 performance lock 下使用原隔离 capture/TUN fixture：新增双栈远端 Native UDP/TCP listen/dial、Noise/E2E probe、同 Runtime prepared 换代与失败关闭、实际 UDP FD STUN；独立 `syscalls-native.log` / `syscalls-managed.log` 审计 pre-bind/connect/listen mark/readback，旧普通同 IP capture 对照继续运行。server peer 仅在无 capture 的隔离 server namespace 使用 system sockets；client Native Runtime 必须通过 prepared 工厂。Linux 与 host target 独立，旧 run 证据不修改；本轮结果在完成实测后追加。

Host 最终检查：Agent lib 134 tests 通过；dataplane lib 80 tests（含 4 个新增 native owning/failure/rebind/等待唤醒行为测试）通过；测试 fixture 拆出固定端口入口后，Agent `managed_connections::tests::` 11 tests 再通过。Agent/dataplane all-features/all-targets Clippy `-D warnings`、fmt check、syscall auditor 2 tests 均通过。默认无 features、仅 managed-connections、仅 transport-lab 的 lib Clippy 使用独立 `target/native-underlay-features` 串行通过。

过程中的失败保留：首个全套 Agent run 的既有延迟历史 `received_at >= measured_at` 时间顺序断言失败，隔离复测与随后完整 134 tests 均通过，未修改该断言。dataplane 原 engine-abort 测试依赖旧 `Inner.handles[1]` 的位置，已改为明确取消独立 native owner 的 engine，仍验证 pending STUN 关闭、端口释放和 permit 回收，最终完整 80 tests 通过。

Linux fixture 进一步区分关闭 listener 窗口的 **socket-less kernel RST/ACK** 与受保护 socket 的发送：记录完整 raw packet、TCP flags 与 payload 长度，只允许明确 native listener 源端口的 `RST|ACK=0x14`、零 payload、正确双栈源/目的组合作为该控制包；任何 SYN/SYN-ACK、数据或其它额外 socket 仍报错。此 RST 继续进入自有 capture/sink，未新增豁免。Native UDP 保持原 unconnected sendto 模型，STUN 与邻接握手复用实际 FD；没有为规避捕获而改成 connected UDP 或跳过其验证。

### 2026-10-02 Native 实测结果

最终 `mw-rust-underlay-60c61dc9ee96`：`status=ok`、`cleanup_errors=[]`，UTC `05:55:28.154945` → `05:56:29.383110`，镜像 `sha256:a4b0760e8d431a33c3d8de6e2f1a03e18d34c70589fc230903e0c83ffc1459ec`。材料在批准 TMP `meshway-underlay-artifacts/mw-rust-underlay-60c61dc9ee96/`，`result.json` SHA256 `9d3d428694c63d0652f859096c02d5ecdcd1dc30ad0b3322b8a82f65797d31f1`。

- Native 双栈 UDP/TCP dial/listen、Noise/E2E 往返、prepared 退休后的失败关闭、同 Runtime 新代恢复均通过；peer test exit 0。`syscalls-native.log` 验证 76 次 protected pre-network calls、4 个 premarked listeners，ordinary bind/connect 均为 0。
- Manager 固定端口 owner 安装、保护退休使 Applied 撤回、新 owner 恢复、withdraw 均通过。`syscalls-managed.log` 验证 6 次 protected pre-network calls、2 个 premarked listeners，ordinary bind/connect 均为 0。测试不用 host 的端口可用性 allocator，因此没有未标记的测试预探 listener。
- 4 次真实 STUN 返回对应实际 UDP FD 的端口，IPv4/IPv6 源地址按真实系统选择校验。capture 共 12 包：原 10 个 ordinary socket 对照，另 2 个 native listener 关闭窗口的 RST/ACK，flags 均 0x14、payload 均 0；raw packet 保留。没有 protected SYN/SYN-ACK、DNS、UDP 或应用字节泄漏。
- 同次重新执行旧 production_paths / UR1 / UR2 / permission_denied：基本路径 137 次、4 个 listeners premark；UR2 76 次且无 ordinary bind/connect；权限不足拒绝、双栈 alias 与 backup DNS 规则都通过。Linux image 构建同样执行 Agent lib/tests Clippy `-D warnings`。

此前 run 均保留为失败：`8d8545d52360` 构建期间源码继续编辑导致 image/source hash 不同；`60e2506d6c9a` fixture 对通配 IPv6 bind 误要求唯一源地址；`50c5d5d7669e` 原 capture 计数未区分关闭端口 RST；`0836b8d48052` 逐包 RST 校验已通过，但 Manager fixture 调用了 host 端口 allocator 的 ordinary availability listener，严格 syscall auditor 正确拒绝。最终用独立固定端口 fixture 消除此测试噪声，没有放宽 mark/readback 或应用 I/O 验收。

## Native rebind 公共 dispatcher fence 修复

独立审查发现 `e5e7f94` 的 socket I/O fence 没有覆盖已经读入公共 dispatcher 的旧代 ingress：一次瞬时 active/token 检查之后，处理线程可在 endpoint mutex 等处暂停，导致 rebind 返回之后才追加旧 payload 到 `ReceivedPacket`。这不是已发送字节或消费者晚读取的问题。审查报告及 TMP 原反例保持原样，不将本修复的结果回写到旧证据。

- Native UDP 的接收记录与 core TCP authenticated channel 均携带 originating `SocketGeneration`，进入公共 `Incoming` 队列后仍持有这一身份。
- dispatcher 的同步 admission 持有原 socket fence，覆盖邻接控制、路径检查、endpoint/cache mutation、next-hop enqueue 与最终 delivery。已经进入临界区的 dispatch 可先完成；retire/rebind 等待其完成后返回。尚未 admission 的旧代记录在退休后拒绝，不能靠新 channel/token 或仍有效的 PacketGeneration 续命。
- 锁顺序保持 **PacketGeneration → SocketGeneration**，与 packet/carrier 写 poll 一致；不创建或更换 packet fence，不清空/复活独立 packet admission，也不变更身份、grants 或路径。
- native TCP child authentication 显式监听 originating owner 取消；认证完成后的 channel 安装仍在同一 owner admission 下进行。channel read/write loop 也监听 owner 取消，已读出的记录通过上述 dispatcher fence 终止后续提交。child handles 仍由 Runtime 全局 task ledger 在 shutdown 时 join；rebind 的强边界是 owner cancellation、安装/应用 I/O/dispatch admission fence，并不宣称任意持有者的 FD 或所有 child task 已在 rebind 返回时销毁。

新增 host 回归分别覆盖真实 Native UDP/TCP 的最终 endpoint delivery 和三节点 relay forwarding：真实 Noise/E2E warmup 后，用 endpoint/transit mutex 暂停已 admission 的同步 dispatch；验证 rebind 尚未返回、释放 mutex 后提交先于 rebind 完成，返回后无旧 delivery/forwarding 新提交，且同 Runtime 新 carrier 仍可恢复业务。仅这些测试显式启用 test-only `block_in_place` 调度辅助，生产 dispatch 不改变调度方式。

验证使用 `CARGO_BUILD_JOBS=2`、独立 `CARGO_TARGET_DIR=target/native-underlay-p2-fix`；dataplane all-features lib **82 tests 通过**，Agent/dataplane all-features/all-targets Clippy `-D warnings`、fmt check、diff check 通过。真实回归保留在本树 `runtime/tests/native_sockets.rs`；原 TMP review 反例和 target 未修改。

Agent 消费者行为命令 `cargo test --locked -p meshway-agent --all-features --lib managed_connections:: -- --test-threads=1` 同上述独立 target / jobs，**34 tests 通过**，覆盖 Native/TLS/WS、实际包流、lease/withdraw、STUN、IPC probe 与历史采样。

对应既有 Linux fixture 复测命令 `python3 apps/agent/tests/underlay_fixture/run.py --image meshway-native-underlay-p2-fix:local`，同一 performance lock 与原隔离 namespace 约束，未新增网络/测量矩阵或改变 RST 验收条件。新 run `mw-rust-underlay-a920b015c443`：`status=ok`、`cleanup_errors=[]`，Native/Manager/peer tests exit 0；Native 76 次 protected pre-network calls / 4 listeners / 0 ordinary bind/connect，Manager 6 次 / 2 listeners / 0 ordinary bind/connect。capture 仍为原 10 个普通对照与 2 个严格验证的关闭端口 RST/ACK。新材料在批准 TMP `meshway-underlay-artifacts/mw-rust-underlay-a920b015c443/`，result SHA256 `1efb63cf83d41094dfa39379d1cc3550cf4402e40d91734fcc3317c2c6481b57`，镜像 `sha256:de371ee13f552342684a6e1176ba5aebdddd40531276d41085444fc24b6242ec`。此 run 验证改动后的正常 Linux 路径，确定性 dispatcher 交错由上述 host 回归验证；不将其宣称为生产主循环激活或真实组网闭环。
