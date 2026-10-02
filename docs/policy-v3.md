# 四层策略引擎 v3

本页描述 `meshway_policy::v3` 的纯 Rust 实现，按 2026-10-01 的最终确认：连接、授权是整张网络共享、顺序无关的配置；Mesh 内路由与按**路由方案**组织的出口路由都使用有序 first-match。前三层仍为所有设备共享的一套策略。产品源模型为 `NetworkPolicy`，入口为 `compile_network`；接收同一网络、同一 revision 的节点、管理员 tag、listener、出口开关、可达性事实与已解析规则集。不执行 DNS、拨号、NAT、加密、下载或网络 I/O。Controller 与 Agent 仍使用 v2，编译成功不表示隧道或出口已生效。底层 `Policy` / `compile` 保留为纯选路与迁移验证内核，不是产品方案配置入口。

## 四层语义

### 连接

`connections` 的 `a` / `b` 使用 `node:<完整 ID>`、`tag:<名称>` 或 `*`，规则没有业务方向。允许取并集，任意匹配的 `block` 优先于所有 `allow`。新网络模板显式包含 `allow * ↔ *`；删除它后未匹配的边不连接。

有效无向连接图还要与调用方的物理事实取交集：至少存在一个可拨通的方向，且被拨入节点有 listener。`Dial { from, to }` 是方向事实；即使业务从 B 发往 A，也可以由 A 主动连接 B。编译只输出授权主备路径所需要的连接，不创建全连接。

### 授权

`grants` 是只允许、未匹配拒绝的并集，不使用具体度覆盖或规则顺序。每条规则的 `src` / `dst` 列表内部 OR，两端 AND；`bidirectional` 显式授权反方向发起业务，不代表状态型回包。

`*` 只匹配本网络节点，不包含离开 Mesh 的流量。`autogroup:internet` 只授权公网目的地址，不包含出口 LAN。按 Tailscale / Headscale 的 `TheInternet` 基线，IPv4 集合从 `0.0.0.0/0` 排除 RFC1918（`10/8`、`172.16/12`、`192.168/16`）、CGNAT（`100.64/10`，包含 Mesh IPv4 网段）和链路本地（`169.254/16`）；IPv6 使用 global-unicast `2000::/3`，不含 ULA（`fc00::/7`，包含 Mesh IPv6 网段）和链路本地（`fe80::/10`）。参考 `references/headscale/hscontrol/util/net.go:39-63`。

访问私网目的地须有覆盖实际地址的具体 `ip:` grant，例如 `ip:192.168.1.0/24`；即使有可信域名或 `domain:openai.com` grant，也不能把解析到私网的地址当成公网授权。公网上下文中的域名 / IP grants 仍取并集；域名 grant 不授权裸 IP，IP grant 只按实际目的地址匹配。只有 internet grant、没有解析 IP 的域名请求无法证明公网范围，返回 Denied；显式域名 grant 可先匹配域名上下文，但实际包准入仍须带解析 IP 重新校验。路由规则本身不授予访问。外部 grant 不能双向；统一出口仍覆盖公网与 LAN，不另设 subnet router。

`user:` / `group:` 为后续身份语法保留，当前返回带字段路径的“不支持”诊断。v2 没有端口 / 协议维度，因此 v3 没有删除已有维度，也没有伪造尚不存在的包过滤能力。

### Mesh 内路由

`mesh` 的 `src`、`dst` 各为一个 selector，`via` 是有序 node / tag selector。未匹配时自动选路。路由存在不会创建 grant；未授权流不进入选路输出。

规则按源文档数组顺序从上到下检查，第一条同时命中 src / dst 的规则生效。不比较 node / tag / wildcard 具体度，不按 ID 排序，不禁止重叠或不同 via 的规则。第一条命中的必经约束不可达时，报告该流不可达，不能回退到后续规则或自动路径来绕过必经节点。

编译输出 `base.mesh_lints`，按当前网络库存展开 node / tag，对不同节点间的有向源 → 目的流统计（不以 grants 或当前可达性过滤潜在匹配）：

