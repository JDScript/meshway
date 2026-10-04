# 持久数据库字段

Server 使用 SQLite 保存私有身份、完整配置、执行恢复依据和历史。下表使用实际 SQL 列名，描述存储职责；它们不是 HTTP DTO，也不提供直接修改数据库的接口。对外字段以[生成契约](/v2/contracts/openapi.json)为准。

账户与执行表的 UUID 使用 16-byte BLOB，部分观测／资源表使用规范文本 UUID；时间使用 UTC Unix 毫秒 INTEGER，对外编码为 RFC 3339。秘密原文与完整配置使用认证加密，认证 token 保存摘要。主键、外键和不可逆约束由 schema 共同验证。

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

认证同时检查账户、版本与期限；会话失效也终止相应 SSE 和诊断订阅。

## PasswordReset

`password_resets` 以 `token_hash` 为主键，保存 `user_id`、`credential_version`、`expires_at`、`revoked_at`、`consumed_at`。主机维护签发、一次性兑换，网络角色不获得全局密码重置权。消费推进账户版本并撤销旧会话。

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
| `publication_sequence` | 本网络执行发布的单调上界 |

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

有效身份与静态设置从当前配置读取，运行事实从 State 读取。永久删除同事务吊销接入、移除配置、记录地址退出并撤回执行；最小记录保留以拒绝旧凭据和历史恢复。

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
| `node_challenges` | id、controller_id、network_id、node_id、key_id、nonce、request_digest、operation、created_at、expires_at、consumed_at；固定签名 challenge 和一次消费 |
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
| `execution_targets` | 本次 apply 固定 NodeId → ConfigurationRef，按实际切片填入并封存 |

完整历史不可覆盖。原文和规范值都可能含秘密，必须同样保护。读取旧版本不重新运行当前默认补全；Node identity、enabled、地址和协议参数没有另一份可编辑源。

execution_targets 不随后续 demand／refresh 扩大分母；删除节点不列为等待其 ACK 的目标。进度根据这些引用读取真实回执，并另算 freshness。

### ConfigurationBindings

| 字段 | 内容 |
| --- | --- |
| `users` | Email → UserBinding，策略中的用户／组／Tag owner 引用 |
| `nodeUsers` | NodeId → UserBinding，设备用户身份的稳定解析 |
| `listeners` | NodeId → listener key → 创建 revision |
| `ruleSets` | ruleSet key → ArtifactId，源版本初始资源绑定 |

UserBinding 包含 userId 和 membershipId。策略 users 与设备 nodeUsers 分开：旧关系 M1 失效后，新接入设备建立 M2 不会重新激活旧策略引用。原样保存保留旧绑定，历史恢复检查原 membership，不将同邮箱新主体当成原主体。

listener 同名重建使用新创建 revision；socket 另有运行 generation。协议材料直接保存在 configuration，规则集刷新固定到新 publication，不改写源历史。

### ConfigurationCommit

`configuration_commits` 保存 network_id、user_id、request_id、base_revision、加密 input 和 result。它固定提交幂等结果，同请求重试不再次生成材料、创建版本或发布事件。

## CredentialRevocation

`credential_revocations` 以 network_id、kind、fingerprint 为键，保存 revoked_at、revoked_by。kind 为 vlessUuid／trojanPassword／tlsCertificate。

这些是不可逆拒绝依据，不能通过复制材料、更换 listener 名称或恢复配置绕过。秘密指纹不对外公开，完整材料值仍在受保护的配置历史中。Node 身份吊销独立由 nodes／node_keys 负责。

## DatasetArtifact

`dataset_artifacts` 保存 id、可空 network_id、kind、source、format、data、digest、created_at。kind 为 ruleSet／geoip／asn；规范化内容经过完整校验后不可变保存，原始下载为有界临时输入。

资源引用精确 ArtifactId，不能重新下载最新 URL 冒充历史内容。来源与摘要不包含凭据。

### RuleSetCheck

`rule_set_checks` 以 network_id、revision、rule_set_key 为键，保存 problem、checked_at、last_success_at、failures、check_version。failures 有界，check_version 是内部 CAS，避免依赖毫秒时间戳唯一性。

检查记录拥有刷新结果与退避，不拥有当前内容；实际 artifact 绑定来自配置／执行发布。失败保留最后成功内容和成功时间，validate 不保存检查结果。

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

## ExecutionPublication

`execution_publications`：

| 字段 | 含义 |
| --- | --- |
| `id`／`network_id`／`sequence` | 不可变发布 ID、网络及单调顺序 |
| `revision`／`cause` | 源配置与 configuration／demand／refresh 来源 |
| `supersedes` | 更早的被替代发布 |
| `rule_set_bindings` | 本次精确 artifact map |
| `dependency_digest` | 可信编译输入摘要 |
| `payload` | 加密完整发布内容，包含实际节点切片 |
| `created_at`／`withdrawn_at` | 创建与可空撤回时间 |

