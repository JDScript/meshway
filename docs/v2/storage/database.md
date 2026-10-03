# 持久数据库字段

**V2 逻辑记录提案。** 字段逐项列出，表名与物理拆分未定。`T?` 为可空；时间为 UTC，ID／Digest／Revision 是独立强类型。网络内关联使用 networkId 和复合约束。

## User

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | UserId | 主键，稳定身份 |
| `email` | string | 规范化、唯一的登录标识 |
| `passwordHash` | string | Argon2id PHC，私有字段 |
| `credentialVersion` | u64 | 密码／全会话撤销的版本 |
| `createdAt` | Timestamp | 创建时间 |
| `disabledAt` | Timestamp? | 账户停用时间；null 为有效 |

邮箱不是已验证所有权凭证。修改邮箱不改变 ID，当前配置可读引用同步形成新版本。

## UserSession

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `tokenHash` | Digest | 主键，随机 session 原值的摘要 |
| `userId` | UserId | 关联 User |
| `credentialVersion` | u64 | 签发时版本，认证时与 User 核对 |
| `createdAt` | Timestamp | 签发时间 |
| `expiresAt` | Timestamp | 绝对到期时间 |
| `revokedAt` | Timestamp? | 单会话撤销 |

用户退出／停用／密码重置也结束相关 SSE。session 原值只交付客户端，不在此保存。

## Network

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | NetworkId | 主键，隔离作用域 |
| `name` | string | 网络显示名；实体元数据提案 |
| `createdBy` | UserId | 创建者，不替代当前 Owner membership |
| `createdAt` | Timestamp | 创建时间 |
| `deletedAt` | Timestamp? | 删除标记；删除／保留流程仍待定版 |
| `currentRevision` | Revision | 当前完整配置版本指针 |
| `publicationSequence` | u64 | 本网络已分配执行发布的单调上界 |

创建 Network、Owner membership、初始配置同事务。配置指针只由提交事务推进。

## Membership

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | MembershipId | 主键，一次加入关系 |
| `networkId` | NetworkId | 所属网络 |
| `userId` | UserId | 用户 |
| `role` | owner \| admin \| member | 网络内角色 |
| `joinedAt` | Timestamp | 加入时间 |
| `revokedAt` | Timestamp? | 离网／移除时间 |

`(networkId,userId)` 至多一条有效记录；重新加入产生新 id。最后 Owner 保护在事务中执行。

## Node

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | NodeId | 主键，持久接入身份 |
| `networkId` | NetworkId | 不可任意切换的接入网络 |
| `createdBy` | ActorRef | 原始接入来源，仅追溯 |
| `registeredAt` | Timestamp | 注册时间 |
| `credentialsRevokedAt` | Timestamp? | 永久吊销整个接入；所有旧／派生凭据拒绝 |

**没有 owner、subject、tags、enabled、hostname、lastSeen、applied 字段。** 有效身份与启停从 Config 获取；运行观测从 RAM 获取。永久接入吊销不可通过配置、换钥或旧注册重试恢复。

## NodeKey

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | KeyId | 公钥绑定 ID |
| `networkId` | NetworkId | 关联作用域 |
| `nodeId` | NodeId | 关联 Node |
| `purpose` | control \| dataplane | 认证／数据面用途 |
| `algorithm` | KeyAlgorithm | 已支持算法的封闭枚举，随握手定版 |
| `publicKey` | bytes | 公钥，不保存私钥 |
| `generation` | u64 | 同用途密钥代际 |
| `createdAt` | Timestamp | 绑定时间 |
| `retiredAt` | Timestamp? | 正常轮换后停止新用途，旧会话按既定边界退出 |
| `revokedAt` | Timestamp? | 永久吊销该密钥 |

`(nodeId,purpose,generation)` 唯一；公钥网络绑定不可被重新认领。实际允许的轮换窗口由认证／执行合同规定。

## AddressPool

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 所属网络 |
| `family` | ipv4 \| ipv6 | 地址族，与 networkId 构成键 |
| `prefix` | IpPrefix | IPAM 管理网段 |
| `nextCandidate` | IpAddress? | 可重建的分配游标，不能跳过唯一性校验 |

地址池管理范围仍为提案；不会由 Node State 自报改变。

## AddressAllocation

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | AllocationId | 分配记录 ID |
| `networkId` | NetworkId | 所属网络 |
| `nodeId` | NodeId | 地址持有者 |
| `address` | IpAddress | 实际 Mesh 地址，含地址族 |
| `allocatedAt` | Timestamp | 分配时间 |
| `releasedAt` | Timestamp? | 释放时间；具体回收条件另定 |

