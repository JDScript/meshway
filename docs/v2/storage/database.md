# 持久数据库字段

**V2 精简记录提案。** 只为需要独立持久生命周期的数据建立记录；配置内容随完整版本保存，嵌套值不是独立实体。字段逐项列出，表名与物理拆分未定。`T?` 为可空；时间为 UTC，ID／Digest／Revision 是独立强类型。网络内关联使用 networkId 和复合约束。

新 Controller 已实现的账户存储切片中，User／UserSession／账户 ActivityEvent 的时间统一以 Unix 毫秒 INTEGER 保存，对外仍为 UTC RFC 3339。资源 ID 以非零的 16-byte UUID 保存，不把零值作为缺失身份；其他记录的物理布局仍随各领域实现确定。

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
| `id` | SessionId | 非秘密资源 ID，用于会话管理 |
| `tokenHash` | Digest | 唯一认证摘要，不作为对外 ID |
| `userId` | UserId | 关联 User |
| `credentialVersion` | u64 | 签发时版本，认证时与 User 核对 |
| `createdAt` | Timestamp | 签发时间 |
| `lastUsedAt` | Timestamp? | 最近实际会话使用；未记录为 null，不等于用户交互时间 |
| `clientLabel` | string? | 粗粒度、脱敏的客户端摘要；仅展示，不参与认证 |
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
| `creationRequestId` | RequestId | 建网幂等请求，与 createdBy 构成唯一键 |
| `creationRequestDigest` | Digest | 原建网参数绑定，响应丢失重试返回本网络 |
| `deletedAt` | Timestamp? | 删除标记；删除／保留流程仍待定版 |
| `currentRevision` | Revision | 当前已受理的期望配置版本，非整网 applied 版本 |
| `publicationSequence` | u64 | 本网络已分配执行发布的单调上界 |

创建 Network、Owner membership、初始配置同事务。配置指针只由提交事务推进。同幂等键不同参数拒绝；网络已删除也不通过旧请求重建，删除保留期须覆盖幂等承诺。

网络地址段属于当前 ConfigurationVersion.addressPools，可修改；Network 记录不保存另一份地址池。

## Membership

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | MembershipId | 主键，一次加入关系 |
| `networkId` | NetworkId | 所属网络 |
| `userId` | UserId | 用户 |
| `role` | owner \| admin \| member | 网络内角色 |
| `version` | u64 | 成员修改的并发基准 |
| `joinedAt` | Timestamp | 加入时间 |
| `revokedAt` | Timestamp? | 离网／移除时间 |

`(networkId,userId)` 至多一条有效记录；重新加入产生新 id。最后 Owner 保护在事务中执行。

## JoinLink

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | JoinLinkId | 非秘密链接记录 ID |
| `networkId` | NetworkId | 固定加入网络 |
| `role` | owner \| admin \| member | 固定授予角色，须在签发者权限内 |
| `tokenHash` | Digest | 唯一 token 摘要 |
| `createdBy` | UserId | 签发者 |
| `issuerMembershipId` | MembershipId | 当次签发权限关系，离网再加入不复活旧链接 |
| `createdAt` | Timestamp | 创建时间 |
| `expiresAt` | Timestamp | 首次消费期限 |
| `revokedAt` | Timestamp? | 未消费链接的撤销时间 |
| `consumedByUserId` | UserId? | 成功领取者 |
| `consumedMembershipId` | MembershipId? | 唯一加入结果 |
| `consumedAt` | Timestamp? | 消费时间 |
| `creationRequestId` | RequestId | 签发幂等键 |
| `creationRequestDigest` | Digest | 原创建请求绑定，私有字段 |

