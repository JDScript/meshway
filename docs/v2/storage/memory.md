# Controller 内存字段

**V2 逻辑对象提案。** 本页列内存字段；持久恢复依据在[数据库字段](/v2/storage/database)。`T?` 为可空，所有容器／队列有预算，重启后重建。

## ConfigurationCache

键：`(networkId, revision)`。当前版本指针来自数据库。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 网络 |
| `revision` | Revision | 配置版本 |
| `configuration` | Arc&lt;NetworkConfiguration&gt; | 解密／校验后的不可变完整值 |
| `bindings` | Arc&lt;ConfigurationBindings&gt; | 稳定引用／默认值依据 |
| `digest` | Digest | 加载后计算的内容摘要 |
| `compiled` | Map&lt;CompilationKey, Arc&lt;CompiledConfiguration&gt;&gt; | 本版本有界编译缓存 |

只有缓存，无独立编辑入口；User／Tag 身份从 configuration.nodes[id].identity 读取。CompilationKey 字段为 `dependencyDigest: Digest`、`compilerVersion: string`；动态依赖变化必须换 key，即使源 revision 没变。编译缓存嵌入版本缓存，容量有界，可随版本回收。

CompiledConfiguration 是 `meshway-configuration` 的内部类型，不序列化成给 Node 自授许可的任意图。

## NetworkState

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 网络 |
| `stateEpoch` | StateEpoch | Controller 重建此状态集合时更新 |
| `sequence` | u64 | 本 epoch 内变化顺序 |
| `configurationRevision` | Revision | 当前解释视图的配置版本 |
| `nodes` | Map&lt;NodeId, StoredNodeState&gt; | 每个 Node 的当前接受报告 |

每网络只保留一份，查询按权限生成视图。

### StoredNodeState

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `instanceId` | InstanceId? | 当前接受的报告实例 |
| `reportSequence` | u64? | 当前报告序号；未收到为 null |
| `receivedAt` | Timestamp? | 当前报告的接受时间 |
| `reportExpiresAt` | Timestamp? | 当前报告联系期限 |
| `lastAuthenticatedAt` | Timestamp? | 最近真实认证联系，与报告新鲜度分开 |
| `report` | NodeState? | 最新已接受报告；未报告不是空对象 |

freshness 按当前时间／配置／实例从以上输入计算：current、awaiting_report、expired、superseded、config_mismatch、revoked。不能靠查询或重复报告延长原观测。

### NodeState

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `system.hostname` | string | OS hostname，非规范配置名称 |
| `system.os` | string | 实际 OS |
| `system.arch` | string | 实际架构 |
| `system.agentVersion` | string | 构建版本 |
| `capabilities` | Map&lt;CapabilityKey, Capability&gt; | 当前实际构建能力，value 为已知类型 |
| `configuration.saved` | ConfigurationRef? | 本实例已持久保存配置 |
| `configuration.applied` | ConfigurationRef? | 当前实际执行配置 |
| `listeners` | Map&lt;ListenerKey, ListenerState&gt; | 监听运行状态 |
| `sockets` | Map&lt;SocketId, SocketState&gt; | 实际 socket 与候选 |
| `links` | Map&lt;NodeId, LinkState[]&gt; | 自身与 peer 的认证通道 |
| `paths` | Map&lt;PathId, PathState&gt; | 自身参与路径的观测 |
| `selection.appliedProfile` | ProfileKey? | 当前实际方案 |
| `probes` | ProbeObservation[] | 少量最新真实测量 |
| `problems` | Problem[] | 有来源的类型化问题 |

`ConfigurationRef` 字段：`revision: Revision`、`executionPublication: PublicationId`、`digest: Digest`。Capability 的 roles／streams 等协议能力字段见[能力参考](/v2/state-reference/capabilities)。各实例报告只能由该 Node 身份更新。

## ListenerState

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `phase` | preparing \| listening \| stopped \| failed | 本机资源状态 |
| `socketIds` | SocketId[] | 关联实际 sockets |
| `loadedCertificateSha256` | Digest? | 实际加载证书指纹；非 TLS 为 null |
| `problem` | Problem? | 失败原因 |
| `configuration` | ConfigurationRef? | null 表示沿用报告 applied；另版 preparing 须显式填写 |

key 来自 nodes[id].listeners，不能作为新的配置源。