有效地址在网络内唯一。历史配置引用旧 allocation 不得重新授予已释放／转分配地址。

## ConfigurationVersion

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 与 revision 构成主键 |
| `revision` | Revision | 递增配置版本 |
| `parentRevision` | Revision? | 首版本为 null |
| `sourceJsonc` | Sealed&lt;string&gt; | 编辑原文，含注释／可能的协议秘密 |
| `configuration` | Sealed&lt;NetworkConfiguration&gt; | 规范化完整配置，类型见配置参考 |
| `bindings` | ConfigurationBindings | 本版本固定的引用／默认值依据 |
| `digest` | Digest | 确定内容摘要，不代替权限 |
| `createdBy` | ActorRef | 用户或系统修改来源 |
| `createdAt` | Timestamp | 提交时间 |
| `requestId` | RequestId? | 幂等／事件关联 |
| `rollbackFrom` | Revision? | 回滚来源；本记录仍是新 revision |

NetworkConfiguration 的完整字段见[配置参考](/v2/configuration-reference)；其中 `nodes[id].identity` 与 `enabled` 是 Node 授权身份／业务启停的唯一源。历史原文、规范化值与绑定不可单独覆盖。

### ConfigurationBindings

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `defaultsVersion` | string | 保存时固定的默认值集合版本 |
| `users` | Map&lt;JsonPointer, UserBinding&gt; | 可读引用 → 稳定主体，不按当前邮箱重解析 |
| `resources` | Map&lt;JsonPointer, ResourceBinding&gt; | 配置资源 → 确切材料／数据集版本 |

`UserBinding` 字段：`userId: UserId`、`membershipId: MembershipId`。`ResourceBinding` 字段：`resourceId: ResourceId`、`materialIds: MaterialId[]`、`artifactIds: ArtifactId[]`。这些是该配置版本的解析结果，无独立管理写入口。

历史 identity 回滚涉及 Tag→User／改绑 User 时仍须新的设备登录授权；保存了旧 UserBinding 不等于持有当前转换权限。

## ResourceInstance

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ResourceId | 内部资源代际标识 |
| `networkId` | NetworkId | 所属网络 |
| `nodeId` | NodeId? | listener 关联 Node；网络规则集等为 null |
| `kind` | listener \| ruleSet | 本稿需跟踪生命周期的配置资源 |
| `key` | string | 配置 map 中的 key |
| `createdRevision` | Revision | 首次创建版本 |
| `removedRevision` | Revision? | 已提交删除版本 |

同名重建使用新 id。记录不包含另一份 listener 参数或 tags；资源参数仍只从配置版本读取。

## MaterialVersion

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | MaterialId | 不可变材料版本 ID |
| `networkId` | NetworkId | 权限作用域 |
| `resourceId` | ResourceId | 所属配置资源代际 |
| `generation` | u64 | 该资源材料代际 |
| `value` | Sealed&lt;ProtocolMaterial&gt; | 类型化 UUID／密码／TLS 等材料 |
| `digest` | Digest | 内容摘要，受材料读取权限保护 |
| `createdAt` | Timestamp | 创建时间 |
| `revokedAt` | Timestamp? | 已吊销材料不能通过回滚有效 |

ProtocolMaterial 沿用协议配置的判别类型，不是任意 JSON。管理员显式值与自动生成值同样保护。

## DatasetArtifact

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ArtifactId | 不可变数据版本 |
| `networkId` | NetworkId? | 网络规则集所属网络；实例 GeoIP 可为 null |
| `kind` | ruleSet \| geoip | 数据类型 |
| `source` | string | 来源 URI／名称，不含凭据 |
| `format` | DatasetFormat | 支持的原始格式枚举 |
| `sourceBytes` | bytes | 原始数据 |
| `compiledBytes` | bytes | 已校验规范化数据／查询内容 |
| `compilerVersion` | string | 转换实现版本 |
| `digest` | Digest | 规范化内容摘要 |
| `createdAt` | Timestamp | 接受时间 |

下载错误与进度在 RAM；只有完整校验通过的产物入库。是否保留全部源 bytes 可在存储预算中定版，不用最新 URL 内容冒充历史依赖。

