# Changelog

按合入 main 的顺序记录工作包、提交与证据 ID。规则见[工程流程](/v2/process)；性能数字的口径见[分层性能测量](/v2/node/performance)，验收状态见[验收范围](/v2/node/validation)。

## 未发布

系统 DNS 健康检查（D3，`d3/dns-observe`）：wire 6、IPC 12（`lastErrorDetail` 语义扩展，字段不变）、`compilerVersion` `meshway-netmap-3`、Server schema 8 与 HTTP 契约均不变。planck 四次 DNS owner 阻断（alpha.8／alpha.9）的触发源是宿主 WiFi 驱动 PTK 重新协商时约 5 秒的宿主范围 RTNL 占用，见[D3 证据](/v2/node/validation#d3-dns-health)。

- 中断或失败的锚点读取不再结束 DNS 锚点：只关闭该读取 socket，下一次读取另开；锚点只在 owner 删除它或宿主内核归属结束时结束。停顿后 owner 的每秒重证即成功并自动重装（此前阻断至重启）。
- DNS owner 的每次失败记录唯一一项检查：WARN 带 `check`、`expected`、`actual`、`source`，同一失败只记一次；Status `lastErrorDetail` 给出同一行文本；observe 超时为 `observe_deadline` 并带实际耗时。检查项与可恢复边界见[健康检查与恢复](/v2/node/dataplane#dns-health)。
- 结束宿主内核归属的 egress capture owner 与控制中继监听失败现在各记一条 WARN（此前无日志）。

## v0.1.0-alpha.10

范围：wire 6、IPC 12、`compilerVersion` `meshway-netmap-3`、Server schema 8 均不变（普通升级，无断网窗口；CLI 与 daemon 一起升级，IPC 12 新增命令 `ClearProfile`）。HTTP 契约新增 `DELETE /agent-api/v1/routing-selection`（`clearNodeRoutingProfile`）与 `DELETE /api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection`（`clearNodeRoutingSelection`），`ActivityTarget.routingSelection.profile` 改为必有可空。X8：出口角色失败只撤该角色并自动重装、echo 流寿命（等待回复至多 10 秒、回复后 2 秒结束）、echo 连接 CT 换代按归属处理、出口 cipher 随最后一条流结束向来源发 `Close`；H2：清除方案选择（`meshway exit --default`）、`idlePath` 多中继排序修正；以及下列 H2、X8 工作包。

OPS8 后续三项（H2，`h2/ops8-followups`）：wire 6、IPC 12、`compilerVersion` `meshway-netmap-3`、Server schema 8 均不变。

- 清除方案选择：新增 `clearNodeRoutingProfile`（`DELETE /agent-api/v1/routing-selection`，`{instance, expectedVersion}`，CAS 同 `selectNodeRoutingProfile`）与 `clearNodeRoutingSelection`（`DELETE /api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection`，`{requestId}`，主体与重试规则同 `setNodeRoutingSelection`）；节点回到 assignment 默认方案（含 `auto`），版本计数保留；活动仍为 `routing.select`，`ActivityTarget.routingSelection.profile` 改为必有可空（清除为 null）。IPC 12 新增命令 `ClearProfile`（旧 daemon 不认识该命令，旧 CLI 不受影响），CLI `meshway exit --default`，无请求方案时显示 `none · following the default`。Console 活动列表显示“已清除方案选择”。`exit/off` 第 14 阶段末尾加 ⑥ `meshway exit --default` 回到 `auto` 解析。见 [Routing · 选择的入口](/v2/configuration-reference/routing#选择的入口)、[出口方案](/v2/node/runtime#exit-profile)。
- `idlePath` 与 state 一致：`unreachable`／`offline` 的 peer 总是 `no path`（此前 it7→planck 离线时显示 `relay (idle) via meshway-vm → meshway-lax`），`relayIdle` 的 `idlePath` 即 `via` 那条路径。根因在中继排序：`ranked` 只看第一个中继是否与其后节点有通道，于是“vm 与 lax 相连”使两中继路径排到所有单中继路径之前；现在要求路径上每个中继都报告与其后节点的通道（与 `relayIdle` 同一条件），新 circuit 的选路随之改正。见[中继兜底](/v2/node/connectivity#relay-fallback)、[运行状态 · peers](/v2/node/runtime)。
- 升级文档：[安装 · compilerVersion 变化的版本](/v2/install#compiler-version-upgrade)——全部 Node 与 Server 同一窗口升级、先备齐全部产物（含 lab 构建）、仍先 Node 后 Server，每台节点从升级到 Server 完成之间无 Mesh（`Sync reconnecting (contract)`）；changelog 中 `compilerVersion` 变化的版本在范围句首标注 “⚠ 断网窗口”（alpha.9、alpha.4 已补标注）。
- `exit/off` 第 12 阶段等待 A 每秒一次的 MSS clamp 刷新再读表（H1 的偶发失败即在刷新前读表）。

验证：Planck 18 行矩阵 `20261014T210943Z-8bc0b498`（候选 `3a47c644`，并行 4）全部通过，见[出口场景](/v2/node/validation#exit-scenario)。

出口失败不恢复与出口投影容量（X8，`x8/exit-recovery`）：wire 6、IPC 12、`compilerVersion` `meshway-netmap-3`、Server schema 8 与 HTTP 契约均不变。OPS8 planck 出口 17:47:10Z 的完整链（隔离复现于 main 产品代码）：每个 ICMP echo 请求一条流、投影存活约 57 秒，2 次／秒的 ping 即占满 128 个投影（拒绝新流，丢包 68–74 %）；被拒期间该 echo id 的 conntrack 条目 30 秒超时，后一序号新建条目，此后同 id 的安装一律 `Conflict`；`maintain` 退役记录旧条目的投影时把新条目当作外来条目，出口 owner 停止且清理失败；daemon 把运行中出口角色的失败当作整个数据面失败撤回全部内容，Engine 重试撤回时 `Kernel::suspend` 在 `remove_live` 失败后照常关闭 Mesh TUN（已脱离出口 TUN 的出口规则带 `IIF_DETACHED`，不再等于登记的出口规则，被当作外来规则），内核删除其 IPv6 capture 路由，monitor 报 `unexpected_del_route`，保护冲突永久，控制中继、DNS 与 capture 全部失败直至重启。alpha.8 15:35Z 的阻断不同源：DNS owner 先失败，出口角色随后被干净撤回。根因与证据见私有 `.meshway/validation/x8/root-cause.md`。

- echo 流寿命：每个 `(id, sequence)` 流等待回复或其 ICMP 错误至多 10 秒，收到回复后 2 秒结束（每一跳流表相同，不长于空闲设置）；出口投影随流退役。逐序号流与 quarantine 不变（对比按 `(src, dst, id)` 聚合：需要按 id 的宽 tombstone 或无界的逐序号 tombstone、迁移 nft 程序与 journal，不在本包）。见[数据面](/v2/node/dataplane)、[IPv4 出口角色与 NAT 投影](/v2/node/runtime#ipv4-出口角色与-nat-投影)。
- echo 连接的 CT 换代不是归属冲突：退役记录旧条目的投影时，若当前条目可归属于同一连接另一条未退役投影（持有或尚待确认），旧条目已证明不存在，按其 reply tuple 与序号进入 quarantine、不删除当前条目；安装新序号同样只要求可归属。不可归属仍为冲突。
- 出口角色失败只撤该角色（X7 语义延伸到运行中的角色）：driver 在 Host owner 自行停止时报告 `ended`；`ExitFailed` 与回包 reader 失败只结束出口 runtime，立即真实探测，`ready` 即 `Engine::reapply` 重装；netmap 保持 applied。Underlay monitor 观察到出口自有路由或规则（含 `IIF_DETACHED`）在 owner 之外被删除时只停止出口角色；外来新增仍是整个 Host 的冲突。已自行失败的角色不再使 Host 的新内容安装或撤回失败，其保留规则不使 Host 校验失败。monitor 冲突日志新增 `table`／`interface`／`prefix`。
- 出口 cipher 随最后一条流结束时向来源发出逐跳认证的 `Close`（中继转发），来源下一包新建 circuit，不再在出口已没有的 cipher 上等 15 秒无回复。
- 容量依据：活投影 128 同时是成对回程规则槽位（257 个 RPDB 优先级，位于 512 个保留优先级内）；nft 集合与 journal 历史 4096；现状保持 128；journal 增量记录与活投影 1024（对数 ≤ 128）为后续 X9。见 [Exit 设计 · 安全边界与限制清单](/v2/node/exit-design#安全边界与限制清单)。
- 验收：`exit/off` 新增第 15 阶段（5 次／秒 ping 60 秒加 traceroute 时投影峰值 ≤ 32 且零丢失；echo 条目两次删除后角色保持；删除出口回程路由后只撤角色并自动重装，期间 E 保持 applied 与 connected，A 的 MagicDNS 与经 E 的普通名称每 0.5 秒解析成功）；组件测试 `exit_recovery::netmap_exit_echo_reconnect_and_route_loss_stop_only_the_role`（main 上在 CT 换代后第 3 个序号安装即失败；仅去掉 monitor 分类时 Host 新 serial 安装 `Conflict`）。见[验收范围 · 出口场景](/v2/node/validation#exit-scenario)。

- 发现、未修复：节点持有的 peer 事实可早于其租约到期（Server 只为内容变化唤醒 hold），HTTP 中断或 hold 期间新出口流可在租约到期前被本地拒绝；第 15 阶段结束前等 A 在角色重装之后取得一次 sync，见[剩余边界](/v2/node/validation#remaining-boundaries)。

验证：合入 H2 后 Planck 18 行矩阵 `20261014T233036Z-e7000147`（`909f2fd6`）全部通过，`exit/off` 行 `20261014T233523Z-9bb7fd1e`；合入前 18 行矩阵 `20261014T220407Z-2c791342`（`09bc15ad`）全部通过，`exit/off` 行 `20261014T221147Z-ccff6958`（第 15 阶段 300／300 回复、投影峰值 26；main 上 120／300、峰值 128）；OPS8 全链在 main 上的复现行 `20261014T200754Z-13ebca51`；证据 `.meshway/validation/x8/`。

## v0.1.0-alpha.9

范围：⚠ 断网窗口（`compilerVersion` 变化，见[升级 · compilerVersion 变化的版本](/v2/install#compiler-version-upgrade)）；wire 6、IPC 12、`compilerVersion` `meshway-netmap-3`、Server schema 8（无迁移，开发环境重新初始化数据目录、节点重新接入；须先升级 Node（CLI 与 daemon 一起）再升级 Server）。AUTO-1：`auto` 方案按管理员声明的 `nodes.<id>.region` 解析（解析结果计入 `egress.generation`，`routing.autoResolve` 活动，`meshway exit` 显示解析结果）；H1：出口投影按轮安装、`meshway status` 的 `idlePath`、listener 退役时释放待接管 socket、Server 测试离线；以及下列 AUTO-1、H1 工作包。

`auto` 方案按地区选择第一版（AUTO-1，`auto1/region-profile`）：wire 6 不变；`compilerVersion` 升为 `meshway-netmap-3`；IPC 升为 12；Server schema 版本 8（`activity_events` 允许 `routing.autoResolve`；开发环境重新初始化数据目录）。HTTP 契约：`NodeSettings`／`NodeSettingsInput`／`NodeSettingsView` 新增可省略的 `region`，`NodeSettingsPatch.region`（null 清除）；`AutoResolution` 新增必有的 `region`（可为 null），`AutoBasis` 新增 `label`、`noRuleMatched`；`ActivityAction` 新增 `routing.autoResolve`；`IssueCode` 新增警告 `auto_region_unmatched`、`auto_region_missing`，`invalid_country_code` 另用于 `/nodes/<id>/region`。升级顺序：先升级 Node（CLI 与 daemon 一起）再升级 Server——旧 Node 对 `meshway-netmap-3` 的 netmap 验证失败而拒收并报告，新 Node 把缓存的 `-2` netmap 视为过时并重取。

- 地区来源：管理员在 [`nodes.<id>.region`](/v2/configuration-reference/nodes#region) 声明 ISO 3166-1 alpha-2 代码，只有 Owner／Admin 经配置提交写入；接入初始设置与节点自助接口不能携带。GeoIP 推断、驻留／防抖与 RTT 未实现（设计稿第二步）。
- 解析：`auto` 取第一条 `countries` 含该地区的规则的方案，否则 `auto.fallback`；basis `label`／`noRuleMatched`／`noTrustedRegion`。分割、捕获、`egress` 计划、DNS plan、可信关联、路径检测与节点判定共用 `matching::concrete_profile`（唯一实现；此前五处各自把 `auto` 写死成 fallback）；`region` 随投影的 `NodeSettingsView` 下发，中继与出口在自己的投影上重算。见 [Routing · 自动方案](/v2/configuration-reference/routing#auto)。
- 修复：`egress.generation` 对 `auto` 只记字面量 `auto`，解析结果改变时 selection 身份不变；现在解析结果（方案、basis、地区）计入 generation，变化即结束旧出口存量连接、删除旧 generation 的可信关联（与手动切换同一路径）。`LocalCompiler` 的分割缓存按解析后的方案为键，不再让不同地区的源共用 `auto` 的分割。
- 选择：手动 `routing-selection` 总是覆盖 `auto`（locked 除外）；选中方案的出口不可用不换方案。`RoutingSelectionState.auto` 给出 `{profile, basis, region}`（不进入 State）；`meshway exit` 显示 `auto → <profile> (region <XX>, basis <basis>)`。
- 活动：解析结果随新 netmap 改变时，Server 在同一事务记录 `routing.autoResolve`（actor 为 system，target 为 `routingSelection {nodeId, profile}`）。
- 验收：`exit/off` 新增第 14 阶段（无地区 → `mesh`／US → `exit` 经 E／改 JP → 旧流断开、旧关联消失、新名称经 E 走 `exit-alt`、活动各一条／手动覆盖与恢复／FR → `noRuleMatched`），见[验收范围 · 出口场景](/v2/node/validation#exit-scenario)。Console 只做字段映射与 en／zh 文案。

小待办（H1，`h1/backlog`）：

- listener 同端口改绑：重试 owner 已绑定、尚未被 Manager 接管的 socket 在该 owner 退役（内容变化使声明改变或 key 消失、Manager 重启／`shutdown`）后仍留在接管队列里，随后同端口的新 bind 得到 `EADDRINUSE`、再进入 1 s 重试。现在退役时同步关闭该 socket 再 bind（唯一 owner `manager::rebind`）；首个 instance 到达的普通 listener → 协调 socket 改绑已是先 join 再 bind，补真实同端口测试。见 [listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)。
- `meshway status` 的兜底路径：此前只有 `relayIdle` 显示 `via <R>`，`idle`／`unreachable`／`offline` 的 peer 不显示路径。IPC 12 的 `peers[]` 新增可省略的 `idlePath`（`route` 为 `direct`／`relay`／`none`，`via`）：任一既无通道也无 circuit 的 peer 给出新 circuit 此刻会走的路径，与首包同一选择（`SecureNode::idle_route`，复用 `ranked`，不另排序）；有通道或 circuit 时省略、显示实际路径。ENDPOINT 列为 `direct (idle)`／`relay (idle) via <R>`／`no path`；旧 daemon 不发该字段时 CLI 保持原显示。见[运行状态](/v2/node/runtime)。
- 出口投影安装按轮摊销：此前每条出口流（每个 ICMP echo 请求也是一条流，键为 `(id, sequence)`）的投影安装各自串行做一次表归属 dump、两次日志落盘、一次 nft 事务与一次回读 dump（planck 隔离测试单条约 55–90 ms），traceroute 的各探测依次排队，第 12 阶段三跳 RTT 为 66／119／173 ms。逐流的精确语义（流键与按序号 quarantine）不变；投影 driver 把紧随第一条到达的新增投影（间隔 ≤ 3 ms，一轮 ≤ 10 ms、64 条）交给 Underlay 作为一轮，dump 校验、日志落盘、nft 事务与回读每轮各一次，每条仍各自做归属／冲突检查、各自先写 journal 记录、各自在回读中核对元素后才 ACK，结果逐条回执，一条失败不影响其他条（整轮事务失败时逐条重试）；续期、退役与 conntrack 确认不合轮。实测：8 条一轮 44 ms，对照逐条 373 ms；`exit/off` 第 12 阶段三跳 66.26／66.61／66.62 ms（后续跳增量 0.35 ms，断言 ≤ 20 ms）。新增隔离测试 `netmap_exit_projection_round_acknowledges_each_flow_by_itself`（同轮内重复 tuple、回环目标与已过截止各自被拒，其余逐条确认）。见 [Exit 设计](/v2/node/exit-design)。
- Server 测试离线：`App::open_with_connector` 是 App 装配资源取回连接器的注入点（`resources::download::{Connector, Https, Offline}` 公开；`App::open` 仍为生产 `Https`）。Server 集成黑盒测试与各测试 Server 改用 `Offline`；启动生产可执行文件的 SSE 停机测试改在离线预先准备、配置中停用内置 provider（`ruleSetProviders` 的 `geosite`／`geoip` 为 null）的网络上运行，不新增 Server 设置。planck 上 `strace` 跟踪全量 Server 测试（245 通过）：除回环外零连接。见[规则集设计](/v2/rule-sets-design)。

## v0.1.0-alpha.8

范围：wire 6、IPC 11、`compilerVersion` `meshway-netmap-2`、Server schema 不变；HTTP 契约删除 4 个从未产生的枚举值，netmap 投影新增可省略的 `eligibleExits`（须先升级 Node 再升级 Server）。F5a 25 条审阅修复（出口自指 `eligibleExits`、MTU 探测签名绑长度／只升不降修正／路由 MTU 9000 不致命、listener 接管改绑、出口防火墙判定含 prerouting＋记忆化预算、路径暂停退避、调度 EDF、游标 digest、完整 State 报告 adjacencies 裁剪、缓冲截断口径、DNS 预约 cookie 归档、ctnetlink `DUMP_INTR`、MSS 重试）；F5b 卫生（删 4 个枚举值、`meshway_rules::ranges`、观察历史游标共用 cursor 信封）；F5c 大文件拆分；以及下列 F5c、F5b、F5a 工作包。

大文件拆分与偶发卡死测试（F5c，`f5c/splits`）：只移动模块，wire 6、IPC 11、`compilerVersion` 与 Server schema 均不变，无行为变化。

- `manager::datagram` 重放耗尽测试偶发卡死（alpha.7 起即有）为测试竞态：重启后新绑定的 socket 尚未观测到写就绪，首个 challenge 交给 Session worker 发送，测试线程停顿超过 `OUTPUT_WAIT`（100 ms）时这一尽力而为报文被丢弃，而测试只收一次、不像拨号端那样重传 INIT。产品行为不变（丢失由 INIT／ACK 重传恢复，新增注入停顿的重传测试）；测试改为先确认写就绪，每次等待控制报文有统一期限。

第二轮审阅卫生清理（F5b）：wire 6、IPC 11、`compilerVersion` 与 Server schema 均不变。HTTP 契约：删除从未产生的枚举值 `UnavailableReason` 的 `missingCapability`／`missingDns`／`invalidProfile` 与 `NotApplicableReason` 的 `meshAddress`。

- 观察历史分页改用 Server 唯一的 cursor 信封（分页起点不变）：cursor 文本上限 2,048 B，拒绝未来签发，与其他列表一致。
- 区间归并与 CIDR 覆盖只有 `meshway_rules::ranges` 一份实现：规则集构建、Policy 分割的路由计数与 Underlay 捕获表路由共用同一惰性迭代器。
- 删除无消费者的 API（`validate_configuration`、`domain_name_matches`、`admit_path_carrier`、`dropped_first_packets`、`diagnostic_circuit`）与 `meshway-rules` 的 dev 依赖 `tempfile`；`validate.yml` 的 PR 路径覆盖被验证二进制的全部 crate（含 `meshway-netns`），由 `scripts/test_validate_workflow.py` 检查。
- 文档：[Exit 设计](/v2/node/exit-design) 与[运行时](/v2/node/runtime#ipv4-出口角色与-nat-投影)按签名 netmap、Engine `Desired`、Policy `Decision` 与 quarantine 生命周期重写。

第二轮审阅的行为／安全修复（F5a，`f5a/review-fixes`）：wire 6、IPC 11、`compilerVersion` `meshway-netmap-2` 与 Server schema 均不变。HTTP 契约：netmap 投影新增可省略的 `ProjectedConfiguration.eligibleExits`（为空时省略，已存 netmap 字节与 digest 不变）；旧 Node 严格解码会拒绝带非空该字段的 `sync` 回答，须先升级 Node 再升级 Server。Server 的 netmap 输入指纹改变：升级后某网络第一次重编译（任一配置、密钥、能力或选择变化）即按新投影刷新该网络全部 netmap，内容未变者保留 serial。UDP 邻接新增路径大小探测消息 `Probe`（类型 7），健康 Ping 不再接受填充：新旧节点混跑时该通道路径 MTU 为未知（不影响健康检查），两端都升级后恢复。系统 DNS 预约（`/run/meshway/dns-*/owner.json`）新增可省略的 `namespace.cookie`：降级到本版本之前的 daemon 会以 `storage` 拒绝读取已写入 cookie 的预约。

- A1 修复：出口集合含源自身（`exit [A, S]`）而 A 被 topology 隔离、未登记或已吊销时，S 的投影里没有 A，节点把它当作“未启用”而判 direct，捕获边界放开。现在 Server 在投影里给出全量配置判定的出口资格（`eligibleExits`：保留规则的出口集合中排在另一节点之前、启用且 `exitEnabled` 的节点），节点只按它判定自身之前的出口，不再从“不在投影里”推断；A 可用时 S 经 A 或失败闭合。投影等价测试以全量资格为 oracle，生成器加入被孤岛隔开的自指规则。见 [Routing · Action](/v2/configuration-reference/routing#action)、[投影规则](/v2/node/netmap-design#内容字段)。
- A2 修复：自身之前不可用的出口仍留在执行结果里，名称规则因此被算成多出口而误报 `dns_exit_plan_required`。现在自身之前的出口按全量资格过滤并保序（与 A1 同一实现 `matching::executed`）。
- A3 修复：周期调度器总是先处理到期的目录且按 key 从头扫，两个 `updateInterval` 极短的 provider 可饿死其余目录与规则集刷新。现在网内在到期的目录与集合中取到期时刻最早的一项（同刻目录在前）。见[规则集 · 周期更新](/v2/configuration-reference/rule-sets#周期更新与生命周期)。
- A4 修复：目录／成员翻页游标内含目录 URL（至多 2,048 B），封装后超过游标文本上限，长 URL 的第二页必为 `invalid_cursor`。游标改存查询与内容的 SHA-256 身份。
- A5 修复：完整 State 报告的 `adjacencies` 未按读取者可见范围裁剪，权限收缩后读旧报告会泄露不可见 peer；现在与 `links` 同样裁剪。见 [网络 State](/v2/state-reference/network)。
- A6：已接受目录的解析缓存以（sequence, 签名 digest）为内容身份，不再用（sequence, 验签毫秒）；同一毫秒、同一 sequence 接受的不同文档不再读到旧解析。
- B1 修复：路由 MTU 9000（巨型帧）时路径 MTU 探测超过报文编解码上限，构造失败经 `?` 结束整个 UDP endpoint owner、Manager 退出。探测上限改为 `min(路由 MTU, 2,048 + IP/UDP 头)`，构造失败只结束这一次测量。见[外层路径 MTU](/v2/node/connectivity#path-mtu)。
- B2 修复：listener 重试绑定后、被接管前到达新 instance 时，接管的 socket 仍带旧 instance 引用；离线绑定的普通 Native UDP 在首个 instance 到达后被原样接管，没有转为协调 socket。现在接管时按当前 instance 重新确认模式与引用，需要改绑时先同步释放旧 socket（不再撞上自己的端口，U1 中“自占 `EADDRINUSE`”的线索）。见 [listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)。
- B3 修复：数据面失败撤回（`reconfig(None)`）后 `meshway ping` 的本机直出 raw ICMP、出口与解析任务仍继续；现在撤回时取消并等待这些任务，请求以 `stopped` 结束。见 [诊断 · 探测](/v2/node/diagnostics)。
- B4 修复：路径 MTU 搜索只升不降，路径静默变小后永远发现不了。每 10 分钟的重新搜索先重验已确认大小（期间旧值作为暂存事实），连续两次丢失即在 1,280 与它之间重新二分，连 1,280 都不通过时撤回为未知。
- B5 修复：通道内层上限低于 1,280 时 IPv6 packet too big 被强制写成 1,280，源继续发 1,280 仍被拒，永不收敛。这样的通道现在不承载 IPv6：IPv6 包一律丢弃（`Error::Ipv6Unavailable`），本机一端回 ICMPv6 destination unreachable（no route）；不做外层分片重组。见[通道 MTU](/v2/node/dataplane#channel-mtu)。
- B6 修复：路径 MTU 探测的签名不覆盖长度（填充未签名），被截短的探测仍得到有效 Pong，测量偏大。探测改为独立的 `Probe` 消息，签名含整个报文长度，接收方按实际收到的长度验签后才应答，并与健康 Ping 分开限速（此前同一轮发出的探测与健康 Ping 共用 250 ms 应答限速，探测常被吞掉）。
- B7 修复：路径暂停在到期时连同次数一起被删，`10 s × 2^(n−1)` 退避累积不起来。到期只让路径重新可试，次数保留；应答、路径内容变化或有界回收（256 条，让位给暂停结束最久的）才清零。见[路径选择](/v2/node/connectivity)。
- B8 修复：出口防火墙判定漏了 prerouting（含 raw），丢弃出口 TUN 入包或外部回包的 prerouting 规则不被发现。现在判定从出口 TUN 与出口接口进入的包；为不误伤 Docker 为发布容器加的 raw 规则（`-d <容器> ! -i <网桥> -j DROP`），目的地址按方向判定（转发为 Internet 分类地址，回包为出口接口地址或 Mesh 池）。见[出口前置条件](/v2/node/runtime)。
- B9 修复：已安装角色的周期探测把出口 TUN 当作“尚不存在的新 TUN”，针对实际 `tun1` 的阻断被判为不命中。现在已安装的出口 TUN 按名字精确匹配。
- B10：防火墙链遍历按（链, 包）记忆结果，共享子链不再成倍展开；一次判定至多评估 2^20 条规则，超过为 `firewallConflict`。
- B11：协调暂停表在内容切换时剔除离开投影的 peer，且至多 1,024 个 peer。
- B12 修复：conntrack dump 未检查 `NLM_F_DUMP_INTR`，被中断的 dump 被当作“无匹配”；现在为 `conntrackUnavailable`（已找到的条目仍算数）。
- B13 修复：MSS clamp 入队即被当作已安装，nft 事务失败后同一上限永不重试；现在按执行回执，失败的同一内容按 1 s 起翻倍、上限 60 s 重试。
- C1 修复：Linux 读回的缓冲是授予值的两倍，日志、`meshway status` 与默认值提升按“读回 < 请求”判断，被截到请求与两倍请求之间时漏报 `limited`（例如 4 MiB 请求被截到 3 MiB，读回 6 MiB）。判定集中到 `raw::satisfies`（Linux 上读回需不小于请求的两倍）。见 [UDP 批量接口](/v2/node/transport#udp-批量接口)。
- C6：系统 DNS 的 namespace 预约带 namespace cookie，并与 Underlay 记录共用唯一比较规则 `meshway_netns::same`：inode 被新 namespace 复用（容器重建）时，旧 namespace 的预约与后端记录按 Underlay journal 的规则归档（保留最新 3 份，共用 `meshway_private_storage::STALE_ARCHIVES`）并由当前 namespace 接管，不再被当作同一 namespace；同 cookie 的另一所有者或持锁的活进程仍是冲突；本版本之前的无 cookie 预约按 inode 接管并改写为带 cookie。见[生命周期](/v2/node/runtime#生命周期)。
- C11：`meshway ping` 的 echo 构造改用 packet 模块唯一的 Internet checksum，删除第二份实现。

验证：每条有对应反例的行为测试（A1、A4、A5、A6、B2 的测试已在修复前代码上实测失败）；Linux 专属单元测试（防火墙、ctnetlink、DNS namespace 与归档、netns 规则）在 Planck 323 项全部通过；合入 main（F5b）后 Planck 18 行矩阵 `20261014T042959Z-0d2b7b3f`（`3988157d`）全部通过（合入前 `20261014T003034Z-604c44b1`／`9500dc12` 亦全部通过）；证据 `.meshway/validation/f5a/`。

## v0.1.0-alpha.7

范围：wire 6、`compilerVersion` `meshway-netmap-2`、Server schema 不变；IPC 11（`Status.sockets`，CLI 与 daemon 一起升级）；Native UDP socket 默认缓冲接收 4 MiB／发送 2 MiB，经 `SO_RCVBUFFORCE`／`SO_SNDBUFFORCE` 不受 `rmem_max` 截断；出口 conntrack 经内核 ctnetlink（宿主不再需要 `conntrack` 工具）；Underlay namespace 身份含 netns cookie；listener 绑定失败退避重试；HTTP 契约 `IssueCode` 新增 `ephemeral_listener_port`（警告）；以及下列 M3、U1、M2 工作包。

出口 conntrack 经 ctnetlink（M3）：wire 6、IPC 10、HTTP 契约、`compilerVersion` 与 Server schema 均不变。

- 出口的 conntrack 精确读取、删除与 mark 过滤读取改由 Underlay 的 conntrack 模块直接经内核 ctnetlink 完成，删除按 original tuple、默认 zone 与已核对的 ct id（同 tuple 的替换条目不会被删）并回读不存在。出口宿主不再需要安装 `conntrack`（conntrack-tools）；`conntrackUnavailable` 只表示内核 ctnetlink 不可用，内核拒绝权限报 `netAdminMissing`。见[出口宿主](/v2/install#出口宿主)、[NAT 技术选择与宿主条件](/v2/node/exit-design#nat-技术选择与宿主条件)。
- Node lab／fixture 镜像仍含 `conntrack` 工具，只用于 `exit/off` 的独立证据与人工排查。

启动期瞬时失败自愈（U1）：wire 6、IPC 10、`compilerVersion` 与 Server schema 均不变。HTTP 契约：`IssueCode` 新增 `ephemeral_listener_port`（警告）。Underlay 私有记录新增可省略字段（`namespace_cookie`／`net_cookie`）：降级到本版本之前的 daemon 会以 `storage` 拒绝读取已写入 cookie 的记录。

- 修复：容器重建后新 network namespace 复用旧 nsfs inode 时，`exit-child.json` 被当作当前 namespace 的记录，PID namespace 不同又无法证明旧工具已退出，Underlay 永久停在冲突退避（`Underlay startup failed: kernel ownership or routing protection conflict`，N5 nyc、M1 lax，只能重启容器）。capture journal、出口／egress 账本与 exit-child 记录现在共用 Underlay 唯一的 namespace 身份，含 namespace cookie（`SO_NETNS_COOKIE`，一次 boot 内不复用）；无 cookie 的旧记录与内核仍按 inode。cookie 经新 crate `meshway-netns` 读取，是 workspace 唯一经审计的 `unsafe`。见[生命周期](/v2/node/runtime#生命周期)。
- 修复：listener 绑定失败（`EADDRINUSE` 等）不再使节点在重启前一直没有该 listener（ops5 nyc 只能经中继）：Manager 中该 key 的 owner 按 1 s 起翻倍、上限 60 s 重试（`EACCES` 每 60 s），其他资源照常安装；期间 State 中该 listener 为 `failed`＋`resource_unavailable`，Status `lastError: transport`，`lastErrorDetail` 含 bind 错误、占用进程与临时端口范围提示；绑定后同一内容再 apply 并清除错误。完整 apply 现在清除先前 Degraded apply 的 `lastError`。见 [listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)。
- listener 端口在 Linux 默认临时端口范围（32768–60999）内时配置校验给出警告 `ephemeral_listener_port`，节点按本机 `ip_local_port_range` 记 WARN。
- `udp/off` 行新增阶段：B 的 listener 端口在其 daemon 首次绑定时被占用，验证 Status 原因、释放后自行绑定与清除错误。

验证：Planck 18 行矩阵 `20261013T211635Z-273c8e3a`（`7a6dab15`）全部通过，lab 三容器八轮重建见 [U1 启动期自愈证据](/v2/node/validation#u1-startup-self-heal)；证据 `.meshway/node-lab/u1/`、`.meshway/validation/u1/`。

UDP 接收缓冲（M2）：IPC 11（`Status.sockets`）；wire 6、HTTP 契约、`compilerVersion` 与 Server schema 均不变。CLI 与 daemon 须一起升级（IPC 10 的 CLI 对新 daemon 得到 `version` 错误）。

- Native UDP socket 默认缓冲：接收 4 MiB、发送 2 MiB（原为 7／7 MiB 的普通请求）；请求先用 `SO_RCVBUFFORCE`／`SO_SNDBUFFORCE`，daemon 有 `CAP_NET_ADMIN` 时不再被 `net.core.rmem_max` 截到 416 KB（it7、planck 此前均为 416 KB，it7 的 socket 有接收丢包）；没有该权限时退回普通请求。显式 `socket.receiveBuffer`／`sendBuffer` 按配置请求。见 [UDP 批量接口](/v2/node/transport#udp-批量接口)。
- 实际值可观测：每个 Mesh UDP socket 一条 `UDP socket buffers` 日志（被截断时为 warn），IPC Status 新增 `sockets`（资源、地址、读回值与请求值；U1 重试绑定中的 listener 为 `unavailable` 行并带失败原因，绑定成功后同样读回），`meshway status` 新增 SOCKET 表并标注 `limited`。见[运行状态](/v2/node/runtime)。
- 接收路径核对：已是 `recvmmsg`（32 × 64 KiB）＋`UDP_GRO`，未改。按通道的发送节奏实测无收益，未加入。
- 文档：[高吞吐宿主](/v2/install#high-throughput) 的 sysctl 建议（daemon 不修改）。

验证：Planck `perf:dataplane`（每容器 2 CPU）L3 direct／L4 relay TCP 2,540／2,229 → 2,674／2,468 Mbit/s，重传 618／512 → 0；60 ms RTT（netem）内层 TCP 266–315 → 410–412 Mbit/s；4 MiB 与 7 MiB 各格无差别。见[接收缓冲](/v2/node/performance#接收缓冲)；证据 `.meshway/validation/m2/`、`.meshway/node-lab/m2/`。合入 main（U1）后 Planck 18 行矩阵 `20261013T220137Z-ac7e2c25`（`52d01af7`）全部通过，`status --json` 显示 socket 读回 8 MiB／4 MiB，`udp/off` 的被占端口阶段 B 的 `sockets` 有 `listener tcp` 的 `unavailable` 行。真机（alpha.7，OPS6）：it7 的 10 个 Mesh UDP socket 读回 `rb8388608 tb4194304`；planck→it7 20 s TCP 由 116–138 Mbit/s（alpha.6，it7 socket 丢 29–164）升至 171–189 Mbit/s（it7 socket 丢 0，`UdpRcvbufErrors` 不再增长），剩余重传在路径上；it7→VM `-R` 450 Mbit/s 前后一致。证据 `.meshway/node-lab/ops6/`。

## v0.1.0-alpha.6

范围：wire 6、`compilerVersion` `meshway-netmap-2`、Server schema 不变；IPC 10（`peers[].state` 新增 `relayIdle`，`Command::Ping.target` 为 `PingTarget`，CLI 与 daemon 一起升级）；HTTP 契约自 alpha.5 起的 `PeerFacts.connected`、`IssueCode` 新增 `dns_exit_plan_required`（错误）与 `resolver_outside_access`（警告）；验收矩阵行 `nat/relay`、`nat/reconnect`（18 行）；通道 MTU（MSS clamp、本机 fragmentation needed／packet too big）；`meshway ping` 公网 IP／域名目标；以及下列 M1、D2、N5 工作包。

路径 MTU 收口与出口 DNS 语义（M1）：wire 6、IPC（随 D2 为 10）、`compilerVersion` 与 Server schema 均不变。HTTP 契约：`IssueCode` 新增 `dns_exit_plan_required`（错误）与 `resolver_outside_access`（警告）。

- 出口 DNS 条件收窄：普通查询只在方案有经出口的名称规则（`domain`、`domainSuffix`、含名称的规则集）或 fallback 经出口时经出口解析，用那一个出口；只有 IP 规则（含 `geoip:*`）、fallback direct／unmanaged 的方案用本机 resolver，经几个出口都可以。名称规则／出口 fallback 经不同出口仍为 `exitPlanRequired`，Server 保存时即以 `dns_exit_plan_required` 拒绝（指向规则）。唯一实现 `meshway_policy::compile::dns_resolution`。见 [DNS · 使用出口时](/v2/configuration-reference/dns#使用出口时)。
- `useWithExit` 解析器在出口区间内时是源分割的 direct 例外：`global:<exit>` 下 DNS 不再以 “resolver plan is incomplete” 卡在 synchronizing；节点 Access 不覆盖其地址时保存给出 `resolver_outside_access` 警告。
- 通道 MTU：本节点超过首跳通道内层上限的包在消耗 nonce 前丢弃，并向本机 TUN 合成 fragmentation needed（IPv4）／packet too big（IPv6），源地址为原目的；TCP SYN 的 MSS 由 Underlay 自有 nft 表 `meshway_mss_<installation>` 限到通道上限（Mesh 目的／来源、出口 TUN 的源节点）。数据面另接受 ICMPv6 destination unreachable／packet too big／time exceeded 作为相关报文。见[通道 MTU](/v2/node/dataplane#channel-mtu)。
- 可信域名关联：源端持有名称规则回答等到该查询截止前 250 ms（至多 5 秒），只有该查询等待。
- `exit/off`：第 11 阶段切到 `global:E`；第 12 阶段新增窄 A–R 通道（MSS 限制、本机 fragmentation needed）。

验证：Planck 18 行矩阵 `20261013T163418Z-ca02b918`（`287e2385`）与合入 main 后的 `20261013T170843Z-f7c75f4f`（`1b8e8df2`）全部通过；实网 it7 → VM 不限 MSS 由 0 变为 408／394 Mbit/s，纯 IP 双出口六台正常，见 [M1 实网证据](/v2/node/validation#m1-real-network)；证据 `.meshway/node-lab/m1/`、`.meshway/validation/m1/`。

`meshway ping` 公网目标（D2）：IPC 10；wire 6、HTTP 契约、`compilerVersion` 与 Server schema 均不变。CLI 与 daemon 须一起升级（IPC 9 的 CLI 对新 daemon 得到 `version` 错误）。

- `meshway ping` 接受公网 IP 与域名：先打印本节点的本地判定（方案、规则 pointer／remark／规则集条目、exit／direct／unmanaged／denied 与完整路径；域名显示经本机 DNS 路径的解析与 X6 关联剩余秒数），再探测：exit 经出口路径 ICMP echo（ICMP 错误显示为 `unreachable from <hop>`），direct／unmanaged 本机直出并标注 not via Mesh，denied 不探测。只报告总 RTT。见 [Mesh 外目标](/v2/node/diagnostics#external)。
- IPC 10：`Command::Ping.target` 为 `PingTarget`（`name`／`address`，取代 `meshIp`）；`PingEvent` 新增 `decision`（在任何测量事件之前，Mesh 目标也有）与 `echo`；`--json` 依次输出。
- `meshway status` 的 `relay … via <R>` 与 `ping` 的判定路径取新流实际会加入的 circuit（与 `choose_path`／`joinable` 同一排序），不再是多条等长 circuit 中 ID 序的第一条（N5 遗留）。
- `exit/off` 新增第 13 阶段：公网 IP 经 E、split resolver direct、名称规则域名经 E resolver 与关联、`domain:` 撤权后 denied。

验证：Planck 18 行矩阵 `20261013T160631Z-761b2963`（`79ef1788`）全部通过，`exit/off` 行 `20261013T161638Z-b6eb53a0`；证据 `.meshway/node-lab/d2/`。真机（lab／VM／it7／planck）部署后验收待补。

重启后自愈（N5）：wire 6、IPC 9（`peers[].state` 新增 `relayIdle`）、HTTP 契约、`compilerVersion` 与 Server schema 均不变。

- 修复：instance 换代（Server 重启、instance 租约断续后重新取得）时穿透 socket 的 Underlay 授权不再停留在旧 instance。此前之后的每次协调尝试都在激活时被拒（`target_not_authorized`），节点从不发 punch，需要打洞的边直到新内容或 daemon 重启都不恢复；这是 alpha.5 部署后实网分区与 12:28–12:57 Server 不可达后再次分区的根因。见[连接获取 · Node 协调契约](/v2/node/connectivity#node-协调契约)。
- 显式拨号与协调穿透的暂停上限 300 → 60 秒；协调暂停只约束本节点主动发起，对端仍有效的穿透请求即时查询并接受。见[候选节奏](/v2/node/connectivity#candidate-pacing)、[重启后的恢复](/v2/node/connectivity#restart-recovery)。
- 数据面路径暂停上限 300 → 60 秒，且只在该路径自身内容（节点密钥、各跳承载）变化时清除。见[中继兜底](/v2/node/connectivity#relay-fallback)。
- `meshway status`：有需求但无通道、可经有通道的中继到达的 peer 显示 `relay (idle) via <R>`；`unreachable` 只表示没有任何路径。见[运行状态](/v2/node/runtime)。
- 文档：Server 重启后的事实重建与 ≈35 秒不变量，见[事实重建](/v2/node/netmap-design#facts-rebuild)。
- 新矩阵行 `nat/reconnect`（18 行）：端点相关映射的 A ↔ 公网 R，NAT 映射丢失、真实 Server 原地重启、单独重启 R、单独重启 A 后各 ≤60 秒重新直连。

验证：Planck 18 行矩阵 `20261013T135230Z-e8df696e`（`db193112`）全部通过；修复前复现 `20261013T131635Z-426130a9`；实网见[N5 实网证据](/v2/node/validation#n5-real-network)；证据 `.meshway/node-lab/n5/`、`.meshway/validation/n5/`。

## v0.1.0-alpha.5

范围：wire 6、`compilerVersion` `meshway-netmap-2` 不变；IPC 9；Server schema 7；HTTP 契约新增可省略的 `SyncRequest.lease` 与 `PeerFacts.connected`（先升 Node 再升 Server）；新增验收行 `nat/relay`；以及下列 C3、S3、F4a、R2d、N4、X7 等工作包。

wire 6、`compilerVersion` 不变，IPC 9（C3）；Server schema 版本 7（F4a 新增 `rule_set_record_heads`，C3 为 6，R2d 新增 `rule_set_catalog_checks` 并允许回收 artifact 为 7；开发环境重新初始化数据目录）。HTTP 契约：`SyncRequest` 新增可省略的 `lease`；`listRuleSetCatalog` 描述更新（无字段变化）；S3 的 State／库存／诊断变化见下。

中继候选排序与退避重置范围（N4）：wire 6、IPC 9、`compilerVersion` 与 Server schema 均不变。HTTP 契约：`PeerFacts` 新增可省略的 `connected`。旧 Node 严格解码会拒绝含此字段的 `sync` 回答，须先升级 Node 再升级 Server（新 Node 接受省略）。

- 中继候选不再按节点 ID 截断：Policy 列出同一长度的全部路径后才搜索更长的，在已满 8 条（`MAX_PATHS`，原为 4）的长度结束，每目标至多 2,048 条，仍在 16,384 工作项预算内。源节点按事实排序：直连在前，中继按（本机与它有通道，它在 `connected` 中与下一节点有通道）排列，取前 8 条，截断的中继数记为 `operation=path_candidates`。新流只加入排序位置不低于所选路径的已建立 circuit。见[中继兜底](/v2/node/connectivity#relay-fallback)。lax、sha、nyc、VM → planck 由超时变为 10/10 `via relay us-lax-it7`。
- `PeerFacts.connected`：peer 报告中已认证、未到期的通道对端，限查看者投影；只用于排序。
- 退避重置收窄：只有该 peer 的边（`Plan::edge`：密钥、双向承载与拨号候选、listener、常驻）、端点事实、本机映射或该 peer 的通道变化才重新开始候选与协调暂停；其他节点、Access、出口等修订不再重置。见[候选节奏](/v2/node/connectivity#candidate-pacing)。
- `nat/relay` 扩为 5 节点（R1 失去 B、R2 与两端有通道、R3 ID 更小且与 A 无通道），要求经 R2、30 秒内。
- node lab：daemon 日志过滤取部署时的 `RUST_LOG`。

验证：Planck 17 行矩阵 `20261013T091200Z-d6fd0e7c`（`1a5a1274`）全部通过；`mise run --jobs 1 check` 通过；见[中继兜底行](/v2/node/validation#relay-row)与[N4 实网证据](/v2/node/validation#n4-real-network)；证据 `.meshway/node-lab/n4/`、`.meshway/validation/n4/`。

出口宿主（X7）：wire 6、IPC 9、HTTP 契约、`compilerVersion` 与 Server schema 均不变。

- 宿主防火墙只在能匹配出口流量时为 `firewallConflict`：forward／output／postrouting 上的阻断判决或 `policy drop`，且接口约束不排除出口 TUN 与通往 Internet 的出口接口；经 `jump`／`goto` 遍历，legacy iptables 按 `iptables-legacy-save` 内容同样判定（不再因表存在即冲突）。Docker 的网桥隔离规则与出口共存，FORWARD `policy drop`、ufw、firewalld 区域与不限接口的 drop 仍冲突。见[出口宿主前置探测](/v2/node/runtime#出口宿主前置探测)、[Linux 安装 · 出口宿主](/v2/install#出口宿主)。
- 出口角色准备失败不再阻断 netmap：前置条件不满足或角色安装失败时内容照常 applied，只报 `exitCapability: unavailable`；周期能力探测观测到 ready 后以同一内容重新 apply（`Engine::reapply`）安装角色，无需重启或新 netmap。
- 修复：`rp_filter` loose 的宿主（如 Docker-on-Linux 容器）上出口不转发——无地址的出口 TUN 入包被反向路径检查丢弃。回程成对规则不再要求出口 mark，兼作反向查找；每个新投影在 ACK 前取内核对其 forward 包的实际入路由判定，失败即拒绝该流并记录原因；探测另读取新 TUN 继承的 `forwarding`。
- `exit/off` 第 2 阶段改为断言 `ip_forward=0` 时 E 已 applied，打开 forwarding 后不重启即安装角色。

验证：Planck 17 行矩阵 `20261013T092932Z-0d781dd4`（`5183bf6e`）与 `20261013T090318Z-539d8762`（`df591c3f`）全部通过；`mise run --jobs 1 check` 通过；真实宿主见 [X7 真实出口宿主](/v2/node/validation#x7-exit-hosts)；证据 `.meshway/node-lab/x7/`、`.meshway/validation/x7/`。

连通性三条（N3）：wire 6、IPC 9、HTTP 契约、`compilerVersion` 与 Server schema 均不变。

- 候选不再按地址类别过滤（撤回 N2a 的“私网／CGNAT／ULA `localEndpoints` 只在本机有包含它的接口前缀时拨号”）：所有候选都尝试，每个失败候选按 `1 s × 2^(n−1)` 暂停、上限 300 秒；与 peer 有通道即停止探测，候选集合、通道或 netmap 变化时重新开始。协调穿透对无果结束的 peer 同样暂停 prepare 与为它发的查询。见[候选节奏](/v2/node/connectivity#candidate-pacing)。lax（Docker）↔ meshway-vm（OrbStack machine）恢复为 `direct native_udp`。
- 修复：`keepalive` 只对字面 `node:<id>` 的规则生效，`* ↔ *`、Tag、Group 选择器的 keepalive 边仍按需（lab 两两 `idle`）。现由决定该边的首条 allow 规则（与 `transports` 同一条）给出，Policy `LocalCompiler::keepalive` 唯一实现。
- 中继兜底：业务首包与 `meshway ping`／诊断任务共用 `SecureNode::choose_path`：取首跳有通道、且最近源端握手未无应答结束的第一条路径；之前的直连路径登记邻接需求。无应答结束的路径暂停 `10 s × 2^(n−1)`（≤300 秒）。修复 `meshway ping` 只绑定首条路径、直连不可用时超时而业务已经经中继的问题。见[中继兜底](/v2/node/connectivity#relay-fallback)。
- 来源学习不再要求与登记候选同 IP：经 offer digest 签名验证的开场消息来自任意合法单播来源都可成为本尝试的目的（仍 ≤2、随尝试结束）。本机映射变化（如 WARP 换出口 IP）使全部候选节奏重新开始。
- 新矩阵行 `nat/relay`（17 行）。

验证：Planck 17 行矩阵 `20261013T053433Z-a688a9f6`（`45ec6e99`）与 `20261013T060913Z-86100acd`（`3e1142ee`）全部通过；`mise run --jobs 1 check` 通过；实网见[N3 实网证据](/v2/node/validation#n3-real-network)；证据 `.meshway/node-lab/n3/`、`.meshway/validation/n3/`。

真实网络穿透与外层 PMTU（N2a）：IPC 9 新增（`peers[].mtu`、`lastErrorDetail`、netcheck `sizeProbe`／`pathMtu`），wire 6 不变（接收方另接受以零填充的 UDP 健康 Ping）。

- 来源学习：对称（端点相关映射）NAT 下，本 offer 对端的签名开场消息从同 IP 的另一端口到达时，该来源成为本尝试的第二目的（每尝试 ≤2、随尝试结束），通道端点取握手实际完成的地址；身份仍只由签名证明。见[来源学习](/v2/node/connectivity#source-learning)。
- 外层 UDP 一律 DF（连 1,500 B 路径也装不下的单个控制报文，如出口 DNS 大回答，仍可分片）；`UDP_SEGMENT` 因段长超过路径 MTU 被拒时只拆这一批，GSO 不再被永久关闭。每条 Native UDP 通道以填充的签名 Ping 搜索路径 MTU，`meshway status` 的 MTU 列显示 `path/inner`。见[外层路径 MTU](/v2/node/connectivity#path-mtu)。
- netcheck：映射类型只比较不同 IP 的观测点；每个观测点一个 DF 大小探测与路由 MTU。开发 Server 增加 Cloudflare STUN 观测点。
- 私网／CGNAT／ULA 的 `localEndpoints` 只在本机有包含它的接口前缀时拨号（N3 撤回，见上）。
- listener bind 失败的日志与 `lastErrorDetail` 带地址与系统错误（如 `Address in use`）。
- 修复：未 up 的接口创建／删除不再算外部网络变化（us-lax-it7 上反复重建的空闲 `ppp0` 每隔数秒关闭全部控制连接，sync 以 `Network` 失败）；没有 Native UDP listener 的节点在首个 instance 前启动时补绑拨号穿透 socket；受保护控制中继按 Happy Eyeballs 交替地址族连接；中继的固定 300 秒连接寿命改为双向 120 秒无字节才关闭（原寿命截断恰在挂起的 sync 长轮询，us-lax-it7 每次 `Network` 失败都与之精确对应）。
- 实网：us-lax-it7（公网 dial-only）↔ meshway-vm（WARP）Native UDP 直连，见[实网 NAT 证据](/v2/node/validation#real-nat)。

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

出口方案切换（C3）：IPC 升为 9，Server schema 版本 6（`activity_events` 新增 Node actor 与路由方案目标；开发环境重新初始化数据目录）；wire 6、`compilerVersion` 不变。

- `meshway exit [profile|off]`：无参数显示本机 netmap 中的方案选择（生效／请求的方案、默认与锁定、`auto` 解析、每个允许方案的摘要）；带方案时请 Server 切换并等待本机应用带新版本的 netmap，`off` 即 `mesh`。人读输出、`--json` 与退出码见 [出口方案](/v2/node/runtime#exit-profile)。
- IPC 9：命令 `SelectProfile`、结果 `Selection`、`NetmapStatus.routing`，错误码 `profileNotAllowed`／`profileLocked`／`selectionConflict`；`meshway status` 新增 Profile 行。
- Control 调用 `selectNodeRoutingProfile`，CAS 基准为持有 netmap 中自身的选择版本；方案被拒或版本过期不结束 session／instance。本机只缓存 netmap，不保存选择。
- HTTP：`selectNodeRoutingProfile` 的回答改为 `{selection: RoutingSelectionState}`，版本不一致改为 `409 version_conflict`、方案不可选为 `422 routing_profile_unavailable`（带允许的方案）；新增 `getNodeRoutingSelection`、`setNodeRoutingSelection`（`/api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection`，成员限自己的 User 设备，Owner／Admin 任意节点，`requestId` 幂等）；`NodeInventoryView.routing` 摘要；活动 `routing.select`（actor `node`／`user`，target `routingSelection`）。三个入口共用同一提交，选择版本跨身份单调递增，过期身份的选择不作为 CAS 基准。
- `RoutingSelectionState` 由 `meshway-policy` 唯一计算，Server 与节点对同一配置给出同一值；`auto` 没有可信地区输入，总是采用 fallback（`basis: noTrustedRegion`）。
- Console 仅补齐新增枚举的映射。
- `exit/off` 第 11 阶段：真实 CLI 切换后旧选择的 TCP／UDP 流结束、新流经出口，`exit off` 后无外部捕获。

验证：Planck 16 行矩阵 `20261012T224334Z-469128d0`（`7facc522`）与合入 main 后的 `20261013T000304Z-56e3679e`（`f2441b12`）全部通过；证据 `.meshway/validation/c3/`。已知：切到 `global:<exit>` 与 `useWithExit` split resolver 组合时节点 DNS 安装拒绝新 netmap，见[验收范围 · 剩余边界](/v2/node/validation#remaining-boundaries)。

此前：wire 6、IPC 8、`compilerVersion` 不变；Server schema 版本 5（新增 `rule_set_record_heads`；开发环境重新初始化数据目录）。HTTP 契约：`SyncRequest` 新增可省略的 `lease`；S3 的 State／库存／诊断变化见下。

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

⚠ 断网窗口（`compilerVersion` 变化，见[升级 · compilerVersion 变化的版本](/v2/install#compiler-version-upgrade)）。wire 6 不变；IPC 升为 8；`compilerVersion` 升为 `meshway-netmap-2`；Server schema 版本 4（开发环境重新初始化数据目录）。HTTP 契约：新增 `GET /agent-api/v1/artifacts/{digest}`、`POST /agent-api/v1/dns-associations`（`reportDnsAssociation`）、规则集 `report`／`entries`／`match` 与 `rule-set-catalog` 端点；配置新增 `ruleSetProviders` 与 `<provider>:<name>` 引用；netmap `ruleSets` 改为 digest 引用，`EgressPlan` 改为 `{generation, rules, fallback, unavailable}`（删除 `capture`／`routes`／`EgressRoute`）；`SyncRequest`／`SyncResponse` 增加 `associations`；`RuleSetView`、`ArtifactSummary`、`PreparedRuleSet`、`ResourceProblem` 与警告 `egress_partition_limit` 按下列 R2a、R2c、X6、R3 变化。

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
