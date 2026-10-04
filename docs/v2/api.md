# API 设计

**V2 操作索引及交付范围。** `/api/v1` 面向 Console，`/agent-api/v1` 面向 Node。下面列出完整目标操作面；已实现的切片见本节，其他条目仍为设计，具体字段和错误以 Rust 契约／实际路由生成物为准。

## 当前实现切片

新 `apps/meshway-controller` 的隔离开发基线已提供账户与管理／配置 JSON 操作；基础账户与网络入口如下，完整操作以生成 OpenAPI／SDK 为准：

| operationId | 方法与路径 | 成功响应 |
| --- | --- | --- |
| `getAuthOptions` | `GET /api/v1/auth/options` | 200 AuthOptionsResponse |
| `register` | `POST /api/v1/auth/register` | 201 SessionIssuedResponse |
| `login` | `POST /api/v1/auth/login` | 200 SessionIssuedResponse |
| `getAuthSession` | `GET /api/v1/auth/session` | 200 AuthSessionResponse |
| `logout` | `POST /api/v1/auth/logout` | 204 |
| `createNetwork` | `POST /api/v1/networks` | 201 NetworkResponse |
| `listNetworks` | `GET /api/v1/networks` | 200 NetworkListResponse |
| `getNetwork` | `GET /api/v1/networks/{networkId}` | 200 NetworkResponse |
| `renameNetwork` | `POST /api/v1/networks/{networkId}/rename` | 200 NetworkResponse |
| `listNetworkMembers` | `GET /api/v1/networks/{networkId}/members` | 200 NetworkMembersResponse |

这些操作已接真实账户、网络和成员关系存储。建网同事务保存 Owner、完整加密初始配置和操作记录；初始 Mesh 地址池／DNS 后缀只保存在配置中。改名使用 expectedName 校验，不改变配置 revision；分页 limit 为 1–200，cursor 绑定用户／范围、创建序号上界和 15 分钟期限，每页重检当前权限。成员管理列表当前仅 Owner／Admin 可读。

管理／配置候选现已扩展真实 HTTP／SQLite 操作：账户读取、邮箱／密码修改、会话分页及撤销、主机 CLI 密码恢复、角色变更、一次性加入链接、无 Node 依赖的移除／离网、配置普通视图与显式秘密 source、JSONC／强类型编辑校验及 CAS 保存、版本分页／对照／恢复、账户／网络活动、配置 schema 与承载目录。名称规则通过 `getAuthOptions.networkNameRules` 发布；成员 actions 由当前权限推导，网络／成员列表 total 在同一查询事务内按创建上界和当前权限计算。邮箱同步更新可读引用但保留稳定主体绑定；恢复历史版本不会把旧邮箱绑定到同名新账户。

Node 控制流程已整合真实 HTTP／SQLite：签名接入、PKCE 批准与兑换、接入密钥、持钥会话与轮换恢复、实例／lease、完整执行目标与增量、demand、saved／applied／failed 回执、永久删除及成员删除联动。需求重试不重复扰动无关节点，回执按尝试／序号处理乱序；节点身份与策略引用使用独立的稳定绑定。当前生成物包含 72 个路径、77 个操作，精确方法、输入与错误以实际路由导出的 OpenAPI 为准。

State 查询／Node 上报、历史接纳／查询、按 revision 的配置进度，以及用户／Node SSE 已接入正常 Router 和真实权限 provider。固定 apply 目标随配置版本保存；配置、权限与进度变化在已接纳 SQLite job 的提交后进入统一 journal。用户首次完整同步及后续增量经过权限裁剪，正常配置变化保持通知连续，失权、期限和重放断档分别处理。

主动诊断需求合成及其 HTTP 操作、远端规则集绑定、可信 DNS／GeoIP 数据输入与自动方案仍待接通。`reassignAddresses`／`rotateMaterial` 与完整预览计划尚未成为可用 HTTP 操作；地址退出安全下界仍需实际 Agent 证据。Controller HTTP／SSE 与持钥测试 Node 的验收不等于 Agent 安装、TUN 或真实组网验收。

