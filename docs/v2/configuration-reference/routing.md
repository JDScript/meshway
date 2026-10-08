# Routing · 流量路由

定义获准业务如何在允许的图上选择路径。access 管理业务权限，topology 定义允许的图；routing 分为 Mesh 内部和出口两部分。

## 与 topology 的关系

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
- 当前完整路径最多8个节点、每目标最多4条备选路径；一次目标搜索共享16,384工作项预算（递归调用、规则扫描和边候选），队列最多2,048项。达到搜索预算后只可保留已经完整验证的候选；没有候选则返回受限结果。出口候选各自有界。
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
| `auto` | 国家／地区到方案的有序映射及无法判断时的 fallback |

有效规则 = 公共规则 → 当前方案规则 → 方案 fallback。空公共／方案规则不覆盖 fallback。

重叠分配的 allowed 取并集；匹配项的 default 或 locked 不一致就拒绝保存，不按数组顺序覆盖。合并后 default 必须在 allowed 内，auto 的所有目标／fallback 也须在主体允许范围内。

### Rule match

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `domain` | `string[]` | 精确 DNS 名称，只匹配该名称本身；为空时规范输出省略 |
| `domainSuffix` | `string[]` | DNS 标签边界后缀，匹配该名称及其所有子名称 |
| `ipCIDR` | `string[]` | 目的网段 |
| `ruleSets` | `string[]` | [规则集](/v2/configuration-reference/rule-sets) key |

目的条件取并集，至少一项非空；规则从上到下 first-match。域名匹配需要可信域名上下文，不能从普通 IP 包直接推断。名称按 DNS 规范化比较（小写、ASCII／IDNA、忽略结尾点）；`domain: ["example.org"]` 不匹配 `a.example.org`，`domainSuffix: ["example.org"]` 两者都匹配。前面规则已完全覆盖的规则（含被后缀覆盖的精确名称）报告 `shadowed_rule` 警告。

### Action

| 值 | 含义 |
| --- | --- |
| `{ "type": "exit", "nodes": ["node:<id>"], "via": [] }` | 有序主备出口，可指定出口前的有序中转 |
| `{ "type": "direct" }` | 匹配流量从源节点本地网络直接发出 |
| `{ "type": "unmanaged" }` | 不接管 |

出口必须启用 `nodes[id].exitEnabled`，目的访问必须获 access 允许：IP 目的地须在源节点的 CIDR／`autogroup:internet` 授权内；域名目的地（`domain`、`domainSuffix` 与规则集内容）须被 `domain:` 后缀授权或 `autogroup:internet` 覆盖，不满足时报告 `exit_outside_access`（`params.node`、`params.destination`）。域名授权按解析地址集合执行，见 [Access](/v2/configuration-reference/access#domains)。没有可用获准路径时拒绝，不能回退到后续规则或偷偷 direct。

## 出口联合许可

```text
业务目的 access
  ∩ 管理员允许方案中的 routing 动作
  ∩ exitEnabled
  ∩ topology / 完整路径约束
```

Server 为最终出口生成绑定 origin、实际目的范围、出口和完整 path 的许可。公共规则抢先指定 E 的范围，不会又从 F 的 fallback 获得准入。

### 执行捕获与精确许可

`meshway-demand-6` 的 `NodeConfiguration.egressCapture` 是独立的包归属计划，不扩大 `meshAddressPools` 或 Access。`managedIpv4`、`directIpv4`、`unmanagedIpv4` 是按 first-match 切分后的互斥 CIDR；`defaultCapture` 明确表示默认捕获。受管 IPv6 通过 `sinkIpv6` 拒绝，先于外部默认路由，已授权 Mesh IPv6 仍走 Mesh 路由。源端没有 IPv6 地址也不能省略该拒绝边界。

Policy 按顺序从尚未命中的地址集合扣除每条规则的 CIDR，然后应用 fallback。最多输出 4,096 个前缀，单次切分最多 131,072 工作项；超限或缺少依赖发布带 `unavailable` 的全范围捕获／sink，不发布 direct 例外。涉及出口且需要域名上下文的归属在首版返回 `egressConflict`，等待可信解析与冲突处理；不能把 DNS 查询猜成普通 IP 包的业务身份。

`PathPermit` 仍绑定精确源地址、目的 IP、协议／端口、完整路径、到期与原路径回复。外部许可另有 `exit: ExitPermit`：`family`、`generation`、`destination`（`internet` 或显式 `cidr.prefix`）和按出口索引的 `roles`。范围分类只限制精确目的，不授予范围内其他目的。`ExitRolePlan` 绑定 instance、角色 generation、IPv4 与 masquerade；network／node／publication 和执行截止由外围 target／lease 绑定。职责在能力探测前发布，职责本身不能转发包。

出口能力未知、过期或失败时返回 `unavailable/exitUnavailable`，保留捕获边界；路径检测的 `targets[].exitUnavailableReason` 展示新鲜的具体失败。IPv6 出口返回 `unsupportedFamily`。能力事实不能增加 Access 或绕过 topology。

capture／exit permit 的 generation 包含源 revision、有效方案和持久选择版本；切换回同名方案也获得新 generation。`meshway_policy::permit_transition` 仅在旧许可仍活且完整授权摘要一致时返回 `Renewal`，其余为 `Replacement`；Control 仍须检查实例和快照连续性。真实出口安装与包流验收范围见 [Node](/v2/node)。

新方案分配含主体未获授权的目标时拒绝保存。后续身份／access／出口撤销必须可以执行，不能被旧方案引用阻止；受影响执行变为拒绝／不可用，不偷偷恢复 direct 或其他更宽路径。

### 内置方案与当前选择

- `mesh`：公共规则，其他外部流量不接管。
- `global:<exit-node-id>`：为 exitEnabled 节点生成目录项；**目录存在不等于获准使用**，需要 assignment.allowed 明确包含并通过联合校验。
- `auto`：根据当前底层公网 IP 推导地区，在已允许的方案中选择；无有效观测走配置 fallback，不递归指向 auto。

用户选择属于持久的使用状态。重启后仍在 allowed 且未被锁定时恢复；否则采用当前管理员默认。任何方案切换都结束旧出口存量连接，不能携带旧方案的 NAT／流资格跨过去。

### direct / unmanaged

两者从源节点本地网络发出，不受 Mesh access 的额外过滤。必须是明确的有效配置／使用选择；出口故障、授权撤回、lease 到期不能当成隐式切换为不接管。

域名授权采用地址集合语义；同 IP 服务隔离限制见 [Access](/v2/configuration-reference/access#domains)。域名/IP 规则混合时须保留正常客户端的域名上下文和 first-match，无法确定相容动作时报告冲突／不可执行，不静默选更宽路径。

要查看某个来源到某个目的地实际由哪条规则决定、走哪条路径，用[路径检测](/v2/configuration-reference/evaluation)。配置与执行载荷见[生成契约](/v2/contracts/openapi.json)，运行范围见 [Node](/v2/node)。
