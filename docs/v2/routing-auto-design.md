# 自动方案（`auto`）设计

> **第一版已实现**（[第一版切片](#first-slice)：管理员标签 `nodes.<id>.region`＋方案级 `auto`），现行语义的唯一定义是 [Routing · 自动方案](/v2/configuration-reference/routing#auto)、[Nodes · region](/v2/configuration-reference/nodes#region) 与 [Netmap 设计](/v2/node/netmap-design#内容字段)。**第二步（GeoIP 推断、驻留／防抖、`observed`／`held` basis、P2 投影）仍是设计，未实现。**下文“现状”一节描述的是第一版之前的状态；全部 15 条决定均已按推荐采纳（[决定清单](#决定清单)）。本页不加入文档导航。

## 现状

| 项 | 当前行为 | 定义位置 |
| --- | --- | --- |
| 配置 | `routing.egress.auto = { rules: [{countries, profile}], fallback }` 已可保存；目标与 fallback 必须是主体 `allowed` 内的非 `auto` 方案（`auto_target_unknown`／`auto_target_not_allowed`），`countries` 为大写两字母（`invalid_country_code`） | [Routing · Fields](/v2/configuration-reference/routing#fields) |
| 解析 | 编译不使用任何地区输入，`auto` 恒等于 `auto.fallback`；`RoutingSelectionState.auto = {profile, basis: noTrustedRegion}` | [Routing · 内置方案与当前选择](/v2/configuration-reference/routing#内置方案与当前选择) |
| 选择 | 持久选择只归 Server（`routing_selections`，CAS 版本）；节点只读 netmap 中的 `selectedProfiles`／`selectionVersions` | [Routing · 选择的入口](/v2/configuration-reference/routing#选择的入口) |
| selection 身份 | `egress.generation` ＝ digest(网络、节点、**有效方案名**、选择版本、`routing.egress`、所用集合 digest)；有效方案为 `auto` 时方案名就是字面量 `auto` | [Netmap 设计 · 内容字段](/v2/node/netmap-design#内容字段) |
| 出口集合 | 有序主备 `[A, B, S]`；源自身 S 是其位置上永远可用的 direct；S 之前的出口只按 `eligibleExits` 判定可用（`matching::executed`） | [Routing · Action](/v2/configuration-reference/routing#action) |
| DNS | `dns_resolution` 四形态 `Local`／`Exit{exit, via}`／`PlanRequired`（及 `useWithExit` 例外）；保存时对每个 `allowed` 方案判定，`exitPlanRequired` 以 `dns_exit_plan_required` 拒绝 | [DNS · 使用出口时](/v2/configuration-reference/dns#使用出口时) |
| 地址事实 | State 只给 `addressScope`（public／nonPublic／mesh）与 `location`（本机 STUN／公网 interface 观测）；S3 起 State 不含 GeoIP／ASN；GeoIP 数据集由运维导入 Server，**只**留给自动方案 | [地址类别与位置依据](/v2/state-reference/ip-info) |
| RTT | State `links` 不带 RTT；`PeerFacts.connected` 只是“有已认证通道”的布尔事实，仅给中继排序；邻接 probe 进入观测历史（抽样、有预算），不是编译输入 | [Connectivity](/v2/node/connectivity#relay-fallback)、[观测与历史](/v2/state-reference/observations) |
| 切换 | 任何方案切换都结束旧出口存量连接（selection 变化即 `SecureNode::apply` 重判失败） | [Routing · 内置方案与当前选择](/v2/configuration-reference/routing#内置方案与当前选择) |

不变量（本设计的所有方案都必须满足）：

- **选择只归 Server**。节点不判定自己的地区、不选方案，只在签名 netmap 的投影上用同一 `meshway-policy` 重算。
- **`auto` 不扩大授权**。它只能选出主体本来就能手动选的方案；地区输入被伪造的最坏结果是落在另一个已允许的方案上。
- **中继／出口可重算**。出口与中继在自己的投影上重算源端 selection，因此决定 `auto` 结果的全部输入必须在投影里。
- **失败闭合**。地区未知、数据集缺失、观测过期都不能让流量变成隐式 direct／unmanaged；只能落到管理员显式配置的 fallback 方案。

## 1. `auto` 是什么 {#what-is-auto}

### 选项

| | (a) 出口级 `auto` | (b) 方案级 `auto`（现有结构） | (c) 规则级 `exit: auto`（按目标地区） |
| --- | --- | --- | --- |
| 写法 | 方案规则 `action: {type: exit, nodes: ["auto:<pool>"]}`，Server 按**源**地区从候选出口池里选 | `assignments.allowed` 含 `auto`；`auto.rules` 把源地区映射到一个具体方案 | `action: {type: exit, nodes: "auto"}`，按**目标**地址的地区选最近出口 |
| 地区输入 | 源节点地区＋每个出口的地区 | 只需源节点地区 | 目标地区（节点已有 `geoip:*` 规则集）＋出口地区 |
| 与 `[A, B, S]` | 选择结果展开为一个有序集合后再走 `executed`；池含源自身时按 E1 处理（同地区即 direct） | 不变：选中的方案内部仍是普通有序集合 | 展开为“`geoip:<cc>` → 该地区出口”的多条规则，每条仍是普通集合 |
| 与 `fallback` | 池为空／地区未知时需要池级 fallback，与方案 fallback 是两层 | `auto.fallback` 即唯一兜底，是一个完整方案 | 未覆盖地区落到方案 `fallback` |
| 与 `dns_resolution` | 名称规则引用的池在编译期不确定，必须对池内每个可能结果判定，任意两个结果不同即 `PlanRequired` | 每个目标方案本身就在 `allowed` 内，保存时已逐个判定；`auto` 的 DNS 等于所选方案的 DNS | 名称规则无法在解析前知道目标地区，只能用于 IP 规则；名称规则写 `exit: auto` 必须拒绝 |
| 与 `eligibleExits` | 池内全部出口都要进入 `eligibleExits` | 不变（目标方案已被投影保留） | 展开后的出口按现有规则进入 |
| 本质 | 新的出口选择算法 | 已存在的方案选择器，缺地区输入 | 规则模板（宏），今天可以手写 |

### 推荐：(b)

(b) 是唯一与现有结构完全兼容的形态：配置字段、校验、`allowed` 约束、DNS 逐方案校验已经存在，缺的只是可信地区输入与 selection 身份。它把“地区 → 意图”交给管理员写成完整方案，DNS／分割／出口主备／自指都复用已验收的单方案语义，没有“编译期不知道选谁”的问题。

(a) 需要出口地区、池语义和第二层 fallback，并把 DNS 校验从“逐方案”变成“逐方案 × 池内每个结果”，收益可由 (b) 为每个地区写一个方案得到。(c) 回答的是另一个问题（“访问某国目的地走该国出口”），且可以完全展开成现有的 `geoip:*` 规则——建议作为 Console 的规则模板，不进入 Policy。

### 待用户决定

- **Q1**：`auto` 的语义定为 (b)「按源节点地区选方案」，(a)／(c) 不做？推荐：是；(c) 以 Console 模板提供。

## 2. 地区从哪来 {#region-source}

### 选项

| 来源 | 内容 | 可信度 | 延迟 | 投影要求 |
| --- | --- | --- | --- | --- |
| (i) 显式标签 `nodes.<id>.region` | 管理员写入的大写两字母代码，与 `auto.rules.countries` 同一词表 | 管理员声明，确定 | 配置保存即一次 revision，随下一次 `sync` | `NodeSettingsView` 已在投影里，多一个字段即可 |
| (ii) Server GeoIP 推断 | Server 用已导入的 GeoIP 数据集查询 State 的 `location`（STUN 优先于 interface、IPv4 优先于 IPv6）得到国家 | 反映 underlay 公网地址的登记位置，VPN／代理／WARP 下不代表设备位置；interface 候选为节点自报 | 观测到达＋驻留时间（[§3](#stability)） | Server 持有解析结果，必须作为新字段投影 |
| (iii) RTT | 选到某出口 RTT 最低的方案 | Server **当前没有**可用输入：State `links` 无 RTT，按需邻接使未使用的出口没有通道和样本，观测历史是抽样且不得拼接成端到端 RTT | 需要新的主动测量与上报 | 新的 State 字段与测量调度 |

各情况的行为（适用于 (ii)，(i) 只有“有标签／无标签”两种）：

| 情况 | `basis` | 采用 |
| --- | --- | --- |
| 有标签 | `label` | 标签（覆盖推断） |
| 有 `location` 且数据集给出国家 | `observed` | 该国家（经驻留确认后） |
| `location` 为 nonPublic／`locationAbsent`（noObservation／ambiguousObservation） | `held` 或 `noTrustedRegion` | 保留上次已确认的地区直到保留期满，之后 `auto.fallback` |
| `observationExpired`、节点离线 | `held` | 同上；离线节点本就收不到新 netmap |
| 数据集未导入／不含该地址 | `noTrustedRegion` | `auto.fallback` |
| 国家不匹配任何 `auto.rules` | `noRuleMatched` | `auto.fallback` |

### 生效时间与租约

- 地区变化只经 Server 重新编译 netmap 生效：新 serial、重签租约、唤醒挂起的 `sync`。不新增推送通道，也不缩短租约。
- 节点与 Server 分区期间保留 netmap 中的解析结果直到租约到期（与其他选择一样），到期 fail closed；节点不因本机地址变化自行改选。
- (ii) 的观测本身来自节点 State，与 Server 断联的节点不会产生新地区；这与“选择只归 Server”一致。

### 推荐

第一版只做 (i)。(ii) 作为第二步，复用 (i) 的 basis 与投影形状，但要先定驻留参数和 VPN 行为；(iii) 不做，直到有独立的出口 RTT 测量设计（并且 RTT 只能用于在**同一方案的主备集合内**排序时才不改变 selection 语义，那是另一个问题）。

### 待用户决定

- **Q2**：地区来源第一版只用管理员标签 `nodes.<id>.region`，GeoIP 推断放第二步，RTT 不做？推荐：是。
- **Q3**：标签词表用 ISO 3166-1 alpha-2（与 `countries` 一致），不支持自定义地区名（如 `us-west`）？推荐：是；自定义地区需同时改 `auto.rules` 字段，留待需求出现。
- **Q4**：标签只允许 Owner／Admin 改（普通配置写权限），成员不能自声明自己设备的地区？推荐：是。
- **Q5**（第二步）：GeoIP 推断是否接受节点自报的 interface 候选，还是只接受 STUN 观测？推荐：只接受 STUN（Server 侧观测的反射地址），interface 只作展示。
- **Q6**（第二步）：识别到 VPN／代理出口地址（如 Cloudflare WARP 段、ASN 属于 VPN 服务商）时，是否视为 `noTrustedRegion`？推荐：不内置 VPN 名单；需要时由管理员在该节点上写标签覆盖。

## 3. 稳定性 {#stability}

### 存量流

`auto` 结果改变即方案切换，按现有行为结束经旧出口的存量连接。不提供“存量流按旧判定保留”：那需要源、中继、出口三方在同一份 netmap 里同时持有两个 selection，违反单一授权状态，也会让 X6 关联跨 selection 复用。

selection 身份必须随解析结果变：`egress.generation` 当前对 `auto` 只含字面量 `auto`，标签变化不改 `routing.egress` 也不改选择版本，**会得到相同 generation 而规则已不同**。设计要求把解析结果（所选方案与 basis 的地区）并入 generation，使出口、中继从投影重算时得到同一值，任何解析变化都使旧流在重判时结束。

切回的处理：C3 要求“任何选择切换（含切回）都改变 generation”，依靠的是选择版本加一。标签驱动的 US→JP→US 不改选择版本，回到 US 时 generation 等于最初的值。中间的 JP netmap 一经应用，旧流已全部结束、关联已删除，因此回到相同 generation 不会复活任何旧资格；若节点从未取得中间的 JP netmap（一次 `sync` 内合并），它前后看到的是同一策略，保留存量流是正确的。

### 抖动抑制

(i) 标签只由配置保存改变，没有抖动，不需要抑制。

(ii) 推断需要 Server 侧持久状态（每节点一行：已确认地区、确认时间、待定地区、待定起始）：

| 参数 | 含义 | 建议值 |
| --- | --- | --- |
| 确认驻留 | 新地区必须在连续观测中保持多久才采用 | 10 min |
| 最小驻留 | 采用一次后多久内不再改变 | 30 min |
| 保留期 | 观测缺失／歧义／过期时保留已确认地区多久 | 24 h（与默认租约同量级） |

状态持久化在 Server 数据库，Server 重启不重新确认；节点身份改变（重新接入）后旧状态不适用，与选择一样按身份保存。每次采用记一条活动（actor 为 Server）。

### 待用户决定

- **Q7**：`auto` 结果变化时立即结束经旧出口的存量连接（与手动切换一致），不保留？推荐：是。
- **Q8**：标签驱动的切回得到与最初相同的 selection generation 可以接受（中间切换已结束旧流），不为此引入额外序号？推荐：接受；第二步的推断引入持久确认序号后自然区分。
- **Q9**（第二步）：确认驻留 10 min／最小驻留 30 min／保留期 24 h，是否作为网络级可配置项？推荐：先固定，不开放配置。
- **Q10**：`auto` 结果变化是否写入活动记录？推荐：是，`routing.autoResolve`，actor 为 Server。

## 4. DNS {#dns}

- **判定**：`dns_resolution` 对有效方案为 `auto` 的源，改用解析出的具体方案判定（当前用 `auto.fallback`）。四形态不变。
- **保存校验**：`auto` 的每个目标与 fallback 已被要求在主体 `allowed` 内，而保存校验已对每个 `allowed` 方案判定 `dns_exit_plan_required`，因此 (b) 下“编译期不知道会选谁”不产生新问题：每个可能结果都已单独通过。无需新的 IssueCode。
- **切换**：解析变化改变 selection generation，`DnsService` 随之改变，出口的 `ServeExitResolver` 同步改变；Server 在新 serial 后重验 X6 关联，旧 generation 的条目在同一次 `sync` 回答中删除（[Exit 设计 · 可信域名关联](/v2/node/exit-design#trusted-dns-associations) 的“失效”）。不新增机制。
- (a) 若被采纳：名称规则引用的出口池必须对池内每个可能结果判定，任意两个结果的 `(exit, via)` 不同即 `dns_exit_plan_required`——实际上等价于“名称规则不能用 `auto:<pool>`”。(c) 的名称规则直接拒绝。

### 待用户决定

无（推荐的 (b) 不改变 DNS 产品语义）。

## 5. 投影、校验与展示 {#projection}

### 投影

| 选项 | 内容 | 适用 |
| --- | --- | --- |
| P1 投影输入 | V 中节点的 `NodeSettingsView.region`；节点在投影上用同一函数解析 | (i)：结果是配置的纯函数 |
| P2 投影结果 | `ProjectedConfiguration.autoResolutions: NodeId → {profile, basis, region}`，限于 V | (ii)：结果依赖 Server 持久状态，节点无法从配置重算 |
| P3 只投影候选 | 投影全部 `auto.rules` 目标，由节点按某个事实自选 | 违反“选择只归 Server”，不采用 |

推荐 (i) 用 P1，(ii) 时加 P2 并让 P2 优先于 P1 的推断部分（标签仍覆盖推断，标签本身在 P1 里）。投影规则补充：`profiles` 保留条件已包含“被 `auto` 引用的”，不变。

### compilerVersion

升到 `meshway-netmap-3`。理由：`auto` 的解析与 generation 计算改变，旧节点在新投影上会算出不同的 generation，混合版本的源／出口握手会失败；版本号让旧节点作废缓存并重取，而不是用旧语义执行新投影。

### 配置校验

| 代码 | 级别 | 路径 | 条件 |
| --- | --- | --- | --- |
| `invalid_country_code`（复用） | error | `/nodes/<id>/region` | 不是大写两字母 |
| `auto_region_unmatched`（新增） | warning | `/nodes/<id>/region` | 节点 `allowed` 含 `auto`、有标签，但标签不在任何 `auto.rules.countries` 中（恒为 fallback） |
| `auto_region_missing`（新增） | warning | `/routing/egress/assignments/<i>` | assignment 的 `default` 为 `auto`，其命中的节点中有无标签者（第一版恒为 fallback）；`params.nodes` |

### 状态字段

`AutoResolution` 扩为 `{profile, basis, region}`；`AutoBasis` 增加 `label`、`noRuleMatched`（第二步再加 `observed`、`held`）；`region` 为采用的代码，`noTrustedRegion` 时为 null。Server、节点 IPC `NetmapStatus.routing`、`meshway exit` 由同一 `routing_selection` 求出。

### Console 需展示的字段（不含 UI 设计）

- 节点：`region` 标签（可编辑）；第二步另显示推断地区、basis 与确认时间。
- 选择：`RoutingSelectionState.auto.{profile, basis, region}`；`profiles[].kind = auto` 时其目标映射（`auto.rules` 与 fallback）。
- 配置编辑：上述两个 warning；`auto` 目标须在 `allowed` 内的既有错误。
- 活动：`routing.autoResolve`（若 Q10 采纳）。

### 待用户决定

- **Q11**：接受 `compilerVersion` 升到 `meshway-netmap-3`（需先升节点再升 Server，或同时升级；旧节点在升级前停用 `auto` 即无影响）？推荐：是。
- **Q12**：RoutingSelectionState 向节点与用户显示采用的国家代码（第二步即是 GeoIP 结果，S3 只从 State 去掉了 GeoIP）？推荐：显示，只限选择视图，不回到 State。

## 6. 边界 {#boundaries}

| 交互 | 规则 |
| --- | --- |
| 手动选择 | `auto` 只是一个方案。节点或用户 `routing-selection` 选具体方案即覆盖 `auto`；再选 `auto` 回到地区解析。优先级：有效的持久选择 → assignment default（可为 `auto`）→ 解析。 |
| locked | assignment 锁定为 `auto` 时用户不能覆盖；锁定为其他方案时 `auto` 不生效。地区输入被伪造只能落到另一个 `auto` 目标，均已被管理员允许。 |
| 自指 | 解析出的方案含源自身（如 `global:<self>`，或 `[A, self]`）时按现有 E1 规则：self 为 direct，之前的出口按 `eligibleExits`。“东京节点按地区选到东京出口方案”就是它自己的 direct，不需要特判。 |
| `auto` 递归 | 目标与 fallback 不得为 `auto`（已有 `auto_target_unknown`）。 |
| IPv6 | 地区来源与地址族无关；IPv6 出口仍 `unsupportedFamily`，受管 IPv6 仍 sink。第二步推断时 `location` 本就 IPv4 优先。 |
| 多出口 DNS plan | 仍不支持；每个目标方案单独满足单出口 DNS 规则（[§4](#dns)）。 |
| 出口可用性 | 解析不看出口就绪、可达或 RTT；选中方案的出口不可用照常 fail closed，不因此换方案。 |
| 规则集缺失 | 解析与 artifact 无关；所选方案缺 artifact 时照常 `missingDependency`。 |

### 待用户决定

- **Q13**：用户手动选择始终覆盖 `auto`（除非 locked），不提供“auto 优先、手动仅临时”的模式？推荐：是，与现有选择模型一致。
- **Q14**：所选方案的出口不可用时不自动换到其他地区的方案？推荐：是；可用性切换属于方案内主备，不属于地区选择。

## 7. 第一版切片 {#first-slice}

> **已实现**，见 [Routing · 自动方案](/v2/configuration-reference/routing#auto)；验收为 `exit/off` 第 14 阶段（[验收范围 · 出口场景](/v2/node/validation#exit-scenario)）。

### 范围

- 配置：`nodes.<id>.region`（可省略，大写两字母）。
- 解析：`auto` 的有效方案 ＝ 首个 `countries` 含该标签的 `auto.rules[].profile`，否则 `auto.fallback`；basis `label`／`noRuleMatched`／`noTrustedRegion`。
- generation：并入解析出的方案与地区；`compilerVersion` → `meshway-netmap-3`。
- DNS、分割、`egress` 计划、路径检测、节点判定、中继／出口重算全部经同一解析函数。
- 校验：`/nodes/<id>/region` 复用 `invalid_country_code`；新增 warning `auto_region_unmatched`、`auto_region_missing`。
- 不做：GeoIP 推断、驻留状态、RTT、(a)／(c)、新 HTTP 端点或数据库表。

### 改动范围

| 位置 | 内容 |
| --- | --- |
| `crates/meshway-configuration` | `NodeSettings`／`NodeSettingsView.region`；`AutoResolution.region`、`AutoBasis` 新值；`compilerVersion`；IssueCode |
| `crates/meshway-policy` | 解析函数（`compile/selection.rs` 与 `capture::selection` 共用）、generation（`netmap/local.rs`）、`dns_resolution` 输入、校验（`validation/routing`、nodes）、投影等价测试 |
| `apps/meshway-server` | 无业务逻辑改动；OpenAPI／SDK 重生成 |
| `apps/meshway-cli` | `meshway exit` 显示 basis／region |
| `apps/meshway-web-console` | 节点 region 字段与选择视图字段 |
| `docs/v2` | Routing、Nodes、Netmap 设计、changelog、验收范围 |

### 验收

在 `exit/off` 增加一个阶段（复用 A→R→E 与 `exit`／`exit-alt` 方案，见[验收范围 · 出口场景](/v2/node/validation#exit-scenario)）：A 的 `allowed` 为 `[auto, exit, exit-alt, mesh]`、default `auto`，`auto.rules = [{US → exit}, {JP → exit-alt}]`、fallback `mesh`。

1. A 无标签：`meshway exit --json` 给出 `auto.profile = mesh`、`basis = noTrustedRegion`，外部服务不见 A 的访问。
2. 管理员写 `region: US`：A 应用新 netmap 后 `profile = exit`、`basis = label`，新连接经 E 到达服务；E 重算的 selection 与 A 一致（握手成功）。
3. 存量 TCP／UDP 流建立后改为 `JP`：A 应用后旧流全部失败，新连接经 E（`exit-alt`）到达；旧 generation 的可信关联在同一次 `sync` 后消失。
4. `meshway exit exit` 覆盖 `auto`；`meshway exit auto` 回到 `exit-alt`。
5. 改为 `FR`：`basis = noRuleMatched`，回到 `mesh`。

单元／差分层：解析的投影等价测试（P1 下投影与全量配置逐项相同），以及 “auto 结果变化 ⇒ generation 变化” 的性质测试。

### 待用户决定

- **Q15**：第一版按上述切片（标签＋方案级 `auto`，不碰 GeoIP），验收为 `exit/off` 的新阶段而不是新矩阵行？推荐：是。

## 决定清单

| 编号 | 问题 | 推荐（均已采纳） |
| --- | --- | --- |
| Q1 | `auto` 语义 | (b) 按源地区选方案；(c) 作 Console 模板 |
| Q2 | 地区来源 | 第一版只用标签；GeoIP 第二步；RTT 不做 |
| Q3 | 词表 | ISO 3166-1 alpha-2 |
| Q4 | 谁能写标签 | 仅 Owner／Admin |
| Q5 | 推断接受的观测 | 只用 STUN |
| Q6 | VPN 地址 | 不内置名单，用标签覆盖 |
| Q7 | 切换时存量流 | 立即结束 |
| Q8 | 标签切回同 generation | 接受 |
| Q9 | 驻留参数 | 固定 10 min／30 min／24 h，不开放 |
| Q10 | 活动记录 | 记录 `routing.autoResolve` |
| Q11 | compilerVersion | 升到 `meshway-netmap-3` |
| Q12 | 显示国家代码 | 在选择视图显示，不回到 State |
| Q13 | 手动覆盖 | 手动选择始终覆盖（locked 除外） |
| Q14 | 出口不可用 | 不换地区方案 |
| Q15 | 第一版切片 | 标签＋方案级 `auto`，`exit/off` 新阶段 |
