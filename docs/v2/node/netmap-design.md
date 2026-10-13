# Netmap 设计

本页定义 Node 授权的唯一来源——每节点签名 netmap——的形状、分发、本地准入、租约与撤销。字段与文档注释以所属 crate 源码为准，HTTP 契约以生成的 OpenAPI 为准；验收状态见[验收范围](/v2/node/validation)。

## 目标与结论

Server 在配置 revision 固定时为每个节点生成 **netmap**（配置投影），签发租约（默认 24 h）；节点本地准入新流、建邻接、ping，到期 fail closed；配置变更／撤销经长轮询 `sync` 送达新投影，Server 不可用时撤销最迟随到期生效。Server 承担 coordination：netmap 分发、STUN 协调、State 汇聚。唯一依赖运行时事实的按需签发是可信域名关联。

1. **执行权限只来自 netmap 租约**。Control 发布已验签的内容与租约，Engine 是租约的唯一计时者并向组件应用不可变 `Desired`；没有执行快照或 execution lease。session 只约束 HTTP，instance 只约束 State、回执与协调请求。Server 停机 10 分钟，数据面不受影响。
2. **两条线严格分开**。稳定策略（Access ∩ topology ∩ routing ∩ 身份库存 ∩ 方案选择）只在少数提交事务中投影成 netmap；动态事实（候选端点、STUN 观测、出口就绪、实例资格、观察任务）走同一 `sync` 长轮询中独立的 `peerFacts` 部分，由各自 owner 消费，**永不触发投影**。
3. **首包、ping、State、续租都不改变执行身份**。只有 netmap 内容变化（serial 递增）才产生一次资源差量更新；同内容续期是 O(1) 截止延长。

## Netmap 的形状

### 文档与租约

一个节点在任一时刻有一份 `NodeNetmap`（内容）和一个 `NetmapLease`（有效期）。内容只在稳定输入变化时改变；租约可在内容不变时反复续期。字段与文档注释以源码为准：