- **完全遮挡**：规则所有当前可能命中的流都被前序规则拿走，`fully_shadowed = true`。
- **部分抢先**：只有部分流被前序规则拿走，给出至少一个具体示例；`example.src / dst` 指向稳定 nodes 表，附 winning_rule_id / winning_rule、winning_via 与 shadowed_via，供 Web 表达“laptop → prod 命中 rule-1（via hk），未命中 rule-2（via bastion）”。

每条 lint 保留原始规则 ID / 数组位置、shadowed_flows / total_flows 与 `inventory_dependent = true`。结果只说明**当前库存**：tag 成员改变须重新编译；当前没有匹配成员的规则不报告为永久无效，不用 hypothetical 节点推断冲突。Lint 不使保存失败，也不授予访问权限。

### 出口路由与默认选择

`public_rules` 和方案 `rules` 的 destinations 为 DNS suffix、IPv4 / IPv6 CIDR 或规则集引用的 OR 列表；target 为 nodes 或 direct。nodes 的 candidates 为有序 node / tag 出口候选，via 是到出口之前的必经约束；direct 要求空 candidates / via，表示不接管，不需要 Mesh 出口授权。

- 域名先规范化为小写 IDNA，无通配符和 URL 语法；suffix 匹配完整 DNS 标签边界。策略保存使用规范值。
- IP 段使用规范网络地址。规则从上到下，先检查 src，再检查该条任意域名 / IP 条件，第一条命中即生效；src 具体度和最长匹配不覆盖规则顺序。
- 域名和 IP 规则混排时，使用同一“可信域名 + 解析 IP”上下文逐条求值，与 sing-box 的逐条匹配方式一致。不存在额外的“域名一定先于 IP”优先级；无域名上下文时只评估 IP 条件。
- 重叠不再保存报错，输出完全遮蔽、部分 matcher 遮蔽统计 lint。只有在当前有效源节点的所有匹配上下文都被前序规则覆盖时才报告遮蔽；不把没有当前成员的 tag 断言为永久无效。
- 命中的 nodes 规则没有可用路径时报告 Unreachable，不回退到 final 或本地；未被外部 grant 覆盖则 Denied。命中 direct 始终 Local。

设备有效出口规则为 **public_rules + 当前方案 rules + 方案 fallback**。公共规则始终排最前，连“仅 Mesh”也不能绕过；设备用户只在管理员允许集合内切换方案，不维护设备配置。

内置方案 ID：

- `mesh-only`：只有公共规则，final 不接管。
- `global:<完整节点 ID>`：为已开启出口且该源 grants 覆盖全部公网目的地的节点生成，final 为该节点。`global:*` 只用于分配列表，展开为该源可用的全部全局方案，不是路由 matcher。实际私网目的地仍须另有具体 IP grant，选择出口不会扩大授权。
- `auto`：由 automatic.regions（两位大写国家 / 地区代码 → 其它方案 ID）与 fallback 选择具体方案。调用方传入 `StunLocation`，来自 STUN 观测到的设备公网 IP 的国家 / 地区；绝不使用 controller 连接来源 IP。未知 / 无定位走 automatic.fallback；自动不能引用自动，拒绝循环。

自定义 profiles 包含 id、rules 与 fallback。fallback.kind 为 nodes（candidates / via）、direct、none。`assignments` 用 src selector 分配 allowed、default_profile、locked；源等级 node > tag > *，较具体分配替换较宽分配；同级可能重叠但 allowed / default / locked 不同保存报错，顺序无关。auto 引用的每个方案也须在该源 allowed 中显式允许。无匹配分配时的引擎模板为 mesh-only、可用 global:*、auto，默认 mesh-only；自定义方案不会自动赋给成员。

锁定时忽略用户请求，使用分配的 default_profile；否则只允许集合内的选择，未给请求时用 default_profile。公共规则与方案规则始终先于 final。所有可选方案预编译，切换不需要新 revision、不改变前三层；调用方应用切换时须中断旧出口存量连接，本纯引擎不执行运行时代际撤回。

