# V2 API：Console 的需求与消费反馈

本页由 console 工作流维护，记录三件事：console 需要 V2 Controller 提供什么、与后端已经商定的约定，以及对照最新已接入的后端版本还有哪些问题。

- 接口以 Rust 路由导出的 `packages/protocol/openapi.json` 和生成的 SDK 为准；设计文档在 `docs/v2`（入口 [API 索引](../../../docs/v2/api.md)）。
- 以往各轮的问答和复核记录在 git 历史中，本页只保留现行内容。
- 路径省略 `/api/v1` 前缀。

## 已定的产品决定

| # | 决定 |
| --- | --- |
| D1 | 已有账户通过一次性、有期限、绑定网络和角色的加入链接加入网络，由已登录用户接受。不提供按邮箱直接添加，不恢复邀请注册，链接也不绕过注册关闭的开关。签发和兑换时都检查当前权限。 |
| D2 | 账户恢复不靠邮件：运维在 Controller 主机上用 CLI 生成重置链接，console 只提供兑换页 `/reset-password`。网络 Owner 不能重置他人的全局账户密码。 |
| D3 | 网络概览直接从已有 ConfigurationView 的 topology、defaults、nodes.identity 和 groups 展开配置声明的邻接图，State 只叠加实际连接状态。没有当前通道、节点离线或尚未上报不删除配置边。图是配置可视化，不承担稳定身份授权或执行编译。没有完整配置读取权限时只展示授权可见的运行连接。历史层默认关闭，不为画线额外探测。 |
| D4 | 删除节点是永久的：提交时吊销凭据并移除 `nodes[id]`，没有回收站，也没有"仅从配置移除"。JSONC 或 ops 删去已有节点同样是删除。重新接入就是一个新节点。业务上的停用用 `enabled=false`，可以回滚。 |
| D5 | 每个用户、每个网络一份草稿，跨页面保留，只有一个应用入口。应用前必须展示有效修改数量和前后对照，并把永久删除、改址、身份和材料变化单独列出。严格 CAS，不按"pointer 没变"自动合并。草稿里要么是 JSONC 整份文本，要么是强类型 ops，两者不互相转换。 |
| D6 | 路径检测是节点详情里的"路径"标签：来源是当前节点，输入一个目的地，不填协议和端口；有草稿时并排检测当前版本和草稿，结果不同就标出。判定全部来自服务端，结果是规划，不是实测；console 不自行计算规则。保存检测用例和节点×节点矩阵以后再定。 |

## 已商定的接口约定

- **权限**：每类资源带服务端计算的 `actions`，console 不从角色推导；mutation 仍由服务端实时重新检查。
- **错误**：`ErrorResponse { requestId, status, problem: { code, params, message } }`，`code` 是每个操作的封闭枚举。改 `ProblemCode`／`IssueCode` 时一并补 console 的消息，并跑 `mise run check`（含 console 的 typecheck）再合。配置诊断带 pointer；State 的问题带资源和观测时间。
- **幂等与秘密**：
  - 创建类请求都带 `requestId`。
  - 接入密钥和加入链接的原值只在首次响应里交付。重试拿到的是同一资源的元数据，并注明原值不可重取。
- **命名与定位**：字段统一 camelCase；规则用 `remark`，不带 id；定位一律用 revision + JSON Pointer。
- **配置编辑**：
  - 提交有两种：整份 JSONC 和强类型 ops（`replaceAccess` 等明确分支，不接受任意 JSON）。
  - 历史版本有提交时间、cause 和 actor。系统写入（接入、邮箱同步、删除、身份转换）也会产生新 revision。草稿基于旧版本时，用同一批 ops 在新版本上重新 validate，展示新的对照，由用户应用。
  - `validate` 返回诊断、修改数量与对照，以及规范化后的脱敏 `preview`；它不生成材料、不预留地址。
  - 表单里的秘密只写：保持、覆盖或轮换。读取秘密只有整份 `configuration/source` 一条路。
