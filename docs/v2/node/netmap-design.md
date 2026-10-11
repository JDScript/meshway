# Netmap 预发布设计草案

> **未实现、未验收。** 本页是 N0 设计研究稿，不加入文档导航，不构成可用 API 或功能承诺。当前行为仍以[按需执行许可](/v2/execution)、[Control](/v2/node/control)、[数据面](/v2/node/dataplane)与[验收范围](/v2/node/validation)为准。字段名为草案，最终类型由所属 crate 定义并生成 OpenAPI／SDK。

## 目标与结论

已定产品决定（2026-10-08）：Server 在配置 revision 固定时为每个节点编译完整 **netmap**，签发 24 h；节点本地准入新流、建邻接、ping，到期 fail closed；配置变更／撤销经长轮询 `sync` 送达新投影，Server 不可用时撤销最迟随到期生效。Server 退为 coordination：netmap 分发、STUN 协调、State 汇聚。按需编译只保留给真正依赖运行时事实的出口能力与可信域名关联。

本稿的核心结论：

1. **执行权限只来自 netmap 租约**。Control 发布已验签的内容与 24 h 租约，Engine 是租约的唯一计时者并向组件应用不可变 `Desired`；不再有 publication、执行快照或 session／instance／60 秒 execution lease 作为数据面截止。session 只约束 HTTP，instance 只约束 State、回执与协调请求。Server 停机 10 分钟，数据面不受影响。
2. **两条线严格分开**。稳定策略（Access ∩ topology ∩ routing ∩ 身份库存 ∩ 方案选择）只在少数提交事务中编译成 netmap；动态事实（候选端点、STUN 观测、出口就绪、实例资格、观察任务）走同一 `sync` 长轮询中独立的 `peerFacts` 部分，由各自 owner 消费，**永不触发编译或 publication**。
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

一个节点在任一时刻有一份 `NodeNetmap`（内容）和一个 `NetmapLease`（有效期）。内容只在稳定输入变化时改变；租约可在内容不变时反复续期。类型已由 N1a 冻结，字段与文档注释以源码为准：