**分配时检查完整授权覆盖**：nodes 规则的每个目的范围须被源的外部 grants 联合完整覆盖，允许两个 /25 合并覆盖 /24；只覆盖一部分仍报错。internet 编译为排除私网后的共享前缀，因此包含私网的 CIDR 不能靠 internet 授权通过分配检查，须有联合覆盖的具体 IP grants。域名须有覆盖 suffix 的 grant，或 internet 授权（运行时只允许解析到公网）；不凭 DNS 猜测将某个 IP grant 扩大为域名授权。nodes final / global 需要全部公网授权（internet 或联合覆盖公网前缀集合），运行时仍按实际目的地址检查；direct / none 不需要出口 grant。即使规则被前序遮蔽，也检查其声明目标，不以遮蔽绕过方案分配授权。

规则 `/0` 是有序 catch-all，可能遮蔽其后规则，不移到末尾；nodes `/0` 声明了私网目标，单凭 internet 不满足完整覆盖。方案 fallback.nodes 就是“全局经 X”的兜底，按实际地址做 grant fence；优先使用 fallback 表达兜底，避免与末尾 `/0` 重复。规则 /0 可覆盖单个家族；domain-only 具体 suffix 可匹配，使用 nodes final 没有 IP 时明确报上下文错误。

出口节点必须具有调用方提供的 `exit_enabled = true` 管理员属性。出口 node selector 指向未开启节点时报字段错误；tag 只展开已开启节点，展开为空时报“候选不可用”，整个策略不编译。此开关不影响中转，任何满足连接事实的节点仍可承担路径中转。

### 规则集与设备等价表

`{ "kind": "rule_set", "value": "cn" }` 引用 Snapshot.rule_sets 中调用方提供的 ParsedRuleSet。Controller 负责 inline / local / remote、下载更新、sing-box source / binary 或 mihomo domain / ipcidr / classical 的解析。纯引擎只接收类型化条目：suffix / CIDR 可规范展开；ExactDomain、Keyword、Regex、Unsupported 条目逐条报告，整个展开返回 Err，不近似转换或跳过。空 / 缺失规则集也报告不可用。

IP 使用共享 bit trie，域名使用反向 DNS 标签 trie。每个前缀节点记录有序规则号，匹配选择最小规则号。针对不同源的适用规则集合，IP trie 递归携带前序 winner，分割后继覆盖区间，再合并相同 winner 的相邻兄弟；输出互不重叠的 IPv4 / IPv6 前缀表。等价源集合共享同一表，不为 1000 个设备各复制 8000 条 CIDR。**设备原始 IP 上下文**可按此表安装；有可信域名上下文的流量还必须使用保留顺序的域名 + IP matcher，不能只看该 IP 表。

