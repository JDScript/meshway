# Routing · 流量路由

定义获准业务如何在允许的图上选择路径。access 管理业务权限，topology 定义允许的图；routing 分为 Mesh 内部和出口两部分。

## 与 topology 的关系

topology 定义哪些直接边存在及其承载顺序，routing 定义流量经过的路径与出口。直接边隔离用 topology block／默认 deny，必经节点用 routing `via`；同一意图不在两处重复表达。listeners／dialers 只声明节点能力，业务权限由 access 定义。

组网规则见 [Topology](/v2/configuration-reference/topology)。routing 只能使用其允许的边，不能因更快或可达而绕过 block／必经节点。运行时已选路径见 [State.paths](/v2/state-reference#paths)。

## routing.mesh

以下为该字段的数组值示例，组、标签和中转 Node 须在本网络有效：

```json
[
  {
    "remark": "后端访问必经 core",
    "src": ["group:dev"],
    "dst": ["tag:backend"],
    "via": ["node:<relay-id>"]
  }
]
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `remark` | `string` | 自由备注，省略补空字符串，可重复 |
| `src` / `dst` | `string[]` | 源与目的节点选择器 |
| `via` | `string[]` | 必须按顺序经过的节点，使用具体 Node 选择器 |

每一段都从上到下 first-match；无匹配时自动选路。选中规则的 `via` 按下述规则递归展开，所有备选路径都必须保留展开后的必经点。已建立会话及回复固定原完整路径，切换承载不等于改逻辑路径。

### 递归路段

匹配使用**当前路段的实际起点／终点**。先为原业务 A→D 选规则；如果命中 `via=[C]`，则分别求 A→C、C→D，各段继续 first-match 和递归展开：

```text
A→D via C
A→C via B
               ⇒ A→B→C→D

再加入 C→D via E
               ⇒ A→B→C→E→D
```

规则中已经位于段起点／终点的边界节点视为满足：A→C 命中 `via=[C]`、C→D 命中 `via=[C]` 均结束该次要求；`A→* via C`、`*→D via C` 因此可以正常展开。仅忽略首位等于段起点、末位等于段终点的边界出现；`C→D via [B,C]` 仍要求重访 C，会被拒绝。已命中的空 `via` 或边界满足规则不会再尝试后续规则。

展开只求转发路径，**不改变原业务源 A、最终目的 D、服务和 Access**，也不额外授予 A→C、C→D 的业务权限。业务、Mesh IP 归一化后的需求及独立诊断共用同一 Policy 路径编译。出口动作仍按原业务目的选定，出口前的 Mesh 路段按相同递归规则求路；实际出口运行能力仍见 [Node](/v2/node)。

### 可行性、循环与失败

- 展开得到有序必经节点后，在 topology 允许且有真实承载能力／材料的边上找路。自动补入的邻接也须满足该段 routing；不能利用一条物理可直连的边绕过它要求的中继。补入节点到当前段目标的剩余路段继续求值。
- 每条实际边还须满足 [topology.transports 的双向拨号交集](/v2/configuration-reference/topology#承载交集)；两个方向都无候选时不能使用该边，不能改用集合外的协议。
- 最终完整路径不允许重复节点。`A→B via C` 与 `A→C via B` 的相互依赖、递归后重复必经节点均不可执行，不回退到后续 routing 规则，也不删除必经点。
- 按需编译返回 `unavailable/routingCycle` 表示递归依赖／必经节点循环，`unavailable/routingLimit` 表示展开或搜索预算不足；必经节点禁用、库存／承载缺失、拓扑阻断等无可行路径返回 `unavailable/noPath`。自动搜索中某个可选分支无效时可以排除该分支，不能把已验证的其他路径改成宽松路径。
- 当前完整路径最多8个节点。备选路径按长度、同长按节点 ID 排列；同一长度的路径全部列出后才搜索更长的，在已列满 8 条（`MAX_PATHS`，含直连）的那个长度结束，因此能同时到达两端的每个两跳中继都是候选、不按节点 ID 截断。源节点按自己已知的通道事实排序并取前 8 条，见[中继兜底](/v2/node/connectivity#relay-fallback)。一次目标搜索共享16,384工作项预算（递归调用、规则扫描和边候选），队列最多2,048项，每目标最多2,048条（`MAX_SEARCH_PATHS`）。达到搜索预算后只可保留已经完整验证的候选；没有候选则返回受限结果。出口候选各自有界。
- Configuration 的保存校验负责字段、引用等约束；依赖具体节点身份、需求与库存的递归可执行性由 Policy 按需编译判断。单条不可执行需求不阻断其他需求的撤销／执行发布；没有许可的路径不会下发为可用路径。

与 access／topology 一致，路由规则不要求用户维护 id，定位使用 revision＋JSON Pointer。profile、ruleSet 的命名 key 仍用于引用，不属于规则行 ID。

## routing.egress

以下为该字段的完整结构示例；引用的出口、用户组及方案还须通过网络上下文校验：

<!-- schema: EgressRouting -->
```json
{
  "rules": [],
  "profiles": {
    "work": {
      "rules": [
        {
          "remark": "办公网",
          "match": { "ipCIDR": ["10.20.0.0/16"] },
          "action": { "type": "exit", "nodes": ["node:00000000-0000-4000-8000-000000000001"], "via": [] }
        }
      ],
      "fallback": { "type": "unmanaged" }
    }
  },
  "assignments": [
    { "target": ["group:dev"], "allowed": ["mesh", "work"], "default": "mesh", "locked": false }
  ],
  "auto": {
    "rules": [{ "countries": ["US"], "profile": "work" }],
    "fallback": "mesh"
  }
}
```

### Fields

| 字段 | 说明 |
| --- | --- |
| `rules` | 公共有序规则，排在所有方案前面 |
| `profiles` | 管理员定义的方案；每项有有序 `rules` 和 `fallback` |
| `assignments` | 选择器对应的 allowed 方案、默认方案和 locked 状态 |
| `auto` | 按源节点声明的地区（[`nodes.<id>.region`](/v2/configuration-reference/nodes#fields)）选方案：有序 `rules: [{countries, profile}]` 与 `fallback`，见[自动方案](#auto) |

有效规则 = 公共规则 → 当前方案规则 → 方案 fallback。空公共／方案规则不覆盖 fallback。

重叠分配的 allowed 取并集；匹配项的 default 或 locked 不一致就拒绝保存，不按数组顺序覆盖。合并后 default 必须在 allowed 内，auto 的所有目标／fallback 也须在主体允许范围内。

### Rule match

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `domain` | `string[]` | 精确 DNS 名称，只匹配该名称本身；为空时规范输出省略 |
| `domainSuffix` | `string[]` | DNS 标签边界后缀，匹配该名称及其所有子名称 |
| `ipCIDR` | `string[]` | 目的网段 |
| `ruleSets` | `string[]` | [规则集](/v2/configuration-reference/rule-sets)：本网络 `ruleSets` key，或 provider 目录集合 `<provider>:<name>`（如 `geosite:openai`、`geoip:cn`） |

目的条件取并集，至少一项非空；规则从上到下 first-match。本规则自身列表与每个引用集合都经同一 matcher（`meshway-rules`）判定；引用集合的内容按 digest 下发，节点尚未取得时该规则不可用，first-match 在此停止、不跳到后续规则、不 direct，见[规则集 · 下发](/v2/configuration-reference/rule-sets#下发)。域名匹配需要可信域名上下文，不能从普通 IP 包直接推断，见下文[域名规则的执行](#域名规则的执行)。名称按 DNS 规范化比较（小写、ASCII／IDNA、忽略结尾点）；`domain: ["example.org"]` 不匹配 `a.example.org`，`domainSuffix: ["example.org"]` 两者都匹配。前面规则已完全覆盖的规则（含被后缀覆盖的精确名称）报告 `shadowed_rule` 警告。

### Action

| 值 | 含义 |
| --- | --- |
| `{ "type": "exit", "nodes": ["node:<id>"], "via": [] }` | 有序主备出口，可指定出口前的有序中转 |
| `{ "type": "direct" }` | 匹配流量从源节点本地网络直接发出 |
| `{ "type": "unmanaged" }` | 不接管 |

出口集合含源节点自己时，该源把自己当作那个位置上永远可用的 `direct`：`nodes` 是有序主备，自己之前在全量配置上可用（启用且 `exitEnabled`）的出口按原顺序保留、不可用的跳过，一个都没有才 direct——`[self]`、`[self, other]` 为 direct，`[other, self]` 在 other 可用时经 other、否则 direct。运行时出口就绪与可达性不是输入：可用但未就绪、被 topology 隔离或未登记／已吊销的出口照常失败闭合，不会让源转为 direct；节点不在自己投影里推断出口可用性，只读 Server 按全量配置给出的 `eligibleExits`（[投影规则](/v2/node/netmap-design#内容字段)）。direct 时它自己就是出口，流量从本机直接发出，`via` 是空段；普通 DNS 按 `noExit` 用本机系统 resolver（与它为其他源服务的是同一系统 resolver）。分割、捕获、`egress` 计划、DNS plan、路径检测与节点判定都由 `meshway-policy` 的同一函数给出（`matching::executed`），Access 不变；其他源照常经该出口。因此一条“`geoip:cn` → exit sha”的公共规则对 sha 自身是 direct、对其余节点经 sha。

出口必须启用 `nodes[id].exitEnabled`，目的访问必须获 access 允许：IP 目的地须在源节点的 CIDR／`autogroup:internet` 授权内；域名目的地（`domain`、`domainSuffix` 与规则集内容，规则集的 subdomain 成员按其后缀判断，keyword 成员只有 `autogroup:internet` 能覆盖）须被 `domain:` 后缀授权或 `autogroup:internet` 覆盖，不满足时报告 `exit_outside_access`（`params.node`、`params.destination`）。域名授权按解析地址集合执行，见 [Access](/v2/configuration-reference/access#domains)。没有可用获准路径时拒绝，不能回退到后续规则或偷偷 direct。

### 域名规则的执行

名称成员（`domain`、`domainSuffix` 与规则集的名称成员）不拥有任何地址：出口分割只由 IP 成员切分，没有可信上下文时名称规则不匹配任何包。名称规则**现在生效**的条件与范围：

- 源节点的普通查询经出口 resolver 解析（`viaExit`）：方案里经出口的名称规则（与出口 fallback）共用一个出口，查询就交给它（[DNS · 使用出口时](/v2/configuration-reference/dns#使用出口时)）。只有 IP 规则的方案（`noExit`，几个出口都可以）、MagicDNS 与 `useWithExit` split 不产生上下文，其中的名称规则不匹配任何地址。名称规则经不同出口的方案保存时即以 `dns_exit_plan_required` 拒绝。
- 出口回答该源的查询后，Server 受控校验并签发**可信关联**（名称 → 公网地址集合，寿命为回答的最短 TTL，取 30 秒到 1 小时），源节点据此判定这些地址：规则顺序 first-match，命中条件为“匹配地址或匹配其关联名称”。私网、Mesh 与其他非公网回答不进入关联。
- 源节点把该名称的回答交给本机客户端前等待关联生效，直到该查询截止前 250 ms（至多 5 秒），未生效则回 SERVFAIL；只有这一个查询等待；回答的 TTL 截为关联剩余寿命。关联生效期间对这些地址的连接（不经名称也一样）都按名称规则判定；未经出口 resolver 得到的地址只按 IP 成员判定。
- 同一地址关联了多个名称且它们的首个命中规则动作不同时，该地址不可执行（`egressConflict`），不选更宽路径。
- 撤销 `domain:` 授权、停用出口、切换方案或删除名称规则后关联随新 netmap 失效。

任务、overlay、往返与界限见 [Exit 设计 · 可信域名关联](/v2/node/exit-design#trusted-dns-associations)。

## 出口联合许可

```text
业务目的 access
  ∩ 管理员允许方案中的 routing 动作
  ∩ exitEnabled
  ∩ topology / 完整路径约束
```

源节点在自己的投影上按首包求出这一交集（见下文 `Decision`），Server 不签发逐流许可。公共规则抢先指定 E 的范围，不会又从 F 的 fallback 获得准入。

### 执行捕获与精确许可

出口分割（源节点的包归属）由**节点**用 `meshway-policy` 从自己的 netmap 与持有的规则集 artifact 计算，Server 用同一实现做校验与预览，netmap 不携带前缀列表。netmap 的 `egress`（`EgressPlan`）只带生效规则的引用与动作、fallback、选择身份 `generation` 与 `unavailable`：

| 字段 | 含义 |
| --- | --- |
| `generation` | 选择身份：网络、节点、有效方案、有效方案为 `auto` 时的解析结果（方案、basis、地区）、持久选择版本、`routing.egress` 与这些规则所用集合的 digest；与节点持有哪些 artifact 无关，出口节点为源算出的值相同 |
| `rules[]` | 公共规则在前、有效方案规则在后：`{rule: JSON Pointer, action}` |
| `fallback` | 无规则命中时的动作 |
| `unavailable` | 持有全部 artifact 时分割仍不可用的原因（`egressConflict`、`routingLimit` 等）；可用为 null |

分割语义：按顺序从尚未命中的 IPv4／IPv6 地址中扣除每条规则自己的 CIDR 与所引用集合的合并区间（first-match），余下归 fallback。结果是覆盖两族全部地址的有序、互斥区间，相邻区间动作不同。节点安装的捕获边界只有受管部分：出口动作的 IPv4 区间（受管，进本节点 TUN）与出口动作的 IPv6 区间（`sinkIpv6`，拒绝；已授权 Mesh IPv6 仍走 Mesh 路由，源端没有 IPv6 地址也不省略），direct／unmanaged 是其补集，交宿主路由。fallback 为出口且还剩 IPv4 时为默认捕获。

规则引用的集合未持有（artifact 缺失）时该规则 `missingDependency`，分割不可用。名称成员不参与分割；源节点在自己的分割之上加入其可信关联地址（见[域名规则的执行](#域名规则的执行)）：名称规则决定为出口（或名称间冲突）的地址并入受管区间（IPv6 并入 sink），决定为 direct／unmanaged 的移出；关联到期即由 Engine 以新的 `Desired` 移除。加入后超出节点侧上限时全范围捕获、`routingLimit`。不可用时安装全范围 IPv4 捕获与全范围 IPv6 sink，没有 direct／unmanaged 例外，新流拒绝；不能把 DNS 查询猜成普通 IP 包的业务身份。

#### 节点侧上限 {#capture-limits}

| 上限 | 值 | 计数 |
| --- | --- | --- |
| 捕获区间 | 16,384 | 受管 IPv4 区间＋sink IPv6 区间（nft interval 元素） |
| 捕获路由 | 32,768 | 受管 IPv4 区间的最小 CIDR 覆盖（私有捕获表路由） |
| 计算量 | 2²⁴ 次区间操作 | 一次分割计算 |

超限时该方案 `unavailable: routingLimit`（全范围捕获），节点对一切外部目的 fail closed，路径检测对外部目的同样给出 `outcome: unavailable`（原因同 `capture.unavailable`）。配置校验对每个可分配方案计算同一分割，超限给出警告 `egress_partition_limit`（`params.profile`／`ranges`／`maxRanges`／`routes`／`maxRoutes`，路径为方案或 `/routing/egress/rules`；分割计算先耗尽其工作预算时没有计数，同一警告只带 `profile` 与 `work`（预算），节点同样 fail closed）；路径检测的 `profile.capture` 给出 `{managedIpv4, sinkIpv6, routes, unavailable}`，与节点 IPC `NetmapStatus.capture`（CLI `meshway status` 的 Egress 行）同一计算；节点侧的数字另含本源当前生效的可信域名关联（按实际安装的捕获边界计数，关联变化或到期后随之更新）。`geoip:cn` 的直连方案（补集）约 4,341 个受管 IPv4 区间、12,680 条路由。

上限由实测定出（`mise run perf:capture`，Linux 6.8／AMD 9950X、Linux 7.0／OrbStack VM，nftables 1.0.9；原始结果在私有证据 `.meshway/validation/r3/perf/`）：

| 区间 | nft 提交 | nft 替换 | nft 读回 | 读回字节 | 内核 slab | 带回显替换（`-e`） |
| --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 4–11 ms | 12–17 ms | 3–7 ms | 47 KB | ≤ 0.2 MB | 成功 |
| 6,000 | 16–28 ms | 27–31 ms | 12–24 ms | 285 KB | ≤ 0.8 MB | 成功 |
| 10,000 | 25–36 ms | 40–43 ms | 21–33 ms | 476 KB | ≤ 1.2 MB | 成功 |
| 16,384 | 41–59 ms | 59–62 ms | 36–50 ms | 786 KB | ≤ 2.0 MB | 成功 |
| 32,768 | 92–113 ms | 114–118 ms | 84–104 ms | 1.57 MB | ≤ 4.1 MB | ENOBUFS |
| 50,000 | 148–179 ms | 174–190 ms | 141–160 ms | 2.39 MB | ≤ 6.4 MB | ENOBUFS |

nft 进程峰值 RSS 约 1.6 KB／区间（16,384 区间 28 MB）。带回显的大批次在 32,768 区间起被 netlink 以 ENOBUFS 拒绝，因此元素替换不回显、以读回核验；16,384 个区间使最坏情况（全为 IPv6）的批次与读回仍低于 1.6 MB，且留出暂存并集（两份方案）的空间。生产路径（release，Planck 隔离 namespace，`large_partitions_install_switch_read_back_and_retire`）在 16,077 个区间、16,034–19,240 条路由时暂存 0.30–0.45 s、提交 0.38–0.50 s、读回 0.07 s、暂停 0.37 s；路由变更每 64 条等待内核监视器观测，不使其通知缓冲（约 200 条）溢出。

节点首包判定（`LocalCompiler`）对一条源流给出 `Decision`：`source`、精确 `destination`、`target`（`mesh { node }`，或 `exit { nodes, via }`：有序主备出口及其前的有序中转）、该源对该目的可用的全部 `services`、全部允许的 `paths`（完整节点序列与按实际拨号方分组的承载候选）以及出口决定的 `selection`。范围分类只限制精确目的，不授予范围内其他目的；回复沿原完整路径返回，由数据面绑定。`selection` 即源端 `egress.generation`：不含 revision，无关配置变更不改变它，任何选择切换（含切回同名方案）与 `auto` 解析结果的变化都改变它；数据面握手把它绑入路径授权摘要，出口节点从自己的投影重算源端 selection 并核对。

出口节点的 netmap 带 `exitRole: ExitRole { generation, family, nat }`：当前固定为 `ipv4`／`masquerade`，`generation` 是网络、节点、family 与 nat 的 digest，角色内容不变即不变（不含 revision、选择或 instance）。只有节点 `exitEnabled` 且 inventory 中有 `feature:exit-ipv4` v1 能力时才有角色；角色本身不转发包，转发仍需上述 `Decision`。

出口能力未知、过期或失败时返回 `unavailable/exitUnavailable`，保留捕获边界；路径检测的 `targets[].exitUnavailableReason` 展示新鲜的具体失败。IPv6 出口返回 `unsupportedFamily`。能力事实不能增加 Access 或绕过 topology。真实出口安装与包流验收范围见 [Node](/v2/node)。

新方案分配含主体未获授权的目标时拒绝保存。后续身份／access／出口撤销必须可以执行，不能被旧方案引用阻止；受影响执行变为拒绝／不可用，不偷偷恢复 direct 或其他更宽路径。

### 内置方案与当前选择

- `mesh`：公共规则，其他外部流量不接管。
- `global:<exit-node-id>`：为 exitEnabled 节点生成目录项；**目录存在不等于获准使用**，需要 assignment.allowed 明确包含并通过联合校验。
- `auto`：按源节点声明的地区在已允许的方案中选择，见[自动方案](#auto)。

用户选择属于持久的使用状态。重启后仍在 allowed 且未被锁定时恢复；否则采用当前管理员默认。任何方案切换都结束旧出口存量连接，不能携带旧方案的 NAT／流资格跨过去。

#### 自动方案 {#auto}

`auto` 是一个方案选择器：源节点的有效方案为 `auto` 时，采用 `auto.rules` 中第一条 `countries` 含该节点 `region`（管理员在 [`nodes.<id>.region`](/v2/configuration-reference/nodes#fields) 声明的 ISO 3166-1 alpha-2 代码）的规则的 `profile`；没有规则包含它、或节点未声明地区时采用 `auto.fallback`。

| `basis` | 条件 | 采用 |
| --- | --- | --- |
| `label` | 节点声明了 `region`，且某条规则的 `countries` 含它 | 第一条这样的规则的 `profile` |
| `noRuleMatched` | 节点声明了 `region`，没有规则含它 | `auto.fallback` |
| `noTrustedRegion` | 节点未声明 `region` | `auto.fallback` |

```json
{
  "rules": [
    { "countries": ["US", "CA"], "profile": "us-exit" },
    { "countries": ["JP"], "profile": "tokyo" }
  ],
  "fallback": "mesh"
}
```

- **只选已允许的方案**：目标与 fallback 必须是主体 allowed 内的非 `auto` 方案（`auto_target_unknown`／`auto_target_not_allowed`）。地区只决定落在哪个已允许方案上，不扩大授权。
- **同一解析**：分割、捕获、`egress` 计划、DNS plan（[`dns_resolution`](/v2/configuration-reference/dns#使用出口时) 按解析出的方案判定）、可信关联、路径检测与节点判定都用 `meshway-policy` 的同一解析函数；`region` 随投影下发（[Netmap 设计 · 内容字段](/v2/node/netmap-design#内容字段)），中继与出口在自己的投影上重算同一结果。保存时对每个 allowed 方案各自判定 `dns_exit_plan_required`，因此 `auto` 的每个可能结果都已单独通过。
- **生效**：地区只经配置保存改变；保存即一次新 revision，Server 重新编译并签发 netmap，节点经下一次 `sync` 取得。解析结果计入 selection 身份（`egress.generation`）：结果变化时与手动切换一样结束经旧出口的存量连接，旧 generation 的可信关联随新 netmap 失效。标签切回原值时 generation 等于最初的值（中间那份 netmap 已结束旧流、删除关联）。
- **手动选择优先**：节点或用户以 `routing-selection` 选具体方案即覆盖 `auto`（assignment 锁定时除外）；再选 `auto` 回到按地区解析。优先级为：有效的持久选择 → assignment default（可为 `auto`）。
- **不换方案**：解析不看出口就绪、可达或延迟；选中方案的出口不可用照常失败闭合，不换到其他地区的方案。
- **活动**：解析结果（方案、basis、地区）随新 netmap 改变时，Server 在同一事务记录一条活动 `routing.autoResolve`（actor 为 `system`，target 为 `routingSelection {nodeId, profile}`，`profile` 为 `auto` 新采用的方案）。手动选择本身记 `routing.select`。

配置警告：

| 代码 | 路径 | 条件 |
| --- | --- | --- |
| `auto_region_unmatched` | `/nodes/<id>/region` | 节点 allowed 含 `auto`、声明了 `region`，但没有规则包含它（`auto` 总是 fallback）；`params.node`、`params.region` |
| `auto_region_missing` | `/routing/egress/assignments/<i>` | assignment 的 default 为 `auto`，其目标中有节点未声明 `region`；`params.nodes`（逗号分隔的节点 ID） |

限制：地区来源只有管理员标签；按底层公网地址的 GeoIP 推断、推断结果的驻留／防抖与按 RTT 选择未实现（第二步设计见[自动方案设计](/v2/routing-auto-design#stability)）。自定义地区名（如 `us-west`）不支持。

#### 选择的入口

持久选择只有一份，归 Server（`routing_selections`，按节点当前身份保存；身份改变后旧选择不再适用）。以下选择与清除入口共用同一提交：

| 入口 | 调用者 | 约束 |
| --- | --- | --- |
| `selectNodeRoutingProfile`（`POST /agent-api/v1/routing-selection`） | 节点自身（`meshway exit`），需当前 instance | `expectedVersion` 为节点 netmap 中自身的选择版本，不一致 `409 version_conflict` |
| `getNodeRoutingSelection`（`GET /api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection`） | 用户 | 成员只能读自己的 User 身份设备，Owner／Admin 可读任意节点；其他为 404 |
| `setNodeRoutingSelection`（`POST` 同一路径，`{requestId, profile}`） | 用户 | 可见范围同上，Tag 设备只有 Owner／Admin 可切；同一 `requestId` 重试返回当前选择，换节点或方案得 `409 request_id_conflict` |
| `clearNodeRoutingProfile`（`DELETE /agent-api/v1/routing-selection`，`{instance, expectedVersion}`） | 节点自身（`meshway exit --default`），需当前 instance | CAS 基准同 `selectNodeRoutingProfile` |
| `clearNodeRoutingSelection`（`DELETE /api/v1/networks/{networkId}/nodes/{nodeId}/routing-selection`，`{requestId}`） | 用户 | 主体与 `requestId` 规则同 `setNodeRoutingSelection`；同一 `requestId` 不能既清除又选择 |

**清除**：持久选择被移除，节点回到 assignment 的默认方案（含 `auto`，随即按地区解析）。请求的方案已不在 allowed（状态为 `requestedUnavailable: notAllowed`，`meshway exit` 显示 `not applied (no longer allowed)`）时，清除是去掉这份残留选择的方式；锁定不拒绝清除（锁定钉住的正是默认方案）。接受的清除同样使版本加一（版本计数保留，之后的选择仍大于所有旧版本），记 `routing.select` 活动（target 的 `profile` 为 null），并重新编译 netmap；新 netmap 不再带本节点的 `selectedProfiles` 条目，`selectionVersions` 条目保留清除所用的版本：它是下一次选择的 CAS 基准，清除前读到的基准（null 或旧版本）此后一律 `version_conflict`，不会经清除状态 A-B-A。没有选中方案时清除不改变选择、版本与 netmap；用户入口的这次请求仍记一条 `routing.select` 活动（`profile` 为 null）作为幂等回执，同一 `requestId` 的重试只返回当前选择（不清除之后的选择），换节点或改为选择则 `request_id_conflict`；Node 入口不记活动（以 CAS 版本防重放）。

方案不在节点的 allowed 内、或 assignment 已锁定而不是默认方案时，得 `422 routing_profile_unavailable`（`params.reason` 为 `notAllowed`／`locked`，`params.allowed` 为允许的方案）。每次接受的切换都使版本加一（含切回同名方案），在同一事务记录活动 `routing.select`（actor 为节点或用户，target 为 `routingSelection {nodeId, profile}`，清除时 `profile` 为 null），并重新编译 netmap；节点经下一次 `sync` 取得。节点库存 `NodeInventoryView.routing` 给出列表摘要 `{requestedProfile, appliedProfile, locked, canSelect}`。

#### 当前选择的状态 {#selection-state}

`RoutingSelectionState` 由 `meshway-policy` 的同一函数求出：Server 用当前配置回答以上接口，节点用自己的 netmap 投影求出 `NetmapStatus.routing`，两者对同一配置相同。

| 字段 | 含义 |
| --- | --- |
| `requestedProfile`／`version` | 节点当前身份下的持久选择与版本；没有选择为 null（下一次节点 CAS 基准为 null） |
| `appliedProfile` | 生效方案：请求的方案仍在 allowed 且未被锁定时为它，否则为 `defaultProfile` |
| `requestedUnavailable` | 请求的方案为何未生效：`notAllowed`、`locked`；生效或无请求为 null |
| `defaultProfile`／`locked` | 合并后的 assignment 默认方案与锁定 |
| `profiles[]` | 允许的方案（assignment 顺序，去重）：`name`、`kind`（`mesh`／`global`／`auto`／`custom`）、`rules`（方案自身规则数，不含公共规则）、`exits`（规则与 fallback 引用的出口节点）、`fallback`（未命中动作；`auto` 为 null） |
| `auto` | 生效方案为 `auto` 时的解析：`profile`（实际采用的方案）、`basis`（`label`／`noRuleMatched`／`noTrustedRegion`）与 `region`（采用的地区代码，`noTrustedRegion` 为 null）；否则为 null。只出现在选择视图，不进入 State |

### direct / unmanaged

两者从源节点本地网络发出，不受 Mesh access 的额外过滤。必须是明确的有效配置／使用选择；出口故障、授权撤回、lease 到期不能当成隐式切换为不接管。

域名授权采用地址集合语义；同 IP 服务隔离限制见 [Access](/v2/configuration-reference/access#domains)。域名/IP 规则混合时须保留正常客户端的域名上下文和 first-match，无法确定相容动作时报告冲突／不可执行，不静默选更宽路径。

要查看某个来源到某个目的地实际由哪条规则决定、走哪条路径，用[路径检测](/v2/configuration-reference/evaluation)。配置与执行载荷见[生成契约](/v2/contracts/openapi.json)，运行范围见 [Node](/v2/node)。
