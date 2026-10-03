# 持久数据库字段

**V2 精简记录提案。** 只为需要独立持久生命周期的数据建立记录；配置内容随完整版本保存，嵌套值不是独立实体。字段逐项列出，表名与物理拆分未定。`T?` 为可空；时间为 UTC，ID／Digest／Revision 是独立强类型。网络内关联使用 networkId 和复合约束。

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
| `ipv4Prefix` | Ipv4Prefix | 本网络 Mesh IPv4 地址池 |
| `ipv6Prefix` | Ipv6Prefix | 本网络 Mesh IPv6 地址池 |
| `createdBy` | UserId | 创建者，不替代当前 Owner membership |
| `createdAt` | Timestamp | 创建时间 |
| `creationRequestId` | RequestId | 建网幂等请求，与 createdBy 构成唯一键 |
| `creationRequestDigest` | Digest | 原建网参数绑定，响应丢失重试返回本网络 |
| `deletedAt` | Timestamp? | 删除标记；删除／保留流程仍待定版 |
| `currentRevision` | Revision | 当前完整配置版本指针 |
| `publicationSequence` | u64 | 本网络已分配执行发布的单调上界 |

创建 Network、Owner membership、初始配置同事务。配置指针只由提交事务推进。同幂等键不同参数拒绝；网络已删除也不通过旧请求重建，删除保留期须覆盖幂等承诺。

地址字段按当前“一网每族一个池、每节点每族一个固定地址”的简单模型提案，不建立独立地址池实体或持久分配游标。地址池调整／回收流程仍待定版。

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
| `meshIpv4` | Ipv4Address | 持久分配的 Mesh IPv4 |
| `meshIpv6` | Ipv6Address | 持久分配的 Mesh IPv6；不表示 IPv6 数据面已实现 |
| `createdBy` | ActorRef | 原始接入来源，仅追溯 |
| `registeredAt` | Timestamp | 注册时间 |
| `credentialsRevokedAt` | Timestamp? | 永久吊销整个接入；所有旧／派生凭据拒绝 |

**没有 owner、subject、tags、enabled、hostname、lastSeen、applied 字段。** 有效身份与启停从 Config 获取；运行观测从 RAM 获取。永久接入吊销不可通过配置、换钥或旧注册重试恢复。

地址在网络内唯一、在接入事务分配。当前提案在保留 Node 期间不换绑／回收它的地址，避免需要另一份分配生命周期；若后续支持在线换址或提前回收，再明确分配代际和审计记录。保留网络内地址唯一约束，候选地址可从现有分配重建。

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

## ConfigurationVersion

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 与 revision 构成主键 |
| `revision` | Revision | 递增配置版本 |
| `sourceJsonc` | Sealed&lt;string&gt; | 编辑原文，含注释／可能的协议秘密 |
| `configuration` | Sealed&lt;NetworkConfiguration&gt; | 规范化完整配置，类型见配置参考 |
| `bindings` | ConfigurationBindings | 本版本固定的用户／listener 代际／数据集绑定 |
| `createdBy` | ActorRef | 用户或系统修改来源 |
| `createdAt` | Timestamp | 提交时间 |
| `requestId` | RequestId? | 幂等／事件关联 |
| `rollbackFrom` | Revision? | 回滚来源；本记录仍是新 revision |

NetworkConfiguration 的完整字段见[配置参考](/v2/configuration-reference)；其中 `nodes[id].identity` 与 `enabled` 是 Node 授权身份／业务启停的唯一源。历史原文、规范化值与绑定不可单独覆盖。

规范化配置完整保存已补全的默认参数、UUID／password／证书／私钥；读取旧版本不会再运行当前默认补全。sourceJsonc 保留编辑意图和注释，configuration 保存确定结果，两者用途不同。线性完整历史可查得前序 revision；配置摘要需要时计算／缓存，CAS 使用 revision。

### ConfigurationBindings

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `users` | Map&lt;JsonPointer, UserBinding&gt; | 可读引用 → 稳定主体，不按当前邮箱重解析 |
| `listeners` | Map&lt;NodeId, Map&lt;ListenerKey, Revision&gt;&gt; | listener 此次创建所在 revision，更新沿用、删除后重建换新 |
| `ruleSets` | Map&lt;RuleSetKey, ArtifactId&gt; | 此源版本提交时固定的数据集内容 |

`UserBinding` 字段：`userId: UserId`、`membershipId: MembershipId`。listener 以 `(networkId,nodeId,key,createdRevision)` 标识资源代际；运行 socket 仍有自己的 generation。上述绑定嵌入配置版本，无独立资源 CRUD。

材料值已在完整 configuration 内；初次省略时生成、已有 listener 省略时沿用旧值，新建代际生成新自动值。规则集后续自动更新由执行发布绑定确切 artifact，不覆盖这个历史版本。回滚明确选用源版本初始绑定或某次历史发布的绑定，不能猜用最新内容。

历史 identity 回滚涉及 Tag→User／改绑 User 时仍须新的设备登录授权；保存了旧 UserBinding 不等于持有当前转换权限。