- **成员移除、离开网络和改角色**：
  - 移除和离开永久删除该成员当前的 User 身份设备，保留 Tag 设备。确认前先读 `removalImpact` 或 `leaveImpact`。
  - 改角色前读 `role-change-impact/{role}`：降级吊销该成员签发、未使用、未到期且新角色不能签发的加入链接（已到期的保持到期）；变更按读到的 `version` 提交。
- **接入**：
  - 标准安装命令只含公开信息，接入密钥不进进程参数或 shell 历史。
  - 首次接入时，`initialSettings.hostname` 只是建议名，重名时自动加后缀。之后改名若重名，直接报错。
  - 设备授权链接由服务端按启动参数 `--console-origin` 生成：`EnrollmentRequestView.authorizationUrl` = `<origin>/authorize-device/<enrollmentRequestId>`，未配置时为 null。这个路径是 console 的契约，Node 只显示该字段，不自行拼接。
- **部署进度**：按 revision 由服务端去重计数，标明 full 还是 partial 范围；GET 和 SSE 用同一类型。
- **SSE**：
  - 首次同步发送完整可见的 Config、实体和 State 快照，之后是增量。实体和 `actions` 有 `entities.delta`。
  - `configuration.changed` 带 changedPointers 和 cause。
  - 服务端给出保活间隔，reset 和关闭都带原因。
  - SSE 与 GET 复用同一组片段类型。
- **State 与历史**：
  - 服务端从节点的实测地址（STUN、合格的公网网卡地址）选出位置所依据的地址，带来源和时间；选不出时给出缺失原因。地点和 ASN 由 console 在浏览器里向 geojs.io 查询：只发公网地址；默认开启，可在本浏览器关闭；结果在本浏览器保存 7 天，退出登录时清除。服务端的 GeoIP 只留给自动方案（`routing.egress.auto`），两边对同一地址的地区可能不同。
  - links 和 paths 的聚合保留每个报告者的证据。两端都报告自己是发起方时判为 `conflicting`，`inUse` 为 null；等 Node 数据面定义 linkId 协商后再调整。
  - 历史层通过 `history/query` 的 `recentLinks` 模式读取，不进入默认的实时流。
- **域名经出口**：`autogroup:internet` 覆盖域名目的地（路由规则和规则集里的名称），与执行期按解析地址判定一致；只有 CIDR 授权的来源仍需 `domain:`。access 不引用规则集。
- **加入链接与重置链接**：兑换前可以用 `inspect` 预览，不会消费；最终兑换时重新校验。
- **实时观察**：`auto` 模式由 Controller 计算候选、执行端选择路径；`explicit` 模式固定路径。console 只在页面可见时续租。

## 接入状态（契约 `8d50e873`）

console 已接入 `/api/v1` 下全部 Console 操作：