## SocketState / EndpointCandidate

| 对象 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Socket | `resource` | ResourceRef | listener／dial／协调资源 |
| Socket | `generation` | u64 | 该资源代际 |
| Socket | `network` | udp \| tcp | 外层 socket 协议 |
| Socket | `boundAddress` | SocketAddress | 内核实际 bind 地址 |
| Socket | `endpoints` | EndpointCandidate[] | 带来源的候选 |
| Endpoint | `address` | EndpointAddress | 配置域名或实际 IP＋端口 |
| Endpoint | `source` | configured \| interface \| stun | 来源判别 |
| Endpoint | `validation` | declared \| observed | 此来源的观测程度，不是 peer 认证 |
| Endpoint | `observedAt` | Timestamp | 观测／配置资源关联核对时间 |
| Endpoint | `expiresAt` | Timestamp | 本次观测期限 |
| configured 分支 | `configPath` | JsonPointer | 当前配置引用 |
| interface 分支 | `interface` | string | 本机接口标识 |
| stun 分支 | `observer` | SocketAddress | 该 socket 使用的 STUN 观测端 |

各来源只携带自己的分支字段。`ResourceRef` 字段：`kind: listener | dial | coordination`、`key: string`。peer 认证记录在实际 Link／probe 中；UDP 观测不证明 TCP 可达。

## LinkState

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `linkId` | LinkId | 实际握手绑定的公开通道标识 |
| `transport` | TransportKey | 真实承载 |
| `initiatorNodeId` | NodeId | 实际发起端，不代表业务方向 |
| `socketId` | SocketId | 本机 socket |
| `phase` | preparing \| authenticated \| closed \| failed | 本端实际状态 |
| `businessDependencies` | u32 | 当前业务依赖数，非包速率 |
| `observationIds` | ObservationId[] | 临时诊断依赖 |
| `observedAt` | Timestamp | 本次状态观测时间 |
| `expiresAt` | Timestamp | 报告有效期 |

authenticated 且无依赖可以是 idle。双侧匹配由 Controller 派生，不能由 A 的报告生成 B 的事实。

## PathState

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `pathId` | PathId | 本次执行／观测路径标识 |
| `nodes` | NodeId[] | 完整有序路径 |
| `publicationId` | PublicationId | 对应许可 |
| `reporterRole` | origin \| relay \| target \| exit | 本报告者角色 |
| `evidence` | selected \| sent \| peerConfirmed \| delivered | 实际可证明阶段 |
| `observedAt` | Timestamp | 原观测时间 |
| `expiresAt` | Timestamp | 期限 |

这是此前路径语义的字段提案。中转不能自行上报目标已明文交付；缺测量证据时不增加 RTT／流量字段假值。

## ProbeObservation / Problem

| 对象 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Probe | `measurementId` | EventId | 与历史去重使用同一来源 ID |
| Probe | `kind` | neighbor \| path \| connect \| controller | 测量类型 |
| Probe | `path` | NodeId[] | 实际被测路径／端点 |
| Probe | `publicationId` | PublicationId | 执行上下文 |
| Probe | `startedAt` | Timestamp | 开始时间 |
| Probe | `completedAt` | Timestamp? | 未完成为 null |
| Probe | `outcome` | pending \| success \| timeout \| failed | 真实结果 |
| Probe | `durationUs` | u64? | 有计时依据时填写；超时不填零 |
| Problem | `code` | ProblemCode | 已知失败类型 |
| Problem | `resource` | ResourceRef? | 涉及资源 |
| Problem | `observedAt` | Timestamp | 发生时间 |
| Problem | `detail` | ProblemDetail | 按 code 对应的封闭、脱敏字段 |

精确 ProblemCode 分支由实际功能契约补齐，不保留任意 JSON 逃生字段。

## DerivedNodeState（查询投影）

以下字段由当前配置、已接受报告、RoutingSelection 和固定 GeoIP 数据集计算。它们是返回视图，不是另一份必须同步更新的 StoredNodeState 字段；内部性能缓存可重建，失效依据必须覆盖所有输入。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `location` | GeoObservation? | 合格底层公网 IP 的地区推导 |
| `requestedProfile` | ProfileKey | 持久使用选择经当前 identity／allowed／locked 校验后的目标，或当前配置默认 |
| `selectedAutoProfile` | ProfileKey? | 根据当前有效观测选择；无观测用配置 fallback |
| `linkEvidence` | Map&lt;LinkId, LinkEvidence&gt; | 单侧／双侧／冲突／过期聚合 |