错误为 `ErrorResponse { requestId, status, problem: { code, params, message } }`；requestId 仅在其安全生成失败时为 null。请求和响应对象拒绝数组表示、重复字段和未知字段。

从该开发基线运行独立实例：

```sh
cargo run -p meshway-controller --bin meshway-controller --locked -- \
  --data-dir /absolute/private/data-dir \
  --listen 127.0.0.1:0 \
  --registration-enabled true \
  --session-ttl-seconds 86400
```

dataDir 使用无符号链接的绝对路径，父目录须存在；进程创建／核验私有目录和文件并独占锁。端口 0 由系统分配，启动输出实际监听地址。此命令运行新实例，部署切换和旧服务数据另行协调。

`export-openapi` binary 从同一份真实路由声明导出 `packages/protocol/openapi.json` 的内容，不启动数据库或服务。生成 SDK 的可用操作以这份产物为依据，不从本页目标索引推断尚未交付的接口。

上述 JSON／SSE 操作已统一生成 Rust／TypeScript 客户端和运行时校验 schema，包含 `watchNetwork`／`watchNode`、`getNetworkState`／`getNodeState`、`reportNodeState`／`queryPeerState`、`reportNodeHistory`／`queryObservationHistory`、`getConfigurationProgress`。SDK 用法见 `packages/control-client/README.md`；SSE 契约见[事件流](/v2/api/events)。

主机运维签发密码重置链接（先停止占有同一 dataDir 的 Controller）：

```sh
cargo run -p meshway-controller --bin reset-password --locked -- \
  --data-dir /absolute/private/data-dir \
  --email user@example.com \
  --console-origin https://console.example.com \
  --expires-in-seconds 900
```

命令显式输出一次性链接，普通日志及列表不返回原秘密。inspect／redeem 使用 `/api/v1/auth/password-reset/*`；兑换成功使该账户旧会话失效。Console 兑换页面由消费端实现，命令不证明该页面已部署。

## 基础规则

- 默认开放账户注册，可由部署配置关闭；不提供邀请注册。有效用户可创建网络，创建者成为本网 Owner。
- 已有用户用一次性加入链接入网；密码恢复由主机运维 CLI 签发链接。Node 删除永久生效，不提供恢复。
- 用户使用不透明会话，Node 使用独立持久密钥身份。Node 的网络／自身 ID 从认证绑定推导。
- Console 基于同一草稿／CAS 一次提交，Node 正常拉取、保存、应用并回报；不用两阶段发布。
- 首屏 SSE 同步当前可见 Config／State，后续推送变化；历史和秘密材料按需读取。
- Node／User 身份、Config、实际 State 和操作／测量历史各有来源，列表或成功 ACK 不替代执行证据。

Controller 正常启动初始化空库所需结构和私有材料，随后走普通注册／建网。没有实例初始化或首用户认领 API；未知／损坏数据库仍报错。网络角色不授予全局账户管理或实例运维权限。

## Console API

以下表格省略 `/api/v1`。`{networkId}` 均逐请求／事务检查权限；Owner／Admin 的具体角色权限仍按[权限规则](/v2/permissions)，不是统一的跨网管理员。