- 账户、会话和账户活动（邮箱修改显示原邮箱）；
- 网络、成员（改角色前确认要撤销的加入链接）、加入链接（签发人、使用者）和网络活动；
- 配置：JSONC 和全部区段表单（出口规则的动作只有出口和直连，"不接管"只用于方案兜底；目的地里 `example.com` 是后缀，`=example.com` 是精确域名 `domain`，inline 规则集相同；组网允许规则的 `transports` 是有序允许列表，前项优先，可用箭头或 Alt+↑/↓ 调整；省略使用平台顺序 UDP → TCP → VLESS → Trojan，`[]` 没有允许承载；连接方式默认按需，`keepalive: true` 为常驻，只有一端能拨出的边总是常驻；阻断规则两者都不设置）、validate（含规则集准备与监听器材料计划）、apply、历史、对照和恢复；顺序变化会保留为草稿修改，dialer 不再有 priority；按 `NetworkActions.readConfiguration` 决定读完整配置还是成员视图；
- 配置诊断：按 `IssueCode` 和 `params` 本地化（节点按主机名，传输和规则集的 `reason` 按已知取值；`edge_transport_unavailable` 说明哪条 Mesh 路由要经过哪两端之间的边，挂在 `/topology/<i>/transports` 的承载上），问题挂在 pointer 所在的规则或字段下，并标到具体的值；`warnings` 显示在应用确认、对应规则下和源文件里；应用确认使用 `affectedNodeIds`、`identityChangedNodeIds`、`changedListeners`；
- 地点与 ASN：console 不再读服务端的 GeoIP 结果（`derived.location.data`、`datasetId`、`ipInfo` 的查询结果），只用 `derived.location` 的地址和 `derived.ipInfo` 的键（`notApplicable` 视为非公网，不发出），向 geojs.io 查询。在服务端按下面"请后端调整"改完之前，`derived.location` 只在服务端自己的 GeoIP 有结果时才出现，没有数据集的部署里节点仍然没有位置。8420 的节点通过 STUN 上报了公网映射，地球上已经有节点；三个节点共用宿主出口，同一地址的节点合成一组，没有 STUN 观测的列在「待定位」。
- 节点、节点运行 State（对端以 State 的 hostname 命名）、节点设置（成员通过 `getNodeSettings` 读取）、监听（`ListenerUpdate` 省略 WebSocket 响应头即保留，界面按名字显示）和节点密钥；
- 接入密钥和设备授权（`authorizationUrl` 打开的就是 console 的授权页）；
- 按 revision 的部署进度；
- 网络 SSE，含观察事件、`stream.closed` 和增量里的 State 元数据；管理者的配置快照按 GET 响应写入缓存；续传不重发初始批次，console 以 SDK 的 `onOpen`（连接被接受）确认服务端接受了 cursor；
- 规则集（刷新带 `requestId`：结果未知时再刷新沿用同一 ID，服务端返回当前视图、不再下载）；
- 路径检测（`evaluateConfiguration`）：节点的“路径”标签（D6），当前版本和草稿并排，不同的部分标出；规则按 pointer 命名，当前版本的可以打开到源文件。需要 `readConfiguration`，成员看不到这个标签；
- 节点上报的本机地址（`SocketState.localEndpoints`：省略即未采样，空数组即没有可用地址）列在传输页对应套接字下；连接按节点自己的报告显示获取方式（`LinkState.acquisition`：拨号到已发布的端点、拨号到对端上报的本机地址、拨号到 STUN 观察到的对端映射地址、接受对端拨入、UDP 打洞），没有报告时仍显示由谁发起。在 8420 上用 `meshway ping` 建立连接后核对过；
- 每个错误响应的 `description` 列出该状态下可能的 problem code，站点的 API 参考随状态码显示；
- 站点的委托登录（`createDelegation`）：同意页 `/authorize-site`，授权后跳回站点地址并带上 `code` 和站点的 `state`；会话列表按 `delegationClient` 标出授权给哪个站点。站点用 `exchangeDelegation` 换到 1 小时的会话，在 API 参考里试用（只放内存，写操作先确认）。8420 以 `--delegation-client docs=http://localhost:3002` 启动，在 8420 上走通了授权、读、写操作取消后不发送和确认后发送、会话列表、结束试用；委托会话改邮箱等被拒（403）没有单独验证；
- 观测历史；
- 主动路径观察。

8420 已由后端升级到 N1 netmap（旧库在 `.meshway/backups/server-20261010T144459Z-cb2b03abb8c8/`）。console 按 `8d50e873` 的 SDK 编译和测试通过；在 N1 服务上的实际运行核对，等 8420 恢复后进行。

撤销已失效的加入链接、接入密钥，以及重复吊销节点密钥时的 409 `state_conflict`，console 提示"这次没有撤销"并重读列表。

## 尚未提供的接口