### GeoObservation / LinkEvidence

| 对象 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Geo | `address` | IpAddress | 合格 socket 观测的底层公网 IP |
| Geo | `sourceSocketId` | SocketId | 本 Node 观测来源 |
| Geo | `sourceGeneration` | u64 | socket 代际；实例沿用所属 StoredNodeState |
| Geo | `datasetId` | ArtifactId | 地区数据版本 |
| Geo | `country` | string? | 推导国家／地区；无法确定为 null |
| Geo | `observedAt` | Timestamp | 输入观测时间 |
| Geo | `expiresAt` | Timestamp | 推导结果有效期 |
| LinkEvidence | `phase` | oneSided \| matched \| conflicting \| expired | 聚合结果 |
| LinkEvidence | `reports` | ReportRef[] | 实际来源，最多对应两端 |

`ReportRef` 字段：`nodeId: NodeId`、`instanceId: InstanceId`、`reportSequence: u64`。没有收到另一端报告时不构造第二条引用。

## StreamSubscription / EventBuffer

| 对象 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Subscription | `id` | SubscriptionId | 本进程订阅标识 |
| Subscription | `principal` | PrincipalRef | 已认证用户会话或 Node 绑定 |
| Subscription | `networkId` | NetworkId | 作用域 |
| Subscription | `filter` | ViewFilter | 固定枚举和资源子集 |
| Subscription | `authorizationDigest` | Digest | 当前裁剪上下文，权限变化重新授权 |
| Subscription | `cursor` | ViewCursor | 已发送序列位置，不是客户端 applied ACK |
| Subscription | `expiresAt` | Timestamp | 身份／订阅本身期限上界 |
| Buffer | `epoch` | StateEpoch | 缓冲所属世代 |
| Buffer | `events` | Deque&lt;StreamEvent&gt; | 有界事件与游标 |
| Buffer | `oldestCursor` | ViewCursor? | 可补发下界 |

