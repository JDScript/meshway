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

从上到下 first-match；无匹配时自动选路。所有备选路径都必须保留必经点。已建立会话及回复固定原完整路径，切换承载不等于改逻辑路径。

匹配使用原业务源与最终目标，不会对 `via` 中的节点递归应用其他 `routing.mesh` 规则。例如 `A→C via B` 与 `A→D via C` 不会自动合并为 A→D 必经 B、C；要表达后者，应让 A→D 命中的规则明确指定 `via=[B,C]`。实际路径仍须使用 topology 允许的边。

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
| `domain` | `string[]` | 精确 DNS 名称，只匹配该名称本身 |
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

新方案分配含主体未获授权的目标时拒绝保存。后续身份／access／出口撤销必须可以执行，不能被旧方案引用阻止；受影响执行变为拒绝／不可用，不偷偷恢复 direct 或其他更宽路径。

### 内置方案与当前选择

- `mesh`：公共规则，其他外部流量不接管。
- `global:<exit-node-id>`：为 exitEnabled 节点生成目录项；**目录存在不等于获准使用**，需要 assignment.allowed 明确包含并通过联合校验。
- `auto`：根据当前底层公网 IP 推导地区，在已允许的方案中选择；无有效观测走配置 fallback，不递归指向 auto。

用户选择属于持久的使用状态。重启后仍在 allowed 且未被锁定时恢复；否则采用当前管理员默认。任何方案切换都结束旧出口存量连接，不能携带旧方案的 NAT／流资格跨过去。

### direct / unmanaged

两者从源节点本地网络发出，不受 Mesh access 的额外过滤。必须是明确的有效配置／使用选择；出口故障、授权撤回、lease 到期不能当成隐式切换为不接管。

域名授权采用地址集合语义；同 IP 服务隔离限制见 [Access](/v2/configuration-reference/access#domains)。域名/IP 规则混合时须保留正常客户端的域名上下文和 first-match，无法确定相容动作时报告冲突／不可执行，不静默选更宽路径。

配置与执行载荷见[生成契约](/v2/contracts/openapi.json)，运行范围见 [Node](/v2/node)。