- 删除网络（含删除时节点凭据的处理和保留期）。
- 成员切换路由方案：`requestedProfile`、`appliedProfile`、允许的方案、`locked` 和自动方案的判断依据。
- 节点的版本兼容状态 `versionStatus`（current、outdated、unsupported），由服务端按兼容门槛判定。
- `reassignAddresses`、`rotateMaterial`（editing.md 已不再列出，需要时重新定义）。
- 安装脚本和 Agent 发布。
- 可信 DNS 的数据输入，以及基于它和 GeoIP 的自动方案。
- **API 参考需要的契约内容**（站点从 `openapi.json` 生成参考页）：
  - ~~示例~~：已提供。fixture 在 `packages/protocol/examples/<tag>.json`，按 operationId 给 `request` 和 `responses.<status>`；Server 导出契约时写进 `requestBody.content[...].example` 和 `responses[...].content[...].example`，所以 `openapi.json` 直接带示例。SDK 测试（`packages/control-client/test/examples.test.ts`）用生成的 zod schema 校验每个示例，并要求所有 JSON 请求体和 2xx 响应都有示例；错误响应不用手写，Server 用该响应首个 problem code 的真实 `ErrorResponse` 作为示例（`password_weak` 带真实 PasswordRules，`configuration_invalid` 在 validate 上有手写的 `invalid_domain` 例子）。所有示例描述同一个 `lab` 网络（paris 中继、NAT 后的 office-gw、alice-laptop、bob-phone），ID／时间在各操作间一致。注意示例对象的键按字母序输出（Rust 侧 `serde_json::Value` 不保序）。
  - tag 的顺序和分组：现在是平铺列表，顺序跟着路由注册走。

## 待处理的问题

以下按 `01da4515` 核对（`233a8233` 的路径检测另在 8420 上用 console 检测过节点、域名、公网目的地和草稿并排；`8ca49b0f` 的组网承载在 8420 上用表单检查过：`[]` 加一条必经路由被拒，问题挂在承载上并按两端主机名说明）：契约按 `packages/protocol/openapi.json`；行为用同一份源码构建的隔离 Server 和真实 `meshwayd`（只有控制流程：接入、续租、保存配置、上报状态）验证。

**已修复并已验证**（不再跟踪）：