PrincipalRef 为 user／node 判别联合：user 分支保存 userId／sessionHash，node 分支保存 nodeId／keyId／instanceId。ViewFilter 字段为 `nodes: NodeId[]?`、`include: StateField[]`、`observations: ObservationId[]`；null nodes 表示服务端计算的可见集合，空数组明确不选节点，不能通过 include 扩权。StreamEvent 类型见 [SSE 事件](/v2/api/events#事件结构)。

按当前权限投影输出，不把有秘密的完整 Config 广播。客户端另存自己的最后成功应用 cursor；断档／范围变化 reset。

## ObservationTask

| 对象 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Task | `id` | ObservationId | 测量任务 |
| Task | `networkId` | NetworkId | 网络 |
| Task | `sourceNodeId` | NodeId | 发起者 |
| Task | `path` | NodeId[] | 指定完整路径 |
| Task | `publicationId` | PublicationId | 当前获准发布 |
| Task | `instanceId` | InstanceId? | 实际执行实例；尚未调度到当前实例为 null |
| Task | `metrics` | MetricKind[] | rtt／traffic／probeLoss |
| Task | `intervalMs` | u32 | 实际获准周期 |
| Task | `phase` | preparing \| running \| completed \| failed \| expired | 实际阶段 |
| Task | `expiresAt` | Timestamp | 下发节点的短租约 |
| Task | `latest` | ObservationSample? | 最新真实样本 |
| Task | `subscribers` | Map&lt;ObservationSubscriptionId, ObservationSubscriber&gt; | 任务内部的独立使用需求 |
| Subscriber | `principal` | PrincipalRef | 每个订阅者独立鉴权 |
| Subscriber | `expiresAt` | Timestamp | 该订阅者续租期限 |

ObservationSubscriber 是上述 map 的嵌套值，任务关联由外层给出，不另维护反向关系表。每个订阅者的权限／租约仍独立；取消一项不取消别人的有效需求。任务不因 SSE 连接存在自动续期，最后需求离开后释放，崩溃后不恢复旧任务为 running。

### ObservationSample

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `measurementId` | EventId | 来源采样／窗口 ID，重复投递不累加 |
| `windowStart` | Timestamp | 原始统计窗口开始 |
| `windowEnd` | Timestamp | 原始统计窗口结束 |
| `partial` | boolean | 窗口是否仍有未完成结果 |
| `probe` | ProbeObservation? | 当次真实探测，无样本为 null |
| `txBytes` | u64? | 源端业务提交字节；未测为 null |
| `rxBytes` | u64? | 源端验证交付字节；未测为 null |
| `counterGeneration` | u64? | 字节计数器重置／路径分段依据 |
| `sent` | u32? | 实际发出的探测数 |
| `repliedWithinDeadline` | u32? | 时限内最终成功数 |
| `timeout` | u32? | 最终超时数 |
| `pending` | u32? | 尚未到最终结果的数目 |

方案字段建议使用窗口内字节量，由窗口时长推导速率；不能把累计计数当窗口字节。未回应率由 timeout／(repliedWithinDeadline＋timeout) 推导，无分母为 null；准备阶段不记一次发送。指标的精确测量口径见[测量参考](/v2/state-reference/realtime)。

## 其他有界临时对象

| 对象 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Demand | `id` | RequestId | 合法业务需求标识 |
| Demand | `networkId` | NetworkId | 网络 |
| Demand | `nodeId` | NodeId | 请求者，来自认证 |
| Demand | `target` | BusinessTarget | Mesh／外部目的的类型化输入，精确分支随执行 API 定版 |
| Demand | `configurationRevision` | Revision | 求值基准 |
| Demand | `expiresAt` | Timestamp | 有界需求期限 |
| Demand | `result` | DemandResult | preparing、unavailable 或已提交 publication 引用 |
| DnsAssociation | `networkId` | NetworkId | 网络 |
| DnsAssociation | `publicationId` | PublicationId | 授权／解析上下文 |
| DnsAssociation | `sourceNodeId` | NodeId | 关联的业务源 |
| DnsAssociation | `name` | string | 可信查询名称 |
| DnsAssociation | `cnameChain` | string[] | 验证的 CNAME 链 |
| DnsAssociation | `addresses` | IpAddress[] | 真实解析地址集合 |
| DnsAssociation | `expiresAt` | Timestamp | TTL 有效期 |
| ResourceJob | `id` | JobId | 下载／转换任务 |
| ResourceJob | `target` | DatasetTarget | 要更新的规则集或 GeoIP 来源 |
| ResourceJob | `phase` | queued \| running \| failed \| ready | 运行阶段 |
| ResourceJob | `problem` | Problem? | 真实失败 |
| AuthChallenge（若采用） | `id` | ChallengeId | 唯一挑战 |
| AuthChallenge（若采用） | `publicKey` | PublicKey | 证明对象 |
| AuthChallenge（若采用） | `contextDigest` | Digest | 被证明的认证上下文 |
| AuthChallenge（若采用） | `expiresAt` | Timestamp | 有效期 |
| AuthChallenge（若采用） | `consumed` | boolean | 一次性消费 |
| NodeControlSession（若采用） | `tokenHash` | Digest | 短期凭据摘要 |
| NodeControlSession（若采用） | `nodeId` | NodeId | 认证节点 |
| NodeControlSession（若采用） | `keyId` | KeyId | 签发依据，后续核对吊销 |
| NodeControlSession（若采用） | `instanceId` | InstanceId | 实例绑定 |
| NodeControlSession（若采用） | `expiresAt` | Timestamp | 短期认证上界 |
| RateBudget | `scope` | BudgetScope | 账户／来源／节点／任务等封闭用途范围 |
| RateBudget | `available` | u64 | 当前可用预算 |
| RateBudget | `updatedAt` | MonotonicInstant | 补充预算依据 |

DatasetTarget 为判别联合：ruleSet 分支携带 networkId／configurationRevision／ruleSetKey，geoip 分支携带部署的 source 标识。Job 完成时核对目标仍有效，不能把旧下载发布到新配置。

这些是内部算法／工作队列对象，不是新的公开 Entity 或自授权 API。AuthChallenge 的实际 nonce／证明编码随认证协议定版；扩展字段须有明确类型与生命周期。

Agent 本地 socket、加密会话、conntrack、TUN I/O 和 DNS 缓存由对应 agent-*／dataplane 库拥有，Controller 只保存本页列出的报告／协调视图。