### 账户与会话

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `GET /auth/options` | 匿名 | 注册开关、密码规则和公开登录选项 |
| `POST /auth/register` | 匿名 | 邮箱／密码注册，返回用户会话 |
| `POST /auth/login` | 匿名 | 登录，返回不透明会话 |
| `GET /auth/session` | 用户 | 当前有效身份与会话期限 |
| `POST /auth/logout` | 用户 | 撤销当前会话及其活动订阅 |
| `GET /account` | 用户本人 | 账户信息 |
| `POST /account/email` | 用户本人 | 修改邮箱，同步稳定用户引用；需本人凭据验证 |
| `POST /account/password` | 用户本人 | 修改密码并执行既定会话撤销 |
| `GET /account/sessions` | 用户本人 | 会话元数据列表，使用非秘密 sessionId |
| `POST /account/sessions/{sessionId}/revoke` | 用户本人 | 撤销自己的某个会话 |
| `POST /account/sessions/revoke-others` | 用户本人 | 撤销除本次请求会话以外的会话 |
| `POST /account/sessions/revoke-all` | 用户本人 | 撤销自己的全部会话 |
| `GET /account/activity` | 用户本人 | 全局账户操作历史 |
| `POST /auth/password-reset/redeem` | 一次性恢复凭据 | 运维 CLI 生成的重置链接兑换，检查账户版本／期限并撤销旧会话 |
| `POST /auth/password-reset/inspect` | 一次性恢复凭据 | 不消费地核对重置链接状态；不返回额外账户秘密 |

session token／摘要不进入列表、URL 或活动记录。会话列表返回非秘密 sessionId、创建／最近使用／到期时间、current 与粗粒度客户端摘要；recent use 没有可信记录时为 null。账户恢复已采用 CLI 签发，停用／注销入口仍待定；网络 Owner 不能重置别人的全局账户。

当前 `lastUsedAt` 在已验证会话的事务内记录，并以 30 秒粒度合并元数据写入；每次请求仍重检真实会话与权限，不延长 expiresAt。`clientLabel` 来自注册／登录请求的 User-Agent 粗粒度产品／平台摘要，不存原始 header，不作为身份依据；缺失或无法识别时为 null。

当前密码错误为 `400 invalid_current_password`，不会撤销仍有效的会话；会话失效继续为 `401 invalid_session`。各分页入口对不可用 cursor 统一返回 `400 invalid_cursor`。重置凭据错误均为 400：无法验证为 `invalid_password_reset`；仅在匹配真实凭据后区分 `password_reset_used`、`password_reset_expired`、`password_reset_replaced`（包含凭据代际已更新）。

### 网络与成员

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `GET /networks` | 用户 | 自己的 networkId、name、role、createdAt 与 actions |
| `POST /networks` | 用户 | 幂等创建网络、Owner membership 与初始配置 |
| `GET /networks/{networkId}` | 成员 | 网络元数据及当前角色 |
| `POST /networks/{networkId}/rename` | 网络管理权限 | 修改网络实体显示名 |
| `GET /networks/{networkId}/members` | 网络成员视图权限 | 成员列表 |
| `POST /networks/{networkId}/members/{membershipId}/role` | 角色管理权限 | 调整角色，保护最后 Owner |
| `POST /networks/{networkId}/members/{membershipId}/remove` | 成员管理权限 | 撤销 membership 并永久删除其当前 User 身份设备，Tag 设备保留 |
| `POST /networks/{networkId}/leave` | 成员本人 | 离网并永久删除本人的 User 身份设备；提供本人 removalImpact，保护最后 Owner |
| `GET /networks/{networkId}/join-links` | 成员管理权限 | 加入链接元数据、固定角色／期限／消费状态 |
| `POST /networks/{networkId}/join-links` | 成员管理权限 | role、expiresIn、requestId，首次返回一次性原值 |
| `POST /networks/{networkId}/join-links/{linkId}/revoke` | 成员管理权限 | 撤销未消费链接；不移除已加入成员 |
| `POST /join-links/inspect` | 已登录用户＋链接凭据 | 接受前读取链接授予的网络／角色／期限，不消费 |
| `POST /join-links/redeem` | 已登录用户＋链接凭据 | 消费链接并建立当前用户的 membership |

成员修改使用 expectedVersion 防止覆盖并发操作；列表返回 membershipId、userId、email、role、joinedAt、version 与 actions。有权移除者取得当前 User 身份设备的 removalImpact／nodeCount。移除／离开同事务撤销 membership 并永久删除其当前 User 身份节点，Tag 节点保留；只有实际配置改变才推进 revision。全局 User 不删除，重新加入不恢复旧关系／Node。网络删除另有生命周期。