- OpenAPI 组件重名：IP 查询改用 `LookupUnavailableReason`。
- 成员的设置变化后，服务端发 `sync.reset` 并重发完整批次；console 不再自行断开重连。
- 会话被撤销时先发 `stream.closed`（`session_expired`）再结束。
- 断开的流立即释放名额：同一会话连续打开并断开 7 次均为 200。
- 续传时响应头立即送达，之后只补发缺失的事件。
- `state.delta` 带 `asOf`、`sequence`、`configurationRevision`；概览的"截至"随增量保持最新。同一 sequence 下，流写入的 State 与重新 GET 的只差 `asOf`（各自的投影读取时间）。
- 配置变更、节点删除和成员移除的事件都在提交后立即送达，被拒的提交不产生事件。
- 新增 VLESS listener 可以预览：`willGenerate` 列出将生成的 UUID、证书和私钥，`requirements` 列出待完成的证书校验。
- 成员的 SSE 库存与 `GET /nodes` 一致，只有自己的 User 身份设备。
- 观察鉴权失败不再返回 401：源节点重新 acquire 后，订阅被移除（读取为 404），console 不会登出。
- SDK 不再使用 DOM 类型 `ReadableStreamReadResult`。
- 所有分页列表（网络、账户会话和活动、成员、加入链接、网络活动、接入密钥）的游标失效时都返回 `invalid_cursor`。
- `removeMember` 的 revision 不匹配时返回 `revision_conflict`。
- 删除节点的预览不再把该节点列进 `addressChangedNodeIds`。
- 撤销已使用、过期或撤销的加入链接和接入密钥返回 409 `state_conflict`；这些撤销操作和 `getNodeSettings` 的 OpenAPI 声明了实际的错误状态。
- `docs/v2/api/events.md` 不再给 `entities.delta` 写 `configurationRevision`。
- console 提出的契约增补：`NetworkActions.readConfiguration`（成员不再被拒一次）、`NodeStateView.hostname`、`JoinLinkView.issuedBy`／`consumedBy`、`config.snapshot.requestId`、`ActivityTarget.account.previousEmail`，console 均已消费。
- 降级 admin 后，它签发的链接在列表中是已撤销，inspect 也返回 `join_link_revoked`。
- 没有活实例的节点判为 `expired`：`meshway down` 后读到 `expired`，`up` 后回到 `current`。判定顺序见 `network.md`。
- 节点报告刚被替换的已保存配置时按事实接受：1 秒内连续 3 次 apply，节点保持 `controlReady`，保存到最新 revision。
- schema 不符时，启动错误指出对象：用旧库副本启动得到 `table activity_events is defined differently from the current schema`。仍然没有迁移，开发环境 schema 变更后要重新初始化数据目录。
- `ip-datasets`／`reset-password` 给相对路径时报 `data directory must be an absolute path without parent traversal`，只有存储确实被占用时才说"Stop the Server first"。
- 域名出口：只有 `autogroup:internet` 授权、没有 `domain:` 的来源，经出口的域名后缀和精确域名规则都能应用。
- 规则集转换接受精确域名：MetaCubeX `geosite` 的 google（1075 条）和 cn（111224 条）都能准备。
- 精确域名的诊断：`invalid_domain` 指到 `…/match/domain/<i>`；被前面后缀覆盖的精确域名规则报 `shadowed_rule`。
- 改角色的影响预览：降级 admin 时只计入未到期的链接，确认后这些链接变为已撤销，已到期的保持到期。
- 出口规则动作改为 `RuleAction`，`ListenerUpdate` 省略即继承 WebSocket 块和响应头：在界面上修改带响应头的 VLESS 监听后响应头保留，改成 TCP 后 WebSocket 块删除。

**按设计，不再跟踪**：

- 响应里的 `requestId` 是服务端为这次 HTTP 请求生成的关联 ID（与 `x-request-id` 一致），不回显请求体里的 `requestId`。
- 设备请求不带系统和 Agent 版本（不在签名的 intent 里），console 不显示。

**后端已说明、console 未实测**：

- 旧版本保存的配置可以读回：`RouteMatch.domain`、inline `RuleSet.domain`、`RuleSetData.domain` 为空时不再序列化，旧版本的规范 JSON 和 artifact 摘要逐字不变；映射成 503 的服务端错误都记一条 error 日志。8420 这次是重新初始化的，没有用旧库验证。
- `createEnrollmentKey` 的请求 ID 冲突改为 `request_id_conflict`；同一 `requestId` 同内容仍返回同一 key 的元数据（`secretDelivery: unavailable`）。`binding_conflict` 只剩设备接入请求／兑换的绑定不匹配。
- Node 控制联系失败按真实错误分类恢复：网络、暂态响应和 session／instance 失效不再永久停在 `Contract`／`blocked`。
- `revokeNodeKey` 重复吊销返回 409 `state_conflict`，已退休代际仍可吊销（204，不撤当前执行），generation 不匹配是 409 `version_conflict`。
- 绑定大规则集（8483 条）后 validate 很慢的问题，在 `70f5e95f` 改为线性覆盖计算。
- 托管证书改 `serverName` 时重新生成证书；provided 材料仍按省略继承。
- 规则集检查记录：来源与绑定内容都不变就跨版本延续，包括还没有成功内容的集合。console 报过的 `not_ready`／`checkedAt: null` 后端没有复现，再遇到时补充步骤。

### 请后端调整