## EnrollmentRequest {#enrollmentrequest}

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | EnrollmentRequestId | 临时请求 ID，不是 bearer |
| `publicKey` | PublicKey | 本次持钥接入绑定 |
| `requestDigest` | Digest | 固定接入元数据／返回方式的摘要 |
| `codeChallenge` | string | S256 PKCE challenge |
| `returnChannel` | EnrollmentReturn | 受限制 callback 或持钥领取 |
| `createdAt` | Timestamp | 创建时间 |
| `expiresAt` | Timestamp | 首次批准／兑换期限 |
| `approval` | EnrollmentApproval? | 网络、用户／接入密钥与本次授权依据 |
| `resultHash` | Digest? | 一次性兑换结果的摘要 |
| `delivery` | Sealed&lt;EnrollmentResult&gt;? | 需要响应丢失后重交付的短期密文 |
| `deniedAt` | Timestamp? | 已拒绝时间 |
| `consumedAt` | Timestamp? | 首次成功消费时间 |
| `nodeId` | NodeId? | 消费绑定的 Node |
| `bindingDigest` | Digest? | 最终持钥／PKCE／原元数据的恢复绑定 |

`EnrollmentReturn`：`{type:callback, uri:string}` 或 `{type:poll}`。`EnrollmentResult`：`code:string`、`expiresAt:Timestamp`。具体传输仍待认证 API 定版。

`EnrollmentApproval` 为判别联合：

| 分支 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| user | `networkId` | NetworkId | 批准的网络 |
| user | `userId` | UserId | 批准用户 |
| user | `membershipId` | MembershipId | 当次有效入网关系 |
| user | `sessionHash` | Digest | 批准会话绑定 |
| user | `credentialVersion` | u64 | 批准时账户版本 |
| user | `approvedAt` | Timestamp | 批准时间 |
| key | `networkId` | NetworkId | 接入密钥所属网络 |
| key | `keyId` | EnrollmentKeyId | 接入密钥记录 |

每个分支均有对应的 `type: user | key`。批准是一次性写配置 identity 的依据，不是 Node 的第二份当前身份。消费字段原子绑定；成功后清理 delivery 密文，保留最小摘要／Node 绑定供幂等恢复。密码、verifier、设备私钥不入库。

## EnrollmentKey

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | EnrollmentKeyId | 接入密钥 ID |
| `networkId` | NetworkId | 不可改绑的网络 |
| `tokenHash` | Digest | 唯一 token 摘要 |
| `tags` | string[] | 该次接入获准写入配置的非空 Tag 集合 |
| `createdBy` | UserId | 签发者 |
| `createdAt` | Timestamp | 创建时间 |
| `expiresAt` | Timestamp | 首次使用期限 |
| `revokedAt` | Timestamp? | 密钥撤销 |
| `consumedRequestId` | EnrollmentRequestId? | 一次性消费绑定 |

消费时复核当前 Tag 授予资格。tags 是接入授权输入，设备注册后以 Configuration.identity 为唯一当前值。

## PasswordReset（如保留一次性恢复）

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ResetId | 恢复记录 ID |
| `tokenHash` | Digest | 一次性凭据摘要 |
| `userId` | UserId | 目标用户 |
| `credentialVersion` | u64 | 有效账户版本 |
| `expiresAt` | Timestamp | 首次消费期限 |
| `revokedAt` | Timestamp? | 撤销时间 |
| `consumedAt` | Timestamp? | 消费时间 |

恢复入口待定，此记录不要求实例初始化 API。

## ExecutionPublication

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | PublicationId | 不可变发布标识 |
| `networkId` | NetworkId | 所属网络 |
| `sequence` | u64 | 本网络单调发布顺序 |
| `configurationRevision` | Revision | 源配置版本 |
| `dependencies` | PublicationDependencies | 参与身份／密钥／材料／规则集的固定依据 |
| `dependencyDigest` | Digest | 编译输入摘要 |
| `compilerVersion` | string | 编译实现版本 |
| `createdAt` | Timestamp | 提交时间 |
| `withdrawnAt` | Timestamp? | 发布撤回时间 |

`PublicationDependencies` 字段：`userBindings: UserBinding[]`、`keyIds: KeyId[]`、`allocationIds: AllocationId[]`、`materialIds: MaterialId[]`、`artifactIds: ArtifactId[]`。实际许可还来自引用的完整配置；持续使用仍检查当前撤销／租约。

## NodeConfigurationArtifact

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 网络 |
| `publicationId` | PublicationId | 所属发布 |
| `nodeId` | NodeId | 目标 Node，与 publicationId 构成键 |
| `digest` | Digest | 节点切片摘要 |
| `configuration` | Sealed&lt;NodeConfiguration&gt; | 完整确定产物和所需材料 |

只保存实际发布／恢复所需切片，不预生成全网节点对；同一发布重发保持同 digest。

