# 连接与路由

字段结构草案，语义沿用已确认的四层模型：连接与 grants 是许可；Mesh／出口路由决定路径。

## connections

```json
[
  { "id": "default", "action": "allow", "a": "*", "b": "*" },
  { "id": "isolate", "action": "block", "a": "tag:dev", "b": "tag:prod" }
]
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `string` | 唯一规则 key |
| `action` | `"allow" \| "block"` | block 优先，规则顺序无关 |
| `a` / `b` | 节点选择器 | 无方向的逻辑邻接许可；不代表业务授权 |

新网络显式生成默认 allow；`connections: []` 没有连接许可。listener 决定能否固定监听，实际可达性来自运行时。允许的边也只按业务需求建立。

## routing.mesh

```json
[
  {
    "id": "backend-via-core",
    "src": ["group:dev"],
    "dst": ["tag:backend"],
    "via": ["node:<relay-id>"]
  }
]
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `string` | 唯一规则 key |
| `src` / `dst` | `string[]` | 源与目的节点选择器 |
| `via` | `string[]` | 必须按顺序经过的节点；本稿先用具体 Node 选择器 |

从上到下 first-match；无匹配时自动选路。所有备选路径都必须保留必经点。已建立会话及回复固定原完整路径，切换承载不等于改逻辑路径。

## routing.egress

```json
{
  "rules": [],
  "profiles": {
    "work": {
      "rules": [
        {
          "id": "office",
          "match": { "ipCIDR": ["10.20.0.0/16"] },
          "action": { "type": "exit", "nodes": ["node:<exit-id>"], "via": [] }
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

有效规则 = 公共规则 → 当前方案规则 → 方案 fallback。空公共／方案规则不覆盖 fallback。重叠分配若有不同 default 或 locked 值，建议保存时报冲突；不凭对象顺序选一个。

### Rule match

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `domainSuffix` | `string[]` | DNS 标签边界后缀 |
| `ipCIDR` | `string[]` | 目的网段 |
| `ruleSets` | `string[]` | [规则集](/configuration-reference/rule-sets) key |

建议这些目的条件取并集，至少一项非空；规则从上到下 first-match。域名匹配需要可信域名上下文，不能从普通 IP 包直接推断。

### Action

| 值 | 含义 |
| --- | --- |
| `{ "type": "exit", "nodes": ["node:<id>"], "via": [] }` | 有序主备出口，可指定出口前的有序中转 |
| `{ "type": "direct" }` | 匹配流量从源节点本地网络直接发出 |
| `{ "type": "unmanaged" }` | 不接管；建议只用于方案 fallback |

出口必须启用 `nodes[id].exitEnabled`，目的访问必须获 grants 允许。没有可用获准路径时拒绝，不能回退到后续规则或偷偷 direct。

内置方案为 `mesh`（仅公共规则）、`global:<exit-node-id>`（全局经允许出口）和 `auto`。用户当前选择及 GeoIP 观测属于 State；选择必须在配置允许范围内，切换中断旧出口连接。auto 只使用当前底层公网 IP 推导的地区；无有效观测走 fallback，目标方案不允许递归引用 auto。

以上字段名、匹配组合和分配冲突规则均待确认，不代表出口主线已交付完整方案能力。