- **相邻连接状态与报告可见性**：已按源码核对 `state/mod.rs`，Server 在 full／peer 裁剪后聚合 links，只读取 `VisibleReport::Full`。对端完整报告不可见时，即使服务端持有双端数据，成员视图也只能得到 `oneSided`；该值不能作为连接降级或“没有连接”的产品条件。真实成员持有 Node 的双视图对照尚未复核。
  - Console 已统一为每对节点一条相邻关系，当前有效的 authenticated 单侧／双侧报告均显示已连接；原始 evidence 留在展开详情，过期／冲突保留未知，关闭记录不充当现有连接。
  - 请确认并提供稳定的产品级相邻连接状态语义，使调用者有权查看的同一关系，不因诊断报告的裁剪而改变连接结论；诊断来源仍按权限保留。这里不要求读取不可见的完整报告，也不把底层通道当成业务端口可达证明。
- **节点详情的 Mesh DNS 名称**：新版概览需要可复制的规范化 FQDN，并能区分 MagicDNS 已启用／已关闭。当前名称规则是配置 hostname 加 `dns.domain`；Owner／Admin 能读完整配置，普通成员的 `NetworkSummary`、`NodeInventoryView`、`NodeSettingsView` 均没有 DNS 后缀和 MagicDNS 开关。
  - 请在调用者有权读取的节点／网络视图提供最小的命名信息（规范化名称与开关，或足以无歧义展示它们的投影），GET 与对应 SSE 一致；节点改名、网络后缀变化和开关变化应随同版配置更新。
  - 这是配置名称，不是系统 resolver 已安装或其他节点已有访问权限的证明；控制台不调用 Node 凭据专用的 `queryNodeDns` 来补齐，也不向普通成员读取完整配置。
- **State 不再返回 GeoIP／ASN**（产品决定：地点由 console 查 geojs.io，服务端的 GeoIP 只给 `routing.egress.auto` 用）。
  - `derived.location` 只描述依据的地址：`{ address, endpointSource, observedAt, expiresAt, sourceSocketId, sourceGeneration }`，去掉 `data` 和 `datasetId`。只要有合格的实测地址就给出，不依赖数据集。
  - 多个合格地址时，选择不能再依赖它们的地区是否一致：请定一个确定的选择规则，或者在不止一个时返回 `ambiguousObservation`。
  - `derived.locationAbsent` 只留 `noObservation`、`observationExpired`、`ambiguousObservation`。
  - `derived.ipInfo` 换成只标明是否公网的映射，例如 `addressScope: { [ip]: "public" | "nonPublic" | "mesh" }`，仍只含调用者可见的 endpoint IP。console 只把 `public` 的地址发给 geojs。
  - `ip-datasets` 和数据集保留给自动方案；`docs/v2/state-reference/ip-info.md`、`endpoints.md` 的 GeoIP 段落和开发环境的数据集说明随之改写。
  - console 的读取集中在 `src/lib/geo.ts` 的 `nodeAddresses`，改完后只需调整这一处和缺失原因的文案。