### 节点与接入

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `GET /networks/{networkId}/nodes` | 当前可见范围 | 节点列表，组合实体／配置摘要与有来源的 State |
| `GET /networks/{networkId}/nodes/{nodeId}` | 当前可见范围 | 节点详情及允许的操作提示 |
| `GET /networks/{networkId}/nodes/{nodeId}/configuration` | 对应配置视图权限 | 当前非秘密 NodeSettings 视图；表单保持／覆盖秘密，不读取旧值 |
| `GET /networks/{networkId}/nodes/{nodeId}/keys` | 对应身份元数据权限 | 公钥用途、代际、轮换／吊销状态 |
| `POST /networks/{networkId}/nodes/{nodeId}/delete` | 网络管理权限 | expectedRevision＋requestId；永久吊销并移除配置，同事务保留退出／历史依据 |
| `POST /networks/{networkId}/nodes/{nodeId}/keys/{keyId}/revoke` | 网络管理权限 | 吊销指定公钥绑定；不能上传私钥或任意替换绑定 |
| `GET /networks/{networkId}/enrollment-keys` | Owner／Admin | 接入密钥元数据、期限、Tag／初始设置摘要与消费结果 |
| `POST /networks/{networkId}/enrollment-keys` | Owner／Admin | tags、expiresIn、initialSettings、requestId；一次性交付密钥，支持 hostname／可选 listener |
| `POST /networks/{networkId}/enrollment-keys/{keyId}/revoke` | Owner／Admin | 撤销未消费密钥，消费竞态在事务中处理 |
| `GET /device-authorizations/{requestId}` | 用户会话 | 浏览器设备批准页所需的非秘密请求信息 |
| `POST /device-authorizations/{requestId}/approve` | 用户会话 | 选择有效网络，绑定本人设备批准 |
| `POST /device-authorizations/{requestId}/deny` | 用户会话 | 拒绝本次请求，不创建 Node |

改名、listener、出站、出口、identity、addresses 等通过统一 configuration 的[强类型 ops](/v2/configuration-reference/editing)修改，不建立第二份 Node 设置源或重复管理写入路径。Tag→User／改绑 User 通过绑定当前 Node 的设备重新登录批准；完整配置和 ops 都不能绕过。

删除 Node 不可恢复；最终 JSONC／ops 移除已有 nodes[id] 具有相同语义，应用对照列出永久删除项。接口／历史可保留最小墓碑，不能重新登录恢复旧 Node。读取同时区分缺配置项、凭据已吊销、尚无报告，不能统一显示为空或离线。

接入密钥原值只在创建结果展示，列表只返回元数据；已消费密钥的状态不等于 Node 仍有效，Node 吊销另行处理。初始 listener 模板在消费事务写入真实 Node 配置，不要求用户在设备上手工补业务参数。

### 配置编辑、历史与发布

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `GET /networks/{networkId}/configuration` | 配置视图权限 | 当前配置视图与期望 revision |
| `GET /networks/{networkId}/configuration/source` | Owner／Admin | 显式读取当前／指定版本原始 JSONC 与完整管理配置，含获准材料 |
| `POST /networks/{networkId}/configuration` | 按输入／逐操作授权 | JSONC／强类型 ops／restoreFrom＋expectedRevision＋requestId，保存并正常 apply |
| `POST /networks/{networkId}/configuration/validate` | 对应输入的操作权限 | 同样的草稿输入，返回规范化脱敏 preview、修改数量、对照摘要、影响范围和诊断 |
| `GET /networks/{networkId}/configuration/revisions` | Owner／Admin | 固定上界的版本摘要列表 |
| `GET /networks/{networkId}/configuration/revisions/{revision}` | Owner／Admin | 该历史配置视图与元数据 |
| `GET /networks/{networkId}/configuration/revisions/{revision}/progress` | 当前可见范围 | 该次应用按 NodeId 去重的目标／结果与 full／partial 汇总 |
| `POST /networks/{networkId}/configuration/compare` | Owner／Admin | 两个明确版本的字段／原文／绑定变化摘要 |
| `GET /networks/{networkId}/publications` | 当前可见范围 | 按源 revision／原因／节点过滤的执行发布列表 |
| `GET /networks/{networkId}/publications/{publicationId}` | 当前可见范围 | 目标、逐节点应用结果、时间与新鲜度 |
| `GET /networks/{networkId}/nodes/{nodeId}/execution` | 当前可见范围 | 非秘密执行摘要：生成候选／路径、来源、publication／digest |