| 类型 | 定义位置 | 要点 |
| --- | --- | --- |
| `NetmapRef { revision, serial, digest }` | `crates/meshway-model/src/netmap.rs` | `digest` 是内容摘要，不含租约、`revision`、`serial`、`dependencyDigest` |
| `NetmapLease { netmap, issuedAt, expiresAt }` | 同上 | `NetmapLease::validate` 要求 `issuedAt < expiresAt ≤ issuedAt + 24 h`；配置下限 `MIN_NETMAP_VALIDITY_MS`＝60 s、`MAX_NETMAP_BYTES`＝8 MiB |
| `NodeNetmap`、`CompiledNetmap { digest, netmap }` | `crates/meshway-configuration/src/netmap/mod.rs` | `NodeNetmap::content_digest`、`CompiledNetmap::verify`／`reference`；`NETMAP_COMPILER_VERSION`＝`meshway-netmap-2` |
| `ProjectedConfiguration` | `crates/meshway-configuration/src/netmap/projection.rs` | 投影本体，见[内容字段](#内容字段) |
| `NetmapDelta` 与 `netmap_delta`／`apply_netmap_delta` | `crates/meshway-configuration/src/netmap/delta.rs` | 投影的 nodes／inventory 与 dialers 按键增删，其余整块替换；重建后必须复现目标 digest，serial 必须增大 |

| 字段 | 含义 |
| --- | --- |
| `revision` | 首次产生这份内容的配置 revision；内容不变的后续 revision 沿用旧值 |
| `serial` | 每节点单调递增的内容序号；内容 digest 不变则不递增 |
| `digest` | 规范序列化内容摘要；Node 校验后保存 |
| `dependencyDigest` | 产生该 serial 的整网投影输入摘要（配置、主体、库存密钥／能力、方案选择及版本、规则集）；与 `revision` 一样随 serial 保留，不进入 `digest` |
| `compilerVersion` | **`meshway-netmap-2`**：投影的 `ruleSets` 为 digest 引用；其他版本的缓存视为过时并作废重取 |
| `issuedAt`／`expiresAt` | 租约，不进入 `digest`；Server 每次成功查询签发 `now + 有效期`（默认 24 h） |

没有“依赖截止”缩短整份 netmap：可信 DNS 关联作为独立 overlay 按条目携带自己的截止，见[出口与 DNS](#出口与-dns)。

### 内容字段

**netmap 不是编译结果，而是配置的投影**。Server 不物化任何节点对 grant 或路径；节点在首包／握手时用同一 `meshway-policy` 编译器在投影上求自己那一对 (source, target) 的服务集合与路径。全部定义在 `crates/meshway-configuration/src/netmap/`：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `self` | `NetmapSelf` | enabled、hostname、identity、addresses、selfKeys、meshAddressPools、本节点完整 listeners（禁用时为空）、exitEnabled |
| `projection` | `ProjectedConfiguration { configuration: ConfigurationView, subjects: SubjectContext, inventory, ruleSets, selectedProfiles, selectionVersions }` | 全量配置的单调子集，复用现有 `ConfigurationView`／`NodeSettingsView`（已脱敏的 listener／dialer 声明）、`AccessRule`／`TopologyRule`／`MeshRoute`、`TrustedNode`、`SubjectContext`（移入 configuration crate）；`ruleSets` 为 `key → RuleSetRef { digest, format, bytes, entries }`，见[规则集 artifact](#rule-set-artifacts) |
| `dialers` | `String → DialCandidate`（候选键） | 本节点到每个 topology 相邻可见节点的拨号候选与客户端材料；只含 configured 端点，local 端点走事实线 |
| `egress` | `Option<EgressPlan { generation, rules, fallback, unavailable }>` | 禁用节点为 null；`rules` 为生效规则 `{rule: JSON Pointer, action: EgressAction::{Exit { nodes, via }, Direct, Unmanaged}}`（公共在前），`fallback` 为未命中动作；不含前缀——分割由节点用投影与 artifact 计算（[Routing · 执行捕获](/v2/configuration-reference/routing#执行捕获与精确许可)） |
| `exitRole` | `Option<ExitRole { generation, family, nat }>` | enabled ∧ exitEnabled ∧ `feature:exit-ipv4` v1 时下发 |
| `dns` | `NetmapDns { meshHosts, resolvers, profile, egress: DnsPlan }` | DNS settings 在投影里；`resolvers` 为每个配置 resolver 键的 `ResolverAction::{Direct, Unmanaged, Exit, Rejected}`；`DnsService` 不含 instance 与截止 |

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
| `ruleSets` 引用 | 只保留公共 egress 规则或被保留 profile 的规则在 `match.ruleSets` 引用的 key（含 `<provider>:<name>`）；`configuration.ruleSets` 与 `configuration.ruleSetProviders` 的定义（URL、目录、inline 列表）一律不投影 |
| `inventory`、`subjects`、`selectedProfiles`、`selectionVersions` | 限于 V；`users`／`members` 只保留 V 中 User 节点的绑定 |
| 其余（defaults、addressPools、dns） | 全量 |

正确性依据：节点作为端点或中继参与的每条路径都在它的连通分量里；路径搜索只为 topology 允许的边计预算，routing 每段只以段端点 first-match，被删的规则不可能匹配分量内的段、边或流，因此投影上的结果与全量配置逐项相同（[差分验证](#规模与预算)）。

- **服务集合以节点对为单位**：Access 规则取并集、顺序无关；`bidirectional` 规则两个方向各自成立，单向规则只成立一个；回复仍只依赖已有正向流。
- **出口**：源端分割（节点用投影规则与持有的 artifact 计算，按方案缓存于 `LocalCompiler`）给出 `(nodes 有序主备, via)`；同一 `(源, nodes, via)` 的路径集合相同，目的地的服务集合是覆盖该目的地的外部 Access 规则并集。
- `observable` 由路径上全部节点在投影 inventory 中的 `feature:path-observation` v1 能力计算。
- `meshHosts` 与精确发现同一谓词：本节点自身及 `mesh_access_allows_any(self, T)` 的已登记活动节点；`magicDNS` 关闭时为空。
- `egress.generation`（即决定的 `selection`）＝digest(网络、节点、有效方案、持久选择版本、routing.egress、生效规则所用集合的 digest)。与节点持有哪些 artifact 无关。**不含 revision**：无关配置变更不改变它，也不结束出口连接；任何选择切换（含切回）都会改变选择版本。出口节点从自己的投影重算源端 selection。`ExitRole.generation` 同样不含 revision。

### 规则集 artifact {#rule-set-artifacts}

规则集内容不进入 netmap。`projection.ruleSets` 的每个引用是 `{digest: "sha256:<hex>", format: 1, bytes, entries}`，进入 netmap content digest；规则集刷新只改变受影响节点 netmap 中的引用（新 serial、重签租约），不需要重新投影其余内容。`dependencyDigest` 覆盖每个 key 的 digest。

| 项 | 行为 |
| --- | --- |
| 端点 | `GET /agent-api/v1/artifacts/{digest}`，节点会话认证（用户会话为 401） |
| 授权 | digest 必须被 Server 为该节点存储的某个 serial 引用，且该 serial 仍可能在租约内：当前 serial，或被替换（或撤回）不足一个租约有效期的 serial；否则 404，不区分不存在与无权。Server 在写入每份 netmap 的同一事务里写 `node_netmap_artifacts(network, node, serial, digest, superseded_at)` 引用索引，请求只做一次索引查询，不解密 netmap |
| 响应 | `200 application/vnd.meshway.rules.v1` 原始字节；`ETag: "sha256:<hex>"`；`Cache-Control: private, max-age=31536000, immutable` |
| 节点校验 | 长度等于 `bytes`、SHA-256 等于 digest、`MWRS` 结构完整且成员数等于 `entries`；任一失败丢弃并退避重试（5 秒起，上限 5 分钟） |
| 缓存 | Control 私有目录 `identity/artifacts/sha256-<hex>.mwrs`，0400、原子 rename 写入、此后不改写，进程每 digest 读入并校验一次；只保留当前 netmap 引用的文件；重启时与 `netmap.json` 一起离线加载并校验 |
| 并发与预算 | 每节点同时最多 2 个下载；当前 netmap 引用合计超过 64 MiB 时不下载 |

新 netmap 立即生效，不等待 artifact。Engine 的 `Desired` 由 netmap 与已加载 artifact 共同算出：全部引用都已加载时即签名内容本身；有缺失时，同一 `LocalCompiler` 在投影上重算本节点出口方案——命名缺失集合的规则是 `UnavailableDependency`，first-match 在该规则停止，结果为全范围 IPv4 捕获、IPv6 sink、无 direct／unmanaged 例外；resolver socket 随之一律拒绝，绑定签名方案的 Mesh DNS 服务不可用。artifact 到达后 Control 以同一 netmap、更多 artifact 再发布，Engine 视为新的期望状态重新 apply（组件以 `Desired::same` 比较）；中继、出口在自己的投影上重算源端 selection 时同样需要该 artifact，缺失即拒绝。

### 规模与预算

每份 netmap 只检查 `MAX_NETMAP_BYTES`（8 MiB）安全上界，超限 `NetmapError::Limit`、整体拒绝；不设节点对或 grant 上限。每节点对的路径按[routing 的搜索](/v2/configuration-reference/routing#可行性循环与失败)列出（最短长度全部列出、满 8 条的长度结束、至多 2,048 条）、每条 ≤ 8 跳、每次搜索 16,384 工作项。为使投影与全量结果在预算边界也一致，路径搜索只为 topology 允许的候选边计预算，routing 段解析每段计 1 次。

实测（M3、release、单线程，私有证据 `.meshway/netmap/n1a/scale.md`），全互访、单一连通分量：

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
| Node 消费者 | 唯一 Control：校验、保存、发布 → Engine 向各组件应用 `Desired` | Control 的最后已知事实，以不可变 `KnownPeers` 快照发布：端点→Manager，出口就绪→出口选择；观察任务由 daemon 控制周期领取 |
| 是否授权 | 是（唯一来源） | 否；只影响连接获取方式与在已授权集合内的选择 |

**判定一项输入属于哪条线**：若缺少它就必须拒绝某个包或某个握手，它属于 netmap；若缺少它只会让某次获取失败或换一个已授权候选，它是事实。端点、STUN、出口就绪都只是选择输入：错误或过期的值最多导致连不上，不能放宽准入。

### 传输：单一长轮询 {#sync-transport}

Node↔Server 只有 `POST /agent-api/v1/sync`（类型在 `crates/meshway-protocol/src/netmap.rs`）：

```text
SyncRequest  { netmap: Option<NetmapRef>, state: Option<StateCursor { epoch, sequence }>,
               associations: Option<Digest> }
SyncResponse { lease: NetmapLease,
               netmap: SyncNetmap::{ full { netmap } | delta { delta } | unchanged },
               peerFacts: Option<PeerFactsDelta { base, cursor, netmapSerial, updated: NodeId → PeerFacts, removed }>,
               associations: Option<DnsAssociationsUpdate { overlay, signature, rotationSignature }> }
```

Server 挂起请求直至 netmap、可见事实或本节点的 DNS 关联 overlay 变化，或 ≤ 60 s 超时；超时回 `unchanged`＋`peerFacts: null`，即内建兜底。

挂起不越过节点持有的租约：一次 hold 的上限为 `min(名义 hold, 节点持有的租约剩余 − 15 s)`，15 s 即节点传输退避上限（`BACKOFF_MAX`），使回答即使失败节点也还有一次重试。“节点持有的租约”是本 Server 进程对该节点最近一次、未被同节点更新请求取代的回答所带租约；被取代（节点已改发新请求）的回答不计入。Server 进程尚未回答过该节点（首次请求、Server 重启后）时立即回答。余量不足时也立即回答，回答本身即续租。唯一 owner 是 Server 的 sync hold（`netmap::Hub`），Node 不另做截断。限制：回答已写出但在传输中丢失、且节点在该请求结束后才重试时，Server 认为的租约可能新于节点实际持有的，由 15 s 余量与节点退避兜底。节点收到即发下一个请求，手里的 `NetmapRef`／`StateCursor` 就是游标，没有事件流与独立重放。`SyncResponse::check(request)` 校验租约指向最终内容、delta 的 base 等于请求、事实增量接续请求游标且属于同一 netmap serial。语义上仍是两条线：netmap 部分授权，`peerFacts` 部分只供选择。

### Server 侧

- State 报告（本机端点、出口能力）只写 State 投影并唤醒挂起的 `sync`，不重新投影；端点不写进 `DialCandidate`。
- 实例 acquire／renew／release 只修改实例；acquire 时库存能力若变化（构建升级），作为库存输入触发该节点及看见它的节点重新投影，这是唯一由 acquire 引起的 netmap 变化。
- 节点 State 的可见范围为本节点投影的 `nodes`；端点细节只披露给 topology 相邻（`dialers` 指向或可拨入本节点）的可见节点。范围只随 serial 变化。
- UDP 穿透 prepare／accept 的授权：双方当前投影互相可见、topology 允许且有 Native UDP 承载；候选 CAS、双边接受、nonce／epoch 与短期限见[连接获取](/v2/node/connectivity)。

### Node 侧

- Control 是唯一联系 Server 的 owner，循环发 `sync`：`netmap` 部分校验、保存、发布 → Engine `reconfig` → 各组件 `apply`；`peerFacts` 部分解码为 `PeerFacts { nodeId, listeners: key → { phase, network, localEndpoints, stunCandidates }, exit: { roleGeneration, status, installedFamilies }, observedAt, expiresAt }`，并入最后已知事实后以 `KnownPeers` 发布，不进入期望内容（[Netmap 发布](/v2/node/control#netmap-发布)）。
- Manager 只读最新 `KnownPeers` 合并候选，不重建 epoch、不换 scope；已有健康通道不因候选变化断开。
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

首包不发 HTTP、不等待 Server。首包队列与握手重发保留现有上限；等待期间不提交流。终点与出口收到握手时以同一函数在自己的投影上为 (origin, dst) 求决定，中继用 `admit_transit`。

### 准入函数

位于 `meshway_policy::compile::netmap` 与 `meshway_policy::admission::netmap`：

| 函数 | 检查 |
| --- | --- |
| `compile_local(netmap, source, dst, (protocol, port)) -> LocalDecision::{Permitted(Decision), Direct, Unmanaged, Denied(reason)}`；`LocalCompiler::new(netmap).decide(…)` 复用一次投影转换 | 与 Server 路径检测同一 `evaluate`：Mesh 分类、地址池拒绝、egress first-match、Access、routing first-match＋递归 via、topology 承载、Internet 边界、出口能力（不含就绪）与 selection |
| `transit(netmap, origin, target)`／`LocalCompiler::transit` | Mesh：该对有任一 Access 服务；出口：源端分割中存在 `(nodes, via)` 动作且有覆盖它的外部 Access；返回全部路径 |
| `admit_handshake(leased, decision, path, origin, previous, reply, now)` | 租约绑定且未到期、本节点 enabled、`origin == decision.source`、path 在 `decision.paths`、本节点是端点、位置与 previous 匹配、路径节点密钥、出口终点 `exitEnabled` 与本地 `exitRole` |
| `admit_forward(…, traffic, now)` | 上项＋源地址属于 origin、目的属于 Mesh target 或等于出口决定的目的且非 Mesh 地址／池、`services.contains` |
| `admit_reply(…, forward, reply, now)` | 上项＋`forward` 来自调用者自己的已建立流，reply 是精确反向 tuple |
| `admit_transit(leased, compiler, origin, target, path, previous, reply, now)` | 租约、本节点是 path 的中间节点、path 在 `transit(origin, target)` 内、位置与密钥 |
| `admit_observation(…, service, now)` | handshake＋Mesh 决定含该服务＋路径全部 observable |
| `admit_adjacency(compiler, netmap, peer, carrier)`／`admit_path_carrier` | topology 与双方声明允许该承载；本节点拨号时 `dialers` 含该候选 |

`now` 只用于租约；决定本身没有截止。DNS 对应 `dns_route::{netmap_resolver, netmap_dns_service}`。

### 流表与 circuit

- 流键为 `{origin, target, node, role, path, tuple}`。
- circuit 上下文为 `(origin, target, purpose, path)`；握手 transcript 绑定 **path 授权摘要**（origin／target、完整路径、路径节点身份／地址／密钥、本跳承载候选、出口 selection），不含服务集合。Access 增删端口不迫使重新握手；每包准入照常按当前 netmap 检查服务。
- circuit wire 为 v6（`MWCIR\0\0\x06`）；稳态 Data 帧 86 B 开销。

### `SecureNode::apply`

`SecureNode::apply(&Desired) -> Applied` 是数据面唯一的期望入口：

| 情况 | 行为 |
| --- | --- |
| 同 `NetmapRef`（只续租） | 不调用组件；只有 Engine 的计时延长；不扫描、不换代、不取消输出 |
| 新 serial | 对每个 circuit 在新投影上重算决定并比较 path 授权摘要：相同则保留 Noise 状态与流，否则退役；对每条流检查其精确服务仍在新决定的 `services` 内，否则以独立流 guard 退役；出口流同时发 `Retire` |
| 本节点身份／地址／密钥变化 | 全部退役（硬屏障） |

不存在“直接后继”判定：Control 只接受 serial 更大的内容，比较的是新旧完整内容本身。跳过中间 serial 不需要中断连续性——若中间曾撤销后又恢复，新内容与旧内容等价，两端在各自观测到的每个时点都持有授权；永久吊销的节点不会以相同身份恢复。

### 邻接按需建立

netmap 覆盖全部可达 peer，若“收到候选即建连”会变成全连接，因此邻接由本地需要驱动：

- dataplane 在首包选路或中继收到经过本节点的握手时，向 Manager 登记 `AdjacencyInterest(peer)`；Manager 只对 `admit_adjacency` 允许的可见 peer 获取连接。
- 无 circuit 且无 interest 的邻接空闲 120 秒后关闭；`keepalive` 边与只有一方能拨出的边常驻，定义见 [Topology · 邻接的建立与常驻](/v2/configuration-reference/topology#邻接的建立与常驻)。
- 协调 UDP 穿透由任一有需求（或常驻）的一方以自己的拨号承载发起；同一 peer 对／地址族 Server 只接纳一个活动尝试，另一方读到 offer 即接受，不需要本机 interest。
- STUN 刷新只对存在需求（interest 或常驻）且有 UDP 获准承载（任一方向）或健康 Native UDP 通道的 socket 进行，预算（每 60 秒 30＋30 次）保持。

## 租约与期望状态

Control 只验证、保存并发布签名 netmap（内容＋租约），不签发执行快照；`meshway-engine::Engine` 是租约的唯一计时者，并把同一 `Desired` 应用到各组件。组件不持有租约、代际、继承物或可转让的输出权限；同内容续租只延长 Engine 自己的计时，到期由空期望 fail closed。行为定义见 [Engine](/v2/node/runtime#engine)。

### lease 语义

| lease | 语义 |
| --- | --- |
| session（300 s） | 只证明可调用 HTTP；过期后后台重新认证，不影响数据面 |
| instance（120 s） | 唯一在线进程资格：State 报告、安装回执、STUN 协调、观察结果上报都要求当前 instance；Server 侧 instance 过期只使其 State 失效 |
| netmap 租约（24 h） | 数据面唯一截止；Node 在剩余 < 23 h 时续期（约每小时一次），每次成功查询都可续 |
| 观察订阅（30 s） | 保留：限制消费者生命周期，不是执行授权 |
| UDP offer（≤ 10 s） | 保留 |

instance 重新取得（同一进程因 Server 停机过期）不重装：Control 以新 instance 重新提交 saved→applied 回执（新 attempt），引用同一已安装 `NetmapRef`。真实进程替换仍产生新 instance，旧 attempt 不能覆盖新结果。

### 连续性交接何时还发生

| 事件 | 交接 |
| --- | --- |
| 业务首包、DNS 查询、ping、Console 观察 | 不发生；无任何执行更新 |
| State 报告、端点／STUN／出口就绪变化 | 不发生 |
| instance 续租／重取 | 不发生 |
| netmap 同内容续租 | 不是交接；只延长 Engine 的计时 |
| 配置 revision、成员、密钥、库存、方案选择导致的新 serial | **发生**：`SecureNode::apply` 按 (origin, target)／path 比较；Manager／Host／DNS 只对实际变化的资源做差量 |
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

- 订阅 30 s 期限、10 s 续租、任务共享、样本窗口与 `observation.*` 事件见[路径诊断](/v2/node/diagnostics)。续订只改任务截止，不改变 netmap。
- 任务执行权限 = source 在当前投影上的决定；任务截止只限制持续多久。netmap serial 变化导致决定或路径消失时，Node 结束任务并上报 `failed`；路径变化开启新分段。

### 首包与准备预算

准备阶段只有本地部分，60 s 为上限：

```text
warm（邻接已健康）：T ≈ RTT_path（握手）＋ RTT_path（首个 probe）＋ 本地处理
cold（静态端点）：  T ≈ Σ每跳串行邻接获取（TCP 1 RTT；TLS 再 1–2 RTT；UDP 1 RTT 认证）＋ 2·RTT_path
```

- CLI ping 的首个结果不含任何 HTTP。
- lab 门槛（容器间 RTT 亚毫秒）：warm 首回复 ≤ 100 ms，cold Native TCP 三节点 ≤ 300 ms；跨地域部署按上式核对，不另设固定秒数。60 s setup 上限保留作失败边界。
- 协调 UDP 穿透需要 Server，冷启动时间另计；Server 停机时只能使用静态端点或已有通道。

## 撤销与到期

### 传播路径与最坏延迟

| 场景 | 路径 | 最坏延迟 |
| --- | --- | --- |
| Server 可用、`sync` 挂起中 | 提交事务投影 → 挂起请求立即返回 delta → Node 校验、保存 → `update` | 秒级；lab 门槛 ≤ 5 s |
| Server 可用、`sync` 失败重试中 | 下一次 `sync` 即取到 | ≤ 重试间隔＋一次请求 |
| Server 不可用或节点分区 | 无法推送 | ≤ 该节点剩余租约（≤ 24 h） |
| 成员撤销 | 被撤销节点下一次 `sync` 收到撤回（已解码 403）即本地失效；接收端随新 netmap 拒绝 | 同上；不依赖被撤销节点配合 |
| 密钥吊销／节点删除 | 所有引用该节点的 netmap 重编，peer 删除其密钥 | 同第一行；接收端先收到即先拒绝 |

撤销按接收端执行（[Access](/v2/configuration-reference/access#enforcement)）：被撤销的源即使不配合，任何已收到新 netmap 的终点、中继和出口都会拒绝。只有当两端都与 Server 分区时，旧授权才可能存续到租约到期。这与 Tailscale 的取舍相同。

### 到期行为：终止全部

netmap 租约到期时 **已有流与新流一并终止**：全部 circuit 退役、排队输出取消、出口投影退役并精确清理 CT；源端 capture 保持负向边界，DNS 保持负向安装。

理由：租约到期是分区状态下撤销的唯一上界。若只拒新流，一条被撤销的长连接（TCP、隧道、持续 UDP）在两端都离线时可以无限存续，撤销失去最坏界限。到期发生在 Server 已至少 1 小时（续期周期）不可达之后，主动断开是可预期的。恢复联系后取得新租约即可重新握手。

### revision 变化时的已有流

由 `SecureNode::apply` 在新投影上重算判定：决定仍为 Permitted、流的精确服务仍在 `services`、所用完整路径仍在 `paths`、路径上全部节点身份／地址／密钥不变、所用承载候选仍在、出口 `selection` 不变——全部满足则保留；任一不满足则结束该流（必要时只结束该 circuit 上的部分流）。方案切换总是结束旧出口连接。

## 出口与 DNS

### 出口

| 项 | 归属 | 说明 |
| --- | --- | --- |
| 出口决定（源→出口的服务、路径、selection） | netmap 投影上本地编译 | Access ∩ 允许方案 ∩ `exitEnabled` ∩ 能力 ∩ 路径；不要求就绪 |
| `exitRole`（`ExitRole`） | netmap（出口节点） | 只要 `exitEnabled` 且声明 `feature:exit-ipv4` 就下发；不绑定 instance。职责本身不转发包 |
| `egress`（规则引用与 fallback；分割与 `EgressCapturePlan` 由节点计算） | netmap（源节点） | 按 revision＋选择版本 |
| `ExitCapability`（probing／ready／unavailable、families） | 动态事实 | 出口节点 State 上报，经 `sync` 的 `peerFacts` 送给看见该出口的节点 |

源端在其出口分割给出的有序主备出口中，选第一个能力事实新鲜且 ready 的出口建流；都不 ready 时丢弃并计数，保留负向捕获，**永不 direct**。出口端准入只看自己投影上的决定与本地已安装 role，不看自己的 State 上报。能力事实错误只会让源选错出口而被拒，不会放宽准入。

### DNS

- 配置 resolver 与系统 resolver 的本地 socket 许可：源端用其出口分割求 resolver 端点的动作——direct／unmanaged 允许本机 socket，exit 动作必须经出口 DNS，unmanaged 范围以外的 Mesh 地址拒绝。UDP 与 TCP 分别判定。
- `DnsService` 在 netmap 中：客户端 `DnsPlan::ViaExit`、出口 `DnsPlan::ServeExitResolver` 列表都按 revision 编译，没有 instance 与截止字段，截止来自租约；出口就绪与出口 resolver 可用性（`NodeState.exitDns`）都是事实。行为见[出口 resolver](/v2/node/dataplane#exit-resolver)。
- 可信域名关联是唯一的按需签发：出口 netmap 的 `ServeExitResolver` service 即 Server 预签的常驻任务；出口在回答名称规则决定的查询后以 `POST /agent-api/v1/dns-associations` 完成一次，Server 校验后为源与出口各签一份 `DnsAssociationOverlay { netmap: NetmapRef（接收者）, associations }`（每条 `expiresAt = resolvedAt + clamp(TTL, 30 s, 1 h)`），经同一 `sync` 交付（请求以 `associations` 指明持有的 digest，未变回答 `null`），随期望内容应用；不改变 netmap serial、不持久化，任一方 netmap 变化后 Server 先重验再交付。往返、等待、界限与失效见 [Exit 设计 · 可信域名关联](/v2/node/exit-design#trusted-dns-associations)。

## 已决产品问题

| 问题 | 决定 | 影响 |
| --- | --- | --- |
| 进程重启后凭缓存的未到期 netmap 离线运行 | **允许**：netmap 由 Server 签名并带有效期，重启后验签通过且未到期即载入；与 Tailscale 一致 | Control record 保存签名 netmap；AGENTS"缓存不复活授权"改为"签名有效期内的 netmap 是授权本身，不是缓存"；被撤销节点在分区中最多存续到到期，与运行中分区同一上界 |
| 有效期可配置 | **Server 参数 `--netmap-validity`**，默认 24 h、最小 60 s（参数单位为秒） | 下限是防止运维误设过短导致 Server 抖动即全网 fail closed 的可用性保护，不是安全边界；更短的 validity 只会让撤销更快生效。`netmap/expiry` 用 60 s 验收 |
| 出口就绪 | **纯动态事实，不编译**：netmap 的出口 grant 是管理员配置的有序主备；哪个出口 ready 走 State 事实线，源端本地选 | 出口能力不是投影输入，`ExitCapability` 为事实 |
| 邻接按需 | **按需建、空闲 120 s 关**；topology 边可标 `keepalive: true` 常驻（必经中继、出口） | topology.md 更新；cold 首包多一次建邻接，lab 门槛 cold ≤ 300 ms |
| 规模上限 | **不设协议级拒绝上限**，参照 Tailscale：首次完整 netmap，之后差量（peer 增删、轻字段 patch）＋按 netmap peers／edges 的可见性裁剪；仅保留 8 MiB 单份安全上界防异常 | 分发通道增加 `netmap.delta`；超 8 MiB 视为缺陷而非容量规划 |
| Node↔Server 同步传输 | **单一长轮询端点 `POST /agent-api/v1/sync`**（Node 不消费 SSE）：请求携带节点持有的 `netmap: NetmapRef` 与 `state: cursor`，Server 挂起至有更新或 ≤60 s 超时，响应 `{ lease, netmap: full | delta | none, peerFacts: delta | none }`；节点收到即发下一个请求。无事件流、无独立 cursor 重放，节点手里的 serial／cursor 即是游标；hold 超时回 `none` 即内建兜底，不再另设定时刷新。与 Tailscale MapRequest long-poll 同形。Console 的用户 SSE 保留 | Node 不消费 `StreamEvent`；类型为 `SyncRequest`／`SyncResponse` |
| netmap 的形状 | **配置的投影，不是编译结果**：`ProjectedConfiguration` 是 `NetworkConfiguration` 的单调子集（可见 peers、涉及本节点的 Access／topology／routing 规则、双方 listeners／dialers、egress capture、exitRole、dns）；节点用同一 `meshway-policy` 在投影上**懒编译**本对 (source, target)，按 (serial, source, target) 缓存，禁止预展开全部节点对；中继用 `admit_transit`。Server 为 Console 做路径检查时在全量配置上用同一 crate | 不物化节点对 grant；没有中继 hub O(N²)；每份 netmap O(可达 peer＋涉及规则) |
| NAT–NAT 按需边的打洞信令 | **Server 只传信令，不转发流量**：一方登记打洞意图后，Server 在对端的 `sync` 回复里带"待处理的穿透请求"，对端据此做 STUN 观测并参与协调打洞，不要求本地 interest；对应 Tailscale 经 DERP 送 disco ping 的角色，但我们没有全员兜底中继，数据面仍只经策略定义的路径 | **已实现**：`SyncResponse.traversalRequests`（请求回显已持有 ID），prepare 提交后唤醒目标的 hold；目标 connectivity 登记 interest 后查询并接受 offer，定义见[穿透信令](/v2/node/connectivity#traversal-signalling) |
| Netmap 签发钥轮换 | **背书交接，离线运维命令触发**：`meshway-server rotate-netmap-key [--window 48h]` 生成下一代钥 K′ 并立即作为当前钥（新接入直接钉 K′）；窗口内回复仍由 K 签，另带 `keyRotation`（K 对 K′ 的背书与 K′ 对同一租约的签名），节点验证背书后原子改钉 K′；窗口后只由 K′ 签。不跳代、不自动轮换、不支持多 Server 实例 | 错过窗口的节点不撤销已持有租约，只拒新回复（`ServerKeyMismatch`，sync 退避重试），库存视图 `serverKey.status=keyMismatch`；恢复须重新接入。定义见 [Server](/v2/server#netmap-签发钥轮换) 与 [Control](/v2/node/control#server-签名钥轮换) |
| 持续 STUN 观测 | 每个 Native UDP listener socket **持续**做 STUN（空闲 ~30 s、有流量 ~10 s，带抖动），映射变化才经 State 上报进 peer facts；不再只对有 interest 的 socket 观测。作用：候选常新、NAT 映射保活、收到穿透提示即可立刻打洞；离线时按最后已知保留。STUN 是一次 UDP 往返，与向 Server 查 offer 的 HTTP 请求无关 | **已实现**：按 socket 的固定节拍（~10 s／~30 s，±10%），未变化的确认不上报、临近到期才续期，Server 合并同映射候选；有效 context 期间不发 HTTP。定义见[持续 STUN 观测](/v2/node/connectivity#continuous-stun) |
| 投影实现 | **不物化 grant／路径／边**：`NodeNetmap { self, projection: ProjectedConfiguration, dialers, egress, exitRole, dns }`；可见节点＝topology 连通分量 ∪ Access 相关 ∪ 保留规则的 via；规则按“能匹配可见节点对”原样保留；节点侧 `compile_local`／`transit` 复用同一 `evaluate`，与全量配置逐查询等价 | 1,000 节点全互访每份 1.52 MB、所有节点相同；中继 hub 消失；路径搜索预算只计 topology 允许的边 |

## 限制

- 中继获知所有可能经过它的 (source, target) 节点对；这是 netmap 模型固有的披露范围。
- 投影披露同一连通分量内全部节点的身份、地址、公钥与脱敏承载声明，以及能匹配它们的规则；不披露 listener 私钥、凭据与敏感头值（`NodeSettingsView`）。
- 同一身份的克隆进程（复制私钥）在 Server 层仍只能持有一个 instance，但两者都能使用各自已取得的 netmap 租约直到到期；instance 替换不能撤回已签发租约。
- Server 停机期间无法进行协调 UDP 穿透；只有静态端点与已有通道可用。
- 端到端保护、源授权、有序必经点、原路径精确回复、重放保护、State 不自授权、失败不静默放行等硬约束不变，由同一 owner 执行。