| 类型 | 定义位置 | 要点 |
| --- | --- | --- |
| `NetmapRef { revision, serial, digest }` | `crates/meshway-model/src/netmap.rs` | `digest` 是内容摘要，不含租约、`revision`、`serial`、`dependencyDigest` |
| `NetmapLease { netmap, issuedAt, expiresAt }` | 同上 | `NetmapLease::validate` 要求 `issuedAt < expiresAt ≤ issuedAt + 24 h`；配置下限 `MIN_NETMAP_VALIDITY_MS`＝60 s、`MAX_NETMAP_BYTES`＝8 MiB |
| `NodeNetmap`、`CompiledNetmap { digest, netmap }` | `crates/meshway-configuration/src/netmap/mod.rs` | `NodeNetmap::content_digest`、`CompiledNetmap::verify`／`reference`；`NETMAP_COMPILER_VERSION`＝`meshway-netmap-1` |
| `ProjectedConfiguration` | `crates/meshway-configuration/src/netmap/projection.rs` | 投影本体，见[内容字段](#内容字段) |
| `NetmapDelta` 与 `netmap_delta`／`apply_netmap_delta` | `crates/meshway-configuration/src/netmap/delta.rs` | 投影的 nodes／inventory 与 dialers 按键增删，其余整块替换；重建后必须复现目标 digest，serial 必须增大 |

| 字段 | 含义 |
| --- | --- |
| `revision` | 首次产生这份内容的配置 revision；内容不变的后续 revision 沿用旧值 |
| `serial` | 每节点单调递增的内容序号；内容 digest 不变则不递增 |
| `digest` | 规范序列化内容摘要；Node 校验后保存 |
| `dependencyDigest` | 产生该 serial 的整网投影输入摘要（配置、主体、库存密钥／能力、方案选择及版本、规则集）；与 `revision` 一样随 serial 保留，不进入 `digest` |
| `compilerVersion` | **`meshway-netmap-1`**。语义不再是“需求编译”，换新族名而非 `meshway-demand-8`；`meshway-demand-1..7` 全部视为过时派生缓存 |
| `issuedAt`／`expiresAt` | 租约，不进入 `digest`；Server 每次成功查询签发 `min(now + 24 h, 依赖截止)` |

“依赖截止”目前只有可信 DNS 关联（X6）的 TTL；它作为独立 overlay 携带自己的截止，见[出口与 DNS](#出口与-dns)，不缩短整份 netmap。

### 内容字段

**netmap 不是编译结果，而是配置的投影**。Server 不物化任何节点对 grant 或路径；节点在首包／握手时用同一 `meshway-policy` 编译器在投影上求自己那一对 (source, target) 的服务集合与路径。全部定义在 `crates/meshway-configuration/src/netmap/`：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `self` | `NetmapSelf` | enabled、hostname、identity、addresses、selfKeys、meshAddressPools、本节点完整 listeners（禁用时为空）、exitEnabled |
| `projection` | `ProjectedConfiguration { configuration: ConfigurationView, subjects: SubjectContext, inventory, ruleSets, selectedProfiles, selectionVersions }` | 全量配置的单调子集，复用现有 `ConfigurationView`／`NodeSettingsView`（已脱敏的 listener／dialer 声明）、`AccessRule`／`TopologyRule`／`MeshRoute`、`TrustedNode`、`SubjectContext`（移入 configuration crate） |
| `dialers` | `CandidateKey → DialCandidate` | 本节点到每个 topology 相邻可见节点的拨号候选与客户端材料；只含 configured 端点，local 端点走事实线 |
| `egress` | `Option<EgressPlan { capture, routes }>` | 禁用节点为 null；`routes` 是 IPv4＋IPv6 的完整互斥分割，每段 `EgressAction::{Exit { nodes, via }, Direct, Unmanaged}`；capture 不可用时 `routes` 为空 |
| `exitRole` | `Option<ExitRole { generation, family, nat }>` | enabled ∧ exitEnabled ∧ `feature:exit-ipv4` v1 时下发 |
| `dns` | `NetmapDns { meshHosts, resolvers, profile, egress: DnsPlan }` | DNS settings 在投影里；`resolvers` 为每个配置 resolver 键的 `ResolverAction::{Direct, Unmanaged, Exit, Rejected}`；`DnsService` 去掉 instance 与截止 |

投影规则（`meshway_policy::compile::netmap::project`）：

| 部分 | 纳入条件 |
| --- | --- |
| 可见节点 V(n) | n 自身 ∪ n 在可用节点（已登记且 enabled）topology 允许图中的连通分量 ∪ 与 n 有任一方向 Access 规则的已登记节点 ∪ 被保留 routing 规则 `via` 的节点（迭代至不动点） |
| `nodes` | V 中节点的 `NodeSettingsView`（listener 私钥、凭据与敏感头值均已脱敏） |
| `access` | src 命中 V 且（dst 命中 V 或 dst 含外部选择器）的规则，原顺序 |
| `topology` | a、b 两端都命中 V 的规则，原顺序 |
| `routing.mesh` | src、dst 都命中 V 的规则，原顺序 |
| `routing.egress` | 公共 `rules` 与 `auto` 全量；`assignments` 只保留 target 命中 V 中某节点的（原顺序）；`profiles` 只保留被保留 assignment（allowed／default）、V 中节点的 `selectedProfiles` 或 `auto` 引用的 |
| `groups`、`tagsOwners` | 名称：被保留规则（access／topology／mesh／assignment）以 `group:`／`tag:` 引用的，或含 V 中 User 节点的用户（组）／V 中节点携带的（tag）；成员与 owner 只保留 V 中 User 节点的用户邮箱 |
| `ruleSets` 及内容 | 只保留公共 egress 规则或被保留 profile 的规则在 `match.ruleSets` 引用的 |
| `inventory`、`subjects`、`selectedProfiles`、`selectionVersions` | 限于 V；`users`／`members` 只保留 V 中 User 节点的绑定 |
| 其余（defaults、addressPools、dns） | 全量 |

正确性依据：节点作为端点或中继参与的每条路径都在它的连通分量里；路径搜索只为 topology 允许的边计预算，routing 每段只以段端点 first-match，被删的规则不可能匹配分量内的段、边或流，因此投影上的结果与全量配置逐项相同（[差分验证](#规模与预算)）。

- **服务集合以节点对为单位**：Access 规则取并集、顺序无关；`bidirectional` 规则两个方向各自成立，单向规则只成立一个；回复仍只依赖已有正向流。
- **出口**：源端分割给出 `(nodes 有序主备, via)`；同一 `(源, nodes, via)` 的路径集合相同，目的地的服务集合是覆盖该目的地的外部 Access 规则并集。
- `observable` 由路径上全部节点在投影 inventory 中的 `feature:path-observation` v1 能力计算。
- `meshHosts` 与精确发现同一谓词：本节点自身及 `mesh_access_allows_any(self, T)` 的已登记活动节点；`magicDNS` 关闭时为空。
- `egress.capture.generation`（即决定的 `selection`）＝digest(网络、节点、有效方案、持久选择版本、routing.egress、分割结果)。**不含 revision**：无关配置变更不改变它，也不结束出口连接；任何选择切换（含切回）都会改变选择版本。出口节点从自己的投影重算源端 selection。`ExitRole.generation` 同样不含 revision。

### 与现有类型的关系

| 现有类型／字段 | N1 去向 |
| --- | --- |
| `NodeConfiguration` 的 network／node／revision／enabled／hostname／identity／addresses／selfKeys／meshAddressPools／listeners／exitEnabled | 原样进入 `self` |
| `NodeConfiguration.publication`、`PublicationId` | 消失；由 `serial` 和 `NetmapRef` 取代 |
| `peers`、`PeerConfiguration` | 由投影的 `nodes`（`NodeSettingsView`）＋`inventory`（`TrustedNode`）取代 |
| `dialers`、`DialCandidate` | 保留；范围是全部 topology 相邻可见节点；`endpoints` 只含 `configured`，`local` 改由事实线提供 |
| `permits: RequestId → PathPermit` | 消失；节点在投影上懒编译 `Decision { source, destination, target, services, paths, selection }`，按 (serial, source, target) 缓存，禁止预展开全部节点对 |
| `PathPermit.exit: ExitPermit` | 变形为 `Decision.target = Exit { nodes, via }` 与 `selection`；`roles` 中的 instance 绑定消失 |
| `LocalEgressPermit`、`localEgress` | 消失；`LocalDecision::{Direct, Unmanaged}` |
| `DiagnosticPermit`、`diagnostics`、`DiagnosticDemand` | 消失；诊断使用决定的服务集合与 `observable` 路径 |
| `DemandDecision`、`decisions`、`Demand` | 退出 Node 合同；路径检测若仍需决定原因，由 `plan` 的评估类型表达 |
| `CompiledNode`／`CompiledPublication` | 变形为 `CompiledNetmap { digest, netmap }`；不存在整网 publication 容器 |
| `EgressCapturePlan` | 保留并扩展为 `EgressPlan` |
| `ExitRolePlan` | 变形为 `ExitRole`；去掉 `instanceId`，generation = digest(network, node, family, nat) |
| `DnsExecution`、`DnsEgress`、`DnsServicePermit` | 变形为 `NetmapDns`、`DnsPlan`、`DnsService`；去掉 `sourceInstanceId`、`role.instanceId`、`expiresAt` 与 `associations` |
| `TrustedDnsAssociation` | 不进入 netmap；X6 时作为每源 overlay，见下文 |
| `InstanceLease.target`／`executionExpiresAt` | 消失；instance lease 只剩 instance 资格；数据面截止来自 `NetmapLease` |
| `CircuitAuthorization::{Business(RequestId), Diagnostic(ObservationId)}` | 变形为 `{ origin, target: Target, purpose: Business \| Diagnostic }` |
| `permit_authorization_digest`／`permit_transition` | 变形为每 (origin, target) 每路径的 `path_authorization_digest`，不含服务集合 |

### 规模与预算

每份 netmap 只检查 `MAX_NETMAP_BYTES`（8 MiB）安全上界，超限 `NetmapError::Limit`、整体拒绝；不设节点对或 grant 上限。每节点对仍最多 4 条路径、每条 ≤ 8 跳、每次搜索 16,384 工作项。为使投影与全量结果在预算边界也一致，路径搜索只为 topology 允许的候选边计预算，routing 段解析每段计 1 次（不再逐规则计）；这对现有按需编译只放宽预算边界行为。

N1a 实测（M3、release、单线程，私有证据 `.meshway/netmap/n1a/scale.md`），全互访、单一连通分量：

| 节点数 | Server 全网 `compile_netmap` | 每份 netmap | 节点 `LocalCompiler::new` | 节点 `decide`（4 条路径） | 中继 `transit` |
| --- | --- | --- | --- | --- | --- |
| 100 | 121 ms | 154 KB | 47 µs | 0.42 ms | 0.35 ms |
| 300 | 1.07 s | 457 KB | 117 µs | 1.2 ms | 1.1 ms |
| 1,000 | 13.1 s | 1.52 MB | 394 µs | 9.5 ms | 5.5 ms |

每份大小随可见 peer 线性（约 1.5 KB／peer），所有节点相同，没有中继 hub。Server 总成本 O(N²)（每份列出全部可见 peer）；节点首包成本是一次有界路径搜索，结果按 (serial, source, target) 缓存。

## 两条线

| | 稳定策略（netmap） | 动态事实 |
| --- | --- | --- |
| 内容 | 配置投影（身份、规则、脱敏声明、库存）、本节点拨号候选、capture、出口职责、DNS 计划 | peer 本机端点、STUN 映射、listener／link 状态、出口能力就绪、实例在线、观察任务 |
| Server 触发 | 配置提交、规则集切换、成员／节点生命周期、密钥轮换与吊销、库存能力变化、方案选择、地址退出窗口 | State 报告、实例 acquire／renew／release、观察创建／续订／取消、STUN 观测 |
| Server 动作 | 同事务重新投影受影响节点 → 新 serial → 唤醒其挂起的 `sync` | 更新 State hub，按投影可见性裁剪后唤醒挂起的 `sync` |
| 传输 | `sync` 响应的 `netmap`（full／delta／unchanged）＋`lease` | 同一 `sync` 响应的 `peerFacts`（按 `StateCursor` 接续，base 为 null 即全量） |
| Node 消费者 | 唯一 Control：校验、保存、发布 → Engine 向各组件应用 `Desired` | daemon 事件任务按类型分发：端点→Manager，出口就绪→SecureNode 出口选择，任务→诊断调度器 |
| 是否授权 | 是（唯一来源） | 否；只影响连接获取方式与在已授权集合内的选择 |

**判定一项输入属于哪条线**：若缺少它就必须拒绝某个包或某个握手，它属于 netmap；若缺少它只会让某次获取失败或换一个已授权候选，它是事实。端点、STUN、出口就绪都只是选择输入：错误或过期的值最多导致连不上，不能放宽准入。

### 传输：单一长轮询

Node↔Server 只有 `POST /agent-api/v1/sync`（类型在 `crates/meshway-protocol/src/netmap.rs`）：

```text
SyncRequest  { netmap: Option<NetmapRef>, state: Option<StateCursor { epoch, sequence }> }
SyncResponse { lease: NetmapLease,
               netmap: SyncNetmap::{ full { netmap } | delta { delta } | unchanged },
               peerFacts: Option<PeerFactsDelta { base, cursor, netmapSerial, updated: NodeId → PeerFacts, removed }> }
```

Server 挂起请求直至 netmap 或可见事实变化，或 ≤ 60 s 超时；超时回 `unchanged`＋`peerFacts: null`，即内建兜底。节点收到即发下一个请求，手里的 `NetmapRef`／`StateCursor` 就是游标，没有事件流与独立重放。`SyncResponse::check(request)` 校验租约指向最终内容、delta 的 base 等于请求、事实增量接续请求游标且属于同一 netmap serial。语义上仍是两条线：netmap 部分授权，`peerFacts` 部分只供选择。

### Server 侧

- `local_endpoints::replace`／`state::exit::replace` 只写 State 投影并唤醒挂起的 `sync`，不再调用 `publish`；`local_endpoints::apply` 删除（端点不再写进 `DialCandidate`）。
- 实例 acquire／renew／release 只修改 `node_execution` 的实例列；acquire 时库存能力若变化（构建升级），作为库存输入触发该节点及看见它的节点重新投影，这是唯一由 acquire 引起的 netmap 变化。
- 节点 State 的可见范围（today `execution/state.rs` 从 permit 推导，R7）改为：本节点投影的 `nodes`；端点细节只披露给 topology 相邻（`dialers` 指向或可拨入本节点）的可见节点。范围只随 serial 变化。
- UDP 穿透 prepare／accept 的授权（today R5 的共同 demand witness）改为：双方当前投影互相可见、topology 允许且有 Native UDP 承载。K6 的候选 CAS、双边接受、nonce／epoch、短期限全部保留。

### Node 侧

- Control 是唯一联系 Server 的 owner，循环发 `sync`：`netmap` 部分校验、保存、发布 → Engine `reconfig` → 各组件 `apply`；`peerFacts` 部分解码为 `PeerFacts { nodeId, listeners: key → { phase, network, localEndpoints, stunCandidates }, exit: { roleGeneration, status, installedFamilies }, observedAt, expiresAt }`，经有界队列交给对应 owner，不进入期望内容。
- Manager 以 `update_peer_facts` 合并候选，不重建 epoch、不换 scope；已有健康通道不因候选变化断开。
- 事实丢失（请求失败、Server 停机）时保留最后已知事实：`expiresAt` 只表示新鲜度，本机端点继续可拨，STUN 映射按各自截止停用；事实只在被替换、`removed`、离开投影或 netmap 结束时消失，并随 `peer-facts.json` 跨离线重启，见[最后已知事实](/v2/node/connectivity#last-known-facts)。

## 本地准入

### 首包

```text
TUN 源包 → Packet::parse → 查唯一 FlowTable（已有流直接走原路径）
  → 新流：decision = cache[(serial, self, target(dst))] 或 compile_local(netmap, self, dst, service)
  → Permitted 且 services.contains(proto, port)；Direct／Unmanaged 本地执行；Denied 丢弃
  → 选路径：paths 中首条下一跳通道已健康者；都没有则按序请求邻接并在有界队列等待
  → SecureNode 握手（完整路径、(origin, target)／path 摘要）→ 流提交 → 加密发送
```

没有 demand RPC、没有 publication 等待、没有 Intent 续租。首包队列与握手重发保留现有上限；等待期间不提交流。终点与出口收到握手时以同一函数在自己的投影上为 (origin, dst) 求决定，中继用 `admit_transit`。

### 准入函数

位于 `meshway_policy::compile::netmap` 与 `meshway_policy::admission::netmap`（旧的按 `NodeConfiguration` 的入口保留至 N1h）：

| 函数 | 检查 |
| --- | --- |
| `compile_local(netmap, source, dst, (protocol, port)) -> LocalDecision::{Permitted(Decision), Direct, Unmanaged, Denied(reason)}`；`LocalCompiler::new(netmap).decide(…)` 复用一次投影转换 | 与按需编译同一 `evaluate`：Mesh 分类、地址池拒绝、egress first-match、Access、routing first-match＋递归 via、topology 承载、Internet 边界、出口能力（不含就绪）与 selection |
| `transit(netmap, origin, target)`／`LocalCompiler::transit` | Mesh：该对有任一 Access 服务；出口：源端分割中存在 `(nodes, via)` 动作且有覆盖它的外部 Access；返回全部路径 |
| `admit_handshake(leased, decision, path, origin, previous, reply, now)` | 租约绑定且未到期、本节点 enabled、`origin == decision.source`、path 在 `decision.paths`、本节点是端点、位置与 previous 匹配、路径节点密钥、出口终点 `exitEnabled` 与本地 `exitRole` |
| `admit_forward(…, traffic, now)` | 上项＋源地址属于 origin、目的属于 Mesh target 或等于出口决定的目的且非 Mesh 地址／池、`services.contains` |
| `admit_reply(…, forward, reply, now)` | 上项＋`forward` 来自调用者自己的已建立流，reply 是精确反向 tuple |
| `admit_transit(leased, compiler, origin, target, path, previous, reply, now)` | 租约、本节点是 path 的中间节点、path 在 `transit(origin, target)` 内、位置与密钥 |
| `admit_observation(…, service, now)` | handshake＋Mesh 决定含该服务＋路径全部 observable |
| `admit_adjacency(compiler, netmap, peer, carrier)`／`admit_path_carrier` | topology 与双方声明允许该承载；本节点拨号时 `dialers` 含该候选 |

`now` 只用于租约；决定本身没有截止。DNS 对应 `dns_route::{netmap_resolver, netmap_dns_service}`。

### 流表与 circuit

- 流键从 `{network, publication, demand, node, role, path, tuple}` 改为 `{origin, target, node, role, path, tuple}`。publication 不再是主键，`Engine::reconciled_flows` 的整表克隆与重标（R1，65 行）删除。
- circuit 上下文从 `CircuitAuthorization` 改为 `(origin, target, purpose, path)`；握手 transcript 绑定 **path 授权摘要**（origin／target、完整路径、路径节点身份／地址／密钥、本跳承载候选、出口 selection），不含服务集合。Access 增删端口不迫使重新握手；每包准入照常按当前 netmap 检查服务。
- 这改变握手上下文的授权字段与 tombstone 规则（见下），**wire 升为 v6**（`MWCIR\0\0\x06`）；稳态 Data 帧 86 B 布局不变。

### `SecureNode::update`

`refresh`／`reconcile` 合为一个入口 `SecureNode::update(identity) -> Update`：

| 情况 | 行为 |
| --- | --- |
| 同 `NetmapRef`（只续租） | 不调用组件；只有 Engine 的计时延长；不扫描、不换代、不取消输出 |
| 新 serial | 对每个 circuit 在新投影上重算决定并比较 path 授权摘要：相同则保留 Noise 状态与流，否则退役；对每条流检查其精确服务仍在新决定的 `services` 内，否则以独立流 guard 退役；出口流同时发 `Retire` |
| 本节点身份／地址／密钥变化 | 全部退役（硬屏障） |

不存在“直接后继”判定：Control 只接受 serial 更大的内容，比较的是新旧完整内容本身。跳过中间 serial 不需要中断连续性——若中间曾撤销后又恢复，新内容与旧内容等价，两端在各自观测到的每个时点都持有授权；永久吊销的节点不会以相同身份恢复。

### 邻接按需建立

netmap 覆盖全部可达 peer，若“收到候选即建连”会变成全连接。建议改为本地需要驱动：

- dataplane 在首包选路或中继收到经过本节点的握手时，向 Manager 登记 `AdjacencyInterest(peer)`；Manager 只对 `admit_adjacency` 允许的可见 peer 获取连接。
- 无 circuit 且无 interest 的邻接空闲 120 秒后关闭；`keepalive` 边与只有一方能拨出的边常驻，定义见 [Topology · 邻接的建立与常驻](/v2/configuration-reference/topology#邻接的建立与常驻)。
- 协调 UDP 穿透由任一有需求（或常驻）的一方以自己的拨号承载发起；同一 peer 对／地址族 Server 只接纳一个活动尝试，另一方读到 offer 即接受，不需要本机 interest。
- STUN 刷新只对存在需求（interest 或常驻）且有 UDP 获准承载（任一方向）或健康 Native UDP 通道的 socket 进行，预算（每 60 秒 30＋30 次）保持。

## 租约与期望状态

Control 只验证、保存并发布签名 netmap（内容＋租约），不签发执行快照；`meshway-engine::Engine` 是租约的唯一计时者，并把同一 `Desired` 应用到各组件。组件不持有租约、代际、继承物或可转让的输出权限；同内容续租只延长 Engine 自己的计时，到期由空期望 fail closed。行为定义见 [Engine](/v2/node/runtime#engine)。

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
| netmap 同内容续租 | 不是交接；只延长 Engine 的计时 |
| 配置 revision、成员、密钥、库存、方案选择导致的新 serial | **发生**：`SecureNode::update` 按 (origin, target)／path 比较；Manager／Host／DNS 只对实际变化的资源做差量 |
| 本节点身份变化、down、re-enroll | 硬屏障，不交接 |

## 诊断

### `meshway ping`

CLI ping 完全本地：目标名由 netmap `meshHosts` 解析；需要 `compile_local(self, target)` 允许且 `services` 含 `icmpEcho`；只在 `observable` 路径上执行；探测帧使用 `purpose = Diagnostic` 的独立 circuit，仍不进入业务 FlowTable。Server 不可用时 ping 照常工作。CLI ping 不创建 Server 订阅，结果只回到 IPC。

Unsupported 与 Denied 的区别保留：决定为 Denied 或服务不含所需 scope 为权限拒绝；允许但无 `observable` 路径为 unsupported。

### Console 观察

```text
Console createObservation
  → Server：用户权限＋在 source 当前投影上用同一编译器检查服务／显式路径（不写 netmap）
  → 写任务（动态事实），唤醒 source 挂起的 `sync`
  → source Control 取 getNodeTasks → 本地按 netmap 执行
  → reportTaskResult → Server 转发订阅者 SSE
```

- 订阅 30 s 期限、10 s 续租、任务共享、样本窗口、`observation.*` 事件保留。续订只改任务截止，不产生 publication。
- 任务执行权限 = source 在当前投影上的决定；任务截止只限制持续多久。netmap serial 变化导致决定或路径消失时，Node 结束任务并上报 `failed`；路径变化开启新分段。
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
| Server 可用、`sync` 挂起中 | 提交事务投影 → 挂起请求立即返回 delta → Node 校验、保存 → `update` | 秒级；lab 门槛 ≤ 5 s |
| Server 可用、`sync` 失败重试中 | 下一次 `sync` 即取到 | ≤ 重试间隔＋一次请求 |
| Server 不可用或节点分区 | 无法推送 | ≤ 该节点剩余租约（≤ 24 h） |
| 成员撤销 | 额外 `stream.closed: membership_revoked` 使被撤销节点立即本地失效 | 同上；不依赖被撤销节点配合 |
| 密钥吊销／节点删除 | 所有引用该节点的 netmap 重编，peer 删除其密钥 | 同第一行；接收端先收到即先拒绝 |

撤销按接收端执行（[Access](/v2/configuration-reference/access#enforcement)）：被撤销的源即使不配合，任何已收到新 netmap 的终点、中继和出口都会拒绝。只有当两端都与 Server 分区时，旧授权才可能存续到租约到期。这与 Tailscale 的取舍相同。

### 到期行为：终止全部

建议 netmap 租约到期时 **已有流与新流一并终止**：全部 circuit 退役、排队输出取消、出口投影退役并精确清理 CT；源端 capture 保持负向边界（与今天 lease 过期一致），DNS 保持负向安装。

理由：租约到期是分区状态下撤销的唯一上界。若只拒新流，一条被撤销的长连接（TCP、隧道、持续 UDP）在两端都离线时可以无限存续，撤销失去最坏界限。到期发生在 Server 已至少 1 小时（续期周期）不可达之后，主动断开是可预期的。恢复联系后取得新租约即可重新握手。

### revision 变化时的已有流

由 `SecureNode::update` 在新投影上重算判定：决定仍为 Permitted、流的精确服务仍在 `services`、所用完整路径仍在 `paths`、路径上全部节点身份／地址／密钥不变、所用承载候选仍在、出口 `selection` 不变——全部满足则保留；任一不满足则结束该流（必要时只结束该 circuit 上的部分流）。方案切换总是结束旧出口连接。

## 出口与 DNS

### 出口

| 项 | 归属 | 说明 |
| --- | --- | --- |
| 出口决定（源→出口的服务、路径、selection） | netmap 投影上本地编译 | Access ∩ 允许方案 ∩ `exitEnabled` ∩ 能力 ∩ 路径；不要求就绪 |
| `ExitRolePlan` | netmap（出口节点） | 只要 `exitEnabled` 且声明 `feature:exit-ipv4` 就下发；去掉 instance 绑定。职责本身仍不转发包 |
| `EgressCapturePlan`／`egress.routes` | netmap（源节点） | 按 revision＋选择版本 |
| `ExitCapability`（probing／ready／unavailable、families） | 动态事实 | 出口节点 State 上报，经 `sync` 的 `peerFacts` 送给看见该出口的节点 |
| X1 `ExitInputs` | 拆分 | `instances`／`instance_expires_at_ms` 不再是编译输入；`selection_generations` 进入 netmap 稳定输入；`ready_until_ms`／`unavailable_reasons` 变成源端选择用事实 |

源端在 `egress.routes` 给出的有序主备出口中，选第一个能力事实新鲜且 ready 的出口建流；都不 ready 时丢弃并计数，保留负向捕获，**永不 direct**。出口端准入只看自己投影上的决定与本地已安装 role，不看自己的 State 上报。能力事实错误只会让源选错出口而被拒，不会放宽准入。

这把“出口能力按需编译”收窄为“出口能力作为选择事实、零编译”，与已定决定的措辞不同，列入[已决产品问题](#已决产品问题)。

### DNS

- 配置 resolver 与系统 resolver 的本地 socket 许可：源端用 `egress.routes` 求 resolver 端点的动作——direct／unmanaged 允许本机 socket，exit 动作必须经出口 DNS，unmanaged 范围以外的 Mesh 地址拒绝。UDP 与 TCP 仍分别判定。R1／R4／F14 的 resolver demand、ID 映射和退休白名单删除。
- X5 `DnsServicePermit` 留在 netmap：客户端 `ViaExit`、出口 `ServeExitResolver` 列表都按 revision 编译，去掉 instance 与截止字段；出口就绪仍是事实。Mesh ingress 适配缺失时安装阻断的现状不变。
- X6 可信域名关联是唯一保留的按需编译：Server 受控解析任务完成后，为该源签发 `DnsAssociationOverlay { netmap: NetmapRef, associations, expiresAt = min(TTL, 任务期限) }`，经同一 netmap 查询端点交付并随期望内容应用。overlay 有自己的截止，只扩大该源与相关出口的域名目的判定，不改变 netmap serial；它只影响源和所选出口两个节点。

## 迁移

| 项 | 方案 |
| --- | --- |
| Node Control 记录 | 记录格式升为 **v5**：身份 envelope 与 netmap 缓存分离（缓存带自己的 `compilerVersion`），今后编译器升级不再改身份版本（S5）。v1–v4 沿用现有规则：校验完整性与绑定、私有归档原字节、清空缓存、保留身份 |
| netmap 缓存用途 | 只作 delta 基准；进程重启后是否可凭未到期缓存离线运行见[已决产品问题](#已决产品问题)，默认不恢复 |
| Server 数据库 | 新的当前 schema：删除 `execution_demands`、`execution_publications`、诊断编译镜像表；新增 `node_netmaps(network, node, serial, revision, digest, sealed payload, created_at)` 与有界历史；`node_execution` 去掉 `target_publication_id`／`execution_expires_at`，回执改引用 `NetmapRef`。按[存储](/v2/storage)现行规则没有迁移链：开发环境重新初始化 Server 数据目录 |
| HTTP | 新增 `POST /agent-api/v1/sync`（`SyncRequest`／`SyncResponse` 已定义于 `crates/meshway-protocol/src/netmap.rs`，接入路由后才进入 OpenAPI），取代 Node SSE；删除 demands、`configuration/refresh`、diagnostic 编译相关字段；`/agent-api/v1` 根不变。重生成 OpenAPI／Rust／TS SDK 并过 drift |
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
| Server R2 | 诊断两遍编译与中间证据（192） | 消失：任务创建在 source 投影上判定；Denied／Unsupported 由决定与 `observable` 表达 |
| Server R3 | 订阅镜像→dirty→republish 泵（98） | 消失：任务是动态事实 |
| Server R4 | retained slice／空 delta／部分 supersedes（67） | 变形：每节点内容 digest 相同则不递增 serial |
| Server R5 | 共同 demand witness（212） | 消失：协调授权改为双方投影互见且 topology 允许 Native UDP |
| Server R6 | demand／republish 合约测试（1,491） | 消失：有效断言（receipt 顺序、撤销、旧实例）迁到 netmap fixture |
| Server R7 | 需求驱动的 State 可见与期限聚合（42） | 变形：可见范围取投影 `nodes` |
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
| Dataplane R1 | publication 为流主键与整表重标（65＋45） | 消失：流键改为 (origin, target) |
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
| **N1a** 契约与编译器 | 建 | `meshway-model`、`meshway-configuration` netmap 投影类型／delta／digest；`meshway-policy` `project`／`compile_netmap`、`compile_local`／`transit` 与准入；`meshway-protocol` 类型 | 首包；冻结类型后其他包并行 | 差分测试：同一配置下旧 `compile_with_exit` 对任一 (source, target, proto, port) 的 Permitted 与新 grant 准入等价；预算、first-match 分割、递归 via、Internet 边界 | — |
| **N1b** Server 分发 | 建 | Server `store/` 新 netmap 表与 schema、`configuration/commit` 编译钩子、库存／成员／选择触发、`http/agent.rs` 长轮询 `sync` | N1a 后；与 N1c 文件互斥并行 | 提交同事务产生 serial；无关提交不变 serial；撤销重编引用节点；租约续期不写 serial；预算超限拒绝提交 | — |
| **N1c** Server 事实与诊断 | 变形 | Server `state/`（service、local_endpoints、exit、provider）、`execution/{state,connectivity,diagnostics}.rs`、`observations/` | N1a 后，与 N1b 并行 | State 报告与实例事件零编译；可见范围＝投影 `nodes`；穿透授权＝双方投影互见且 topology 允许；观察创建只读 netmap，续订不改 netmap | — |
| **N1d** authority 与 Control | 变形 | `meshway-authority` snapshot／clock；`meshway-control` netmap 查询与续期、记录 v5、事件、回执、观察；删除 `demand.rs` | N1a 后；先冻结快照 API 供 N1e–g | Server 停止后 session／instance 过期而快照有效；同内容续租不取消 guard；serial 回退拒绝；v1–v4 记录归档 | — |
| **N1e** dataplane 与诊断 | 变形 | `meshway-dataplane`（flow、secure、codec v6、probe、exit）；`meshway-diagnostics` | N1d API 后，与 N1f／N1g 并行 | 真 SecureNode A→R→B：首包无外部调用、`update` 保留未变 (origin, target) 流、端口撤销只结束该流、路径变化结束 circuit、租约到期全终止、tombstone 窗口回收、wire v6 互验 | — |
| **N1f** connectivity | 变形 | `meshway-connectivity`（Manager、continuity、traversal、workers） | N1d API 后并行 | `AdjacencyInterest` 驱动获取、空闲关闭、事实更新不换 epoch、serial 变化只动变化资源、CAS／ABA 保留 | — |
| **N1g** underlay 与 DNS | 变形 | `meshway-underlay`（protection、kernel exit／egress 接口）；`meshway-dns`（route、dispatcher、system） | N1d API 后并行 | F01／F02 删除后资源撤销及时关闭 FD；resolver 按 `egress.routes` 判定；ScopeFence 只随 netmap／就绪变化 | — |
| **N1h** daemon 切换 | 删 | `apps/meshway-daemon`（control、network、dns、ping）、`meshway-ipc` v6、`apps/meshway-cli` | N1e–g 合入后；Node 侧旧链在本包整体删除 | daemon 进程内三节点：首包、ping、DNS 不产生 HTTP；`sync` 事实分发；netmap 更新差量 | T2 全部既有行重跑 |
| **N1i** Server 旧链删除 | 删 | Server demands、publications、诊断编译、witness、`instances.rs` 编译调用、R6 测试族、cache obsolete 分支 | N1h 后（无旧消费者） | 迁移后的 receipt／撤销／旧实例断言通过 | — |
| **N1j** 验收与文档 | 验 | `tools/meshway-{packet,runtime}-fixture`、`scripts/validate_matrix.py`、`docs/v2/`（execution、node-configuration、topology、routing、access、control、runtime、dataplane、connectivity、diagnostics、realtime、storage、validation）、AGENTS 描述 | fixture 可在 N1a 后准备；运行等 N1i | — | 新行与 lab，见下 |

可并行范围：N1b‖N1c‖N1d（N1a 后）；N1e‖N1f‖N1g（N1d 快照 API 冻结后）；C0 任何时候。

### 新增 T2 行建议

| 行 | 拓扑与步骤 | 通过条件 |
| --- | --- | --- |
| `netmap/offline` | Controller＋A／R／B（Native TCP 与 Native UDP 静态端点各一轮）；建流后停止 Controller 3 分钟（常驻 lab 另做 10 分钟） | 已有 TCP／UDP 持续；新 5-tuple 业务流、`meshway ping`、新静态邻接（预先未连接的 A–R 承载）全部成功；零 HTTP 成功请求；恢复后无第二次 up、无重装 |
| `netmap/revoke` | Controller 在线；撤销 A→B 端口、撤销成员、轮换 B 密钥、方案切换 | 新 serial 送达 ≤ 5 s；受影响流停止、未受影响流 cipher 不变；旧输出不复活 |
| `netmap/expiry` | 停止 Controller 越过租约（需短租约，见未决问题） | 到期时全部 circuit 终止、capture 保持负向、DNS SERVFAIL；恢复后自动重建 |
| 既有十一行 | 原场景 | 另断言 demand／publication 相关路由不存在、首包无控制面请求 |

### 常驻 lab 验收（sha／lax／nyc）

1. 按 [Docker lab](/v2/node/docker-lab) 核对 PID／hash 与 netmap serial；配置 sha→lax→nyc 必经路径与 ICMP、TCP、UDP 授权。
2. 建立持续 iperf3 与 125 s 纯接收流后停止 Server 10 分钟：持续流不断；期间新开 TCP／UDP 流、`meshway ping nyc`（名字本地解析）、首次使用的 lax–nyc 静态承载均成功。
3. 恢复 Server：节点自动重新认证、取新 instance，saved／applied 指向同一 `NetmapRef`，无数据面重装。
4. 撤销 sha→nyc 的 TCP 端口：≤ 5 s 内该流停止、ICMP 仍通；撤销 sha 成员：nyc 与 lax 拒绝其全部流量。
5. 恢复配置，记录全部 serial 与时间戳；地区名只是节点标识，不推断真实跨地域延迟。

## 已决产品问题

| 问题 | 决定 | 影响 |
| --- | --- | --- |
| 进程重启后凭缓存的未到期 netmap 离线运行 | **允许**：netmap 由 Server 签名并带有效期，重启后验签通过且未到期即载入；与 Tailscale 一致 | Control record 保存签名 netmap；AGENTS"缓存不复活授权"改为"签名有效期内的 netmap 是授权本身，不是缓存"；被撤销节点在分区中最多存续到到期，与运行中分区同一上界 |
| 有效期可配置 | **Server 参数 `--netmap-validity`**，默认 24 h、最小 60 s（参数单位为秒） | 下限是防止运维误设过短导致 Server 抖动即全网 fail closed 的可用性保护，不是安全边界；更短的 validity 只会让撤销更快生效。`netmap/expiry` 用 60 s 验收 |
| 出口就绪 | **纯动态事实，不编译**：netmap 的出口 grant 是管理员配置的有序主备；哪个出口 ready 走 State 事实线，源端本地选 | 推翻 10-08"出口能力仍按需编译"；X1 的 `ExitInputs` 退出编译输入，`ExitCapability` 保留为事实 |
| 邻接按需 | **按需建、空闲 120 s 关**；topology 边可标 `keepalive: true` 常驻（必经中继、出口） | topology.md 更新；cold 首包多一次建邻接，lab 门槛 cold ≤ 300 ms |
| 规模上限 | **不设协议级拒绝上限**，参照 Tailscale：首次完整 netmap，之后差量（peer 增删、轻字段 patch）＋按 netmap peers／edges 的可见性裁剪；仅保留 8 MiB 单份安全上界防异常 | 分发通道增加 `netmap.delta`；超 8 MiB 视为缺陷而非容量规划 |
| Node↔Server 同步传输 | **单一长轮询端点 `POST /agent-api/v1/sync`**，取代 Node SSE：请求携带节点持有的 `netmap: NetmapRef` 与 `state: cursor`，Server 挂起至有更新或 ≤60 s 超时，响应 `{ lease, netmap: full | delta | none, peerFacts: delta | none }`；节点收到即发下一个请求。无事件流、无独立 cursor 重放，节点手里的 serial／cursor 即是游标；hold 超时回 `none` 即内建兜底，不再另设定时刷新。与 Tailscale MapRequest long-poll 同形。Console 的用户 SSE 保留 | N1b 不把 `netmap.changed`／`netmap.delta`／`state.delta` 并入 `StreamEvent`，改做 `sync`；N1d 删除 Control `events.rs` 的 Node 侧 Interest 投影；`NetmapQueryRequest`／`Response` 并入 `SyncRequest`／`SyncResponse` |
| netmap 的形状 | **配置的投影，不是编译结果**：`ProjectedConfiguration` 是 `NetworkConfiguration` 的单调子集（可见 peers、涉及本节点的 Access／topology／routing 规则、双方 listeners／dialers、egress capture、exitRole、dns）；节点用同一 `meshway-policy` 在投影上**懒编译**本对 (source, target)，按 (serial, source, target) 缓存，禁止预展开全部节点对；中继用 `admit_transit`。Server 为 Console 做路径检查时在全量配置上用同一 crate | 删除 N1a 首版的 `grants`／`GrantPath` 物化；中继 hub O(N²) 问题随之消失；每份 netmap O(可达 peer＋涉及规则) |
| NAT–NAT 按需边的打洞信令 | **Server 只传信令，不转发流量**：一方登记打洞意图后，Server 在对端的 `sync` 回复里带"待处理的穿透请求"，对端据此做 STUN 观测并参与协调打洞，不要求本地 interest；对应 Tailscale 经 DERP 送 disco ping 的角色，但我们没有全员兜底中继，数据面仍只经策略定义的路径 | `SyncResponse` 增字段；connectivity 收到提示即登记 interest；排在签发钥轮换之后实现，当前 fail-closed 不支持 |
| Netmap 签发钥轮换 | **背书交接，离线运维命令触发**：`meshway-server rotate-netmap-key [--window 48h]` 生成下一代钥 K′ 并立即作为当前钥（新接入直接钉 K′）；窗口内回复仍由 K 签，另带 `keyRotation`（K 对 K′ 的背书与 K′ 对同一租约的签名），节点验证背书后原子改钉 K′；窗口后只由 K′ 签。不跳代、不自动轮换、不支持多 Server 实例 | 错过窗口的节点不撤销已持有租约，只拒新回复（`ServerKeyMismatch`、`blocked`），库存视图 `serverKey.status=keyMismatch`；恢复须重新接入。定义见 [Server](/v2/server#netmap-签发钥轮换) 与 [Control](/v2/node/control#server-签名钥轮换) |
| 持续 STUN 观测 | 每个 Native UDP listener socket **持续**做 STUN（空闲 ~30 s、有流量 ~10 s，带抖动），映射变化才经 State 上报进 peer facts；不再只对有 interest 的 socket 观测。作用：候选常新、NAT 映射保活、收到穿透提示即可立刻打洞；离线时按最后已知保留。STUN 是一次 UDP 往返，与向 Server 查 offer 的 HTTP 请求无关 | connectivity 的观测调度去掉 interest 门槛，预算改为按 socket 的固定节拍；与上一行同一个包实现 |
| 投影实现（N1a 修正） | **删除首版 `grants`／`GrantPath`／`edges`**：`NodeNetmap { self, projection: ProjectedConfiguration, dialers, egress, exitRole, dns }`；可见节点＝topology 连通分量 ∪ Access 相关 ∪ 保留规则的 via；规则按“能匹配可见节点对”原样保留；节点侧 `compile_local`／`transit` 复用同一 `evaluate`，与全量按需编译逐查询等价 | 1,000 节点全互访每份 1.52 MB、所有节点相同；中继 hub 消失；路径搜索预算只计 topology 允许的边 |

## 限制

- 中继获知所有经过它的 (source, target) 节点对，今天只获知活跃需求；这是 netmap 模型固有的披露范围扩大。
- 投影披露同一连通分量内全部节点的身份、地址、公钥与脱敏承载声明，以及能匹配它们的规则；不披露 listener 私钥、凭据与敏感头值（`NodeSettingsView`）。
- 同一身份的克隆进程（复制私钥）在 Server 层仍只能持有一个 instance，但两者都能使用各自已取得的 netmap 租约直到到期；instance 替换不能撤回已签发租约。
- Server 停机期间无法进行协调 UDP 穿透；只有静态端点与已有通道可用。
- 本稿不改变端到端保护、源授权、有序必经点、原路径精确回复、重放保护、State 不自授权、失败不静默放行等硬约束；它们在 N1 下由同一 owner 执行。
