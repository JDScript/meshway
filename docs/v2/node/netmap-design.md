# Netmap 预发布设计草案

> **未实现、未验收。** 本页是 N0 设计研究稿，不加入文档导航，不构成可用 API 或功能承诺。当前行为仍以[按需执行许可](/v2/execution)、[Control](/v2/node/control)、[数据面](/v2/node/dataplane)与[验收范围](/v2/node/validation)为准。字段名为草案，最终类型由所属 crate 定义并生成 OpenAPI／SDK。

## 目标与结论

已定产品决定（2026-10-08）：Server 在配置 revision 固定时为每个节点编译完整 **netmap**，签发 24 h；节点本地准入新流、建邻接、ping，到期 fail closed；配置变更／撤销经 SSE 推送重编译结果，Server 不可用时撤销最迟随到期生效。Server 退为 coordination：netmap 分发、STUN 协调、State 汇聚。按需编译只保留给真正依赖运行时事实的出口能力与可信域名关联。

本稿的核心结论：

1. **执行权限只来自 netmap 租约**。`ExecutionSnapshot` 绑定 netmap 内容摘要与 24 h 租约，不再绑定 publication，也不再以 session／instance／60 秒 execution lease 作为数据面截止。session 只约束 HTTP，instance 只约束 State、回执与协调请求。Server 停机 10 分钟，数据面不受影响。
2. **两条线严格分开**。稳定策略（Access ∩ topology ∩ routing ∩ 身份库存 ∩ 方案选择）只在少数提交事务中编译成 netmap；动态事实（候选端点、STUN 观测、出口就绪、实例资格、观察任务）走 Node SSE 的 State 投影，由各自 owner 消费，**永不触发编译或 publication**。
3. **首包、ping、State、续租都不改变执行身份**。只有 netmap 内容变化（serial 递增）才产生一次资源差量更新；同内容续期是 O(1) 截止延长。审查列出的 29 项 remove 中 **26 项消失、3 项变形、0 项保留**（[对照表](#remove-清单对照)）。
4. 实现按“先建、后切、再删”的顺序进行，在集成分支上完成后一次合入；开发环境不支持新旧 Server／Node 共存。

本页每个问题只在一处定义；与 [Exit 设计](/v2/node/exit-design) 冲突的地方以本页为后续实现依据，X0 中的“精确需求”描述随 N1 改写。

## 当前按需编译入口

V7 盘点文件 `.meshway/validation/v7/n1-inventory.md` 在基线 `b031fc37` 不存在，下表由本稿按代码重新列出。`publish` 均为 `apps/meshway-server/src/store/execution/publications.rs::publish(_with_rule_sets)`：每次调用汇总全网有效需求、诊断、State 端点和出口事实，整网编译后比较节点切片。

| 触发点 | 位置 | N1 处理 |
| --- | --- | --- |
| 业务需求创建／续租／取消 | `publications.rs:959,1040,1092`；HTTP `http/agent.rs` demands 路由 | 删除；首包本地准入 |
| 需求过期清理 | `publications.rs:698`（`refresh_expired_demands`，由 configuration／renew 调用） | 删除 |
| 实例 acquire／续租过期／release | `instances.rs:117,162,201` | 删除编译；实例只改变 State／回执资格 |
| State 报告改变端点或出口能力 | `state/service.rs:345` → `local_endpoints::apply`、`state/exit.rs` | 删除编译；改为 State 投影事件 |
| 观察订阅镜像同步 | `execution/diagnostics.rs:581`（`synchronize_diagnostics`） | 删除；任务是动态事实 |
| 方案选择 | `publications.rs:70`（`select_profile`） | 保留为重编译触发：只重编受影响节点 |
| 配置提交 | `publications.rs:855`（`apply_configuration_effects`） | 保留：netmap 与 revision 同事务提交 |
| 规则集 artifact 切换 | `resources/provider.rs:142` | 保留：是编译输入 |
| 身份撤销／节点吊销 | `publications.rs:713`（`revoke_execution`）及成员生命周期 | 保留：重编所有引用该节点的 netmap |

Node 侧消费按需编译的入口：`apps/meshway-daemon/src/runtime/network/packets.rs`（首包 Intent／续租）、`runtime/network/dns/intents.rs`（resolver 需求）、`runtime/network/mod.rs`（demand 网关、exit Handoff、40 秒节流）、`runtime/control/cycle.rs`（抢先 settle／二次 publish）、`crates/meshway-control/src/{demand,execution,observations}.rs`、`crates/meshway-authority/src/snapshot.rs::continuation_from`、`crates/meshway-dataplane/src/{flow/mod.rs,secure/mod.rs::reconcile}`、`crates/meshway-connectivity/src/manager/continuity.rs`。

## Netmap 的形状

### 文档与租约

一个节点在任一时刻有一份 `NodeNetmap`（内容）和一个 `NetmapLease`（有效期）。内容只在稳定输入变化时改变；租约可在内容不变时反复续期。

```text
NetmapRef   = { revision, serial, digest }        // digest 覆盖完整内容，不含租约
NetmapLease = { netmap: NetmapRef, issuedAt, expiresAt }   // expiresAt ≤ issuedAt + 24 h
```

| 字段 | 含义 |
| --- | --- |
| `revision` | 编译所用配置 revision |
| `serial` | 每节点单调递增的内容序号；内容 digest 不变则不递增 |
| `digest` | 规范序列化内容摘要；Node 校验后保存 |
| `dependencyDigest` | 本节点 netmap 使用的稳定输入摘要（配置、主体、库存密钥／能力、方案选择、地址退出、规则集） |
| `compilerVersion` | **`meshway-netmap-1`**。语义不再是“需求编译”，换新族名而非 `meshway-demand-8`；`meshway-demand-1..7` 全部视为过时派生缓存 |
| `issuedAt`／`expiresAt` | 租约，不进入 `digest`；Server 每次成功查询签发 `min(now + 24 h, 依赖截止)` |

“依赖截止”目前只有可信 DNS 关联（X6）的 TTL；它作为独立 overlay 携带自己的截止，见[出口与 DNS](#出口与-dns)，不缩短整份 netmap。

### 内容字段

```text
NodeNetmap {
  networkId, nodeId, revision, serial, compilerVersion, dependencyDigest,
  self:    { enabled, hostname, identity, addresses, selfKeys, meshAddressPools,
             listeners, exitEnabled },
  peers:   NodeId → PeerConfiguration          // 身份、Mesh 地址、两把公钥
  dialers: CandidateKey → DialCandidate        // 只含 configured 端点；local 端点改走事实线
  edges:   NodeId → { dial: [CandidateKey], accept: [listenerKey] }
  grants:  GrantKey → Grant
  egress:  Option<EgressPlan>                  // 仅源节点：capture 计划＋每个范围的动作
  exitRole: Option<ExitRolePlan>               // 去掉 instanceId
  dns:     { settings, meshHosts, resolvers, egress: DnsEgress }
}

GrantKey = { source: NodeId, target: Mesh(NodeId) | Exit(NodeId, ExitScopeId) }
Grant {
  source, sourceAddresses, target,
  services: Option<ServiceSet>,   // 端点（source／target／exit）持有；中继为 null（不适用）
  exit: Option<{ destination: Internet | Cidr(prefix), family, selection: Digest }>,
  paths: [ { nodes: [NodeId], dialCandidates, observable: bool } ],   // ≤ 4 条，含全部必经点
  replies: StatefulExactReversePath,
}
ServiceSet = 有序互斥的 { protocol: tcp|udp|icmpEcho, ports: [range] }
```

- **grant 以节点对为单位**。Access 规则取并集、顺序无关，Mesh routing 每段只按节点起终点 first-match，因此同一 (source, target) 的服务集合和完整路径集合都只依赖节点对。`bidirectional` 规则生成两个方向各自的 grant，单向规则只生成一个；回复仍只依赖已有正向流。
- 每个节点只收到**自己出现在某条路径上**的 grant：作为源或终点时带服务集合，作为中继时 `services` 为 null。中继仍只做会话级转发，不解析业务事实。
- `edges` 是 topology ∩ 双方 listeners／dialers 的承载交集，只列出本节点在某条 grant 路径上的相邻节点。它是邻接与 STUN 协调的唯一许可来源，不授予业务。
- `observable` 由路径上全部节点的 `feature:path-observation` 能力计算；能力来自认证 acquire 的库存，是稳定输入。
- `meshHosts` 改为本节点 **Access 可达的完整名称集合**（不是今天“路径上恰好出现的 peer”），使 MagicDNS 与 `meshway ping <name>` 本地解析，删除精确发现往返。
- `egress.routes` 扩展现有 `EgressCapturePlan`：每个互斥 managed 前缀带动作 `exit { nodes: 有序主备, via }`、`direct` 或 `unmanaged`。源节点据此本地判定外部目的和系统 resolver 端点的动作，取代 `LocalEgressPermit`。
- `exit` 的 `selection` 等于源端 capture generation（源 revision、有效方案、持久选择版本）；方案切换必然改变它，旧出口连接结束。

### 与现有类型的关系

| 现有类型／字段 | N1 去向 |
| --- | --- |
| `NodeConfiguration` 的 network／node／revision／enabled／hostname／identity／addresses／selfKeys／meshAddressPools／listeners／exitEnabled | 原样进入 `self` |
| `NodeConfiguration.publication`、`PublicationId` | 消失；由 `serial` 和 `NetmapRef` 取代 |
| `peers`、`PeerConfiguration` | 保留；范围改为全部 grant 路径节点 |
| `dialers`、`DialCandidate` | 保留；`endpoints` 只含 `configured`，`local` 改由事实线提供 |
| `permits: RequestId → PathPermit` | 变形为 `grants`；`demandId`、精确 `target` IP／单端口、`expiresAt` 消失，`paths`／`sourceAddresses`／`replies`／`exit` 保留 |
| `PathPermit.exit: ExitPermit` | 变形为 `Grant.exit`；`roles` 中的 instance 绑定消失 |
| `LocalEgressPermit`、`localEgress` | 消失；源端按 `egress.routes` 本地判定 |
| `DiagnosticPermit`、`diagnostics`、`DiagnosticDemand` | 消失；诊断使用 grant 的服务集合与 `observable` 路径 |
| `DemandDecision`、`decisions`、`Demand` | 退出 Node 合同；路径检测若仍需决定原因，由 `plan` 的评估类型表达 |
| `CompiledNode`／`CompiledPublication` | 变形为 `CompiledNetmap { digest, netmap }`；不存在整网 publication 容器 |
| `EgressCapturePlan` | 保留并扩展为 `EgressPlan` |
| `ExitRolePlan` | 保留；去掉 `instanceId`，generation = digest(network, node, revision) |
| `DnsExecution`、`DnsEgress`、`DnsServicePermit` | 保留结构；`DnsServicePermit` 去掉 `sourceInstanceId`、`role.instanceId` 和 `expiresAt` |
| `TrustedDnsAssociation` | 不进入 netmap；X6 时作为每源 overlay，见下文 |
| `InstanceLease.target`／`executionExpiresAt` | 消失；instance lease 只剩 instance 资格；数据面截止来自 `NetmapLease` |
| `CircuitAuthorization::{Business(RequestId), Diagnostic(ObservationId)}` | 变形为 `{ grant: GrantKey, purpose: Business \| Diagnostic }` |
| `permit_authorization_digest`／`permit_transition` | 变形为每 grant 每路径的 `path_authorization_digest`，不含服务集合 |

### 规模与预算

今天每网络最多 256 条有效需求（`compile::MAX_DEMANDS`）。netmap 按节点对展开，实际规模由可达节点对决定：

| 项 | 建议上限 |
| --- | --- |
| 每网络有 grant 的节点对 | 65,536 |
| 单份 netmap 的 grant | 16,384（中继 hub 最可能接近） |
| 单份 netmap 序列化 | 8 MiB（Control 记录上限现为 16 MiB） |
| 每节点对路径 | 4，每条 ≤ 8 跳（不变） |
| 一次全网编译的路径搜索工作量 | 2²⁴ 工作项，独立于每目标 16,384 |

超限不做部分 netmap：配置提交、节点批准／接入密钥接入、方案选择在事务内返回 `netmapLimit` 校验错误，原 revision 保持。撤销与删除只缩小规模，永不被预算阻挡。N1a 须给出 1,000 节点全互访时的实际编译耗时；若在提交事务中不可接受，再评估节点对级增量复用（同一 (source, target) 的匹配规则、拓扑与库存未变则复用上一结果）。

## 两条线

| | 稳定策略（netmap） | 动态事实 |
| --- | --- | --- |
| 内容 | 身份、grant、路径、承载候选、capture、出口职责、DNS 计划 | peer 本机端点、STUN 映射、listener／link 状态、出口能力就绪、实例在线、观察任务 |
| Server 触发 | 配置提交、规则集切换、成员／节点生命周期、密钥轮换与吊销、库存能力变化、方案选择、地址退出窗口 | State 报告、实例 acquire／renew／release、观察创建／续订／取消、STUN 观测 |
| Server 动作 | 同事务编译受影响节点 → 新 serial → `netmap.changed` | 更新 State hub，按 netmap 邻接裁剪后推送 `state.delta` |
| 传输 | Node SSE `netmap.changed { serial }` 唤醒 → `POST /agent-api/v1/netmap/query { base }` 取 full／delta＋租约 | 同一 Node SSE 的 `state.snapshot`／`state.delta`；断线后 `query_peer_state` 补齐 |
| Node 消费者 | 唯一 Control：校验、保存、签发快照 → 各组件 `update` | daemon 事件任务按类型分发：端点→Manager，出口就绪→SecureNode 出口选择，任务→诊断调度器 |
| 是否授权 | 是（唯一来源） | 否；只影响连接获取方式与在已授权集合内的选择 |

**判定一项输入属于哪条线**：若缺少它就必须拒绝某个包或某个握手，它属于 netmap；若缺少它只会让某次获取失败或换一个已授权候选，它是事实。端点、STUN、出口就绪都只是选择输入：错误或过期的值最多导致连不上，不能放宽准入。

### Server 侧

- `local_endpoints::replace`／`state::exit::replace` 只写 State 投影，不再调用 `publish`；`local_endpoints::apply` 删除（端点不再写进 `DialCandidate`）。
- 实例 acquire／renew／release 只修改 `node_execution` 的实例列；acquire 时库存能力若变化（构建升级），作为库存输入触发该节点及引用它的 netmap 重编，这是唯一由 acquire 引起的编译。
- 节点 State 的可见范围（today `execution/state.rs` 从 permit 推导，R7）改为：本节点 netmap 的 `peers`；端点细节只披露给 `edges` 中的相邻节点。范围只随 serial 变化，SSE 不再因业务启动而 reset。
- UDP 穿透 prepare／accept 的授权（today R5 的共同 demand witness）改为：双方当前 netmap 的 `edges` 互含对方且有 Native UDP 候选。K6 的候选 CAS、双边接受、nonce／epoch、短期限全部保留。

### Node 侧

- Control 是唯一联系 Server 的 owner；daemon 的 SSE 消费任务把事件分为两类：`netmap.changed`／`stream.closed` 唤醒控制周期；`state.*` 解码为 `PeerFacts { node, listeners, localEndpoints, stunCandidates, exitCapability, observedAt, expiresAt }`，经有界队列交给对应 owner，不经过 `ExecutionSnapshot`。
- Manager 以 `update_peer_facts` 合并候选，不重建 epoch、不换 scope；已有健康通道不因候选变化断开。
- 事实丢失（SSE 断开、Server 停机）时保留最近事实至其自身截止，过期后只剩 netmap 的 configured 端点。

## 本地准入

### 首包

```text
TUN 源包 → Packet::parse → 查唯一 FlowTable（已有流直接走原路径）
  → 新流：grant = netmap.grants[(self, target(dst))]，services.contains(proto, port)
  → 选路径：paths 中首条下一跳通道已健康者；都没有则按序请求邻接并在有界队列等待
  → SecureNode 握手（完整路径、grant／path 摘要）→ 流提交 → 加密发送
```

没有 demand RPC、没有 publication 等待、没有 Intent 续租。首包队列与握手重发保留现有上限；等待期间不提交流。

### 准入函数

`meshway_policy::admission` 的输入从 `(NodeConfiguration, RequestId)` 改为 `(NodeNetmap, GrantKey)`；判定内容不变：

| 函数 | 检查 |
| --- | --- |
| `admit_handshake(netmap, grant, path, origin, previous, reply, now)` | 本节点 enabled、grant 存在、`origin == grant.source`、path 在 `grant.paths`、本节点位置与 previous 匹配、路径节点密钥、出口终点 `exitEnabled` 与本地 `exitRole` |
| `admit_forward(netmap, grant, path, origin, previous, traffic, now)` | 上项＋`traffic.source ∈ sourceAddresses`、目的属于 target（Mesh 地址或出口 scope 内且非 Mesh 池）、`services.contains`；中继不调用 |
| `admit_reply(netmap, grant, path, origin, previous, forward, reply, now)` | 上项＋`forward` 来自调用者自己的已建立流，reply 是精确反向 tuple |
| `admit_adjacency(netmap, peer, carrier)` | `edges[peer]` 含该 carrier；不再遍历全部 permit 求最晚截止 |

`now` 只用于租约检查，grant 自身没有截止。`targetOf(dst)` 由 netmap 的 Mesh 地址表和 `egress.routes` 求出；地址池内未分配地址拒绝，不落入 unmanaged。

### 流表与 circuit

- 流键从 `{network, publication, demand, node, role, path, tuple}` 改为 `{grant, node, role, path, tuple}`。publication 不再是主键，`Engine::reconciled_flows` 的整表克隆与重标（R1，65 行）删除。
- circuit 上下文从 `CircuitAuthorization` 改为 `(grant, purpose, path)`；握手 transcript 绑定 **path 授权摘要**（grant 键、完整路径、路径节点身份／地址／密钥、本跳承载候选、出口 selection），不含服务集合。Access 增删端口不迫使重新握手；每包准入照常按当前 netmap 检查服务。
- 这改变握手上下文的授权字段与 tombstone 规则（见下），**wire 升为 v6**（`MWCIR\0\0\x06`）；稳态 Data 帧 86 B 布局不变。

### `SecureNode::update`

`refresh`／`reconcile` 合为一个入口 `SecureNode::update(identity) -> Update`：

| 情况 | 行为 |
| --- | --- |
| 同 `NetmapRef`（只续租） | 换入新快照，所有活 circuit／flow 的截止上限提高到新租约；不扫描、不换代、不取消输出 |
| 新 serial | 对每个 circuit 比较新旧 netmap 的 path 授权摘要：相同则保留 Noise 状态与流，否则退役；对每条流检查其精确服务仍在 `services` 内，否则以独立流 guard 退役；出口流同时发 `Retire` |
| 本节点身份／地址／密钥变化 | 全部退役（硬屏障） |

不存在“直接后继”判定：Control 只接受 serial 更大的内容，比较的是新旧完整内容本身。跳过中间 serial 不需要中断连续性——若中间曾撤销后又恢复，新内容与旧内容等价，两端在各自观测到的每个时点都持有授权；永久吊销的节点不会以相同身份恢复。

### 邻接按需建立

netmap 覆盖全部可达 peer，若“收到候选即建连”会变成全连接。建议改为本地需要驱动：

- dataplane 在首包选路或中继收到经过本节点的握手时，向 Manager 登记 `AdjacencyInterest(peer)`；Manager 只对 `edges` 中的 peer 获取连接。
- 无 circuit 且无 interest 的邻接空闲 120 秒后关闭；configured TCP／UDP 通道可按 topology 声明保持常驻（见[未决问题](#未决问题)）。
- STUN 刷新只对存在 interest 或健康 Native UDP 通道的 socket 进行，预算（每 60 秒 30＋30 次）保持。

这改变 [Topology](/v2/configuration-reference/topology) 现行“Node 收到候选即建立邻接、不等首包”的描述，需用户确认。

## Control 快照模型

### 快照

```text
ExecutionSnapshot {
  netmap: Arc<CompiledNetmap>, lease deadline (UTC 高水位＋单调),
  instance: Option<InstanceRef>,      // 仅用于 State／回执绑定，不参与 reading() 有效性
  validity, invalidated, clock, lineage
}
```

- `reading()` 有效 ⇔ validity 未撤销、invalidated 未取消、netmap 租约截止未到。session／instance 截止不再参与；`Deadline` 三重数组变为单个租约截止。
- 签发仍只在 `meshway-control`；`crates/` 不出现第二个签发者。读取、克隆不延长租约。
- 同内容续租签发一个新快照：新快照截止更晚，旧快照不被取消（截止较早），已排队输出保留旧 guard。由于旧租约通常仍有十余小时，这不形成旧输出被拒的抖动。
- 本机失效（立即取消全部快照）：显式 down、stop、换钥开始、`membership_revoked`、本地记录 ReloadRequired、收到更大 serial 且本节点身份变化。

### lease 语义

| lease | N1 语义 |
| --- | --- |
| session（300 s） | 只证明可调用 HTTP；过期后后台重新认证，不影响数据面 |
| instance（120 s） | 唯一在线进程资格：State 报告、安装回执、STUN 协调、观察结果上报都要求当前 instance；Server 侧 instance 过期只使其 State 失效 |
| execution（60 s） | 消失 |
| netmap 租约（24 h） | 数据面唯一截止；Node 在剩余 < 23 h 时续期（约每小时一次），每次成功查询都可续 |
| 观察订阅（30 s） | 保留：限制消费者生命周期，不是执行授权 |
| UDP offer（≤ 10 s） | 保留 |

instance 重新取得（同一进程因 Server 停机过期）不重装：Control 以新 instance 重新提交 saved→applied 回执（新 attempt），引用同一已安装 `NetmapRef`。真实进程替换仍产生新 instance，旧 attempt 不能覆盖新结果（K2 保留）。

### 连续性交接何时还发生

| 事件 | N1 |
| --- | --- |
| 业务首包、DNS 查询、ping、Console 观察 | 不发生；无任何执行更新 |
| State 报告、端点／STUN／出口就绪变化 | 不发生 |
| instance 续租／重取 | 不发生 |
| netmap 同内容续租 | 不是交接；O(1) 截止延长 |
| 配置 revision、成员、密钥、库存、方案选择导致的新 serial | **发生**：`SecureNode::update` 按 grant／path 比较；Manager／Host／DNS 只对实际变化的资源做差量 |
| 本节点身份变化、down、re-enroll | 硬屏障，不交接 |

### retained／handoff／fence 逐项结论

| 机制（位置） | 结论 |
| --- | --- |
| `Issue.publication`／`predecessor`、`continuation_from` 直接后继（authority `snapshot.rs:163-196`） | **变形**：比较新旧完整内容，无前驱链 |
| `lineage: Weak<()>`（同上） | **保留**：stop／down／re-enroll 后旧资源不可保留 |
| `publication_pending_grant`（control `execution.rs:364`） | **消失** |
| 三重 deadline [session, instance, execution] | **变形**：单一 netmap 租约 |
| `retention_deadline`：失效快照等待后继（`snapshot.rs:221`） | **消失**：更新原子，没有空窗 |
| `same_execution`／`SecureNode::refresh` | **变形**：`same_netmap(ref)` 判定同内容续租 |
| 保存目标即 `invalidate_guards`（control `storage.rs:298`） | **变形**：只在内容变化时 |
| 流表整表重标（dataplane `flow/mod.rs:316-380`） | **消失** |
| circuit reconcile 大事务（`secure/mod.rs:1484-1576`） | **变形**：`update` 的 path 摘要比较；续租 O(1) |
| 关闭 circuit 的 replay tombstone（S9） | **保留并收紧**：owner 生命周期从分钟变为进程寿命，必须分离紧凑 dead-CID 记录；v6 握手 INIT 绑定发起时刻，tombstone 保留至握手窗口（10 s）之后即可回收，不再计入 `maxCircuits` |
| 中继 probe 关联换绑（`secure/probe/mod.rs:40-69`） | **消失** |
| exit `project_successor` 全表投影（`flow/exit.rs:250-334`） | **消失**；活动续期 `prepare_projection`、旧期限 ACK 门、精确退役回执 **保留** |
| exit pre-receipt Handoff（daemon `network/mod.rs:670-700`） | **消失** |
| Manager 三表 1 ms 轮询 barrier（`continuity.rs:273-361`） | **变形**：资源 owner 只对变化资源回报安装／退休结果 |
| `Shared`／`rebind_scoped`／latest·successor 等待（`continuity.rs`、`scope.rs`） | **变形**：`ResourceLease`（descriptor 生死）＋`OperationGuard`（单次 I/O）；无 successor 等待 |
| selected-channel CAS／ABA tombstone、旧 epoch READY fence（K6） | **保留** |
| 穿透查询的 publication 窗口与抢跑（`traversal.rs` S4） | **变形**：删 publication 窗口，保留双预算与公平批次 |
| underlay `closed()` 空窗等待器、`retained_snapshot`（F01、F02） | **消失** |
| `retain_tun` 续接（F03） | **变形**：按资源兼容性保留 TUN／capture |
| DNS `ScopeFence`／ticket generation（F16） | **保留**：只在 netmap 或系统 resolver 就绪变化时更新 |
| DNS Owner refresh／replace、退休 demand 白名单、续租 ACK 高水位 | **消失** |
| ApplicationTicket attempt／sequence、回执冲突分类（K2） | **保留**；进度多层投影按 S2 收敛 |
| `state_scope_changed` | **保留**，频率降为 serial 变化级 |
| cycle 抢先 settle／二次 publish、40 秒反向节流（R7、R8） | **消失** |
| 观察 `planning` 缓存、`pause/resume_after_refresh`（R5、S8） | **消失**；`retired_results` **变形**为任务 generation 判定 |
| 诊断 setup 60 秒跨交接保持（S7） | **变形**：无交接，预算保留 |
| 回执历史窗口、obsolete 严格校验（K3） | **保留**；obsolete 识别对象改为 `meshway-demand-*` |

## 诊断

### `meshway ping`

CLI ping 完全本地：目标名由 netmap `meshHosts`／peers 解析；需要 grant `(self, target)` 且 `services` 含 `icmpEcho`；只在 `observable` 路径上执行；探测帧使用 `purpose = Diagnostic` 的独立 circuit，仍不进入业务 FlowTable。Server 不可用时 ping 照常工作。CLI ping 不创建 Server 订阅，结果只回到 IPC。

Unsupported 与 Denied 的区别保留：没有 grant 或服务不含所需 scope 为权限拒绝；有 grant 但无 `observable` 路径为 unsupported。

### Console 观察

```text
Console createObservation
  → Server：用户权限＋读取 source 当前 netmap（不编译）检查 grant／service／显式路径
  → 写任务（动态事实），SSE 提示 source
  → source Control 取 getNodeTasks → 本地按 netmap 执行
  → reportTaskResult → Server 转发订阅者 SSE
```

- 订阅 30 s 期限、10 s 续租、任务共享、样本窗口、`observation.*` 事件保留。续订只改任务截止，不产生 publication。
- 任务执行权限 = source 当前 netmap 的 grant；任务截止只限制持续多久。netmap serial 变化导致 grant／路径消失时，Node 结束任务并上报 `failed`；路径变化开启新分段。
- R2／R3／S1 的两遍编译、SQL 镜像、`synchronize`／`reauthorize` 循环全部删除。

### 首包与准备预算

今天准备阶段依次等待：订阅 HTTP → 联合编译 → 各节点 publication 安装 → 首跳通道 → 完整路径握手；实测约 1.5 s 量级，60 s 为上限。N1 后只剩本地部分：

```text
warm（邻接已健康）：T ≈ RTT_path（握手）＋ RTT_path（首个 probe）＋ 本地处理
cold（静态端点）：  T ≈ Σ每跳串行邻接获取（TCP 1 RTT；TLS 再 1–2 RTT；UDP 1 RTT 认证）＋ 2·RTT_path
```

- 去掉“首轮等安装事件”和订阅往返后，CLI ping 的首个结果不含任何 HTTP。
- 建议 lab 门槛（容器间 RTT 亚毫秒）：warm 首回复 ≤ 100 ms，cold Native TCP 三节点 ≤ 300 ms；跨地域部署按上式核对，不另设固定秒数。60 s setup 上限保留作失败边界。
- 协调 UDP 穿透需要 Server，冷启动时间另计；Server 停机时只能使用静态端点或已有通道。

## 撤销与到期

### 传播路径与最坏延迟

| 场景 | 路径 | 最坏延迟 |
| --- | --- | --- |
| Server 可用、Node SSE 已连接 | 提交事务编译 → `netmap.changed` → Node 查询、校验、保存 → `update` | 秒级；lab 门槛 ≤ 5 s |
| Server 可用、SSE 断开 | 定时查询兜底（建议 60 s） | ≤ 60 s＋一次查询 |
| Server 不可用或节点分区 | 无法推送 | ≤ 该节点剩余租约（≤ 24 h） |
| 成员撤销 | 额外 `stream.closed: membership_revoked` 使被撤销节点立即本地失效 | 同上；不依赖被撤销节点配合 |
| 密钥吊销／节点删除 | 所有引用该节点的 netmap 重编，peer 删除其密钥 | 同第一行；接收端先收到即先拒绝 |

撤销按接收端执行（[Access](/v2/configuration-reference/access#enforcement)）：被撤销的源即使不配合，任何已收到新 netmap 的终点、中继和出口都会拒绝。只有当两端都与 Server 分区时，旧授权才可能存续到租约到期。这与 Tailscale 的取舍相同。

### 到期行为：终止全部

建议 netmap 租约到期时 **已有流与新流一并终止**：全部 circuit 退役、排队输出取消、出口投影退役并精确清理 CT；源端 capture 保持负向边界（与今天 lease 过期一致），DNS 保持负向安装。

理由：租约到期是分区状态下撤销的唯一上界。若只拒新流，一条被撤销的长连接（TCP、隧道、持续 UDP）在两端都离线时可以无限存续，撤销失去最坏界限。到期发生在 Server 已至少 1 小时（续期周期）不可达之后，主动断开是可预期的。恢复联系后取得新租约即可重新握手。

### revision 变化时的已有流

由 `SecureNode::update` 判定：grant 仍在、流的精确服务仍在 `services`、所用完整路径仍在 `paths`、路径上全部节点身份／地址／密钥不变、所用承载候选仍在、出口 `selection` 不变——全部满足则保留；任一不满足则结束该流（必要时只结束该 circuit 上的部分流）。方案切换总是结束旧出口连接。

## 出口与 DNS

### 出口

| 项 | 归属 | 说明 |
| --- | --- | --- |
| 出口 grant（源→出口的 scope、服务、路径、selection） | netmap | Access ∩ 允许方案 ∩ `exitEnabled` ∩ 路径，按 revision 与选择版本编译；不再要求就绪 |
| `ExitRolePlan` | netmap（出口节点） | 只要 `exitEnabled` 且声明 `feature:exit-ipv4` 就下发；去掉 instance 绑定。职责本身仍不转发包 |
| `EgressCapturePlan`／`egress.routes` | netmap（源节点） | 按 revision＋选择版本 |
| `ExitCapability`（probing／ready／unavailable、families） | 动态事实 | 出口节点 State 上报，经 `state.delta` 推送给 grant 中引用该出口的源 |
| X1 `ExitInputs` | 拆分 | `instances`／`instance_expires_at_ms` 不再是编译输入；`selection_generations` 进入 netmap 稳定输入；`ready_until_ms`／`unavailable_reasons` 变成源端选择用事实 |

源端在 `egress.routes` 给出的有序主备出口中，选第一个能力事实新鲜且 ready 的出口建流；都不 ready 时丢弃并计数，保留负向捕获，**永不 direct**。出口端准入只看 grant 与本地已安装 role，不看自己的 State 上报。能力事实错误只会让源选错出口而被拒，不会放宽准入。

这把“出口能力按需编译”收窄为“出口能力作为选择事实、零编译”，与已定决定的措辞不同，列入[未决问题](#未决问题)。

### DNS

- 配置 resolver 与系统 resolver 的本地 socket 许可：源端用 `egress.routes` 求 resolver 端点的动作——direct／unmanaged 允许本机 socket，exit 动作必须经出口 DNS，unmanaged 范围以外的 Mesh 地址拒绝。UDP 与 TCP 仍分别判定。R1／R4／F14 的 resolver demand、ID 映射和退休白名单删除。
- X5 `DnsServicePermit` 留在 netmap：客户端 `ViaExit`、出口 `ServeExitResolver` 列表都按 revision 编译，去掉 instance 与截止字段；出口就绪仍是事实。Mesh ingress 适配缺失时安装阻断的现状不变。
- X6 可信域名关联是唯一保留的按需编译：Server 受控解析任务完成后，为该源签发 `DnsAssociationOverlay { netmap: NetmapRef, associations, expiresAt = min(TTL, 任务期限) }`，经同一 netmap 查询端点交付并由 Control 绑定进快照。overlay 有自己的截止，只扩大该源与相关出口的域名目的判定，不改变 netmap serial；它只影响源和所选出口两个节点。

## 迁移

| 项 | 方案 |
| --- | --- |
| Node Control 记录 | 记录格式升为 **v5**：身份 envelope 与 netmap 缓存分离（缓存带自己的 `compilerVersion`），今后编译器升级不再改身份版本（S5）。v1–v4 沿用现有规则：校验完整性与绑定、私有归档原字节、清空缓存、保留身份 |
| netmap 缓存用途 | 只作 delta 基准；进程重启后是否可凭未到期缓存离线运行见[未决问题](#未决问题)，默认不恢复 |
| Server 数据库 | 新的当前 schema：删除 `execution_demands`、`execution_publications`、诊断编译镜像表；新增 `node_netmaps(network, node, serial, revision, digest, sealed payload, created_at)` 与有界历史；`node_execution` 去掉 `target_publication_id`／`execution_expires_at`，回执改引用 `NetmapRef`。按[存储](/v2/storage)现行规则没有迁移链：开发环境重新初始化 Server 数据目录 |
| HTTP | 新增 `POST /agent-api/v1/netmap/query`；删除 demands、`configuration/refresh`、diagnostic 编译相关字段；`/agent-api/v1` 根不变。重生成 OpenAPI／Rust／TS SDK 并过 drift |
| wire | v6（握手授权字段与 INIT 时刻）；全路径协调升级 |
| IPC／CLI | v6：`instance.executionExpiresAt` 改为 `netmap { revision, serial, expiresAt }` |
| 升级顺序 | 停全部 Node → Server 用新数据目录启动 → 重新创建网络配置 → Node 清理旧身份目录后重新接入 |
| 共存期 | **不需要**。dev 环境不保留旧 Node 与新 Server 的兼容；Server 拒绝旧版 agent 请求即可（字段不认识即拒绝） |

常驻 lab 的升级：按 [Docker lab](/v2/node/docker-lab) 分开 image-build 与 deploy，先导出实验配置原文，Server 重新初始化后 CAS 重建配置；三个节点身份卷清理后重新接入（地区名不变，Node ID 改变）。

## remove 清单对照

依据 `.meshway/review-20261009/SYNTHESIS.md` 与四份报告的全部 29 项 remove。合计：**消失 26、变形 3、保留 0**。

| 项 | 内容 | N1 下 |
| --- | --- | --- |
| Server R1 | demand CRUD／续租／过期与全网重编译（313 行） | 消失：首包本地准入 |
| Server R2 | 诊断两遍编译与中间证据（192） | 消失：任务创建只读查 source netmap；Denied／Unsupported 由 grant 与 `observable` 表达 |
| Server R3 | 订阅镜像→dirty→republish 泵（98） | 消失：任务是动态事实 |
| Server R4 | retained slice／空 delta／部分 supersedes（67） | 变形：每节点内容 digest 相同则不递增 serial |
| Server R5 | 共同 demand witness（212） | 消失：协调授权改为双方 `edges` |
| Server R6 | demand／republish 合约测试（1,491） | 消失：有效断言（receipt 顺序、撤销、旧实例）迁到 netmap fixture |
| Server R7 | 需求驱动的 State 可见与期限聚合（42） | 变形：可见范围取 netmap peers／edges |
| Server R8 | 忽略参数的 reauthorize、不可达 Cancel（20） | 消失：独立清理 C0 |
| Node R1 | DNS demand 编排（233） | 消失：resolver 本地按 `egress.routes` 判定 |
| Node R2 | packet Intent 续租（159） | 消失 |
| Node R3 | demand 网关与回复（180） | 消失 |
| Node R4 | 退休 DNS 白名单（48） | 消失 |
| Node R5 | confirmed-task 规划缓存（45） | 消失：无 publication 可等 |
| Node R6 | exit pre-receipt Handoff（50） | 消失：netmap 差量在唯一安装事务内 |
| Node R7 | cycle 抢先 settle／二次 publish（31） | 消失 |
| Node R8 | 40 秒反向节流（16） | 消失：租约续期按剩余时间调度 |
| Node R9 | 仅测试 pause/resume（22） | 消失：独立清理 C0 |
| Node R10 | 中间 publication 顺序测试（78） | 消失 |
| Dataplane R1 | publication 为流主键与整表重标（65＋45） | 消失：流键改为 grant |
| Dataplane R2 | 全表 successor 投影（59） | 消失：活动续期保留 |
| Dataplane R3 | 中继 probe 换绑（40） | 消失 |
| Dataplane R4 | 第二套 crypto Batch API（66） | 消失：独立清理 C0 |
| Dataplane R5 | refresh／reconcile 双入口（14） | 变形：单一 `SecureNode::update` |
| Dataplane R6 | 诊断复制的承载一致性循环（13） | 消失：诊断与业务共用 path 授权摘要 |
| Underlay F01 | publication 空窗 `closed()` 等待器（36） | 消失 |
| Underlay F02 | exit `retained_snapshot` 与 gap 跳过（20） | 消失 |
| DNS F13 | 无生产消费者的 Owner refresh／replace（52） | 消失：独立清理 C0 |
| DNS F14 | resolver key→demand ID 三张映射（57） | 消失 |
| DNS F19 | 丢弃结果的 `GetConnectionUnixUser`（10） | 消失：独立清理 C0 |

S 类简化不在本表；N1 使 S1–S6（Server）、S1–S5（Node）、S1–S5（dataplane／connectivity）、F03 有条件落地，分配见拆包表。审查的 keep 项全部保留，N1 不放宽任何一项。

## 拆包

全部工作在集成分支 `feat/netmap` 上进行，单包在 `.worktree/` 独立工作树中完成后由协调者合入集成分支；主线只在 N1i 删除完成、矩阵通过后合并一次。manifest、lock、生成链和版本号由协调者统一维护。

| 包 | 类 | crate／文件所有权 | 依赖与并行 | 进程内测试 | T2／lab |
| --- | --- | --- | --- | --- | --- |
| **C0** 局部清理 | 删 | Server `observations/service.rs` R8；`meshway-diagnostics` R9；dataplane `secure/pipeline/pool.rs` R4；`meshway-dns` F13、F19 | 不依赖 N1，可立即在 main 进行 | 各 crate 现有行为测试改走生产入口 | 现矩阵不回退 |
| **N1a** 契约与编译器 | 建 | `meshway-model`、`meshway-configuration` netmap 类型／delta／digest；`meshway-policy` `compile_netmap` 与 grant 准入；`meshway-protocol` 类型 | 首包；冻结类型后其他包并行 | 差分测试：同一配置下旧 `compile_with_exit` 对任一 (source, target, proto, port) 的 Permitted 与新 grant 准入等价；预算、first-match 分割、递归 via、Internet 边界 | — |
| **N1b** Server 分发 | 建 | Server `store/` 新 netmap 表与 schema、`configuration/commit` 编译钩子、库存／成员／选择触发、`http/agent.rs` netmap 查询、SSE `netmap.changed` | N1a 后；与 N1c 文件互斥并行 | 提交同事务产生 serial；无关提交不变 serial；撤销重编引用节点；租约续期不写 serial；预算超限拒绝提交 | — |
| **N1c** Server 事实与诊断 | 变形 | Server `state/`（service、local_endpoints、exit、provider）、`execution/{state,connectivity,diagnostics}.rs`、`observations/` | N1a 后，与 N1b 并行 | State 报告与实例事件零编译；可见范围＝netmap peers；穿透授权＝edges；观察创建只读 netmap，续订不改 netmap | — |
| **N1d** authority 与 Control | 变形 | `meshway-authority` snapshot／clock；`meshway-control` netmap 查询与续期、记录 v5、事件、回执、观察；删除 `demand.rs` | N1a 后；先冻结快照 API 供 N1e–g | Server 停止后 session／instance 过期而快照有效；同内容续租不取消 guard；serial 回退拒绝；v1–v4 记录归档 | — |
| **N1e** dataplane 与诊断 | 变形 | `meshway-dataplane`（flow、secure、codec v6、probe、exit）；`meshway-diagnostics` | N1d API 后，与 N1f／N1g 并行 | 真 SecureNode A→R→B：首包无外部调用、`update` 保留未变 grant 流、端口撤销只结束该流、路径变化结束 circuit、租约到期全终止、tombstone 窗口回收、wire v6 互验 | — |
| **N1f** connectivity | 变形 | `meshway-connectivity`（Manager、continuity、traversal、workers） | N1d API 后并行 | `AdjacencyInterest` 驱动获取、空闲关闭、事实更新不换 epoch、serial 变化只动变化资源、CAS／ABA 保留 | — |
| **N1g** underlay 与 DNS | 变形 | `meshway-underlay`（protection、kernel exit／egress 接口）；`meshway-dns`（route、dispatcher、system） | N1d API 后并行 | F01／F02 删除后资源撤销及时关闭 FD；resolver 按 `egress.routes` 判定；ScopeFence 只随 netmap／就绪变化 | — |
| **N1h** daemon 切换 | 删 | `apps/meshway-daemon`（control、network、dns、ping）、`meshway-ipc` v6、`apps/meshway-cli` | N1e–g 合入后；Node 侧旧链在本包整体删除 | daemon 进程内三节点：首包、ping、DNS 不产生 HTTP；SSE 事实分发；netmap 更新差量 | T2 全部既有行重跑 |
| **N1i** Server 旧链删除 | 删 | Server demands、publications、诊断编译、witness、`instances.rs` 编译调用、R6 测试族、cache obsolete 分支 | N1h 后（无旧消费者） | 迁移后的 receipt／撤销／旧实例断言通过 | — |
| **N1j** 验收与文档 | 验 | `tools/meshway-{packet,runtime}-fixture`、`scripts/validate_matrix.py`、`docs/v2/`（execution、node-configuration、topology、routing、access、control、runtime、dataplane、connectivity、diagnostics、realtime、storage、validation）、AGENTS 描述 | fixture 可在 N1a 后准备；运行等 N1i | — | 新行与 lab，见下 |

可并行范围：N1b‖N1c‖N1d（N1a 后）；N1e‖N1f‖N1g（N1d 快照 API 冻结后）；C0 任何时候。

### 新增 T2 行建议

| 行 | 拓扑与步骤 | 通过条件 |
| --- | --- | --- |
| `netmap/offline` | Controller＋A／R／B（Native TCP 与 Native UDP 静态端点各一轮）；建流后停止 Controller 10 分钟 | 已有 TCP／UDP 持续；新 5-tuple 业务流、`meshway ping`、新静态邻接（预先未连接的 A–R 承载）全部成功；零 HTTP 成功请求；恢复后无第二次 up、无重装 |
| `netmap/revoke` | Controller 在线；撤销 A→B 端口、撤销成员、轮换 B 密钥、方案切换 | 新 serial 送达 ≤ 5 s；受影响流停止、未受影响流 cipher 不变；旧输出不复活 |
| `netmap/expiry` | 停止 Controller 越过租约（需短租约，见未决问题） | 到期时全部 circuit 终止、capture 保持负向、DNS SERVFAIL；恢复后自动重建 |
| 既有十一行 | 原场景 | 另断言 demand／publication 相关路由不存在、首包无控制面请求 |

### 常驻 lab 验收（sha／lax／nyc）

1. 按 [Docker lab](/v2/node/docker-lab) 核对 PID／hash 与 netmap serial；配置 sha→lax→nyc 必经路径与 ICMP、TCP、UDP 授权。
2. 建立持续 iperf3 与 125 s 纯接收流后停止 Server 10 分钟：持续流不断；期间新开 TCP／UDP 流、`meshway ping nyc`（名字本地解析）、首次使用的 lax–nyc 静态承载均成功。
3. 恢复 Server：节点自动重新认证、取新 instance，saved／applied 指向同一 `NetmapRef`，无数据面重装。
4. 撤销 sha→nyc 的 TCP 端口：≤ 5 s 内该流停止、ICMP 仍通；撤销 sha 成员：nyc 与 lax 拒绝其全部流量。
5. 恢复配置，记录全部 serial 与时间戳；地区名只是节点标识，不推断真实跨地域延迟。

## 未决问题

只列需要用户决定的事项：

1. **进程重启后能否凭未到期 netmap 离线运行。** 默认不恢复（沿用“缓存不复活授权”）：Server 停机期间重启的 Node 在 Server 恢复前没有数据面。允许恢复则 Server 停机期间的重启也能自愈，代价是持久缓存成为授权材料，需要额外绑定 boot／设备并接受撤销最迟仍为租约截止。
2. **租约长度是否可配置。** 决定为 24 h；`netmap/expiry` T2 行与故障演练需要更短租约。建议 Server 运行参数 `--netmap-validity`（默认 24 h，最小 5 分钟），而不是测试专用后门。
3. **出口能力是否完全退出编译。** 本稿建议出口就绪只作为源端在已授权有序出口中的选择事实，零编译，可秒级切换主备；与“按需编译保留给出口能力”的措辞不同。若希望出口选择仍由 Server 决定，可改为每源 `ExitSelectionOverlay`（同 X6 overlay 机制），代价是就绪变化仍需 Server 在线。
4. **邻接是否改为按需建立。** netmap 覆盖全部可达 peer 后，“收到候选即建连”等于全连接；建议改为本地需要驱动并空闲 120 s 关闭，可选让 topology 规则声明常驻。它改变现行 topology 文档的行为描述，并让冷路径首包多一次邻接获取。
5. **规模超限的语义。** 建议所有放大规模的操作（提交、批准、方案选择）在超出 netmap 预算时整体拒绝，而不是生成部分 netmap；需要确认这对大网络（例如 hub 中继承担上万节点对）是可接受的上限策略。

## 限制

- 中继获知所有经过它的 (source, target) 节点对，今天只获知活跃需求；这是 netmap 模型固有的披露范围扩大。
- 同一身份的克隆进程（复制私钥）在 Server 层仍只能持有一个 instance，但两者都能使用各自已取得的 netmap 租约直到到期；instance 替换不能撤回已签发租约。
- Server 停机期间无法进行协调 UDP 穿透；只有静态端点与已有通道可用。
- 本稿不改变端到端保护、源授权、有序必经点、原路径精确回复、重放保护、State 不自授权、失败不静默放行等硬约束；它们在 N1 下由同一 owner 执行。