validate 是应用前只读预览，包含 reassignAddresses／rotateMaterial 的预期影响，不引入待激活版本。每用户每网络一份草稿，最终显示有效修改数量和前后对照再应用，严格 CAS，不自动 rebase。详细[编辑合同](/v2/configuration-reference/editing)和[版本／恢复合同](/v2/api/history)。

提交结果至少返回 revision、publicationIds、targetCount、实际 allocations；空 publicationIds 且 targetCount=0 才表示无需节点部署，离线节点不是无目标。公开／完整管理／Schema 三种视图类型分开，秘密只在显式管理读取和私有编辑内存中出现。

execution 返回 Controller 实际编译结果的裁剪视图，不拿手工源候选冒充自动邻接；其中计划候选也不能计为已建立 link。Web 不使用 Node 凭据调用 Agent 的秘密执行配置接口。

### State、路由方案与观测

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `GET /networks/{networkId}/state` | 当前可见范围 | 单次最新网络 State，可按节点／字段缩小 |
| `GET /networks/{networkId}/nodes/{nodeId}/state` | 当前可见范围 | 节点状态、socket／endpoint、GeoIP／ASN、实际 links／paths |
| `GET /networks/{networkId}/events` | 当前可见范围 | 首次 full sync 与后续 SSE |
| `POST /networks/{networkId}/configuration/evaluate` | 预览权限 | 对当前配置计算授权／命中规则／规划路径；草稿预览须管理权限 |
| `GET /networks/{networkId}/nodes/{nodeId}/routing-profiles` | 当前可见范围 | 该节点允许方案、默认／locked、请求／实际选择及自动地区依据 |
| `POST /networks/{networkId}/nodes/{nodeId}/routing-selection` | 该设备的使用选择权限 | 选择允许方案，检查身份及选择版本，不修改管理员配置历史 |
| `GET /networks/{networkId}/observations` | 获准观测范围 | 当前观察需求／任务及实际状态 |
| `POST /networks/{networkId}/observations` | 获准诊断权限 | auto 按目标选择获准路径／explicit 指定路径；返回租约和续租节奏 |
| `GET /networks/{networkId}/observations/{subscriptionId}` | 获准观测范围 | 独立订阅对应的任务参数、lease、状态及最新结果 |
| `POST /networks/{networkId}/observations/{subscriptionId}/renew` | 订阅使用者 | 检查当前权限并续租本页需求 |
| `POST /networks/{networkId}/observations/{subscriptionId}/cancel` | 订阅使用者 | 幂等取消本页需求，不影响同会话其他标签页或其他用户 |
| `POST /networks/{networkId}/history/query` | 相关历史读取权限 | kinds／时间窗口／源与对端／link／原始或分桶查询，返回真实样本与覆盖缺口 |
| `GET /networks/{networkId}/activity` | Owner／Admin | 控制面管理操作历史 |
| `GET /networks/{networkId}/activity/{activityId}` | Owner／Admin | 单条操作及安全的目标／revision 关联 |