消费字段同事务填写，与有效 membership 创建／活动记录一致。状态从消费、撤销、期限派生；已消费结果重试不恢复已撤销 membership。创建重试仅返回原资源元数据，原值交付规则见[认证](/v2/api/authentication#创建请求的幂等与秘密交付)。

## Node

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | NodeId | 主键，持久接入身份 |
| `networkId` | NetworkId | 不可任意切换的接入网络 |
| `createdBy` | ActorRef | 原始接入来源，仅追溯 |
| `registeredAt` | Timestamp | 注册时间 |
| `credentialsRevokedAt` | Timestamp? | 永久吊销整个接入；所有旧／派生凭据拒绝 |

**没有 owner、subject、tags、enabled、hostname、lastSeen、applied 字段。** 有效身份与启停从 Config 获取；运行观测从 RAM 获取。永久接入吊销不可通过配置、换钥或旧注册重试恢复。

用户确认的删除操作在同一事务设置 credentialsRevokedAt、移除当前 nodes[id] 并记录旧地址退出／执行撤回；没有节点恢复入口。本行及公钥／历史关联可作为最小墓碑保留，阻止旧凭据和历史配置复活。

节点期望地址唯一来自 `ConfigurationVersion.configuration.nodes[id].addresses`，允许显式修改／重分配。Node ID 与密钥不随改址变化。配置提交校验同网地址唯一性；地址索引只作为按 revision 绑定的派生数据，没有独立写入口。

## AddressRetirement

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `networkId` | NetworkId | 地址退出的网络 |
| `address` | IpAddress | 已从旧绑定撤下的地址 |
| `previousNodeId` | NodeId | 旧绑定持有者，仅作为退出依据 |
| `removedRevision` | Revision | 引发退出的配置版本，与 networkId／address 构成记录键 |
| `releaseAfter` | Timestamp? | 覆盖旧许可／回复／NAT／DNS 窗口的安全复用下界；未知则 null、继续等待 |

这是独立于当前分配的执行退出记录，不是另一份 IPAM 配置。配置改址／删除和退出依据同事务保存；旧发布停止相关续租后，依据所有已授予许可的最坏期限计算下界。不能用保存 ACK 或“现在没有连接报告”证明地址已释放。

自动分配排除仍在退出的地址；显式目标可以等待激活。复用须满足同一地址全部未完成退出记录，回滚／Controller 重启不缩短窗口。记录可在退出完成且无恢复依赖后清理；旧前缀的系统资源回收仍由 Agent underlay journal 拥有。

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
| `bindings` | Sealed&lt;ConfigurationBindings&gt; | 本版本固定的用户／listener 代际／数据集绑定 |
| `executionTargets` | Map&lt;NodeId, ConfigurationRef&gt; | 当前实现保存的本次配置应用固定目标；不含秘密，不随后续 demand／refresh 扩大分母 |
| `createdBy` | ActorRef | 用户或系统修改来源 |
| `createdAt` | Timestamp | 提交时间 |
| `cause` | ConfigurationChangeCause | 配置版本的主触发来源，独立于 actor |
| `requestId` | RequestId? | 幂等／事件关联 |
| `rollbackFrom` | Revision? | 回滚来源；本记录仍是新 revision |

NetworkConfiguration 的完整字段见[配置参考](/v2/configuration-reference)；其中 `nodes[id].identity`／`enabled` 是身份／业务启停的唯一源，addressPools／nodes[id].addresses 是地址期望的唯一源。历史原文、规范化值与绑定不可单独覆盖。

`executionTargets` 对应当前 SQLite 的 `execution_targets` 列：在相同配置事务中，由实际 publication 切片填入并封存；删除节点不列为等待其 ACK 的目标。GET／SSE 进度依据这些固定引用读取真实回执，并单独给出当前新鲜度，不把旧 applied 当作当前数据面健康。

规范化配置完整保存已补全的默认参数、UUID／password／证书／私钥；读取旧版本不会再运行当前默认补全。sourceJsonc 保留编辑意图和注释，configuration 保存确定结果，两者用途不同。线性完整历史可查得前序 revision；配置摘要需要时计算／缓存，CAS 使用 revision。

### ConfigurationBindings

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `users` | Map&lt;Email, UserBinding&gt; | groups／tagsOwners／access／topology／routing 中的策略引用；原样保存保留其成员生命周期，不按当前邮箱重新绑定 |
| `nodeUsers` | Map&lt;NodeId, UserBinding&gt; | 各 User 身份 Node 的 `Config.identity` 的稳定解析；只由已验证的设备接入／重新登录建立，Tag 身份没有该项 |
| `listeners` | Map&lt;NodeId, Map&lt;ListenerKey, Revision&gt;&gt; | listener 此次创建所在 revision，更新沿用、删除后重建换新 |
| `ruleSets` | Map&lt;RuleSetKey, ArtifactId&gt; | 此源版本提交时固定的数据集内容 |

`UserBinding` 字段：`userId: UserId`、`membershipId: MembershipId`。listener 以 `(networkId,nodeId,key,createdRevision)` 标识资源代际；运行 socket 仍有自己的 generation。上述绑定嵌入配置版本，无独立资源 CRUD。

两类用户绑定按用途独立。用户从 M1 离网后以 M2 重新加入，原策略引用仍绑定已失效的 M1；新设备经正常持钥／PKCE 接入只建立自己的 `nodeUsers[nodeId]=M2`，不重新激活旧 group 或 Tag owner。Node 身份、库存与离网删除读取 nodeUsers；编译 User／group selector 时须同时匹配稳定 User 和有效 Membership，`node`／`*` 仍按当前配置匹配。Node 表不增加 owner／subject 副本，用户可编辑的身份期望仍只在 Configuration.nodes[id].identity。

邮箱维护分别依据两类绑定的 UserId 更新可读引用；仅有 Node.identity 引用的网络也同步。历史恢复分别映射当前名称并核对原 Membership 仍有效，不能借恢复重建永久删除的 Node。当前每个 email 只有一个策略用途绑定，不引入按任意 JSON pointer 编辑授权绑定的接口。

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
| `kind` | ruleSet \| geoip \| asn | 数据类型 |
| `source` | string | 来源 URI／名称，不含凭据 |
| `format` | DatasetFormat | 保存内容的明确格式／版本 |
| `data` | bytes | 已校验规范化规则集／GeoIP／ASN 查询内容 |
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
| `initialSettings` | Sealed&lt;InitialNodeSettings&gt;? | 可选 hostname／listener 模板，可能包含管理员显式材料 |
| `createdBy` | UserId | 签发者 |
| `createdAt` | Timestamp | 创建时间 |
| `expiresAt` | Timestamp | 首次使用期限 |
| `revokedAt` | Timestamp? | 密钥撤销 |
| `consumedRequestId` | EnrollmentRequestId? | 一次性消费绑定 |
| `creationRequestId` | RequestId | 签发幂等键 |
| `creationRequestDigest` | Digest | 原请求绑定，含秘密时采用受保护摘要 |

消费时复核当前 Tag 授予资格。tags 是接入授权输入，设备注册后以 Configuration.identity 为唯一当前值。

InitialNodeSettings 字段为 `hostname: string?`、`listeners: Map<ListenerKey,Listener>`；未提供使用平台接入默认。模板在注册事务落入真实节点配置，自动材料在对应配置准备流程生成并保存，原始 token 不入库。列表只返回安全摘要与消费 Node／时间。

## PasswordReset

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ResetId | 恢复记录 ID |
| `tokenHash` | Digest | 一次性凭据摘要 |
| `userId` | UserId | 目标用户 |
| `credentialVersion` | u64 | 有效账户版本 |
| `expiresAt` | Timestamp | 首次消费期限 |
| `revokedAt` | Timestamp? | 撤销时间 |
| `consumedAt` | Timestamp? | 消费时间 |

由主机运维 CLI 签发，公开兑换接口验证本次凭据；网络 Owner 无他人账户重置权。消费推进 User.credentialVersion 并撤销旧会话，响应丢失后用新密码正常登录。

## ExecutionPublication

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | PublicationId | 不可变发布标识 |
| `networkId` | NetworkId | 所属网络 |
| `sequence` | u64 | 本网络单调发布顺序 |
| `configurationRevision` | Revision | 源配置版本 |
| `cause` | configuration \| demand \| refresh | 用户配置应用、按需执行或依赖刷新；进度按来源区分 |
| `supersedes` | PublicationId[] | 被本次重规划／重试替代的发布，必须同网且顺序更早 |
| `dependencies` | PublicationDependencies | 参与身份／密钥／材料／规则集的固定依据 |
| `dependencyDigest` | Digest | 编译输入摘要 |
| `compilerVersion` | string | 编译实现版本 |
| `createdAt` | Timestamp | 提交时间 |
| `withdrawnAt` | Timestamp? | 发布撤回时间 |
| `nodes` | Map&lt;NodeId, PublishedNodeConfiguration&gt; | 实际参与节点的完整切片，嵌入同一发布 |

`PublicationDependencies` 字段：`userBindings: UserBinding[]`、`nodeIds: NodeId[]`、`keyIds: KeyId[]`、`ruleSets: Map<RuleSetKey,ArtifactId>`。地址与协议材料来自所引用的完整配置版本，编译进每个参与节点的确定切片；持续使用仍检查当前吊销／租约。改址时不能将旧地址许可拼接到新发布。

每份发布固定一致依赖和目标内容，各节点收到后独立应用。替换目标时创建新发布，通过 supersedes 关联，不在原 ID 下改 digest；同一有效目标的重试可复用内容。cause=configuration 的进度与后续 demand／refresh 分开展示。

发布持久提交后即可下发，当前实例／目标／执行 lease 和地址退出条件仍需检查。应用结果不改变已提交配置的含义；撤回独立使相关执行失效。流程见[配置应用](/v2/configuration-reference/deployment)。

### PublishedNodeConfiguration

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `digest` | Digest | 节点切片摘要 |
| `configuration` | Sealed&lt;NodeConfiguration&gt; | 完整确定产物和所需材料 |
| `application` | ApplicationReceipt? | 最后接受的应用成功／失败回执，保留来源与时间 |

所属 network／publication 由外层给出，NodeId 是 map key。只保存实际发布／恢复所需切片，同一发布重发保持同 digest；按需发布有界参与集合，不预生成全网节点对。独立切片行／内容寻址去重留给实际容量优化。

### ApplicationReceipt

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `instanceId` | InstanceId | 应用结果所属实例 |
| `generation` | u64 | 应用时实例代际 |
| `result` | applied \| failed | 本节点真实结果 |
| `failedStage` | ConfigurationApplyPhase? | failed 时记录失败前所处阶段；applied 时为 null |
| `reportedAt` | Timestamp | 原结果时间 |
| `receivedAt` | Timestamp | Controller 接受时间 |
| `problem` | Problem? | failed 时的类型化错误 |

回执核对确切 publication／node／digest、当前实例和实际下发的有效目标；异常自报不能推进当前进度。failedStage 必须是合法的前序阶段。持久回执是最后接受的证据，当前新鲜度另算；Controller 重启或实例变化后不能只凭历史行断言当前 applied。连续下载／步骤进度存 RAM，不逐字节写数据库。

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

targetPublicationId 是节点当前应同步／应用的目标；Node 校验并持久保存后，在当前实例和有效执行 lease 下直接安装。saved 不等于已执行，应用回执按 publication 保存，当前运行事实仍从 NodeState 读取。

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

## ActivityEvent

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | ActivityId | 对外不透明 ID |
| `sequence` | u64 | 内部提交顺序／固定分页上界，不直接暴露为 cursor |
| `scope` | ActivityScope | network(networkId) 或 account(userId) 的封闭分支 |
| `at` | Timestamp | 控制面动作完成时间 |
| `actor` | ActorRef | 服务端确定的操作主体 |
| `action` | ActivityAction | 固定操作枚举 |
| `target` | ActivityTarget | 节点／成员／配置版本／链接等类型化引用 |
| `beforeRevision` | Revision? | 涉及配置时的原版本 |
| `afterRevision` | Revision? | 涉及配置时的新版本 |
| `requestId` | RequestId | 同一次逻辑操作关联 |

成功管理 mutation 与记录同事务提交，幂等恢复不重复插入；scope 和 actor 不由请求者自报。原配置通过 revision 关联，不复制秘密、完整 diff、token 摘要或请求 body。Node apply 结果在 ApplicationReceipt，不能用它改写“配置已提交”的活动事实。

### ActivityCoverage

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `scope` | ActivityScope | 保留范围 |
| `before` | Timestamp | 此时刻前记录可能已裁剪 |
| `reason` | retention \| capacity | 裁剪原因 |

按当前权限查询；账户 scope 仅本人，网络 scope 为管理读取。保留窗口之外明确缺口，不用空列表表示从未操作。字段与接口见[操作历史](/v2/api/history)。

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
