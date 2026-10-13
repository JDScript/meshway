# Changelog

按合入 main 的顺序记录工作包、提交与证据 ID。规则见[工程流程](/v2/process)；性能数字的口径见[分层性能测量](/v2/node/performance)，验收状态见[验收范围](/v2/node/validation)。

## 未发布

wire 6、`compilerVersion` 不变，IPC 9（C3）；Server schema 版本 7（F4a 新增 `rule_set_record_heads`，C3 为 6，R2d 新增 `rule_set_catalog_checks` 并允许回收 artifact 为 7；开发环境重新初始化数据目录）。HTTP 契约：`SyncRequest` 新增可省略的 `lease`；`listRuleSetCatalog` 描述更新（无字段变化）；S3 的 State／库存／诊断变化见下。

E1 出口 ICMP 错误与出口自指（wire、IPC、HTTP 契约与 `compilerVersion` 均不变）：

- 出口 ICMP 错误回传：ICMPv4 destination unreachable（含 fragmentation needed）与 time exceeded 作为其引用活流的 RELATED 报文，沿该流原路径回复方向返回源；出口之外路由器与出口主机自身的错误都经出口 TUN（`route` 链、新 `local` 输出路由链、一条 `fwmark＋ipproto icmp` RPDB 规则），数据面按引用的 forward 包、circuit、路径与来源规则（`admission::netmap::admit_related`）准入，每流每端每秒至多 10 条，`SecureNode::related_counters` 计数；traceroute 显示出口与其后各跳，PMTU 发现经出口收敛。见 [数据面 · ICMP 错误](/v2/node/dataplane#icmp-errors)、[Exit 设计 · ICMP 错误](/v2/node/exit-design#icmp-errors)。
- Underlay 的路由读取（`get_routes`，全部归属核对的唯一来源）忽略内核派生的缓存例外（`RTM_F_CLONED`）：PMTU 例外随 fragmentation needed 出现在源的受管捕获表后，egress owner 不再把它当作外来路由而在下一次门切换（如租约到期暂停）时失败闭合并卡在冲突。
- 出口的 conntrack 确认不再阻塞：首包死于出口（TTL／MTU）时最长 3 秒的等待原先挡住 daemon 出口 driver 与 Underlay owner 的全部后续工作，现与其他流并行。
- 出口集合含源自己时按有序主备执行：自己是那个位置上永远可用的 direct，`[self, other]` 为 direct，`[other, self]` 在 other 配置可用时经 other、否则 direct（`matching::executed` 统一用于分割、`egress` 计划、DNS plan、路径检测与节点判定），`geoip:cn → exit sha` 这类公共规则不再拒绝 sha 自己的流量、也不再使其 DNS 为 `exitPlanRequired`；Access 不变。见 [Routing · Action](/v2/configuration-reference/routing#action)。
- `exit/off` 新增第 12 阶段（router＋窄 MTU 的 far 网段：traceroute、PMTU、伪造／限速）。

验证：合入 main `1211538b` 后（`6aaf979a`）Planck 16 行矩阵 `20261013T022738Z-789636f9` 全部通过（`exit/off` 含第 12 阶段）。合并前：`exit/off`（ICMP 阶段当时为第 11 阶段）Planck `20261013T011243Z-62bc2868` 等通过；三次全量矩阵 15／14／14 行通过，未通过行（`netmap/expiry` 租约到期前的 suspended／TUN 竞态、`exit/off` 第 9 阶段日志读取竞态、一次 `udp/resolv-conf` CLI 样本）与 S3 同类或单行复跑通过，详见[验收范围](/v2/node/validation)；证据 `.meshway/validation/e1/`、`.meshway/node-lab/e1/`。

规则集收尾（R2d）：

- Server 启动后及按 provider 间隔自己取回并验签每个网络生效的 provider 目录（内置与覆盖；同一目录 URL 与信任钥只取一次，共用下载预算，退避持久保存，失败与 sequence 回退保留上一份），`GET /rule-set-catalog` 不再需要先引用集合；`rule_set_catalog_not_ready` 只表示首次取回尚未完成或从未成功。见[规则集 · 目录更新](/v2/configuration-reference/rule-sets#catalog-refresh)。
- 无引用 artifact 回收：未删除网络任一版本的初始或刷新绑定引用的 artifact 保留；被刷新替换或只属于已删除网络的 artifact，在可能仍处于租约内（24 小时）的 netmap 都不再引用后连同记录删除，启动时及每小时一次。见[规则集 · artifact 保留](/v2/configuration-reference/rule-sets#artifact-retention)。
- 发布仓库 `meshway-org/meshway-rules`（`379bdb4`）把 hold 写入签名目录 `CatalogSet.held`，`since` 从上一份目录读取，`status.json` 只是副本；README 写明 held 集合 `upstream.ref` 为其 commit 的上游一致规则。
- runtime fixture：`netmap/expiry` 在 Status 去掉租约后 2 秒内要求 TUN 删除并记录延迟，不再与 daemon 的撤回竞争；根因见[租约到期撤回](/v2/node/validation#lease-expiry-withdrawal)。
- `validation.md` 补齐 R2a、X6、R3、F4a 的矩阵行与证据位置。

验证：Planck 16 行矩阵 `20261012T212926Z-abd9b9c8`（`4164e184`）、`20261012T232408Z-75436f19`（`b4fe0619`）与合入 main（含 S3、C3）后的 `20261013T015222Z-59ebfafc`（`861d58d3`）全部通过；高负载下的未通过矩阵与签名见[验收范围](/v2/node/validation#netmap-rows)；`mise run check` 通过；证据 `.meshway/validation/r2d/`。

出口方案切换（C3）：IPC 升为 9，`activity_events` 新增 Node actor 与路由方案目标（schema 6）。

- `meshway exit [profile|off]`：无参数显示本机 netmap 中的方案选择（生效／请求的方案、默认与锁定、`auto` 解析、每个允许方案的摘要）；带方案时请 Server 切换并等待本机应用带新版本的 netmap，`off` 即 `mesh`。人读输出、`--json` 与退出码见 [出口方案](/v2/node/runtime#exit-profile)。
- IPC 9：命令 `SelectProfile`、结果 `Selection`、`NetmapStatus.routing`，错误码 `profileNotAllowed`／`profileLocked`／`selectionConflict`；`meshway status` 新增 Profile 行。
- Control 调用 `selectNodeRoutingProfile`，CAS 基准为持有 netmap 中自身的选择版本；方案被拒或版本过期不结束 session／instance。本机只缓存 netmap，不保存选择。
- HTTP：`selectNodeRoutingProfile` 的回答改为 `{selection: RoutingSelectionState}`，版本不一致改为 `409 version_conflict`、方案不可选为 `422 routing_profile_unavailable`（带允许的方案）；新增 `getNodeRoutingSelection`、`setNodeRoutingSelection`（`/api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection`，成员限自己的 User 设备，Owner／Admin 任意节点，`requestId` 幂等）；`NodeInventoryView.routing` 摘要；活动 `routing.select`（actor `node`／`user`，target `routingSelection`）。三个入口共用同一提交，选择版本跨身份单调递增，过期身份的选择不作为 CAS 基准。
- `RoutingSelectionState` 由 `meshway-policy` 唯一计算，Server 与节点对同一配置给出同一值；`auto` 没有可信地区输入，总是采用 fallback（`basis: noTrustedRegion`）。
- Console 仅补齐新增枚举的映射。
- `exit/off` 第 11 阶段：真实 CLI 切换后旧选择的 TCP／UDP 流结束、新流经出口，`exit off` 后无外部捕获。

验证：Planck 16 行矩阵 `20261012T224334Z-469128d0`（`7facc522`）与合入 main 后的 `20261013T000304Z-56e3679e`（`f2441b12`）全部通过；证据 `.meshway/validation/c3/`。已知：切到 `global:<exit>` 与 `useWithExit` split resolver 组合时节点 DNS 安装拒绝新 netmap，见[验收范围 · 剩余边界](/v2/node/validation#remaining-boundaries)。

此前：wire 6、IPC 8、`compilerVersion` 不变；Server schema 版本 5（新增 `rule_set_record_heads`；开发环境重新初始化数据目录）。HTTP 契约：`SyncRequest` 新增可省略的 `lease`；S3 的 State／库存／诊断变化见下。alpha.4 代码审阅修复（F4a）：

alpha.4 代码审阅修复（F4a）：

- A1 `POST /agent-api/v1/dns-associations` 接纳前对条目执行与 overlay 相同的校验（`DnsAssociation::validate`：名称／CNAME 为 1–253 B ASCII 小写等），非法条目 400 `invalid_request`，不再让源与出口的 `sync` 持续内部错误。
- A2 出口 overlay 容量在接纳时对源与出口统一预算：任一方未到期条目将超过 4,096 即 429 `state_limit`；交付时不再截断单方副本；源换出口时唤醒原出口。
- A3 规则集 FST 输入索引按完整转移（input、output、target）校验，交换共享目标的表项不再把 exact 读成 suffix。
- A4 删除 `meshway-rules` 的 mmap 后端与 `memmap2`，artifact 只读入堆校验；workspace 零 `unsafe`（`meshway-rules` 使用 workspace lints）；Control 有界读取 0400 缓存文件；`perf:rules` 报告删去 mapped 列。
- A5 规则集转换与目录校验由 `Downloads` 内唯一有界 worker owner 执行：每个任务持有预算至结束（取消或超时的获取不释放），`App::shutdown` join 全部已开始任务。
- A6 同 digest 的刷新更新当前记录（新表 `rule_set_record_heads`：目录 sequence、upstream、held、报告），`rule_set_records` 保持不可变首次溯源。
- A7 路径检测对外部目的在源无分割（`capture.unavailable`）时给出 `unavailable`，与节点判定一致。
- A8 `sync` 挂起只按请求 `lease`（节点实际持有、经其验签的租约）计算，且仅当它是本进程最近签发给该节点的 8 份之一；省略或不可核验即立即回答，回复生成后丢失再重试不再越过节点租约。Control 每个请求带上已发布租约。
- A9 出口分割计算耗尽工作预算时也给出 `egress_partition_limit`（`params.profile`／`work`），与计数超限区分。
- A10 所有分页游标共用 `crate::cursor` 封装（作用域绑定、期限为列表首页起 15 分钟、后续页不续期）；目录游标绑定目录 URL、可信钥集合、collection、sequence 与已验签原文 digest。
- B1 名称规则决定的回答若含任一关联无法完整覆盖的公网地址（超过 16 个、不在 CNAME 链终点、链成环或超过 8 跳），出口不完成任务，源立即 SERVFAIL，不再截取前 16 个验证。
- B2 关联截止在应用时固定为与租约同一时钟的单调 deadline，墙钟回拨不再推迟。
- B3 关联到期时，本节点仍存活、经出口建立的流的目的地址留在捕获中直至这些流退役（`Desired::retaining`、Policy `retained_capture`、`SecureNode::exit_destinations`），已建立流不再从物理网卡绕过出口；源、中继与出口的已建立流在同一内容（同一编译器）下保留准入时的判定，不再逐包按到期后的关联重判；`exit/off` 第 10 阶段新增跨关联到期的 TCP／UDP 持有流检查。
- B4 同内容不同关联的 offer 作为 Desired 变化应用：结果写入状态报告，失败按重试处理。
- B5 只有租约到期或撤回才停止诊断，关联到期不再停止。
- B6 offer 去重键包含 DNS 关联 overlay（含 Some→None）。
- B7 `Directory::remove_staging` 接受经校验的 0600／0400 两态 staging 文件，Control artifact 回收失败写日志。
- B8 IPC `NetmapStatus.capture` 由最终安装的捕获计算（含关联与保留地址），关联变化或到期后更新。
- B9 同一次启动内仍持有资源的其他版本 egress journal 以专用错误（文件名、版本、先用旧版本 `meshway down` 再升级）阻止启动；已 down 或其他启动的记录照旧归档。
- 文档：`dataplane.md` 出口投影截止改为流 idle 与 circuit 生命期、随租约与角色 generation；`execution.md` 改为节点已消费 `sync`。

验证：Planck 16 行矩阵 `20261012T195053Z-bc48b6cc` 全部通过（`exit/off` 第 10 阶段含跨关联到期的 TCP／UDP 持有流）；证据 `.meshway/validation/f4a/`。

Console 提出的 Server 侧欠账（S3）：

- 相邻连接结论：`StateView.adjacencies`（与 `state.delta.adjacencies`）每对节点一条 `{a, b, status: connected | idle | unreachable | unknown}`，由 Server 用两端各自的当前报告判定（含调用者读不到的完整报告），只列调用者已能看到的节点对；成员与 Owner 对同一对得到相同结论，`links` 仍按权限保留为诊断证据。见 [Adjacencies](/v2/state-reference/network#adjacencies)。
- 节点 Mesh DNS 名称：`NodeInventoryView.dnsName`（hostname 加规范化 `dns.domain`）与 `magicDNS`，与库存同一配置版本，GET 与 SSE `entities` 一致。
- State 不再返回 GeoIP／ASN：`derived.ipInfo` 换成 `addressScope: {ip: public | nonPublic | mesh}`；`derived.location` 为 `LocationObservation {address, endpointSource, observedAt, expiresAt, sourceSocketId, sourceGeneration}`（去掉 `data`／`datasetId`，不依赖数据集），多个合格地址按 STUN 先于 interface、IPv4 先于 IPv6 取最高等级，该等级有多个地址时为 `ambiguousObservation`；`LocationAbsentReason` 只剩 `noObservation`／`observationExpired`／`ambiguousObservation`；OpenAPI 删除 `IpInfo`、`GeoObservation`、`GeoIpData`、`LookupResult_*` 等组件。数据集只留给自动方案与 `ip-datasets lookup`，导入数据集不再产生 State 事件。见[地址类别与位置依据](/v2/state-reference/ip-info)。
- 业务路径：Node 报告参与的已建立业务 circuit 为 `NodeState.paths`（完整路径、本节点角色、`netmap`；端点 `peerConfirmed`、中继 `sent`；每份最多 64 条，`STATE_MAX_PATHS`），Server 校验参与与角色位置后接受（此前拒绝任何非空 `paths`），`StateView.paths` 只聚合当前未到期的路径。此前 A→R→B 持续流期间三节点都报 `paths: {}`、links `businessDependencies: 1`（矩阵 `20261012T195053Z-bc48b6cc` `udp-resolved` 行的 `state-traffic-*.json`）；fixture 的承载检查现在要求三节点各报告 A→R→B 路径及自身角色。见 [PathState](/v2/state-reference/resources#paths)。
- 配置校验：`edge_transport_unavailable` 一次报出全部受影响的（Mesh 路由规则，边），一条路由要经过的每条无承载边各一条，上限 64 条；block 组网规则写了 `transports` 或 `keepalive: true` 时接受并给出警告 `ineffective_block_option`（`params.field`）。见[配置问题](/v2/api/problems#configuration-issue)。
- 批量路径检测（一个来源到所有节点）未做，仍按目的地逐个调用 `evaluateConfiguration`。

验证：Planck 16 行矩阵 `20261012T220402Z-33d41c74` 14 行通过（含新的 State 路径检查）；`exit/off`、`netmap/expiry` 失败，同一窗口 main `1852b477` 复跑这两行（`20261012T224249Z-0dc077d7`）同样失败，见[验收范围](/v2/node/validation#traffic-anchor)。

## v0.1.0-alpha.4

wire 6 不变；IPC 升为 8；`compilerVersion` 升为 `meshway-netmap-2`；Server schema 版本 4（开发环境重新初始化数据目录）。HTTP 契约：新增 `GET /agent-api/v1/artifacts/{digest}`、`POST /agent-api/v1/dns-associations`（`reportDnsAssociation`）、规则集 `report`／`entries`／`match` 与 `rule-set-catalog` 端点；配置新增 `ruleSetProviders` 与 `<provider>:<name>` 引用；netmap `ruleSets` 改为 digest 引用，`EgressPlan` 改为 `{generation, rules, fallback, unavailable}`（删除 `capture`／`routes`／`EgressRoute`）；`SyncRequest`／`SyncResponse` 增加 `associations`；`RuleSetView`、`ArtifactSummary`、`PreparedRuleSet`、`ResourceProblem` 与警告 `egress_partition_limit` 按下列 R2a、R2c、X6、R3 变化。

- 规则集格式（R1）：新 crate `meshway-rules` 实现 `MWRS` v1（64 B 头＋段表、反转名称 FST、keyword 段、合并 IPv4／IPv6 区间，digest＝整份 SHA-256，集合相等即字节相等）、确定性 builder、对任意字节要么拒绝要么查询不 panic 的校验 loader（`fst` 段由自有有界读取器在加载时证明结构；堆与 `mapped` mmap 后端，`mapped` 为 crate 唯一审计 `unsafe`）、matcher（`match_domain`／`match_ip`／有序区间迭代器）、与现 Server 相同口径的名称／CIDR 规范化、sing-box source v1–v3／mihomo YAML 与文本／classical／inline 转换（逐条报告跳过原因，不整次失败）及 ed25519 签名的规则目录。`tools/meshway-rules-cli`（bin `meshway-rules`：`convert`／`inspect`／`match`／`index`／`verify`）与 `perf:rules`。R0 的 16 份真实来源与 spike 参考实现 0 差异、arm64／x86_64 digest 一致、loader fuzz 80 万例无 panic；数字见[规则集引擎设计 · R1 实测](/v2/rule-sets-design#r1-实测)。用户决定（对齐 mihomo、`meshway-org/meshway-rules`、`mapped` 例外、regex 跳过、`domainSubdomain`、节点侧出口分割）已写入设计稿；Server／节点接线由 R2a 完成，出口分割由 R3 移到节点。
- 规则集发布（R2b）：`build.yml` 同时交叉编译 `meshway-rules-cli`；`release.yml` 新 job `rules-cli`（`environment: Production`）把 `meshway-rules-x86_64-unknown-linux-musl.tar.gz` 与 `SHA256SUMS` 发布为 `meshway-org/meshway-rules` 的 Release `cli-<tag>`，`MESHWAY_RULES_PUSH_TOKEN` 缺失即失败；该仓库每日签发 meta-rules-dat 全量目录（`catalog-1`／`catalog-2`：2,168 项），见[规则集引擎设计 · 发布仓库](/v2/rule-sets-design#发布仓库)。
- 规则集接线（R2a）：规则集按 digest 下发。配置新增 `ruleSetProviders`（同名替换内置 provider，`null` 禁用）与 `<provider>:<name>` 引用（内置 `geosite`／`geoip` 指向 `meshway-org/meshway-rules` 的 `dist` 目录，只信任 CI 钥），inline 新增 `domainSubdomain`，remote `format` 新增 `mihomo-domain-text`／`mihomo-ipcidr-text`／`classical`，集合 key 不得含 `:`；Server 删除自有转换器，统一用 `meshway-rules`（逐条跳过并报告，不再整次失败），artifact 全局内容寻址、每 (来源, digest) 保存溯源与报告，目录验签并持久检查 `sequence` 单调，每网络按 digest 去重 64 MiB／128 个。netmap 投影 `ruleSets` 改为 `{digest, format, bytes, entries}` 引用并不再含集合定义；新端点 `GET /agent-api/v1/artifacts/{digest}`（只对该节点仍可能在租约内的 netmap 引用的 digest 返回，其余 404）与 `GET /api/v1/networks/{networkId}/rule-sets/{ruleSetKey}/report`；`RuleSetView` 增加 `implicit`／`provider`／`source`／`conversion`／`held`，`ArtifactSummary` 以 digest 为身份（去掉 `id`，增加 `format`／`bytes`），`PreparedRuleSet` 为 `{digest, bytes, entries}`，`ResourceProblem` 增加 `rule_set_budget`／`signature_invalid`／`catalog_regressed`／`unknown_rule_set`／`integrity_failed`。Policy 的路径检测、出口分割与配置校验只经 matcher；节点 `LocalCompiler` 新增 `egress_rule_for_ip`／`egress_rule_for_name`（查询名只小写与去结尾点）。Control 取回 artifact、校验并以 0400 只读文件缓存于 `identity/artifacts/`，按引用回收、重启离线载入；缺失 artifact 时命名它的规则不可用、出口捕获全范围、不发布 direct 例外，IPC 8 `NetmapStatus.missingArtifacts`／`artifactProblem`。出口分割由节点计算，上限与警告 `egress_partition_limit` 见 R3。见[规则集](/v2/configuration-reference/rule-sets)与 [netmap 设计 · 规则集 artifact](/v2/node/netmap-design#rule-set-artifacts)。
- 规则集浏览（R2c）：新端点 `GET /api/v1/networks/{networkId}/rule-set-catalog`（`listRuleSetCatalog`：按有效 provider 列出／按名称前缀搜索已验签目录中的集合，含 `reference`、条目数、上游路径与 `held`；目录未验签为新 problem 409 `rule_set_catalog_not_ready`）、`GET .../rule-sets/{ruleSetKey}/entries`（`listRuleSetEntries`：按 `kind` 分页读取绑定内容的名称、keyword 与合并 IP 区间，名称按反转后缀查询）、`GET .../rule-sets/{ruleSetKey}/match`（`matchRuleSet`：判定名称／地址并返回决定成员），只读不下载，限 Owner／Admin。`meshway-rules` 新增 `RuleSet::domains_ending_with`（反转名称前缀与续页直接定位 FST，不扫描整份集合）。Server schema 4：`rule_set_catalogs` 保存验签通过的目录原文与签名。见[规则集 · 浏览与命中查询](/v2/configuration-reference/rule-sets#浏览与命中查询)。
- 可信域名关联（X6）：名称规则（`domain`／`domainSuffix`／规则集名称成员）现在在单出口 DNS 方案下生效。出口 netmap 的 `ServeExitResolver` service 是 Server 预签的常驻任务；出口回答源节点名称规则决定的查询后经新端点 `POST /agent-api/v1/dns-associations`（`reportDnsAssociation`，`DnsAssociationReport` → `DnsAssociationAccepted`）完成一次，Server 校验双方当前 netmap、service／selection、名称规则、公网地址与 TTL，在内存保存（每源 128 条）并为源与出口各签 `DnsAssociationOverlay`；`SyncRequest.associations` 指明持有的 digest，`SyncResponse.associations`（`DnsAssociationsUpdate`，复用 netmap 签钥，轮换期另附后继签名）只在变化时下发，任一方 netmap 变化先重验。节点：Engine 把关联并入 `Desired` 并按条目到期重新应用；`LocalCompiler::decide_with` 对“地址或其关联名称”first-match，同 IP 多名称动作不同即 `egressConflict`，`domain:` 授权按关联名称执行；源节点捕获加入关联主机；源 DNS owner 在关联生效前不放行该回答（至多 2 秒，否则 SERVFAIL）并截短 TTL；关联变化不重判已有流。名称规则不再使整个出口分割 `egressConflict`：名称成员不拥有地址。见 [Exit 设计 · 可信域名关联](/v2/node/exit-design#trusted-dns-associations) 与 [Routing · 域名规则的执行](/v2/configuration-reference/routing#域名规则的执行)。验收：`exit/off` 第 10 阶段（域名出口），Planck 16 行矩阵 `20261012T101704Z-ae64c486`（`cccc2594`）全部通过；常驻 lab `geosite:openai` 经 nyc、同 IP 名称冲突丢弃，证据 `.meshway/node-lab/x6/`。
- 节点侧出口分割（R3）：出口捕获的地址分割由节点用 `meshway-policy` 从自己 netmap 的规则与持有的 artifact 区间计算（first-match 区间扣除），Server 只用同一实现做校验与预览。netmap `EgressPlan` 改为 `{generation, rules: [{rule, action}], fallback, unavailable}`，删除 `capture`／`routes`／`EgressRoute`；`generation` 绑定生效规则所用集合的 digest，与节点持有哪些 artifact 无关。4,096 前缀与 131,072 步上限取消，改为节点侧 16,384 个捕获区间、32,768 条捕获路由（实测见 [Routing · 节点侧上限](/v2/configuration-reference/routing#capture-limits)）；警告 `rule_set_partition_limit` 改为按方案的 `egress_partition_limit`（`profile`／`ranges`／`maxRanges`／`routes`／`maxRoutes`），路径检测 `ProfileEvaluation.capture` 与 IPC 8 `NetmapStatus.capture`（CLI `meshway status` 的 Egress 行）给出同一 `CaptureSummary`。Underlay：受管区间为 nft interval 元素（无回显批替换、逐元素读回），未经活动 TUN 的受管目的立即拒绝（TCP reset／ICMP admin-prohibited），一条 RPDB 选择器查私有捕获表（受管区间的 CIDR 覆盖），逐前缀 RPDB 规则删除；egress journal 升为 v2（已 down 的记录直接归档；同一次启动内仍持有捕获的 v1 记录阻止启动，须先以旧版本 `meshway down` 或重启主机），路由变更每 64 条等待内核监视器观测，DNS 锚点冲突检查豁免捕获表，单次 netlink 读取默认上限 65,536。`exit/off` 的 split 解析器 direct 规则改为 7,000 个区间的集合，Planck 16 行矩阵 `20261012T105652Z-fcc0189b`（`72144421`）全部通过。可信域名关联（X6）成为分割的输入：关联地址按其名称规则并入或移出受管区间、到期随 `Desired` 重新应用移除，Server 侧 4,096 前缀校验删除；合并后 16 行矩阵 `20261012T115405Z-61e14403`（`c4018706`，含 `exit/off` 第 10 阶段）全部通过；见[规则集引擎设计 · R3](/v2/rule-sets-design#r3-节点侧出口分割)。
- Console：添加节点页给出 Linux 安装步骤（发行版 `install.sh`，再以文件中的接入密钥 `meshway up --enrollment-key @/root/enrollment-key`；Server origin 与发行版本来自构建变量 `VITE_MESHWAY_SERVER_URL`／`VITE_MESHWAY_RELEASE`／`VITE_MESHWAY_RELEASE_REPO`，缺失时保留占位并注明），并只列出仍可用的接入密钥（各带所接入对象、创建者、截止与撤销操作；已用／撤销／过期的见活动记录）。节点运行详情显示其出口状态与原因、已安装地址族及出口 DNS；设备与密钥显示节点钉住的 Server 签名钥（current／rotating／mismatched），不符时提示 `meshway up --reenroll` 并在此批准；概览的注意事项加入出口／出口 DNS 不可用、Server 钥不符与 netmap 超限。源编辑器说明 `{"retained": true}` 的含义并单独提示 `retained_secret_unavailable`。路由目的可输入目录集合（`geosite:openai`／`geoip:cn`）并边输入边搜索目录（`listRuleSetCatalog`），inline 集合支持 `domainSubdomain`，remote 集合支持 mihomo 文本列表与 classical；已绑定内容的集合显示各类条目数与跳过数并可打开转换报告，可查询名称／地址是否命中及决定成员（`matchRuleSet`）、按类浏览成员（`listRuleSetEntries`）；每个未禁用的目录可按名称前缀浏览。

## v0.1.0-alpha.3

wire 6、`meshway-netmap-1` 不变；IPC 升为 7；HTTP 契约新增可选 `NodeState.exitDns`、`leaveNode`，删除 `ExitUnavailableReason.dnsUnavailable`；`ConfigurationSourceResponse.configuration` 改为可编辑文档（`NetworkConfigurationInput`），listener 秘密字段为 `ListenerSecret`。

- CLI（C2）：`meshway logout` 经新 Node 端点 `POST /agent-api/v1/leave` 让 Server 以管理员删除的同一事务永久删除本节点，确认后本机清除身份与 netmap／peer 事实缓存并下线，Server 不可达或拒绝时不变，见 [自助注销](/v2/configuration-reference/nodes#node-self-leave)。`meshway netcheck` 一次有界地复用 Manager 的同 socket STUN 观测与控制客户端，报告映射、端口保持／端点相关性、STUN 与 Server 往返，见 [netcheck](/v2/node/diagnostics#netcheck)。`meshway status` 先显示本节点摘要，再列 peers（direct／relay／idle／unreachable／offline、承载、远端、最近一次测量 RTT），IPC Status 新增 `node`／`peers`。`meshway ping` 每探测一行、末行汇总，preparing 只在 `-v` 下显示。颜色与表格线只用于无 `NO_COLOR` 的 TTY；所有命令有 `--json`；读端关闭时静默退出。IPC 7：`Logout`、`Netcheck`、`ResultBody::Netcheck`，见 [CLI 与 IPC](/v2/node/runtime#cli-与-ipc)。
- 配置秘密（S2）：`GET /configuration/source`（含历史 revision）不再返回已保存的 VLESS `uuid`、Trojan `password` 与 TLS `privateKey`：`configuration` 是完整规范值的可编辑文档、`sourceJsonc` 是注释不变的原文，二者都以 `{"retained": true}` 表示这些值；证书照常显示。原样提交标记保持同一 listener 的已保存值，没有可保持的值（新建／删除再创建、改协议或 mode、managed 改 serverName）时为 422 `retained_secret_unavailable`；显式值照常完整校验。在 dev 配置副本上实测：source 两种形式都不含私钥，规范文档原样 CAS 回去与恢复原文各成一版且 `compare` 无变化，lax 的 listener 视图只变 revision；证据 `.meshway/node-lab/s2/`。见[秘密保留标记](/v2/configuration-reference/editing#secret-retained-marker)。
- 注销意图（S2）：`meshway logout` 先把意图写入 `control.json`，Server 确认或此后拒绝该身份（旧 session `401` 后新认证得 `403 node_identity_rejected`）即完成本机清除；没有回答时保持新 phase `leaving` 按 1–15 秒退避重试、重启后继续，期间 `up`／`rotate-key` 返回 `ErrorCode::Leaving`，只有 `up --enrollment-key` 覆盖；`403 permission_denied`／`422` 撤回意图。注销完成后的目录是普通未接入安装（`daemon.json` 保留 `desiredUp=false`，`identity/` 只剩空身份信封与 `clock.json`），重启不恢复任何注销。lab 一次性第四节点分别以容器内 nft 丢弃回答、`docker network disconnect` 阻止发出实测：`leaving` 跨容器重启保留，恢复网络后自动完成、Server 侧节点消失，之后重启以未接入状态启动；证据 `.meshway/node-lab/s2/`。见[自助注销](/v2/configuration-reference/nodes#node-self-leave)。
- 测试：`keepalive_edge_is_resident_without_interest` 偶发失败是测试取到了随后按 rank 被双向拨号收敛替换的入站 channel（Manager 行为正确）；改为等待两端持有同一收敛 channel，50 次 0 失败。
- 删除不再生产的 `ExitUnavailableReason::DnsUnavailable`；出口 resolver 不可用只见 [exitDns](/v2/state-reference#exitdns)。
- 本机运行（D1）：`daemon.json` 缺失时，目录里只有 daemon 自己及其组件的文件（Underlay journal 与 `*.stale-*` 归档、bootstrap 记录／socket、`identity/`、残留 `control.sock`、私有存储 staging）即以未接入状态启动并写入新状态；仍有未知名称、符号链接或类型不符的条目才拒绝。此前手工删除身份文件后，daemon 自己留下的空 `underlay.json` 使 `meshwayd` 报 `Node state is missing from a nonempty directory.` 退出，systemd 每 5 秒重启且不自愈（`meshway-vm` 1700 余次）。文件归属表与判定矩阵见[数据目录与启动检查](/v2/node/runtime#数据目录与启动检查)；证据 `.meshway/node-lab/d1/`。

- 单出口 DNS（X5）：源节点方案使用单一出口时，普通查询在解析前经完整获准路径上的 DNS circuit 交给出口，由出口机器的系统 resolver 解析并沿原路径回答；MagicDNS 本地回答，`useWithExit` split 解析器按其本机 direct／unmanaged 路由；出口能力事实非 ready、出口上游不可用或回答失败时查询失败，从不回落到源节点本机上游。resolved 后端经其代理 stub `127.0.0.54` 保留按链路／后缀选上游，会回到本节点 DNS 的名称与含本地 stub／锚点／Mesh 地址的服务器清单使出口查询不可用；文件后端使用记录的原 nameserver。行为见 [出口 resolver](/v2/node/dataplane#exit-resolver)。
- DNS：上游 UDP 回答带 TC 且记录被截断（resolved 代理 stub 的行为）或超出通告大小时按截断处理，只由独立 TCP 路由完成。
- 出口 resolver 可用性（X5b）：出口机系统 DNS 服务器为 Mesh 地址、经出口路由的地址、本地 stub 或锚点，清单为空，或 resolved 代理 stub `127.0.0.54` 未监听时，只使出口对源回 SERVFAIL，不再使整份 DNS 安装失败；原因报告为 IPC Status 的 `exit.dnsUnavailable` 与 State 新增可选字段 `NodeState.exitDns`（HTTP 契约新增，Server 只接受描述该节点当前 netmap 且其方案为 `serveExitResolver` 的事实）。`exit.dnsUnavailable` 的值类型改为 `ExitDnsUnavailableReason`（此前从不非 null，IPC 版本不变）。见 [exitDns](/v2/state-reference#exitdns)。
- 出口 resolver 与 `dns.overrideLocal: true`：出口以配置的 default resolver 直接作上游，resolved 与文件后端一致（此前 resolved 后端因根域路由回本节点而全部 SERVFAIL，文件后端改用安装前原 nameserver）。回到出口自身 Meshway DNS 域的名称保持 SERVFAIL，出口模式下的 split 后缀须标 `useWithExit`。resolved 后端的出口查询以 systemd ≥ 248 且 `DNSStubListener=yes` 为前提，见 [Linux 安装](/v2/install)。
- Server sync：一次 hold 不越过节点持有的租约，上限为 `min(名义 hold, 持有租约剩余 − 15 s)`；Server 进程尚未回答过的节点立即回答。60 秒租约下 hold 不再越过到期，见 [netmap 设计 · 传输](/v2/node/netmap-design#sync-transport)。
- 验收：`exit/off` 增加 DNS 阶段（第 9 阶段），Planck 16 行矩阵 `20261011T143117Z-9dedf85e` 全部通过，`exit/off` 245.0 秒（此前 228–238 秒）；X5b 后 `exit/off` 恢复以 E 上的路由阻断其 DNS authority，16 行矩阵 `20261011T192758Z-85bc66e6`（`b5644c06`）与 `20261011T195821Z-aa2222be`（`e0449eae`）全部通过；见[出口场景](/v2/node/validation#exit-scenario)。

## v0.1.0-alpha.2

Linux 系统服务修复预发布，不承诺兼容性；wire 6、IPC 6、`meshway-netmap-1` 不变。

- DNS：resolved 后端的归属改由 namespace、Underlay DNS 锚点和锚点链路上回读一致的标记设置证明，不再绑定 resolved 的 D-Bus 唯一名；`systemctl restart systemd-resolved` 后 Mesh 名称 5 秒内恢复，`stop` 期间 fail closed，`start` 后 10 秒内恢复且无需 `up`，见 [systemd-resolved 重启与停止](/v2/install#systemd-resolved-重启与停止)。已撤回 scope 的查询 owner 不服务后来发布的等价 scope（此前 Mesh 名称可能在 applied 状态下持续 SERVFAIL）；锚点自身的创建不再触发网络代际变化而中断受保护控制连接。
- 本机运行：`SIGKILL` 后的 stale IPC／bootstrap socket 按 inode 回收，状态卷重编号（device 变化）不再阻止启动；上一 boot 的 journal 归档只保留最新三份；bootstrap 查询按当时内容读取 `/etc/hosts`。
- 首次接入：Control 就绪即向 Engine 提交 packet key、instance 与时钟下限，首次接入的 serial 1 不再在邻接启动时因校验失败而等到 serial 2 才安装。
- CLI：`up`／`down` 在 Node 处于 `Retrying` 时按已保存意图收敛，只有 `Blocked` 报告失败。
- 验收：`udp/resolved` 行增加 resolved 重启阶段；NAT 行的 router 在撤权前完成 traffic 阶段内核快照，`nat+fallback/off` 不再因快照晚于撤权而偶发缺少 ESTABLISHED 退路记录。
- 已知限制：显式 `resolv-conf` 后端在 `/etc/resolv.conf` 被外部改写后保持归属冲突，需管理员恢复，见[当前限制](/v2/install#当前限制)。

## v0.1.0-alpha.1

首个 Linux 预发布，用于验证发布与安装流程，不承诺兼容性。

- 产物：`meshway-x86_64-unknown-linux-musl.tar.gz`、`meshway-aarch64-unknown-linux-musl.tar.gz`（`meshwayd`、`meshway`、`meshwayd.service`），`install.sh`，`SHA256SUMS`。
- 安装：`install.sh` 校验并安装为 systemd 服务（`Type=notify`、capability 限于 NET_ADMIN／NET_RAW／NET_BIND_SERVICE／SYS_PTRACE、`ProtectSystem=strict`），支持 `--version`、`--artifact`、`--uninstall [--purge]`；见 [Linux 安装与系统服务](/v2/install)。
- 新命令 `meshway version`：程序版本、git 描述、IPC 6、circuit wire 6、`meshway-netmap-1`。
- 范围：Linux Mesh-only TUN、Native UDP／TCP、VLESS／Trojan TCP/TLS、协调 UDP 穿透、resolved／resolv-conf 系统 DNS、`meshway ping`、签名 netmap 租约内离线运行、IPv4 出口；验收状态见[验收范围](/v2/node/validation)。
- 已知限制：systemd-resolved 重启或停机后 DNS 归属冲突、数据面不自动恢复，需重启机器，见[当前限制](/v2/install#当前限制)。

## 2026-10-07

| 工作包 | 合入提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| H 流水线 | `2fcc2b10` | owner／worker API、PacketBuffer 贯通 Session、L4 `manager` 阶段 | runtime `20261007T010018Z-e430176f` |
| J tun-rs codec | `0a03e15f` | `meshway-tun` 改用 tun-rs 的 `gso_split`／`GROTable` | tun `20261007T021421Z-42104e65` |
| K UDP GSO 边界 | `19938b7b` | 地址族 payload 上限、仅 GSO 上下文的 EIO 降级、接收区间引用 | packet `20261007T022232Z-c09c4a42` |
| I 流式释放 | `85443d60` | 持久 crypto 队列、有序前缀释放、控制优先、harness `--modes` | runtime `20261007T034408Z-e0dce6be` |
| L 密码归因 | `d0e60ba9` | `perf:crypto` 工具与 L0 归因 | crypto `assessment.md` |
| M 密码后端 | `7eac33a6` | ring AEAD、硬件 SHA2 预 key HMAC | runtime `20261007T044453Z-eff72ec3`；L3 `20261007t045428z-fb7362ea` |
| MTU 默认 | `05f0b933` | 内层 MTU 1366 由固定帧开销导出 | — |
| R 切片校验去重 | `83caeb36` | `validate_obsolete_slice` 共享实现 | — |
| S crate 卫生 | `a4dffc3c` | 未用依赖、workspace 继承、测试独占 wrapper | — |
| O owner 路径 | `5a30247d` | UDP 7 MiB 缓冲、Circuit 稳定分配、共享 Context | L3 `20261007t064600z-c4127ba8` |
| Q 控制面 | `ebd42554` | SSE 刷新、换钥、接入密钥、IPC v4、Server SSE 热点 | runtime `20261007T090243Z-fb5f1714`／`20261007T091327Z-063ce0a0` |
| X 回执分类 | `c3cbed6c` | 回执 409 按原因分类，非 fencing 恢复，状态投影 | runtime `20261007T100459Z-ea847031` |
| Y Manager 路径 | `98855128` | 未准入事件队列、批次 scope 检查 | L3/L4 `20261007t112743z-6f3b2103` |
| Z 内存 | `6725d11b` | musl daemon 采用 mimalloc、TUN 存储复用 | `z-validation/receipt.md` |
| AA 控制面风暴 | `f39ba2b6` | SSE 读空闲超时、候选查询节流 | runtime `20261007T154104Z-ac45dd04`／`20261007T155007Z-80295767` |
| PT 性能收尾 | `6e3119b2` | Manager 批前缀消费、`--quick`、performance.md 重组 | L3/L4 `20261007t204126z-cbe7469c` |
| T 二进制 | `3887256b` | ring-only TLS、fat LTO、features 收紧 | `binary-size/20261007T192625Z`；runtime `20261007T204330Z-e843809f` |
| W1 承载模型 | `a8ed42d6` | `node/carriers.md` | — |
| V1 矩阵重验 | `9b30c19b` | 满 MTU／多 MSS 用例；v5 矩阵 5 行通过、4 行未通过 | `validation/v1/20261007-main-3887256b/` |

| CI1 矩阵工具 | `c18f4377` | `validate:matrix`／`validate:remote`、`--short`；planck 全矩阵 14.5 min | `validation/remote/CI1-summary-12ec8a16.md` |
| ST1 状态页 | `f17b59d0` | `scripts/ci/collect.py`、`ci-results` 索引、`/status` 页、`status.yml` | — |
| TQ 测试收敛 | `b5e48bc7` | nextest、共享 Control setup、测试 KDF profile、pipeline 测试合并、fixture 共享 rendezvous | planck `test:rust` 482 → 87 s；M3 全量 check 934 → 202 s |
| 流程／CI | `4385a0c9`…`60e966a3` | process.md、changelog、workflow 拆分、四台 runner | — |

未合入：AB `perf/daemon-dataplane-runtime`（专用数据面运行时，无吞吐收益，分支已打包归档）。

## 2026-10-08

| 工作包 | 合入提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| 脚本整理 | `ead029e6` | 删除无引用的 lab OS 检查；fixture 只记录负载不等待空闲；`perf:crypto:attribute` 任务 | — |
| 流程 | `d91e9ffc`、`a5982814` | 整组验收异步化、靶证据定位；`profile.fast`／`check:fast`；[测试清单](/v2/tests) | M3 fast 945 项 51 s |
| V2 NAT／resolv-conf 回归 | `da736c2b`…`641c27ca` | STUN 观测刷新与邻接查询分开调度；State 采样先处理排队 target 并确认回执；退役需求取消；fixture 识别 v5 帧 | planck 四行各两次通过 `20261008T032743Z-897a549e`、`20261008T050147Z-0c5809cf`；回归行 `…-abbd4545`、`…-7a35b80e`；`.meshway/validation/v2/20261008-nat-regressions/` |
| TD 测试整理 | `6371afbc`…`214df852` | Server／transport 测试文件 ≤800 行与可执行上限检查；2 条真重复测试删除；修复 State provider 重复关闭钩子强引用；`test:fast -- -p` 生效 | `.meshway/td/TD-receipt-20261008.md`；Server 测试行 −1.6%（去重假设基本不成立） |
| main 全矩阵 | `1271be5d` | v5 矩阵首次 9/9 通过（含 `udp/resolved`） | planck `20261008T053753Z-0f9ac9d2`，20.7 分钟 |
| L5 复测 | `5837bd33` | lab 2 CPU／节点 sha→lax→nyc：TCP 1321／1525 Mbps、UDP 475 Mbps | `.meshway/perf/l5/20261008T055333Z-main-1271be5d-2cpu` |
| CI runner 可移植 | `49dc2e8e`…`e9c7a376`、`4e8a6f9d`、`5c2a012f` | DNS 文件后端测试自探测可信目录；`DOCKER_CONTEXT`／buildx builder 跟随 context；状态收集器改标准库 HTTP；`MISE_GLOBAL_CONFIG_ROOT` 排除 runner 个人配置 | mars-14900k／jp-tyo 四类失败各一 |
| W2 流式承载 | `46c65e47`…`36840b3b` | runtime fixture 与矩阵增加 `vless/off`、`trojan/off`；相同材料执行交接保留流式认证通道 | planck 两行各两次通过 `20261008T060724Z-15ece3bb`、`20261008T061606Z-9875d228`；`.meshway/w2-streaming/` |
| TD2 测试拆分 | `d9baecb0`…`06f33739` | connectivity／policy 六个超限测试文件按行为拆分，共用 harness，白名单清零（271 文件 0 例外） | `.meshway/td2/`；测试数 52／86 不变 |
| W4 边级承载 | `cd536833`…`8ca49b0f` | `topology[].transports`、双向拨号交集、`edge_transport_unavailable` 定位规则／边、执行候选过滤、OpenAPI／SDK | policy `tests/execution/topology.rs`、`tests/configuration/topology.rs`；`.meshway/w4-edge-transports/` |
| W3 lab VLESS 边 | 本提交 | lab revision 7 验证 sha→lax VLESS TCP/TLS、双向原路径诊断与高负载 L5；部署版 `1271be5d` 缺 W2 续接修复导致流式链路重建，回滚 revision 8 | L5 `20261008T063234Z-main-1271be5d-2cpu-vless`；`.meshway/w3-lab-vless/`；复现分支已打包归档 |
| V3 中继交接丢回复 | `6af24659`…`c31541bd` | retained 执行交接时中继保留诊断请求关联与原期限；maintenance 保留有界待交接关联；重放保护与旧 scope 失效不变 | CI 证据 `37762142607` `udp/resolved` 首探测超时；复现 `retained_relay_delivers_inflight_diagnostic_and_business_reply_exactly_once`；planck `20261008T170200Z-bbdb811a` 三行通过；`.meshway/validation/v3/` |
| V4 重启不卡死 | `fix/underlay-restart-journal` 系列 | journal 的 boot／netns 与当前不一致时原子归档为 `underlay.json.stale-*` 并按无 journal 启动；当前 netns 内活 owner 冲突仍拒绝但有 1–30 s 退避；损坏 journal 归 Storage | 现场 `.meshway/w3-lab-vless/incident-20261008T1655/`；隔离测试 `container_restart_archives_previous_namespace_journal`、`live_tun_owner_conflict_retries_and_recovers_without_new_up`；planck `20261008T183321Z-9b32fb72` 两行通过；`.meshway/validation/v4/` |
| W5 节点自报端点 | `feat/node-reported-endpoints` 系列 | daemon 在 State `sockets[].localEndpoints` 上报 listener 的本机接口地址（≤8，排除 loopback／link-local／Mesh／TUN）；Server 编译候选 configured→local，带来源；Manager 全部承载按候选拨；`acquisition: localEndpoint`；管理员 `endpoints` 变为可选；lab `configure` 不再写 bridge IP；compilerVersion `meshway-demand-5`、Control record v3、IPC v5 | planck `20261008T185410Z-1d4429c8` 四行通过（`udp/off` 为 reported-only）；`.meshway/w5-endpoints/` |
| V5 运行中接口变化 | `fix/live-interface-change` 系列 | kernel monitor 忽略非内核来源 multicast（原误判为永久保护冲突，使 control 瞬时失败不恢复）；接口／地址／路由外部变化 → 关旧控制连接重连并重枚举本机端点；自有 suspend 的 DelAddress／DelLink 不算外部变化；localEndpoints 按自有接口集合排除 TUN／DNS 锚点；fixture 保留已观测执行截止 | 现场 `.meshway/validation/v5/analysis.md`；`control_reconnects_after_live_interface_replacement`、`owned_suspend_keeps_control_connection_and_reports_no_network_change`；planck `20261008T205449Z-84befa85` 四行通过 |
| X0 Exit/NAT 设计 | `e3ba2519` | `docs/v2/node/exit-design.md`：对照 Tailscale（router_linux／linuxfw／peerapi DoH／table 52）的首版方案：Linux IPv4 内核转发＋masquerade、独立出口 TUN、nft CLI 批事务、ip_forward 只检测、经 Mesh 的出口 resolver、X1–X7 拆包 | 研究基线 tailscale `ab2380a6` |
| X7a Exit 验收准备 | `31f8880f`…`f492fe5d` | `exit/off` 隔离拓扑（Controller／A／R／E＋外部服务＋E LAN 服务）、镜像锁定 nftables／conntrack／tcpdump、fixture 八阶段 `pending-x2-x5`、证据链；默认矿阵不含该行 | planck 短跑 `20261008T222757Z-a9263023` 以 `unsupported` 如实结束；`.meshway/exit/x7a/` |
| X1 Exit 契约与联合编译 | `feat/exit-contract` 系列 | `EgressCapturePlan`／`ExitRolePlan`／`ExitPermit`／`ExitCapability`，first-match CIDR 互斥分割，`is_internet_address` 唯一公网边界，能力不可用投影不回退 direct，compilerVersion `meshway-demand-6`；Control record v4 退役 demand-5 缓存；Console 原因文案 | policy `tests/execution/exit_contract.rs`；`.meshway/exit/x1/` |
| X5 单出口 DNS 契约与客户端 owner | `ef68c08a`、`7b4519ec` | `DnsServicePermit`、`DnsEgress::ViaExit`／`ServeExitResolver`，选择顺序 MagicDNS→useWithExit split→出口 resolver，联合许可与到期交集，客户端密封 `MeshResolverRoute`，默认适配拒绝安装（出口侧接线等 X2／X3／X4） | `.meshway/exit/x5/{receipt,interfaces}.md` |
| X2 dataplane 出口流 | `09da0d7f`…`9dec9b9a` | IPv4 出口 flow 按有界内核投影门控（`ExitProjection` Install／Renew／Retire＋ACK 前不放首包）、回程经唯一 FlowTable 原路径、Replacement 结束旧流、tombstone ≥120 s、容量 1024 流／128 投影／4096 槽；无 Underlay 依赖 | `tests/runtime/exit_lifecycle.rs` 13 项；`.meshway/exit/x2/` |
| V6 NAT 退路偶发 | `957aa298`…`5629fc06` | STUN 刷新复用有效 discovery context、截止前 8 s 续取元数据；已选流式通道结束后按来源优先级重新获取而不被 local UDP 黑洞阻塞 | CI `37849292927` 与 `20261008T230914Z-5380356e` 两次失败证据；`live_discovery_context_refreshes_same_socket_without_another_metadata_rpc`、`selected_tcp_fallback_reconnects_before_blackholed_local_udp_after_peer_health_skew`；planck `20261008T235755Z-8600ca5d` 两行通过；`.meshway/validation/v6/` |
| TD3 测试 lease 赛跑 | `test/lease-race-hardening` 系列 | 46 个真实时间测试逐个分类（A 流程跨 lease 17／B 固定等待 1／C 真到期 28）；A 类改为按产品方式续租或把断言靶近，B 改条件等待；两条已出事测试在 load 15–52 下连续 5 次通过；不改产品代码 | `.meshway/td3/{inventory,verification}.md` |
| X3／X3a Underlay 出口 owner | `cb464ae0`…`d86a80e5` | 出口前置探测（ip_forward／TUN／rp_filter／nft 真事务／conntrack）；`Host::install_exit_role`、`ExitRole::{install→ProjectionAck, confirm→CtConfirmation, renew, retire_flow, reply_reader, retire}`；私有 `inet meshway_exit_*`／`ip meshway_exit_nat_*` 表，仅 NEW∧未归属∧资格命中打 ct mark，masquerade 限所属 ct，精确 ct 删除，`underlay-exit.json` journal 与 SIGKILL 同 namespace 恢复；不要求 nf_conntrack_acct | planck 隔离 `exit_nat_udp_real_reply_qualification_confirm_retire_and_expiry`、`exit_recovers_real_sigkill_live_ct_in_same_namespace` 等 10 项通过；`.meshway/exit/x3/X3a-receipt.md` |
| X2b 精确关流 | `7799b955` | `SecureNode::fail_exit_projection(&FlowIncarnation)`：CT 确认失败后按 incarnation 关流，不影响共享 CID 的其他流 | `exit_confirm_failure_isolates_shared_cid_and_survivor_can_renew` 等 2 项红→绿 |
| V7 诊断抖动与首包延迟 | `fix/diagnostic-publication-churn` 系列 | 切片不变时复用 publication；安装门槛改为"本节点至少参与一条完整候选路径"（可选中转误拒）；`state_scope_changed` 作为正常 scope 刷新不进 Retrying；去掉 5 s 执行通知延迟、1 s 首轮等待、100 ms 握手轮询、Server 250 ms 全量重授权改事件驱动 | planck `20261009T042152Z-24bad550` 两行通过；lab 60 s 诊断 Retrying 4→0、Dataplane failure 6→0；直连首包 7.6–12.3 s→0.6–2.0 s（4/5 ≤1.5 s），一跳中继 3/5 ≤2.5 s；订阅延期仍产生 publication（留 N1）；`.meshway/validation/v7/` |
| X3c Underlay 接口补齐＋X3b capture | `feat/exit-underlay-x3c` | `Host::probe_exit`（已开 Host 上探测，成功 Ready/ipv4）；`ExitRole::send` 带每包 `ExitSendGuard`；`ExitIncarnation([u8;32])`；ICMP 按 identifier＋sequence 资格共享 CT；中继 R 无 exit role 可装外部 permit；`Host::install_egress_capture`／`EgressCapture::switch`（`/0`→客户端 TUN、direct 例外、IPv6 sink、并集负向门切换、SIGKILL 后负向保留） | planck 隔离 9/9＋真实 Control 场景；`.meshway/exit/x3c/receipt.md` |
| X3d 出口 role 合法交接 | `1a3ac88e`…`8ee294b5` | predecessor guard 失效时 role 进入 retained（拒旧 I/O、不删 CT／资格），直接后继续接，retention 截止才清理；停止的 role 先 reconcile 再探测；nft 高位 mark prefix 回读规范化（原误判所有权冲突） | X4 消费者测试 `daemon_exit_control_handoff_preserves_projection`、`daemon_exit_dns_role_refresh_preserves_applied` planck 绿；`.meshway/exit/x3d/` |
| X4 daemon 出口装配 | `8f10572c`…`ec56f8bc` | Control 绑定 role／能力到当前 instance；authority 保留投影原截止；`worker.rs::install` 用 `probe_exit` 上报能力、role 与 capture 安装进原子 applied；`runtime/network/exit/` 桥接 Install→ACK→guarded send→confirm→Renew／Retire 与回程 `exit_reply`；DNS 变体单独 unavailable；IPC v5 可省略 `exit` 字段，`meshway status` Exit 行 | planck 隔离 4 项＋X3 套件 22 项通过（真实 Server＋三 Control＋SecureNode＋真实 NAT）；`.meshway/exit/x4/{receipt,x7b-handoff}.md` |
| 2026-10-09 code review | `.meshway/review-20261009/` | 四份只读审查（Server／node-control／dataplane-connectivity／underlay-dns），93 条 findings：remove 29、simplify 33、keep 31；一致结论：保留安全与一致性内核，不再在逐 demand publication 链上加功能，N1 是最有价值的单一改动 | `SYNTHESIS.md` |
| N0 netmap 设计 | `08410a1a`、`e9f7a47f` | `docs/v2/node/netmap-design.md`：netmap 形状（内容 digest 与 24 h 租约分离）、稳定策略／动态事实两条线、本地准入、`SecureNode::update`、邻接按需、撤销与到期、迁移、remove 清单对照（消失 26／变形 3／保留 0）、拆包 N1a–N1j；五个产品决定已记录 | — |
| TD4 低风险清理 | `ea9d626e`…`7bec72de` | review 中不依赖 N1 的 11 项：死 wrapper／不可达分支、仅测试的 pause/resume、被 streaming 取代的 Batch API、refresh 双入口、复制的一致性循环、丢弃结果的 D-Bus RPC、无消费者的 DNS Owner API、egress 重复 stage、四份 private-write 原语共用、`Shared`→`EpochContext`、`Scope`→`ProtectionScope`；净 −113 行 | planck underlay／DNS 84 项＋X3 17 项通过；`.meshway/td4/receipt.md` |
| X7b `exit/off` 验收行 | `feat/exit-acceptance` 系列 | 八阶段真跑：能力／前置／安装／capture／投影 NAT CT／续期关流／原路径回程／drain 恢复；隔离公网正例 `11.77.98.3` 经 E masquerade 的 UDP／TCP 真实往返；行保持显式选择 | planck 全长 `20261009T080945Z-9fc2ac9c` **未通过**：定位到 X4 产品缺陷（安装期间 `sample()` 1 s 超时被当作 `Report::stopped` 上报，Server 撤回出口资格，三端关流）；`.meshway/exit/x7b/{receipt,x4-sampling-blocker}.md` |
| X4b 采样超时不算 stopped | `fix/state-sample-timeout` | daemon State 采样区分 `Timeout`（跳过本轮完整上报，不改序号／observedAt／lease）与 `Closed`／真实 stopped（如实上报）；X7b 定位的"安装忙→伪 stopped→出口资格撤回→三端关流"消除 | `busy_installation_skips_state_without_withdrawing_exit_then_stopped_withdraws` 红→绿；planck X4 四项通过；`exit/off` 全长仍未通过（新阻塞：A 首流前 20 次 scope 轮换、applied 为空，归因待定）；`nat+fallback/off` 短门槛恢复后 protected relay I/O 超时（归因待定）；`.meshway/exit/x4b/` |
| T1 边级承载顺序 | `topology/transport-order` | `topology[].transports` 为有序承载偏好（重复报 `invalid_transport/repeated_transport`），省略用平台顺序 udp→tcp→vless→trojan；dialer 删除 `priority`，只表达启用与进阶选项；`DialCandidate.priority` 由边序生成，Manager 不变；compilerVersion `meshway-demand-7`，demand-6 缓存退役 | policy `tests/execution/topology.rs`、`tests/configuration/topology.rs`；server `app::tests::cache` demand-6 |

## 2026-10-11

| 工作包 | 合入提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| S1 系统服务与发布 | `s1/install-release` | systemd unit（`Type=notify`、最小 capability、沙箱）、`install.sh`（下载校验／本地产物／升级幂等／卸载）、`meshway version`、`release.yml`（tag＝workspace 版本、复用 `build.yml`、`SHA256SUMS`、changelog 段作说明）；workspace 版本 `0.1.0-alpha.1` | OrbStack Ubuntu 24.04 安装／重启（含 Server 不可达）／resolver 间隙实测，`.meshway/node-lab/s1/` |