GeoIP／ASN 随[派生 State](/v2/state-reference/ip-info)提供，不通过任意 IP 查询扩大节点可见范围。evaluate 不发探测、不授予业务权限；实时测量仍用 observation。返回当前用户适用的诊断预算，超限使用 observation_limit；renewAfter 由服务端给出，后台页面不续租。全局账户操作只在本人 account/activity 中查看。

### 目录、规则集与接入下载

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `GET /transport-catalog` | 用户 | Controller 实际可托管的协议／组合、字段 schema、默认规则与能力要求 |
| `GET /configuration/schema` | 用户 | 当前配置 schema／格式版本，由 Rust 生成 |
| `GET /networks/{networkId}/rule-sets` | 相关视图权限 | 配置定义＋当前有效 artifact／更新结果摘要 |
| `GET /networks/{networkId}/rule-sets/{ruleSetKey}` | 相关视图权限 | 来源、条目数、转换问题、数据版本与当前状态 |
| `POST /networks/{networkId}/rule-sets/{ruleSetKey}/refresh` | Owner／Admin | 有界下载／校验更新，状态由 State／查询返回 |
| `GET /agent-releases` | 公开发布元数据 | 兼容的 Agent 版本、平台／架构、校验和与下载地址 |

规则集定义仍通过 configuration 编辑；refresh 固定成功 artifact 并触发普通执行更新，不让 Node 下载数据集。目录只声明 Controller 可托管项，Node 是否实际编译支持来自当前能力报告；兼容状态由 Controller 按声明规则计算，不能仅比较版本字符串。

安装脚本 `/install.sh` 和版本化二进制下载属于同源静态交付路径，不是管理 mutation。接入凭据通过受保护输入交给 Agent，不放 URL／普通日志／服务配置；下载、幂等安装和兼容检查随发布流水线定版。

可复制的一行命令只包含公开安装信息，不嵌入密钥；接入时在隐藏输入提示符或受保护 stdin 输入，不能以 `sudo KEY=secret ...` 让原值进入 argv／shell 历史。initialSettings.hostname 是首次接入的名称建议，冲突时规范化并加唯一后缀，返回实际名称；已有节点显式改名重名仍报 hostname_taken。

标准交互安装模板如下，替换为可信 Controller origin；这是待交付安装器的调用合同，不表示当前入口已可用：

```sh
curl -fsSL https://controller.example/install.sh | sudo sh
```

该命令只下载并安装公开程序。接入使用后续系统浏览器登录，或从 `/dev/tty` 隐藏读取接入密钥；不能从正在传输脚本的 stdin 混读秘密。无终端时只完成安装，自动化通过独立的受保护凭据输入接入，不回退到 argv、shell 历史或 systemd 环境文件携带原值。

## Agent API

以下路径省略 `/agent-api/v1`。已接入接口从凭据推导自身网络与 Node；临时接入接口只有本次请求权限。普通拉取／安装与精确执行 lease 保持一致。

| 方法与路径提案 | 主体 | 用途 |
| --- | --- | --- |
| `POST /enrollment-requests` | 接入中的设备 | 创建绑定公钥、PKCE 和返回方式的请求 |
| `GET /enrollment-requests/{requestId}` | 本次持钥／领取证明 | 读取批准／拒绝／期限与受保护结果；知道 ID 不够 |
| `POST /enrollment-requests/{requestId}/cancel` | 本次持钥／领取证明 | 取消未消费请求，不能当作撤销已注册 Node |
| `POST /enrollments` | 持钥证明＋一次性批准／接入密钥 | 原子创建／恢复 Node 与配置 identity／地址 |
| `POST /instances` | Node | 获得／恢复当前运行实例并隔离旧代际 |
| `POST /instances/renew` | Node＋当前实例 | 验证当前资格后续执行 lease；SSE 保活不代替 |
| `POST /instances/release` | Node＋当前实例 | 主动结束本实例；不能停止后来替代它的新实例 |
| `GET /configuration` | Node | 当前有效完整配置／delta 与执行上下文 |
| `POST /configuration/receipts` | Node＋当前实例 | saved／applied／failed 类型化回执 |
| `POST /state` | Node＋当前实例 | 自身完整 State 上报 |
| `POST /state/query` | Node | 获准执行依赖的裁剪 State |
| `GET /events` | Node＋当前实例 | 配置可用通知、相关 State 变化与有界任务通知；可重取当前内容 |
| `POST /history` | Node＋来源实例 | 批量真实事件，验证上下文与接纳窗口、幂等去重 |
| `POST /demands` | Node | 请求获准业务目标所需的候选／完整路径依赖 |
| `GET /tasks` | Node＋当前实例 | 本节点获准、未过期的类型化诊断／协调任务 |
| `POST /tasks/{taskId}/results` | Node＋任务绑定实例 | 报告实际任务结果，不自行扩大 source／path |
| `GET /routing-profiles` | Node | 自己允许使用的方案、锁定和选择依据 |
| `POST /routing-selection` | Node | 在允许范围内提交使用选择，受选择版本约束 |