- **`edge_transport_unavailable` 一次只报一条**：同一条组网规则让几条 Mesh 路由都走不通时，validate 只返回其中一条路由和一条边（8420 上 3 条必经路由都经过 `transports: []` 的边，只报了 `/routing/mesh/1` 的 lax–nyc）。改完一条再检查才看到下一条；如果是有意合并，请在 problems.md 里写明。
- **阻断规则带 `transports`**：服务端照收，但阻断总是阻断整条边，这个字段不起作用。建议 validate 给一条 warning 或直接拒绝；console 现在只在规则下注明"阻断规则的承载设置不起作用"，把规则改成阻断时去掉它。
- **业务流量期间 `StateView.paths` 为空**：8420 r5 上从 lax 在 TUN 内持续 ping nyc 的 Mesh 地址（90／90 及 60／60 成功），同期 lax–nyc 的 link 为 `authenticated`／`matched`／`inUse: true`，但 `paths: []`。拓扑的流动方向只取 `paths` 的节点顺序，不从 links 推断，所以现在只能显示"承载业务"的无方向效果。请确认节点是否应上报业务 PathState，或说明 `inUse` 在 `paths` 为空时的含义。
- **节点配置的租约与同步时间**：Server 给每份 netmap 签发的租约（`issuedAt`、`expiresAt`）和节点最近一次 `sync` 的时间只出现在 Node 用的 `SyncResponse` 和节点本机 IPC 里，用户接口（`NodeInventoryView`、State）都没有。console 因此不能说一个离线节点还能按上次的配置运行到何时，节点详情也不显示最近同步时间。请在用户接口上给出：服务端为节点签发的当前租约，和它最近一次 sync 的时间（服务端收到为准）。
- **`netmap` 为 null 的原因**：`NodeInventoryView.netmap` 为 null 时分不清“还没有下发过”和“超出 8 MiB 后被移出可信库存”（密钥轮换、能力或方案变化可以让已存在的节点超限）；console 现在只能都说“服务端还没有给这个节点下发配置”。请给出不可用的原因（至少区分超限），超限时附 `bytes`、`maxBytes`。
- **按需连接的空闲与失败**：组网边默认按需建立、空闲 2 分钟关闭后，State 里一条没有通道的边分不清是“空闲已关闭”还是“连不上”。console 现在对两者都显示“未连接”。如果节点能上报最近一次建立失败（或空闲关闭）的事实，请加到 link 上。
- **按目的地逐个规划**：拓扑选中节点后，对每个其他节点各调一次 `evaluateConfiguration`（console 最多同时 4 个，按 revision 缓存）。节点多时请求数随节点数线性增长；如果服务端能一次返回"从一个来源到所有节点"的主路径，请给批量形式；没有也能用，只是大网络较慢。

### 需要真实 Node 复核

三节点 Docker 环境接在 8420 上。

1. 节点应用配置后重新 acquire instance，原版本中是否仍被标为 `superseded`：未复核。
2. 节点配置内容变化（新的 netmap serial）期间的状态：节点在 `current`／`config_mismatch` 之间如何变化、links 是否过期，以及旧上报过期是否被误当作通道断开。N1 之后未复核。
3. 主动观察的创建、续租、取消对节点 netmap 和 sync 的影响，以及有观察时 Agent 请求的耗时：未复核。
4. 2026-10-09 meshway-sha 自身报告 `Blocked`、`Last problem: Contract`、数据面 `Failed`，State 中为 `expired`；lax／nyc 同期 `current`。console 如实显示为离线，原因待后端或 lab 维护方确认。

### 后端排期中

- **主动观察与历史**：observations 模块冻结，待 Node 实现后按真实消费者重做。
  - 响应没有服务器时间（续租时机按浏览器时钟计算），没有观察权限的 `actions`（console 按"角色 + 身份"重述规则），也没有允许的目标和路径候选清单。
  - 429 `observation_limit` 同时表示续租过早、预算用满、已到终态和 4096 窗口满。
  - 创建时目标没有可用路径（例如目标没有 listener）返回不带参数的 403 `permission_denied`，与"未授权"无法区分，`DemandDecision` 的原因（`noPath`、`missingCapability` 等）没有带出。
  - 订阅因源节点重新 acquire 而结束时只是被移除（404），不说明原因；样本只给 `latest`，重载或接管后看不到之前的序列。
  - `HistoryBucket` 没有 `peerNodeId`，而且按 instance 和 netmap 拆开，同一条曲线在节点重启或配置内容变化时被切开，console 端做了合并。
  - `RecentLink` 没有 RTT 和关闭原因。
  - 一次查询最多 10000 行，计数在按 kind 或节点过滤之前做，繁忙的网络即使只查一个节点也会收到 429；窗口外的清理记录也会返回；bucket 把 `pending` 计入 `failed`。

### 其他

- 给 Agent 实现方的提醒：每个节点最多 8 个会话、8 个 challenge。
