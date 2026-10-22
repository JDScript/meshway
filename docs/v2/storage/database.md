# 持久数据库字段

Server 使用 SQLite 保存私有身份、完整配置、netmap 与历史。下表使用实际 SQL 列名，描述存储职责；它们不是 HTTP DTO，也不提供直接修改数据库的接口。对外字段以[生成契约](/v2/contracts/openapi.json)为准。

账户与执行表的 UUID 使用 16-byte BLOB，部分观测／资源表使用规范文本 UUID；时间使用 UTC Unix 毫秒 INTEGER，对外编码为 RFC 3339。秘密原文与完整配置使用认证加密，认证 token 保存摘要。主键、外键和不可逆约束由 schema 共同验证。

当前 schema 版本为 9（`PRAGMA user_version`，`application_id` 为 `MSHW`）。运行时只接受空库或与当前版本规范 schema 完全一致的库；较早版本的库在打开前单向迁移，见[Schema 版本与迁移](#migration)。Server 部署形态的设计（未实现）见[存储迁移与部署形态设计](/v2/storage/migration-design)。

## User

`users`：

| 字段 | 含义 |
| --- | --- |
| `id` | 稳定 UserId |
| `email` | 规范化、唯一登录标识，不证明邮箱所有权 |
| `password_hash` | 私有 Argon2id PHC |
| `credential_version` | 密码／全会话撤销版本 |
| `created_at`／`disabled_at` | 创建时间与可空停用时间 |

邮箱变化不改变主体 ID。当前配置中的可读引用随维护操作形成新版本，历史 actor 邮箱快照不改写。

## UserSession

`user_sessions`：

| 字段 | 含义 |
| --- | --- |
| `id`／`user_id` | 非秘密 SessionId 与账户绑定 |
| `token_hash` | 唯一认证摘要，不作为对外 ID |
| `credential_version` | 签发时的账户版本 |
| `created_at`／`expires_at`／`revoked_at` | 签发、绝对到期、单会话撤销 |
| `last_used_at` | 可空实际使用时间，不等于用户交互时间 |
| `client_label` | 可空安全客户端摘要，不参与认证 |
| `delegation_client` | 可空：经[委托授权码](#delegationcode)签发给该客户端的会话，不能管理账户或其他会话 |

认证同时检查账户、版本与期限；会话失效也终止相应 SSE 和诊断订阅。

## PasswordReset

`password_resets` 以 `token_hash` 为主键，保存 `user_id`、`credential_version`、`expires_at`、`revoked_at`、`consumed_at`。主机维护签发、一次性兑换。消费推进账户版本并撤销旧会话。

## Network

`networks`：

| 字段 | 含义 |
| --- | --- |
| `sequence`／`id` | 内部分页顺序与公开 NetworkId |
| `name` | 网络名称 |
| `created_by`／`created_at` | 创建者与时间，不替代当前 Owner membership |
| `creation_request_id`／`creation_request_digest` | 建网幂等键与原请求绑定 |
| `deleted_at` | 可空内部生命周期标记，不代表公开网络删除 API |
| `current_revision` | 当前期望配置，不是全网 applied |
| `publication_sequence` | 保留列，恒为 0（已退出的执行发布序号，不再读取） |

网络、Owner membership 和初始配置同事务创建。地址池唯一来自配置，不在 Network 保存可写副本。

## Membership

`memberships` 保存 `sequence`、`id`、`network_id`、`user_id`、`role`、`version`、`joined_at`、`revoked_at`。role 为 owner／admin／member，version 用于并发校验。

同网同用户至多一个有效关系；离网后重加产生新 MembershipId。最后 Owner 保护、成员删除影响与当前身份在最终事务检查。

## JoinLink

`join_links`：

| 字段 | 含义 |
| --- | --- |
| `sequence`／`id`／`network_id` | 分页顺序、非秘密链接 ID、固定网络 |
| `role` | 固定授予角色 |
| `token_hash` | 唯一凭据摘要 |
| `created_by`／`issuer_membership_id` | 签发者及当次权限关系 |
| `created_at`／`expires_at`／`revoked_at` | 创建、首次消费期限、撤销 |
| `consumed_at`／`consumed_by_user_id`／`consumed_membership_id` | 同事务写入的唯一领取结果；视图中的签发者／领取者 email 读取时联接 users，不另存快照 |
| `creation_request_id`／`creation_request_digest` | 幂等输入绑定 |

签发者离网再加入不复活链接。成功领取重试只恢复仍有效的原 membership；创建重试不恢复 token 原值，见[秘密交付](/v2/api/authentication#创建请求的幂等与秘密交付)。

## Node

`nodes` 保存 `id`、`network_id`、`registered_at`、`credentials_revoked_at`。它不保存 owner、subject、tags、enabled、hostname、地址或运行状态副本。

有效身份与静态设置从当前配置读取，运行事实从 State 读取。永久删除同事务吊销接入、移除配置、记录地址退出并撤回其 netmap；最小记录保留以拒绝旧凭据和历史恢复。

## AddressRetirement

`address_retirements`：

| 字段 | 含义 |
| --- | --- |
| `network_id`／`address`／`removed_revision` | 退出记录键 |
| `previous_node_id` | 旧绑定持有者 |
| `release_after` | 可空安全复用下界；null 表示未知，继续阻止复用 |

当前地址分配唯一来自配置。这些记录只保护旧许可／回复／NAT／受管 DNS 退出，不因 Server 重启或配置回滚清除。saved／applied 不能单独证明旧绑定已退出。

## NodeKey

`node_keys` 保存 `id`、`network_id`、`node_id`、`purpose`、`algorithm`、`public_key`、`generation`、`created_at`、`retired_at`、`revoked_at`。

control 固定 Ed25519，dataplane 固定 X25519；公钥唯一，用途与代际分开。私钥不入库。已退休／吊销标记不可清除，公钥不能被另一个 Node 认领。

### Challenge、session 与换钥恢复

| 表 | 字段与用途 |
| --- | --- |
| `node_challenges` | id、controller_id、network_id、node_id、key_id、nonce、request_digest、operation、created_at、expires_at、consumed_at、session_anchor；固定签名申请、challenge 和一次消费，session_anchor 区分真实会话恢复锚点 |
| `node_challenge_clock` | singleton、utc_ms；认证／清理事务共同推进的持久 UTC 高水位，拒绝删除、替换与回拨 |
| `node_sessions` | token_hash、network_id、node_id、key_id、boot_id、created_at、expires_at；短期 bearer 绑定当前 Server boot |
| `node_rotations` | network_id、node_id、request_id、request_digest、challenge_id、challenge、control_key_id、dataplane_key_id、generation、result；保存原证明与公钥结果用于同次换钥恢复 |

恢复不交付私钥或旧 session。证明格式与轮换重试见[认证](/v2/api/authentication#已注册-node-认证)。

## ConfigurationVersion

`configuration_versions`：

| 字段 | 含义 |
| --- | --- |
| `network_id`／`revision` | 完整版本主键 |
| `source_jsonc` | 加密编辑原文，保留注释和输入意图 |
| `configuration` | 加密的完整规范化 NetworkConfiguration |
| `bindings` | 加密的稳定用户／listener／规则集绑定 |
| `created_by`／`created_by_email` | 提交用户 ID 与可空历史邮箱快照 |
| `created_at`／`cause`／`request_id` | 时间、触发来源与请求关联 |
| `rollback_from` | 可空恢复来源；本行仍是新 revision |
| `execution_targets` | 本次 apply 固定 NodeId → NetmapRef（该版本编译出的各节点 netmap），提交时填入并封存 |

完整历史不可覆盖。原文和规范值都可能含秘密，必须同样保护。读取旧版本不重新运行当前默认补全；Node identity、enabled、地址和协议参数没有另一份可编辑源。

execution_targets 不随后续重编译或规则集刷新扩大分母；删除节点不列为等待其回执的目标。应用进度根据这些引用读取真实 `netmap_receipts`，并另算 freshness。

### ConfigurationBindings

| 字段 | 内容 |
| --- | --- |
| `users` | Email → UserBinding，策略中的用户／组／Tag owner 引用 |
| `nodeUsers` | NodeId → UserBinding，设备用户身份的稳定解析 |
| `listeners` | NodeId → listener key → 创建 revision |
| `ruleSets` | ruleSet key → ArtifactId，源版本初始资源绑定 |

UserBinding 包含 userId 和 membershipId。策略 users 与设备 nodeUsers 分开：旧关系 M1 失效后，新接入设备建立 M2 不会重新激活旧策略引用。原样保存保留旧绑定，历史恢复检查原 membership，不将同邮箱新主体当成原主体。

listener 同名重建使用新创建 revision；socket 另有运行 generation。协议材料直接保存在 configuration；规则集刷新写入 `rule_set_bindings`，不改写源历史。

### ConfigurationCommit

`configuration_commits` 保存 network_id、user_id、request_id、base_revision、加密 input 和 result。它固定提交幂等结果，同请求重试不再次生成材料、创建版本或发布事件。

## CredentialRevocation

`credential_revocations` 以 network_id、kind、fingerprint 为键，保存 revoked_at、revoked_by。kind 为 vlessUuid／trojanPassword／tlsCertificate。

这些是不可逆拒绝依据，不能通过复制材料、更换 listener 名称或恢复配置绕过。秘密指纹不对外公开，完整材料值仍在受保护的配置历史中。Node 身份吊销独立由 nodes／node_keys 负责。

## DatasetArtifact

`dataset_artifacts` 保存 id、可空 network_id、kind、source、format、data、digest、created_at。kind 为 geoip／asn（规则集不在此表，见 [RuleSet](#ruleset)）；内容经过完整校验后不可变保存，原始导入为有界临时输入。

资源引用精确 ArtifactId，不能重新下载最新 URL 冒充历史内容。来源与摘要不包含凭据。

`ip_dataset_heads` 以 kind 为主键，保存 GeoIP／ASN 的当前 artifact_id；与导入同事务更新。引用必须对应同 kind 的全局 MMDB artifact；缺失或错误 head 阻断完整性检查，不在启动时猜测修复。

## RuleSet {#ruleset}

| 表 | 字段与用途 |
| --- | --- |
| `rule_set_artifacts` | digest、data、created_at；全局 `MWRS` 内容寻址（digest 为 data 的 SHA-256），各网络共享、不可修改；没有可恢复配置版本、也没有仍在租约内的 netmap 引用时连同记录回收 |
| `rule_set_records` | source、digest、record、created_at；某来源（远程 URL＋格式，或目录集合）首次产生该 artifact 的来源与转换报告，不可修改 |
| `rule_set_record_heads` | source、digest、record、updated_at；该来源最近一次成功准备得到该 digest 的记录（目录 sequence、上游、held 与报告），刷新得到同一 digest 时替换 |
| `rule_set_catalogs` | catalog、trust、sequence、verified_at、document、signature；每个签名目录在一组信任钥下已接受的最高 sequence 及验证过的 `index.json`／`index.json.sig` 原文；sequence 不可下降，更低的目录不再使用 |
| `rule_set_catalog_checks` | catalog、trust、problem、checked_at、failures；Server 对每个生效目录的最近一次获取，决定下次获取与退避；失败保留已接受目录 |
| `rule_set_bindings` | network_id、revision、bindings、bound_at；某配置版本初始绑定被刷新后的当前 ruleSet key → digest；不存在时用版本自身的 bindings |

### RuleSetCheck

`rule_set_checks` 以 network_id、revision、rule_set_key 为键，保存 problem、checked_at、last_success_at、failures、check_version。failures 有界，check_version 是内部 CAS，避免依赖毫秒时间戳唯一性。

`rule_set_refreshes` 以 network_id、user_id、request_id 为键，保存手动刷新命名的 rule_set_key 和 created_at；用于刷新的请求 ID 幂等，写入时清理超过 24 小时的行。

检查记录拥有刷新结果与退避，不拥有当前内容；实际 artifact 绑定来自配置版本的 bindings 与 `rule_set_bindings`。失败保留最后成功内容和成功时间，validate 不保存检查结果。

## EnrollmentRequest {#enrollmentrequest}

`enrollment_requests`：

| 字段 | 含义 |
| --- | --- |
| `id` | 接入请求 ID，不是 bearer |
| `control_key`／`dataplane_key` | 本次两类公钥 |
| `request_digest`／`code_challenge`／`hostname` | 固定 intent 与 PKCE 绑定 |
| `created_at`／`expires_at` | 创建与首次批准／兑换期限 |
| `approval` | 批准网络、用户及当前会话／membership 依据 |
| `result_hash`／`delivery` | 一次性结果摘要与短期受保护重交付内容 |
| `denied_at`／`cancelled_at` | 批准者撤回与设备取消，互斥 |
| `consumed_at`／`node_id`／`binding_digest` | 唯一消费结果及恢复绑定 |
| `existing_binding` | 可空的已注册 Node 重新登录绑定 |

返回方式固定 poll，不保存 callback 目标。消费和配置 identity 写入同事务；批准记录不是第二份当前 Node 身份。设备私钥、密码和 PKCE verifier 不入库。

## EnrollmentKey

`enrollment_keys` 保存 sequence、id、network_id、token_hash、tags、加密 initial_settings、created_by、membership_id、created_at、expires_at、revoked_at、consumed_request_id、creation_request_id、creation_request_digest。

tags 是本次接入输入，注册后当前身份只来自 Configuration。模板支持 hostname 与 listeners；自动材料在真实注册时生成。消费重检签发资格与当前配置，列表仅返回安全摘要。

## NodeExecution

`node_execution` 保存 network_id、node_id、instance_id、generation、boot_id、instance_expires_at、capabilities：当前实例、其期限与已认证 acquire 声明的能力。实例期限与 State 联系新鲜度分开；普通 State 不自行替换能力。`node_instance_claims` 保存 network_id、node_id、instance_id、generation，拒绝释放／过期后的实例 ID 重新认领。

### RoutingSelection

`routing_selections` 保存 network_id、node_id、identity_digest、profile、version、selected_at：节点或用户提交的持久方案选择。只有 identity_digest 与当前身份一致时适用；version 跨身份单调递增。清除后行保留、profile 为空串（方案名不能为空）且不适用，以保持版本单调。身份摘要不赋权；实际方案由 NodeState 报告。

## Netmap

每节点 netmap 由 Server 编译、签名后经 `sync` 交付，见 [Netmap 设计](/v2/node/netmap-design)。

| 表 | 字段与用途 |
| --- | --- |
| `node_netmaps` | network_id、node_id、serial、revision、digest、dependency_digest、payload、created_at、withdrawn_at；加密的 netmap 内容，只保留当前与上一份（delta 基准）。内容不可修改，serial 只增；撤回的当前行保留 serial 高水位，节点回来时不会看到 serial 回退 |
| `node_netmap_history` | network_id、node_id、serial、revision、digest；每个曾签发 serial 的紧凑不可变引用，比内容行保留更久，回执历史不钉住旧内容 |
| `node_netmap_inputs` | network_id、node_id、digest；该节点当前 netmap 的编译输入指纹（含 Server 构建身份），启动重编译据此判断是否需要新 serial |
| `node_netmap_artifacts` | network_id、node_id、serial、digest、superseded_at；每个 serial 引用的规则集 artifact。节点可取回仍可能在租约内的自身 netmap 引用的 digest；被替代超过最长租约的行删除 |
| `netmap_receipts` | network_id、node_id、serial、instance_id、generation、attempt、sequence、phase、reported_at、received_at、failure；节点对确切 netmap 的保存／安装结果，按实例、attempt、sequence 排序，不可修改。phase 为 saved／applied／failed，failure 为明确组件类别或 null；回执只对节点当前 netmap 与存活实例是当前证据 |
| `node_netmap_unavailable` | network_id、node_id、reason、since；非配置触发的重编译中超出上限而被排除的节点，在之后重编译重新满足上限前没有 netmap |

### ServerSigningKey

`server_signing_keys` 保存 id、purpose（`netmap`）、generation、public_key、private_key、created_at、retired_at。私钥以主密钥密封并绑定 (id, generation)；恰好一把当前钥（retired_at 为空），其余为轮换窗口内或已退休的旧钥，永久保留、不可修改或删除。轮换见 [Server · Netmap 签发钥轮换](/v2/server#netmap-签发钥轮换)。

## ActivityEvent

`activity_events`：

| 字段 | 含义 |
| --- | --- |
| `sequence`／`id` | 内部提交顺序和公开 ActivityId |
| `scope`／`account_user_id`／`network_id` | account 或 network 作用域 |
| `at`／`actor_kind`／`actor_user_id`／`actor_node_id`／`actor_email` | 完成时间与真实 user／node／system 主体，邮箱为历史快照；node 主体只用于 routing.select；`routing.autoResolve` 的主体为 system |
| `action` | 固定管理动作枚举 |
| `target_user_id`／`target_session_id`／`target_membership_id`／`target_join_link_id`／`target_node_id`＋`target_profile` | 按动作约束的目标关联；后两者只且必须用于 routing.select 与 routing.autoResolve（清除选择的 routing.select 的 target_profile 为空串，读出为 null），用户提交的 routing.select 以 (network, actor_user_id, request_id) 唯一索引作为重试记录 |
| `before_revision`／`after_revision`／`request_id` | 配置变更与请求关联 |
| `previous_email` | 仅 account.email 允许非空：修改前的地址快照 |

记录与对应成功动作同事务提交；幂等恢复不重复插入。不包含秘密、完整 diff 或原始请求 body。Node 应用结果在 netmap_receipts，不能改写“配置已保存”的活动事实。HTTP 范围见[版本与操作历史](/v2/api/history)。

## ObservationEvent

`observation_events` 保存 row_id、network_id、source_node_id、instance_id、event_id、occurred_at、received_at、payload、payload_digest、admission_expires_at。

同网／来源 Node／eventId 唯一；payload 包含真实事件及配置上下文。正文裁剪后仍保留主键和摘要至完整接纳窗口结束，重试不恢复已裁剪正文。接纳窗口包含允许时钟偏差，不能因墓碑先清理而再次接纳旧点。

### HistoryCoverage

`history_coverage` 保存 network_id、source_node_id、before_at、reason，reason 为 retention／capacity。它记录真实裁剪下界，不能由最早现存样本推断。

collection_gap 则来自 Node 真实事件，保留原始区间和 dropped，不按查询裁剪比例伪造数量。两类覆盖信息与查询权限一起过滤，见[观测历史](/v2/state-reference/observations)。

### ObservationSubscriberId

`observation_subscriber_ids` 只保存已消费的观察订阅者 ID（不可修改、不删除）：它是跨进程重启的幂等墓碑，不是持久化的任务、许可或授权缓存。观察任务与订阅只在内存中，见[内存字段](/v2/storage/memory)。

## MasterKeyBinding

`master_key_binding` 保存 singleton、key_id、key_check，绑定私有主密钥且不可修改／删除。私有密钥文件先持久发布，再与数据库绑定提交；丢失密钥不能生成替代来解密原库。

SQL 行包含私有字段，不能整体作为 Console DTO。完整配置、永久吊销与有效恢复依据不随内存缓存或短期任务清理。

## DelegationCode {#delegationcode}

`delegation_codes` 保存 code_hash、user_id、session_id、client、redirect_uri、code_challenge、created_at、expires_at、consumed_at：已登录用户为委托客户端签发的一次性授权码（只存摘要），PKCE challenge 绑定兑换；兑换要求签发会话仍有效。所得会话在 `user_sessions.delegation_client` 标明客户端，不能管理账户或其他会话。

## Schema 版本与迁移 {#migration}

`schema_migrations` 每次完成的迁移一行，保存 sequence、from_version、to_version、build（执行迁移的构建身份）、started_at、finished_at、backup（保留的旧库文件名）；与迁移结果同一事务提交，不可修改或删除。

运行时只认当前形态：业务代码与读取路径没有旧版本分支。较早版本的库在打开前由 `store::schema::migrate` 单向迁移，服务启动与 `meshway-server migrate` 共用这一份实现（运维行为见 [Server · 存储迁移](/v2/server#存储迁移)）：

1. 读取 (application_id, user_version)：当前版本或空库直接进入正常打开；非 `MSHW`、比本构建新、早于最低可迁移版本（当前为 8）为持久阻断，不修改文件；离线命令遇到可迁移的旧库拒绝。
2. 以 `history/v<旧版本>.sql` 对旧库做与正常打开相同的完整检查（规范 SQL 原文、`integrity_check`、`foreign_key_check`、主密钥绑定／challenge 时钟／IP 数据集 head 不变量）；不符为持久阻断。
3. 旧库 checkpoint 并切换为 DELETE 日志模式，不留下可能被重放到新文件上的 WAL。
4. 每级在 data-dir 内 owner-only 的 `controller.sqlite.migrating.v<N>` 中执行版本 N 的规范 schema，只读 `ATTACH` 上一级文件，按外键拓扑序、主键升序 `INSERT … SELECT` 复制每张表（触发器与外键对每行重新生效）；schema 自带的种子行（challenge 时钟）取旧值；`sqlite_sequence` 高水位不下降；每表行数与内容和源表相同；同一事务内以版本 N 的完整检查验证结果后提交。
5. 旧库文件加第二个名字 `controller.sqlite.v<旧版本>-<Unix 毫秒>` 作为备份，再把 staging 原子 `rename` 为 `controller.sqlite`；备份保留最新 3 份。中断在两步之间时，下一次启动移除多出的名字并重新迁移。

密文逐字节复制（认证加密的 AAD 只绑定领域与业务键，不含表名与版本），迁移不需要主密钥；netmap 内容、netmap 历史与签发钥从不解密或改写。编译产物的变化由启动重编译负责，不由迁移改写。

修改 schema 的规则：

- 任何 SQL 改动在同一提交中提升 `VERSION`、新增快照 `history/v<VERSION>.sql`（等于当前 SQL 拼接）与迁移级 `migrate/v<VERSION>.rs`。快照提交后不可修改，测试固定每个快照的 SHA-256、要求当前 SQL 等于最新快照、版本连续且每级都有迁移；并行分支同时升号在同一文件名上冲突。
- 迁移级只声明偏离默认复制的部分：新表、删除的表、列有变化的表（以读取 `old.*` 的 SQL 填充）；未声明的差异使迁移失败，测试要求声明与两份快照的实际差异一致。
- 新增 NOT NULL 列必须能由已有行推出（迁移级以 SQL 写出它的值）；无法由已有数据推出的事实不加 NOT NULL 列，改用可空列或独立表。CHECK 收窄同理：已有行必须满足新约束，否则该级先给出明确的变换。