同一 IP 由多个域名共享时，L3 包本身没有域名身份。[Tailscale app connector](https://tailscale.com/kb/1281/app-connectors) 明确说明：发现并通告某个域名的 IP 后，其他域名甚至裸 IP 到同地址也会走 connector。Meshway 需要域名范围授权，不能据此静默扩大 grant。本引擎要求调用方提供可信、逐流域名绑定；没有绑定就使用纯 IP 规则 / grants。若调用方只能提供存在冲突的“IP → 多域名”映射，不应任选一个域名，须报告上下文无法证明并拒绝依赖域名的出口 / 授权；DNS/Fake-IP/逐流绑定由后续数据面集成实现，本切片没有伪造这些能力。

## 完整 JSONC 示例

示例 ID 取自已存在的 live 路径；tag 与出口属性需要由本网络管理员配置。`Policy` 自身反序列化严格 JSON，JSONC 注释由编辑器 / controller 的解析层处理，不能通过宽泛 JSON 值绕过 schema。

```jsonc
{
  "connections": [
    { "id": "allow-physical", "a": "*", "b": "*", "action": "allow" },
    { "id": "avoid-direct", "a": "node:ffb3c08f662759e7983e943aeaf33b0bd3c27f34c7a22cbf5d7c8a3f2e41adfe", "b": "node:403d8c0cde9cfd8818bd3dafea3a4601d0539e80ad9488702b0aa6754b2013f8", "action": "block" }
  ],
  "grants": [
    { "id": "members", "src": ["*"], "dst": ["*"], "bidirectional": false },
    { "id": "internet", "src": ["*"], "dst": ["autogroup:internet"], "bidirectional": false }
  ],
  "mesh": [
    {
      "id": "mandatory-hop",
      "src": "node:ffb3c08f662759e7983e943aeaf33b0bd3c27f34c7a22cbf5d7c8a3f2e41adfe",
      "dst": "node:403d8c0cde9cfd8818bd3dafea3a4601d0539e80ad9488702b0aa6754b2013f8",
      "via": ["node:fafdea6da7d1f12de735632175c40a97012bd4abe36a167718aa5f92583a2426"]
    }
  ],
  "public_rules": [
    { "id": "local-range", "src": "*", "destinations": [{ "kind": "cidr", "value": "192.168.0.0/16" }], "target": "direct", "candidates": [], "via": [] },
    { "id": "example", "src": "*", "destinations": [{ "kind": "domain_suffix", "value": "example.com" }], "target": "nodes", "candidates": ["tag:exit"], "via": [] }
  ],
  "profiles": [
    { "id": "regional", "rules": [{ "id": "regional-set", "src": "*", "destinations": [{ "kind": "rule_set", "value": "cn" }], "target": "nodes", "candidates": ["tag:exit"], "via": [] }], "fallback": { "kind": "none" } }
  ],
  "automatic": { "regions": { "CN": "regional" }, "fallback": "mesh-only" },
  "assignments": [
    { "src": "*", "allowed": ["mesh-only", "global:*", "regional", "auto"], "default_profile": "auto", "locked": false },
    { "src": "tag:managed", "allowed": ["mesh-only", "regional"], "default_profile": "regional", "locked": true }
  ]
}
```

所有源策略结构和带字段枚举都使用 serde(deny_unknown_fields)，派生 utoipa::ToSchema。`ProfileFallback` 的 `none` / `direct` 使用空 struct variants，确保反序列化实际拒绝未知字段（internally-tagged unit variants 仅有该注解仍会静默丢字段）。合法 JSON 仍为 `{"kind":"none"}` / `{"kind":"direct"}`；残留 candidates / via 或拼错字段在独立 fallback、嵌套 profile 和完整 NetworkPolicy 输入中均报错。其它 tagged 边界为 adjacently-tagged `ExitMatch`、`RuleSetEntry`（均有必需 value）与 `CompiledFinal`（none/direct unit、nodes paths），负例覆盖额外字段和错误 content；未发现同类静默丢字段。`Compiled`、`NetworkCompilation`、`ProfileMatcher`、`DestinationIndex` 及内部 trie 节点不实现 `Deserialize`：输出只用于可信进程内计算与诊断序列化，不能把解码成功当作执行计划验证成功。独立导出以 NetworkPolicy 为根的 JSON Schema（不向现有 Controller OpenAPI 注册）：

```sh
cargo run -p meshway-policy --example policy_v3_schema > target/policy-v3.schema.json
```

Schema 表达字段与类型；fallback 的每个字段均必需，并以 `maxProperties` 限制为实际字段数（none/direct 为 1，nodes 为 3），弥补当前 utoipa 未把 enum 的 deny_unknown_fields 传播到 variant schema 的行为，与实际 decoder 的封闭性一致。`compile_network` 同时验证公共规则、方案、分配、完整授权与动态能力。底层单表调用 v3::validate。诊断字段如 profiles[0].rules[0].candidates[1]、assignments[0].allowed 可映射到 JSONC AST；规则集展开的错误仍指向原始引用字段，消息注明规则集条目位置，不错误使用展开或排序后的数组下标。规则 ID 在公共 + 同一方案范围内唯一，前三层仍按共享策略的 ID 约束。

## 确定性选路与输出

1. 有效节点按完整 ID 排序成为稳定索引，邻接按该索引排序。
2. 最少边数优先，因而无约束时直连优先；等长路径按完整节点索引序列字典序选择。
3. 必经 tag 在整体路径搜索中选择节点，不先固定一个可能不可达的 tag 成员。完整路径没有重复节点，源 / 目的不能充当中间 mandatory hop。
4. 无约束使用 BFS，直连与共同邻居有快速路径；有约束用保留 visited 历史的 A*，启发式为忽略重复节点约束的有序 walk 距离。不能只用 `(node, via 阶段)` 剪枝，那会错误丢掉可行简单路径。
5. 首版返回一个主路径和至多一个逻辑备选（`CompileOptions.paths = 1..2`，默认 2）。备选通过逐条排除主路径边后再次求最短、全局择优得到；桥边不必重新搜索。备选仍满足全部 mandatory hops，绝不回退到更宽 routing 规则。它不是 transport 主备或同流聚合。
6. 出口候选 selector 顺序是主备顺序，不被路径长度覆盖；同一 tag 内按路径长度与节点序列排序。global 经 X 的方案只在 X 的路径间切换；自定义方案规则 / final 的多出口按管理员候选顺序切换。

`NetworkCompilation` 包括同一份 base（前三层与完整路径池）、共享 matchers / ip_tables 与每源 sources。sources 中包含允许 ID、默认方案、锁定标记、自动位置映射，以及每个具体方案的公共已合并规则、前缀表索引、域名 matcher 索引、final 与 lint。global 方案共享公共 matcher；相同源适用规则集合共享前缀表；相同 `(源, 出口, via)` 在全部方案中共用一套路径。base 的 Compiled 内容：

| 字段 | 内容 |
| --- | --- |
| `nodes` | 稳定完整身份表，路径中使用 u16 索引避免反复复制 64 字节 ID |
| `paths` / `path_scopes` / `mesh` | 完整方向路径池，显式区分 Mesh / External，以及已授权 Mesh 对的主备索引；不可达对保留空 paths 与失败记录 |
| `connections` | 所需段的合法 `(dial from, dial to)` 集合；可同时有两个方向，不授予反向业务 |
| `per_node` | 路径参与者的 path 索引、previous、next；入口 / 出口字段为 null |
| `exits` / `exit_rules` / `exit_index` | 每源有序出口表、共享 matcher 与目标、是否具有外部 grant 的候选规划标记；authorized 不是实际流授权结论 |
| `external_grants` / `grant_index` | 每源外部范围授权与共享范围索引，运行时按真实目的上下文判定 |
| `ip_tables` / `source_ip_table` | 共享等价、互不重叠的纯 IP 前缀表，以及各源指向的表索引 |
| `failures` | 不可达、路径长度或搜索预算失败，不伪造运行状态 |
| `lints` | 完全遮蔽与规则集展开后部分 matcher 遮蔽统计；不使策略失效 |
| `mesh_lints` | 按当前网络库存统计的 Mesh 完全遮挡 / 部分抢先、原始规则位置与具体流量示例；不使策略失效 |

所有允许方案的目标在选择前生成源与出口侧完整路径投影；撤回 grants 或关闭出口需新快照重新编译。`exact_authorization(node)` **只投影 Mesh 路径**，去重后使用现有 MeshPathAuthorization 结构；`external_path_authorization(node)` 单独投影 External 路径，必须联合外部目的 grants 与 route-table 上下文安装，不能授权访问出口节点自身的 Mesh 地址。两者不添加反向、子路径或跳过中继的授权。它们不直接安装到 v2 Agent：v2 routing 交集、128 grants / 64 identities 发布上限、多备选 admission、真实出口过滤 / NAT 仍需后续集成。

## 规模、复杂度与验证

上限：1024 节点、前三层各 128 规则、公共规则与每方案规则各 128、自定义方案 64、分配 128；grant 源 32 selectors、单表 / grant 合计范围 16,384，公共 + 单方案最多 16,384，全部自定义方案展开范围总计 65,536；出口 32 候选、6 mandatory hops、完整路径 8 节点、源策略 2 MiB、最多 4,200,000 输出路径（预算覆盖千节点 Mesh 与全部 global 方案共存）。单次约束搜索最多 100,000 弹出 / 排队状态；预算耗尽明确报告，不装绕过路径；输出预算耗尽整体 Err，不发布部分结果。

输入范围预算与实际安装预算分别检查。`MAX_INSTALLED_PREFIXES = 16,384` 限制每源**任一可选具体方案**的 IPv4 + IPv6 展平表，direct 条目同样计数；nodes final 再计两个家族的默认路由（底层 `Policy.defaults` 按所声明家族计数）。direct / none final 不安装 Mesh 出口默认路由，不增加该计数。测试中的 1024 个孤立 `/32` + 后置 `/0`（合法 1025 输入）产生 17,416 个互不重叠前缀，现整体 Err。展平 emit 在越界时停止，不返回部分表。方案计数在每个 source/profile 引用处执行，**缓存命中也检查该方案自己的 final**：相同共享表能被 mesh-only 使用，不代表增加 final 的 global 仍在安装预算内。不同设备、互斥方案不会被误当成一张同时安装的表；这不替代后续出口侧多源 / 多路径 grants 总量和线协议字节预算。

设 N 为节点数、E 为边、R 为规则数、H 为路径上限、Q 为候选查询数、D 为范围条目数、U 为不同源适用规则集合数、S 为搜索预算。构图、Mesh 匹配与库存 lint 为 O(N²R)，lint 仅保留每规则计数和一个示例，不存储 N²R 个匹配记录。前缀索引 O(D×128)，每个源等价表分割 O(D×128)，共享表共 O(U×D×128)，lookup 只遍历最多 32 / 128 IP bits 与域名标签，再检查命中规则号，最多 R 个规则候选；没有线性扫描 8000 条 CIDR。出口 lint 逐 matcher 检查去重后的前序源上下文，支持多个前缀联合覆盖，最坏成本额外取决于 U 与被访问的索引子树。

无约束路径查询一般 O(N+E)，直接路径 O(log N)，共同邻居 O(degree×log N)；桥识别 O(N+E)。每个 primary 最多 H−1 次备选搜索；约束启发式 O(H(N+E)log N)，简单路径搜索最坏指数，以 S 限制，预算内约 O(S×degree×H + S log S)。总路径输出 O(QH)，存储 O(N² + 索引 + 共享表 + 路径节点 + 转发投影)。不是声称所有任意千节点策略都能在线性时间内编译。

下表保留此前具体度匹配版本的历史计时基线（release、macOS Apple Silicon、1000 节点、所有成员授权；不计构造输入，包含全部编译输出）；尚未对当前 first-match / 库存 lint / 公网授权版本重测。计时测试无需 Docker，不设脆弱的 CI 时间断言：

```sh
cargo test -p meshway-policy --release timing -- --ignored --nocapture
```

| 场景 | 编译时间 | 输出 |
| --- | --- | --- |
| 1000 节点全可达、allow-all、主路径 | 471.86 ms | 999,000 paths |
| 同上，主路径 + 一条备选 | 675.46 ms | 1,998,000 paths |
| 1000 节点星型图，主备搜索 | 195.88 ms | 999,000 paths；桥边没有备选 |
| 底层单表：1000 节点、规则集 8000 CIDR + 前置 direct | 20.18 ms | 7969 个互不重叠前缀，1 份共享表 |
| 路由方案：1000 源、8000 CIDR、公用 direct + 自定义 regional + global | 26.28 ms | 2 份共享表，1000 外部路径，允许方案共用路径池 |

8000 CIDR 场景的 100,000 次出口 lookup 合计 **8.73 ms**（约 87 ns / 次；含 grant fence 与返回决策，不含构造地址）。该次 release 计时测试可并行运行两个场景，数字是本机一次测量，不是实时控制面 SLA。

路由方案 API 的 100,000 次 lookup 合计 **9.93 ms**（含方案选择与 grant fence，约 99 ns / 次）；对应测试为 profile_eight_thousand_cidr_timing。方案和前缀展平 property tests 同时验证三层共享结果、公共优先、按 STUN 地区自动选择、未知兜底、锁定、授权分配失败与所有候选出口侧精确路径覆盖。

行为测试包含四节点全部 64 种无向图的简单路径穷举 oracle；80 组随机 IPv4 / IPv6 策略、每源 200 地址，验证不重叠表 == 顺序求值；另覆盖千节点 mandatory、连接 / grants 规则置换、Mesh 与出口顺序变化、库存 lint 与 tag 成员变更、mixed domain / IP、公网与私网 grant 边界、direct、默认选择、锁定、出口开关、规则集报告与迁移。

## v2 → v3 迁移

`migrate_access` / `migrate_routing` 返回 `Migration { converted: Option<_>, diagnostics }`。有任何不能证明等价的部分时 `converted = None`，不会返回可安装的部分迁移。转换以 selector 未来成员语义为准，不靠当前库存展开来掩盖 country、tag 或 reject 差异。

- live Access `{"default_action":"allow","rules":[]}` → 一条 `* → *` grant，**不会附赠互联网授权**。
- default reject 下的纯 allow，或与后续 allow 不重叠的 reject → allow grants；顺序中 reject 挖出的例外不能变成 additive grants。
- live 的两个相反方向 node → node、via 同一个完整中继 ID、default direct → 两条 Mesh mandatory 规则；行为测试比较两方向完整路径。
- Mesh node / wildcard、无出口动作 → 保留 v2 原顺序无损转换，允许不同动作重叠；同一原始规则的 selector 展开连续排列，不按生成 ID 排序。
- Internet 明确 domain / CIDR → nodes / direct，保留原规则顺序，允许原有重叠；真实使用仍需独立外部 grant 与出口开启事实。
- country、Mesh 域名 / CIDR predicate、any scope、routing reject / default reject 均保留 v2 并报告。Internet wildcard 也匹配无 IP 的 domain-only 上下文，不能静默转换成只看 IP 的两个 `/0`。

典型报告：

```text
rules[0].action: Reject exception office-deny cannot be represented by additive grants; retain v2 until rewritten
rules[1].source.countries: Country selectors unsupported; node expansion is not equivalent for future members
profiles[0].rules[0].candidates[0]: Referenced node is not enabled as an exit by the administrator
```

这是保守的可证明子集转换器：Mesh 重叠无需 predicate subtraction，直接保留 first-match 原顺序；Access reject 例外等不可表达部分仍明确报告需要重写，不声称无损。输出后的引用有效性仍由当前网络 `validate` 检查。原 v2 Access 的节点成员 grants 不会被迁移成 internet 或出口 LAN 授权。

## 后续集成边界

本切片没有新增 controller route、节点本地配置或设备自报开关。类型暂留 policy crate；正式 HTTP 契约接入时迁入 protocol 并同步生成 OpenAPI、运行时 schema、Agent/Web 客户端及 drift check。接入层负责 revision / network fencing、管理员出口开关、用户运行时选择、可信 DNS 上下文、真实出口转发 / NAT、状态型回包与数据面生效证据。

### P3a 发布边界修复与 Controller handoff

本轮是接线前的 policy 库修复，**不表示 P3a 已上线**。最小接线顺序：

1. **源存储与 CAS**：在网络作用域持久化类型化 `NetworkPolicy` 与原始 JSONC 草稿，沿用 `apps/controller/src/store/configuration.rs` 的 `configuration.revision` / `compare_revision` / `bump` 事务边界及 `store/access.rs` 的作用域；新增存储迁移由 Controller 主线处理。保存必须检查网络内管理员身份。源策略、JSONC、库存与发布 revision 一致，冲突返回 CAS 错误；失败不写入部分编译结果、不覆盖未保存草稿。
2. **迁移**：同事务读取原 v2 Access / Routing，调用 `migrate_access` / `migrate_routing`；任一 `converted = None` 时保留原 v2 源与运行版本，返回字段诊断，不标记迁移成功。可证明转换也须经当前库存完整编译，并通过发布预算后才原子切换。尚未定义的外部授权不自动补授；缺少出口属性或授权时保留 v2 并报告。v2 消费者退出前保留既有兼容入口。
3. **同 revision 输入与权限**：一次一致性读取提供本网络有效身份 / 地址 / 公钥、管理员 tag / listener / exit_enabled、连接约束与有来源且未过期的可达事实、已解析规则集版本、账户 / 成员权限及方案分配。`Snapshot` 自身没有 network/revision 字段，调用方须绑定外围类型化作用域与版本，不能把独立请求结果混拼。库存、grants 或出口开关变动重编译；STUN 位置与用户方案选择不能扩张授权。
4. **发布 / 执行预算**：`compile_network` 成功后逐节点计算真实安装投影，除本库逐方案展平 + final 预算外，还校验所有入向源 / 路径的出口 grants 前缀**总和**、完整授权路径 / 身份数、允许方案目录及配置字节上限，不能用共享表的存储去重数代替某节点实际安装总数。既有 v2 128 grants / 64 identities 与出口原型 64 grants / 16 concrete profiles 的限制仍须协商或明确报超限；原型只支持 IPv4，不能把 v3 IPv6 悄悄丢弃后报告完整生效。失败保留最后可用运行代际，并明确区分保存、ACK 和 apply 状态。
5. **执行契约与 underlay**：可信进程内重编译源策略取得 `Compiled` / `NetworkCompilation`；目前没有已验证 compiled wire 解码入口，公开字段仅用于检查，不构成不可信输入的受控构造 API。正式 wire 必须使用类型化契约和受控执行计划构造，校验索引 / 排序 / 唯一身份、完整方向路径 / 授权 / mandatory hops、revision / lease 与安装预算，再生成全部消费者；不能恢复无条件 `Deserialize`。准备系统保护 socket 通路后才启用 TUN / 新承载，撤回与 lease 到期独立终止旧运行代际；该部分由 underlay / Controller / Agent 主线实施。

#### 方案 ID 的兼容边界

保留既有源 ID：自定义 ID 为 1..64 字节 ASCII 字母数字及 `._-`；内置 `mesh-only` / `auto`；具体 global 为 `global:` + **完整小写 64-hex 身份**，长度 71。`v3::valid_profile_id` 是共用的具体 ID 语法检查；自定义定义另外拒绝保留 ID。`global:*` 只允许出现在分配 allowed 中作展开 token，不能进入执行目录；不能截断、哈希改名或把所有自定义 ID 长度泛化到 71。

真实出口消费者为 `apps/agent/src/managed_dataplane/exit.rs` 的 `ExitPlan::with_profiles`。现已把原 `p.id.is_empty() || p.id.len() > 64` 替换为 `!meshway_policy::v3::valid_profile_id(&p.id)`，保留 `p.id == "auto"`（auto 不在 concrete profiles 中）与重复 ID 检查；default / automatic / 用户选择继续对完整 ID 做允许目录成员校验。`exit/profile_tests.rs` 覆盖 71 字节 global 经默认选择、用户切换和自动解析保留完整身份，以及残缺 / 大写 / 非 hex / 重复身份、`global:*`、concrete `auto` 的拒绝。目录 16 项限制及 Controller 投影仍是独立的未接线限制。

**无损边界**：v3 已接受的自定义源 ID 仍原样保留，不截断、不重命名。只读调查现有出口原型构造点（`exit/fixture.rs`、`exit.rs` 单测、`exit/profile_tests.rs`）和 `experiments/exit-dataplane` 使用 `e` / `f` / `mesh` / `global`，均兼容；测试另覆盖含大小写、`._-` 的自定义 ID 和 64 字节边界。旧原型只检查非空及 64 字节，理论上曾接受空格、斜线、冒号、非 ASCII 等不符合 v3 源语法的 ID；本次会明确拒绝这些值，不能声称任意原型输入均无损。原型目录为内部测试构造，未接 Controller 持久化方案；未发现上述字符用于已知配置。未来迁移遇到不合规外部旧值须报告并保留旧源，不能静默改名。

本 worktree 的检查命令（独立 Cargo target，限制两项构建并发）：

```sh
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo fmt -p meshway-policy -- --check
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo test -p meshway-policy --all-features --locked
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo clippy -p meshway-policy --all-features --all-targets --locked -- -D warnings
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo fmt -p meshway-agent -- --check
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo test -p meshway-agent --features managed-connections --lib managed_dataplane::exit --locked
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo check -p meshway-agent --features managed-connections --all-targets --locked
CARGO_TARGET_DIR="$PWD/target/policy-v3-publication" CARGO_BUILD_JOBS=2 cargo clippy -p meshway-agent --features managed-connections --all-targets --locked -- -D warnings
```

Mesh / 出口有序 first-match、公网 internet 范围、库存相关遮挡提醒、路由方案、公共优先、规则集、direct、STUN 定位、用户切换、管理员开启与锁定均已确认，不再列为待决。方案分配仍使用 node > tag > * 的具体度（不是 Mesh 路由匹配）；其 template / auto 允许集合语义及资源预算沿用既有实现。