事件流与 query 使用同一权限／reset 规则；配置通知不带新的隐式授权，实际执行仍读取并验证完整快照。任务是封闭类型，不是任意远程命令。恢复／回调中的秘密不进入普通 State 或日志。

Node 持钥认证握手、密钥轮换／再认证的 proof 编码仍待技术定版，随后补对应操作；不能以一个“上传新公钥”JSON 接口替代所有权证明。设备重新登录转换 identity 复用同一接入授权服务，并明确绑定已有 Node。

## 通用合同

### 分页与新鲜度

列表使用有界 limit＋不透明 cursor，返回 items／nextCursor／asOf；默认与上限写入生成 schema。cursor 绑定身份、网络、筛选、排序和一致性截点；每次续页重新鉴权。

配置列表固定 revision，历史固定读取上界；最新 State 按观测／实例／期限计算，不能把动态分页当成全网同时采样。空、缺测、过期、不适用分别表达，详见[历史分页](/v2/api/history#分页与保留)。

### 错误与编辑

字段统一 camelCase。ErrorResponse／配置诊断／StateProblem 共享 code／类型化 params／安全 message 核心，保留 requestId、severity、资源／观测时间等各自上下文。SDK 校验错误 body，失败时仅保留 HTTP status，不展示原始秘密响应；首批 snake_case code 见[Actions 与 Problem](/v2/api/problems)。

配置 mutation 使用 expectedRevision，使用选择／成员修改使用对应并发基准。只读 validate／evaluate 不生成持久材料、不申请执行许可；最终 mutation 仍重新校验全部当前输入。服务端 actions 驱动 UI，不能替代实时授权。创建 requestId 只保证幂等，秘密重试交付按[认证合同](/v2/api/authentication#创建请求的幂等与秘密交付)。

## 尚需定版的产品／技术项

- 账户停用／注销，以及网络删除后的保留／恢复；加入链接、CLI 密码恢复和不可恢复的 Node 删除已经确定。
- Node 持钥握手、换钥与重新登录的具体编码及密钥代际转换。
- 多公网观测的节点位置优选、GeoIP／ASN 数据源和更新参数；未确定时明确未知／冲突。
- 操作历史保留参数、额外失败尝试记录范围；不自动新增全局用户审计或匿名请求日志 API。

这些待定项不影响先补已清楚的 revision／activity、State 元数据、只读目录和生命周期操作草案，但相应接口不能提前宣称定版。

## 契约与生成

Rust DTO、真实路由、认证和流式声明是权威。当前自有生成链尚需补 query/header 参数、SSE、有界 number（经纬度）、类型化错误以及新 Node 认证接线；不手抄 endpoint／DTO，不直接编辑生成文件。

进一步阅读：[注册与认证](/v2/api/authentication)、[SSE](/v2/api/events)、[历史](/v2/api/history)、[GeoIP／ASN](/v2/state-reference/ip-info)、[数据库字段](/v2/storage/database)。