同 revision 的新发布可改变动态依赖，不覆盖原 publication ID 下的内容。重启还原确切绑定；历史恢复使用源初始绑定或明确的同源历史发布，不猜用最新数据。

### ExecutionReceipt

`execution_receipts` 保存 network_id、publication_id、node_id、instance_id、generation、attempt、sequence、digest、phase、reported_at、received_at、failure。

主键固定网络／发布／节点／实例／attempt／sequence。phase 为 saved／applied／failed，failure 为明确组件类别或 null。applied 须有同次尝试更早的 saved；重复位置必须同内容。历史回执不等于当前 freshness 或连通。

## NodeExecution

`node_execution` 保存 network_id、node_id、instance_id、generation、boot_id、instance_expires_at、target_publication_id、execution_expires_at、capabilities。

实例期限、执行期限与 State 联系新鲜度分开。capabilities 来自已认证 acquire；普通 State 不自行替换执行能力。`node_instance_claims` 保存 network_id、node_id、instance_id、generation，拒绝释放／过期后的实例 ID 重新认领。

### Demand 与 RoutingSelection

| 表 | 字段与用途 |
| --- | --- |
| `execution_demands` | network_id、node_id、id、instance_id、request_digest、payload、expires_at、cancelled_at；有界业务需求与重试绑定 |
| `routing_selections` | network_id、node_id、identity_digest、profile、version、selected_at；受当前身份与 allowed／locked 约束的用户选择 |

身份摘要不赋权，需求不绕过 access。重启恢复方案仍检查当前条件；实际方案由 NodeState 报告。

## ActivityEvent

`activity_events`：

| 字段 | 含义 |
| --- | --- |
| `sequence`／`id` | 内部提交顺序和公开 ActivityId |
| `scope`／`account_user_id`／`network_id` | account 或 network 作用域 |
| `at`／`actor_kind`／`actor_user_id`／`actor_email` | 完成时间与真实 user／system 主体，邮箱为历史快照 |
| `action` | 固定管理动作枚举 |
| `target_user_id`／`target_session_id`／`target_membership_id`／`target_join_link_id` | 按动作约束的目标关联 |
| `before_revision`／`after_revision`／`request_id` | 配置变更与请求关联 |
| `previous_email` | 仅 account.email 允许非空：修改前的地址快照 |

记录与对应成功动作同事务提交；幂等恢复不重复插入。不包含秘密、完整 diff 或原始请求 body。Node 应用结果在 execution_receipts，不能改写“配置已保存”的活动事实。HTTP 范围见[版本与操作历史](/v2/api/history)。

## ObservationEvent

`observation_events` 保存 row_id、network_id、source_node_id、instance_id、event_id、occurred_at、received_at、payload、payload_digest、admission_expires_at。

同网／来源 Node／eventId 唯一；payload 包含真实事件及配置上下文。正文裁剪后仍保留主键和摘要至完整接纳窗口结束，重试不恢复已裁剪正文。接纳窗口包含允许时钟偏差，不能因墓碑先清理而再次接纳旧点。

### HistoryCoverage

`history_coverage` 保存 network_id、source_node_id、before_at、reason，reason 为 retention／capacity。它记录真实裁剪下界，不能由最早现存样本推断。

collection_gap 则来自 Node 真实事件，保留原始区间和 dropped，不按查询裁剪比例伪造数量。两类覆盖信息与查询权限一起过滤，见[观测历史](/v2/state-reference/observations)。

## DiagnosticTask 与 Subscription

| 表 | 字段与用途 |
| --- | --- |
| `diagnostic_tasks` | id、network_id、node_id、instance_id、boot_id、revision、reuse_key、payload、dirty；固定执行上下文和共享任务 |
| `diagnostic_subscriptions` | id、task_id、session_id、membership_id、request_digest、expires_at、cancelled；每个页面需求的独立权限与短租约 |

会话、账户、membership 或配置失效在对应事务取消订阅。维护器撤回诊断发布；已安装许可另受本地短期限约束，Node 普通 renew 不能续活观察。

## MasterKeyBinding

`master_key_binding` 保存 singleton、key_id、key_check，绑定私有主密钥且不可修改／删除。私有密钥文件先持久发布，再与数据库绑定提交；丢失密钥不能生成替代来解密原库。

SQL 行包含私有字段，不能整体作为 Console DTO。完整配置、永久吊销与有效恢复依据不随内存缓存或短期任务清理。
