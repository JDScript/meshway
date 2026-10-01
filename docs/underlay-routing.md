# Linux socket 级系统保护通路

状态：**独立内核机制实验与集成建议**。没有接入生产 Agent；P1c 当前目标 IP `/32` bypass 仍是已知原型限制。实验不证明 VLESS/WebSocket、真实公网 NAT、GeoIP 或完整出口闭环已经完成。

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

当前生成 `apps/agent/src/generated/client.rs::HttpClient::new` 自建 reqwest 客户端，没有注入受保护连接器入口。应在生成器模板增加传入已配置 HTTP transport/client 的构造入口并重生成，继续使用生成 endpoint 与响应；不直接编辑生成文件。

本次核对本地依赖 **reqwest 0.13.5**：`ClientBuilder::interface` 是接口绑定，可能绕过其他 VPN，不是上述机制；`connector_layer` 包装连接服务，并不提供可靠的 pre-connect socket mark hook。它不能被描述为“加一个 layer 就保护了 SYN”。正式 controller 集成尚需可审计的底层 connector pre-connect mark hook（依赖提供/扩展此能力），以及生成客户端注入与消费者验证。这是本实验留下的实际接入缺口。

`dns_resolver` 可以接入专用的 bootstrap resolver，但 resolver 自己的 UDP/TCP/DoH 外层也必须由工厂保护。不能把系统 `getaddrinfo` / `tokio::net::lookup_host` 自动等同于 protected DNS：请求可能交给另一个 daemon 的 unmarked socket。不要按 DNS server IP 或整台 resolver 进程放行。使用现有系统 DNS/VPN 上游语义，只有 controller/peer/probe bootstrap 查询进入此 resolver，业务 DNS 保持其业务策略；缓存绑定 underlay generation、TTL、用途并允许取消。happy-eyeballs 的每个 IPv4/IPv6 候选均须保护，HTTP pool、proxy 连接、DNS TCP fallback、重试和重定向须逐一检查；不得用 `local_address` 代替 mark。

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

协调者后续需：统一所有自有捕获入口；接入上述工厂、reqwest connector 与生成客户端注入；真实 bootstrap DNS 与业务 DNS 隔离；真实 Native UDP/STUN/认证探测、VLESS+WS、TCP listener inheritance；ownership 持久恢复与外部 RPDB/mark 改写监控；换网时并发取消与 R3 fence 联测；生产降权/cap 边界和协议消费者检查。CI 可先加入 host-only fixture tests，再在有 `/dev/net/tun` 的 Linux Docker runner 上运行隔离实验；本分支不编辑根 CI。
