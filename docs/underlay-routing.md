# Linux socket 级系统保护通路

状态：**内核机制证明 + Agent 内部可复用工厂 / HTTP / bootstrap DNS 实现**。`apps/agent/src/underlay` 已提供生产代码与独立 namespace 真实路径 fixture；现有 Agent 主循环尚未接线，P1c 当前目标 IP `/32` bypass 仍是已知原型限制。fixture 不代表生产已部署，不证明 VLESS/WebSocket、真实公网 NAT、GeoIP 或完整出口闭环已经完成。

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
- **Native UDP / core Native TCP 的 Runtime socket 注入仍未接线**；这些 sockets 不经过 P2a 的 VLESS/TLS/WS outer hook。factory 已提供受保护 UDP/TCP，但协调者须补 Runtime I/O 接口后才能激活整张网络的全 IP capture。guard/capture allocator、ownership/恢复、系统 DNS 非 plain 拓扑和 sink 迁移门槛也仍由正式 routing owner 完成。

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