## CredentialRevocation

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 吊销作用域 |
| `kind` | ProtocolCredentialKind | 已支持协议凭据／证书类型 |
| `fingerprint` | CredentialFingerprint | 被吊销的确切材料标识，与网络／类型构成键 |
| `revokedAt` | Timestamp | 永久吊销时间 |
| `revokedBy` | ActorRef | 操作来源 |

这里只保存不可逆拒绝依据，材料值随配置历史保存。低熵密码不能用对外公开的普通散列做指纹；秘密指纹采用受保护的确定识别方式，算法随材料保护实现定版。相同凭据换 listener 名称或放入新配置也不能绕过吊销。保存、回滚、发布、执行续租均检查，并撤回受影响旧许可。

Node 的持久身份／密钥吊销继续由 Node／NodeKey 负责；本记录针对协议层材料。

## DatasetArtifact

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ArtifactId | 不可变数据版本 |
| `networkId` | NetworkId? | 网络规则集所属网络；实例 GeoIP 可为 null |
| `kind` | ruleSet \| geoip | 数据类型 |
| `source` | string | 来源 URI／名称，不含凭据 |
| `format` | DatasetFormat | 保存内容的明确格式／版本 |
| `data` | bytes | 已校验规范化规则集／GeoIP 查询内容 |
| `digest` | Digest | 规范化内容摘要 |
| `createdAt` | Timestamp | 接受时间 |

下载错误与进度在 RAM；只有完整校验通过的内容入库。永久保留的是执行需要的确定内容；原始下载属于有界临时输入，失败条目按转换合同报告。历史引用保留该内容和格式，不能重新下载最新 URL 冒充它。

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
| `nodes` | Map&lt;NodeId, PublishedNodeConfiguration&gt; | 实际参与节点的完整切片，嵌入同一发布 |

`PublicationDependencies` 字段：`userBindings: UserBinding[]`、`nodeIds: NodeId[]`、`keyIds: KeyId[]`、`ruleSets: Map<RuleSetKey,ArtifactId>`。节点地址在本提案中是该 Node 的固定事实，材料来自所引用完整配置；持续使用仍检查当前吊销／租约。

### PublishedNodeConfiguration

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `digest` | Digest | 节点切片摘要 |
| `configuration` | Sealed&lt;NodeConfiguration&gt; | 完整确定产物和所需材料 |

所属 network／publication 由外层给出，NodeId 是 map key。只保存实际发布／恢复所需切片，同一发布重发保持同 digest；按需发布有界参与集合，不预生成全网节点对。独立切片行／内容寻址去重留给实际容量优化。

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
| `payload` | ObservationPayload? | 真实事件；保留期结束后可清空，留去重墓碑 |
| `payloadDigest` | Digest | 重试内容一致性 |
| `admissionExpiresAt` | Timestamp | 这条事件的重试接纳窗口上界 |

`ObservationSource` 为判别联合：node 分支有 `nodeId: NodeId`、`instanceId: InstanceId`、`publicationId: PublicationId`；controller 分支有 `epoch: StateEpoch`，不伪造 Node 报告。双方都有对应 `type`。

Node 事件按 `(networkId,source.nodeId,eventId)` 唯一；Controller 事件按 `(networkId,source.epoch,eventId)` 唯一。payload 字段沿用[真实事件](/v2/state-reference/observations)与[测量](/v2/state-reference/realtime)，不存任意 metadata 或重复心跳制造的样本。

## HistoryCoverage

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 网络 |
| `sourceNodeId` | NodeId | 来源 |
| `before` | Timestamp | 此时刻前可能已裁剪 |
| `reason` | retention \| capacity | 记录覆盖限制 |

ObservationEvent 同时承担去重：payload 清理后，保留原主键／摘要至接纳窗口结束；墓碑不返回成测量样本，重试不恢复已裁剪 payload。接纳窗口之外的旧上报拒绝，不因墓碑已清理重新计数；窗口不能被重试推进。实际采集缺口仍由事件／丢弃计数报告。

墓碑保留上界必须覆盖原事件时间／实例仍可能被接纳的整个窗口，包括允许的时钟偏差；不能只定一个更短的清理 TTL，留下删除后再次接纳的空档。

只保留正文也不足以说明历史是否完整，因此 Coverage 记录真实裁剪下界，不根据“最早现存样本”推测覆盖。全量历史清理仍保留必要覆盖信息。

## 共用值类型

| 类型 | 字段 |
| --- | --- |
| `ActorRef` | 判别 `type`；user 分支有 `userId`，node 分支有 `nodeId`，enrollmentKey 分支有 `keyId`，system 分支无主体字段 |
| `PublicKey` | `algorithm: KeyAlgorithm`、`bytes: bytes` |
| `Sealed<T>` | `keyId: string`、`nonce: bytes`、`ciphertext: bytes`；解密后按 T 校验，认证标签由选定 AEAD 格式携带 |

SQL rows 含私有字段，不能整体复用为 Console DTO。不可逆吊销记录、完整配置和有效引用不随 RAM 或短期观察任务清理。
