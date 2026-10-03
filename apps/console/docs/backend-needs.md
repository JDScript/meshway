# Console 对后端的接口需求

本页由 console 工作流维护，列出控制台需要、但后端尚未提供的能力。每项注明状态：

- **已确认**：用户已经确认的产品需求。
- **提议**：console 提出的方案，后端可以调整形状，但语义边界须保留。

本页不是已实现的合同。字段名只是示意，精确类型以 Rust 契约生成的 OpenAPI 为准。

所有项共同遵守的约束：

- 管理接口挂在 `/api/v1/networks/{network_id}` 下，在同一事务内检查会话身份和网络角色。
- 响应带生成的运行时 schema；不出现 `any` 或宽泛的 JSON。
- 观测类数据都要带来源、观测时间和过期规则。缺测、过期、不适用是三种不同的状态，不能补成零或 false。
- 现有成功响应的 DTO 是严格的，对新增字段敏感。新增字段要和 Apple core、Agent 等消费者协调升级，或者另开新操作。

## 优先顺序

| 顺序 | 项目 | 状态 |
| --- | --- | --- |
| 1 | [节点位置](#1-节点位置) | 已确认 |
| 1 | [接入密钥管理](#2-接入密钥管理) | 部分已确认 |
| 1 | [节点信息补全](#3-节点信息补全) | 提议 |
| 1 | [错误码与诊断码](#4-错误码与诊断码) | 提议（spa-api-audit 已列为待演进） |
| 2 | [承载目录与节点能力](#5-承载目录与节点能力) | 已确认 |
| 2 | [VLESS 证书绑定状态](#6-vless-证书绑定状态) | 已确认 |
| 2 | [一行安装脚本](#7-一行安装脚本) | 已确认 |
| 2 | [策略验证](#8-策略验证) | 提议 |
| 3 | [登录会话清单](#9-登录会话清单) | 提议 |
| 3 | [变更记录](#10-变更记录) | 提议 |
| 随主线 | [连接实情](#11-随主线推进) 与 [出口](#11-随主线推进) | 已确认 |

顺序 1 的几项后端改动小，console 收益最直接。

## 1. 节点位置

**现状**
- 后端没有位置来源，真实网络里所有节点都显示"待定位"，概览地球上没有点。
- `NodeSummary.country` 是旧的管理员手填值，不能当作实测位置。

**需求**
- 用节点上报的底层公网 IP 观测（现有 STUN / endpoint discovery），在控制面做 GeoIP，推导出节点位置。
- 每个节点返回：
  - `location`：
    - `country`（ISO 3166-1 alpha-2）、`region`（可空）、`city`（可空）；
    - `latitude` / `longitude`，精确到城市级即可；
    - `source`：观测的 socket / resource，以及所用的公网 IP 族；
    - `observed_at`、`expires_at`；
    - `geoip_dataset`：GeoIP 数据集版本。
  - 没有位置时返回 `null`，并在 `location_absent` 里给出原因：`no_observation`、`observation_stale` 或 `not_in_dataset`。
- **禁止作为来源**：控制面连接的来源 IP、Mesh IP、所选出口的 IP、管理员填写的 country。
- 同一份结果也是"自动"路由方案判断设备所在地区的输入，两处不得各算一份。

**console 用法**
- 节点在地球上落点。
- 节点详情里显示位置的来源、观测时间和数据集版本。

**清理**
- 位置上线后，删除 `setNodeCountry` 和 `NodeSummary.country`。

## 2. 接入密钥管理

**现状**
- `createEnrollmentToken` / `createManagedEnrollmentToken` 只在创建时返回一次原值。
- 发出之后既不能列出，也不能撤销。
- 不能预先指定标签。

**需求**
- `GET /enrollment-tokens`：列出密钥，只给元数据，永不返回原值。字段：
  - `token_id`、`created_at`、`created_by`（用户 ID 和邮箱）、`expires_at`；
  - `state`：`pending`、`consumed`、`expired` 或 `revoked`；
  - `consumed_by_node_id` 与 `consumed_at`；
  - `tags`；
  - `initial_listener` 摘要：只有 transport、id、bind 端口，不含材料。
- `POST /enrollment-tokens/revoke {token_id}`：只对 `pending` 有效。
  - 撤销已消费的密钥不影响已经接入的节点（撤销节点另走 `revokeNode`）。
  - 和消费并发时，二者只能有一个成功。
- 创建时可选 `tags`。**已确认**：服务器类设备用预分配标签的密钥接入。
  - 标签必须属于本网络的标签目录。
  - 消费时在注册事务内写入标签分配。
- 列表保留已过期和已消费的记录，保留期由后端定，至少覆盖最近若干天，便于核对"这个节点是用哪把密钥接入的"。
- 权限：Admin / Owner 可以列出和撤销；Member 不可以。

**console 用法**
- 节点页加一个"接入密钥"分段。
- 添加节点之后，页面能看到密钥从"待使用"变成"已接入 → 节点"。

**不做**：可重复使用的密钥和临时节点（ephemeral）。

## 3. 节点信息补全

**现状**
- `NodeSummary` 只有 `name`（来自 Agent）、`agent_version`、两个 revision 和时间。
- 管理端读不到 Agent 实际编译支持哪些承载。
- 也读不到 Mesh 执行合同的 ACK。

**需求**
- **显示名**：管理员可以设置 `display_name`；未设置时用 Agent 上报的 `name`。
  - 走 `POST /nodes/rename {node_id, display_name | null, expected_revision}`，带 CAS。
  - 显示名在网络内唯一，后续 MagicDNS 风格的名字也会依赖这一点。
- **平台信息**：注册和心跳时上报 `os`、`arch`、`hostname`、`kernel`（可选），每个字段都带上报时间。
- **版本状态**：在 `agent_version` 之外给出 `version_status`，取值 `current`、`outdated` 或 `unsupported`。判断依据是控制面已知的兼容门槛，不在前端比较版本字符串。
- **两种 ACK 分开**：
  - 配置保存 ACK：现有的 `acknowledged_revision`。
  - Mesh 执行 ACK：节点已确认的执行 epoch / revision 和确认时间。
  - 两者都不等于数据面已经生效，console 也不会这样标注。
- **能力**：见第 5 项的逐节点能力。

## 4. 错误码与诊断码

**现状**
- `ErrorResponse` 只有 `message`，而且生成的 SDK 会丢掉错误 body。
- 所以 console 只能按 HTTP 状态码给出笼统提示（`lib/failure.ts`）。
- `PolicyDiagnostic` 的 `message` 是英文原文，console 只能原样显示。

**需求**
- `ErrorResponse` 增加：
  - `code`：稳定的 snake_case 枚举，按操作列出；
  - `field`：可空，JSON Pointer 形式的字段路径；
  - `params`：可空，带类型的参数，例如冲突的 listener ID 或最低版本号。
- 生成的 SDK 用生成 schema 校验错误 body，把 `code` / `field` / `params` 交给调用方。body 校验失败时仍按状态码处理。
- `PolicyDiagnostic` 和 `ConnectionDiagnostic` 增加 `code` 和 `params`。`message` 保留，作为后备文本。
- `message` 不回显用户输入的秘密，也不回显服务器内部错误。
- 首批需要的 code，按 console 现在笼统处理的场景排列：
  - CAS 过期：`revision_conflict`，`params` 带当前 revision。
  - 最后一个 Owner 不能降级或离开。
  - 邀请或重置已被使用 / 已过期 / 已撤销，三种分开。
  - 密码不符合规则。
  - listener ID 冲突，或企图换绑 owner / transport。
  - bind / endpoint / SNI / WS 组合无效，带 `field`。
  - 策略含未支持的出口内容（目前是一条英文 422）。
  - 标签不存在，或标签仍被策略引用。
  - 请求过大、请求太频繁。

**console 用法**
- 中文 / 英文精确提示。
- 出错字段就地标红；策略诊断按 code 翻译，并定位到对应规则。

## 5. 承载目录与节点能力

**已确认**：由控制面提供目录和逐节点能力，驱动 console 的 listener 表单，不再由前端手写预设。方向见 [承载扩展审查](../../../docs/transport-registry.md)。

**现状**
- 协议写死在 `ConnectionTransport`（`native_udp`、`native_tcp`、`vless`）和 `VlessStreamTransport`（`tcp`、`websocket`）里。
- console 在 `features/nodes/listener-model.ts` 里手写预设、名称和默认值。
- Agent 上报的 `publishManagedTransportCapabilities` 只能从每个 listener 的 `compiled_capability_available` 间接看到。

**需求**
- `GET /transport-catalog`：控制面可以托管的承载组合。每项包含：
  - 稳定 ID，例如 `vless/tcp+tls`、`vless/ws+tls`；
  - 外层 socket（`udp` / `tcp`）；
  - 是否需要 TLS；
  - 字段说明：每个字段的类型、约束、建议默认值，以及是否平台生成（平台生成的不展示）；
  - 合法组合规则；
  - 创建后不可更改的字段。
  
  目录不随某个 Agent 的 Cargo feature 变化。
- 逐节点能力：每个节点 Agent 本构建实际支持的承载 ID，以及能否监听、能否拨号。
  - 绑定到当前运行实例，带上报时间。
  - 失去能力时如实反映。
  - 可以放进节点详情，也可以单独做接口。
- 目录里只出现后端已经实现的组合。

**console 用法**
- 添加节点和 listener 表单只列出"目录里有、这个节点也支持"的组合。
- 不可用的项显示原因：Agent 未编译，或 Agent 版本过旧。
- 字段和默认值由目录给出。以后后端加了协议或字段，前端不需要改代码。

**不做**：REALITY、Vision、uTLS 指纹、ALPN、mux。console 只放后端能执行的字段，目录里没有的不出现在表单上。

## 6. VLESS 证书绑定状态

**已确认**：先不考虑 CDN；VLESS 要有自己的 TLS，并且做好证书绑定。

**现状**
- 平台为每个 listener 生成自签证书，拨号端按下发的信任根、SNI 和指纹精确校验。
- 材料加密存储，AAD 绑定网络 / listener / owner / 版本。
- 管理端只能看到 `certificate_sha256` 和 `material_version`。

**需求**
- listener 摘要增加证书的公有状态：
  - `not_before`、`not_after`；
  - SAN 列表；
  - 签名算法；
  - 绑定对象：listener ID、所属节点、材料版本。
  
  不返回 PEM 和私钥。
- 每个 listener 的执行状态里，分别报告"监听端已加载哪个证书指纹"和"拨号端按哪个指纹校验"。这是证书绑定在运行中真正生效的证据，和保存的配置分开。
- 轮换：`POST /managed-listeners/rotate-material {listener_id, expected_revision}`。
  - 生成新证书和新 UUID，推进 revision。
  - 旧代际通过既有 revision / lease 机制撤退。
- 到期前自动轮换的策略和提前量由后端定，并在摘要中给出 `renews_at`。

**console 用法**
- 节点承载页显示证书的有效期、绑定对象，以及监听端和拨号端的指纹是否一致。
- 提供"轮换证书"操作，带确认和影响说明。

## 7. 一行安装脚本

**已确认**：首版提供一行安装脚本，安装发布好的二进制和 systemd 服务。

**现状**：添加节点时给出的是开发命令 `cargo run … enroll --token=…`，令牌出现在命令行参数里。

**需求**
- 控制面同源提供：
  - `GET /install.sh`；
  - Agent 二进制：至少 Linux amd64 / arm64，带 SHA-256，版本与控制面兼容。
- 脚本负责：
  1. 检测架构，下载二进制并校验 SHA-256。
  2. 安装到固定路径，写入 systemd 单元和私有状态目录。
  3. 用接入密钥完成注册，启动服务。
  4. 重复运行时幂等：已注册就不重复注册，只做升级或重启。
- **令牌不进入进程参数**：脚本通过环境变量或 stdin 把令牌交给 Agent，Agent 的 `enroll` 相应支持这两种方式。也不写入日志和 systemd 单元。
- 脚本不修改宿主机的防火墙和 DNS，也不改控制面能力之外的系统配置。需要的权限（TUN、NET_ADMIN）写进 systemd 单元。
- 后续（已确认的链接登录）：不带密钥运行时，打印浏览器登录链接，复用设备授权服务。

**console 用法**
- 添加节点后显示一条可以复制的命令，例如 `curl -fsSL https://<controller>/install.sh | sudo MESHWAY_AUTH_KEY=… sh`。
- 同时显示密钥的有效期，以及第 2 项的接入状态。

## 8. 策略验证

**现状**：流量预览随旧策略一起删除，管理员没法回答"为什么 A 访问不到 B"。

**需求**
- `POST /policy/evaluate`，输入：
  - 可选的草稿 `policy` 加 `source_jsonc`；不带时使用已保存的策略，并返回所用的 revision。
  - `source`：节点 ID。
  - `destination`：节点 ID、Mesh IP、IP 或域名。
  - 可选的端口和协议。
- 返回：
  - `authorized`，以及命中的 grant 规则 ID 列表；
  - 连接层的结论和命中的规则 ID；
  - Mesh 路由命中的规则 ID 和规划的必经节点；
  - 规划路径（节点 ID 序列），其中每一段是"已允许的连接"还是"需要中转"；
  - 出口决策：目前固定为"未支持"；
  - 诊断。
- 结果明确标为规划，不是实测 trace。
- 权限：
  - 能读策略的成员可以验证已保存的策略。
  - 验证草稿需要 Admin / Owner，复用 32 KiB 的请求预算。
- 不定时轮询，由用户触发。

**console 用法**
- 策略页加"试一下"：选源和目标，给出结论和命中规则，点击可以跳到对应规则行。
- 规划路径可以画到地球上。

## 9. 登录会话清单

**现状**：只有 `revokeAccountSessions`（退出所有登录）。

**需求**
- `GET /auth/sessions`：当前用户的会话。字段：
  - 不透明的 `session_id`（不是令牌）；
  - `created_at`、`last_used_at`、`expires_at`；
  - `current`：是否为本次请求的会话；
  - 粗粒度的客户端描述：浏览器 / 系统，来自 User-Agent 摘要。
- `POST /auth/sessions/revoke {session_id}`：撤销单个会话。撤销当前会话等同于退出。
- 是否记录和展示登录 IP，需要确认隐私边界后再定。默认不展示。

## 10. 变更记录

**需求**
- `GET /audit`，游标分页。每条记录包含：
  - `id`、`at`；
  - `actor`：用户（ID 和邮箱）、实例管理员、节点或系统；
  - `action`：枚举，覆盖策略保存、成员和角色、邀请和重置、接入密钥、节点撤销 / 标签 / 改名、listener、网络设置；
  - `target`：类型和 ID；
  - `revision_before`、`revision_after`。
  
  记录中不含秘密、令牌原值或材料。
- 策略历史：
  - `GET /policy/revisions`：列出 revision、时间和操作者；
  - `GET /policy/revisions/{revision}`：返回当时的 `policy` 和 `source_jsonc`。
- **不需要专门的回滚接口**：console 把旧版本载入草稿，按正常 CAS 保存。
- 保留期和容量上限由后端定，被淘汰的部分如实标出。
- 权限：Admin / Owner 可读。是否开放给 Member 待定。

## 11. 随主线推进

这两项是已确认的需求，跟随后端主线推进，这里只列 console 需要的数据。

**连接实情**（自动建连 / 打洞主线）
- 每对节点：
  - 当前是直连还是中转，中转时经过哪个节点；
  - 建连方式：UDP 打洞、TCP 打洞、反向拨号或中继；
  - 承载、发起方、证据时间；
  - 没能直连的原因和最近的尝试记录。
- 有了它，概览地球的连线改从连接实情读取，不再从 `getConnections` 的配置推断。拨号候选及其优先级继续由 `getConnections` / `saveConnections` 管理；主备切换按候选优先级进行，不另设承载偏好。

**出口**（出口 / 域名分流主线）
- 节点的"可作为出口"开关。
- 出口执行就绪后，控制面放开公共规则、路由方案、方案分配和自动地区，以及目标为互联网、IP、域名的授权。
- 规则集的来源、更新状态、条目数，以及无法转换的条目。
- 设备当前生效的方案和选择依据。

在出口执行就绪之前，console 的出口层保持只读，授权目标只提供节点和标签。