## NodeExecution

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 网络 |
| `nodeId` | NodeId | 键 |
| `instanceId` | InstanceId? | 已确认运行实例；无实例为 null |
| `generation` | u64 | 不回退的实例隔离代际 |
| `targetPublicationId` | PublicationId? | 当前目标发布；无许可为 null |
| `saved` | SavedReceipt? | 最后接受的保存回执 |
| `leaseExpiresAt` | Timestamp? | 已授予执行许可期限，不是联系时效 |

`SavedReceipt` 字段：`instanceId: InstanceId`、`publicationId: PublicationId`、`digest: Digest`、`savedAt: Timestamp`。applied／lastSeen 不在本记录；Controller 重启后须 fresh 认证和当前实例确认才能续租／接受运行报告，不能用旧 ACK 恢复业务。

## RoutingSelection

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 网络 |
| `nodeId` | NodeId | 键 |
| `identityDigest` | Digest | 当时配置身份及稳定引用摘要，仅防跨身份复用 |
| `profile` | ProfileKey | 用户请求的方案 |
| `version` | u64 | 使用选择 CAS 版本 |
| `selectedBy` | ActorRef | 选择来源 |
| `selectedAt` | Timestamp | 选择时间 |

当前身份仍从 Config 读取，identityDigest 不赋权；恢复需检查当前 identity、allowed／locked。实际使用方案由 State 报告。

## ObservationEvent

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `rowId` | u64 | 稳定分页上界／游标依据 |
| `networkId` | NetworkId | 网络 |
| `source` | ObservationSource | Node 或 Controller 的明确来源分支 |
| `eventId` | EventId | 源事件唯一 ID |
| `occurredAt` | Timestamp | 事件发生时间 |
| `receivedAt` | Timestamp | Controller 接受时间 |
| `payload` | ObservationPayload | 连接事件或真实测量的封闭联合 |
| `payloadDigest` | Digest | 重试内容一致性 |

`ObservationSource` 为判别联合：node 分支有 `nodeId: NodeId`、`instanceId: InstanceId`、`publicationId: PublicationId`；controller 分支有 `epoch: StateEpoch`，不伪造 Node 报告。双方都有对应 `type`。

Node 事件按 `(networkId,source.nodeId,eventId)` 唯一；Controller 事件按 `(networkId,source.epoch,eventId)` 唯一。payload 字段沿用[真实事件](/v2/state-reference/observations)与[测量](/v2/state-reference/realtime)，不存任意 metadata 或重复心跳制造的样本。

## HistoryAdmission / HistoryCoverage

| 记录 | 字段 | 类型 | 含义 |
| --- | --- | --- | --- |
| Admission | `networkId` | NetworkId | 网络 |
| Admission | `sourceNodeId` | NodeId | 来源 |
| Admission | `eventId` | EventId | 去重键 |
| Admission | `payloadDigest` | Digest | 已接受内容摘要 |
| Admission | `expiresAt` | Timestamp | 重试接纳窗口结束 |
| Coverage | `networkId` | NetworkId | 网络 |
| Coverage | `sourceNodeId` | NodeId | 来源 |
| Coverage | `before` | Timestamp | 此时刻前可能已裁剪 |
| Coverage | `reason` | retention \| capacity | 记录覆盖限制 |

事件已裁剪后仍在接纳窗口内的重试不能重新入库增加样本。实际采集缺口仍由事件／丢弃计数报告。

## OperationReceipt

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `actor` | ActorRef | 幂等请求主体 |
| `operation` | OperationKind | 有明确恢复需要的操作枚举 |
| `requestId` | RequestId | 与 actor／operation 构成键 |
| `requestDigest` | Digest | 原始请求绑定 |
| `result` | OperationResult | 已提交结果的类型化资源引用 |
| `createdAt` | Timestamp | 提交时间 |
| `expiresAt` | Timestamp? | 可安全结束幂等恢复的期限 |

本稿 OperationKind 先包含 network.create；对应 OperationResult 为 `{type:network.created, networkId:NetworkId}`，其他分支按实际用例补齐。秘密重交付使用专门加密载荷，不塞进此通用记录。

## 共用值类型

| 类型 | 字段 |
| --- | --- |
| `ActorRef` | 判别 `type`；user 分支有 `userId`，node 分支有 `nodeId`，enrollmentKey 分支有 `keyId`，system 分支无主体字段 |
| `PublicKey` | `algorithm: KeyAlgorithm`、`bytes: bytes` |
| `Sealed<T>` | `keyId: string`、`nonce: bytes`、`ciphertext: bytes`；解密后按 T 校验，认证标签由选定 AEAD 格式携带 |

SQL rows 含私有字段，不能整体复用为 Console DTO。不可逆吊销记录、完整配置和有效引用不随 RAM 或短期观察任务清理。
