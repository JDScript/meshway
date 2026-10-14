# 路径检测

`POST /api/v1/networks/{networkId}/configuration/evaluate` 对一个来源节点和一个目的地做规划：按当前版本或草稿，报告哪条规则决定了走向、哪些 access 授权覆盖目的地、规划路径的每一跳及每条边的组网判定，以及没有可用路径时的原因。判定全部来自 Server，使用与执行编译相同的 first-match、方案、access、topology 和路径搜索规则；结果是规划，不是实测。需要 `readConfiguration`。

## Request

```json
{
  "configuration": { "type": "current" },
  "source": "7a0d…",
  "target": { "type": "domain", "name": "web.telegram.org" },
  "profile": "work"
}
```

| 字段 | 说明 |
| --- | --- |
| `configuration` | `current`；或与 validate 相同的草稿输入：`document { expectedRevision, sourceJsonc }`、`operations { expectedRevision, ops }`。草稿按当前版本的变更评估，严格 CAS，走相同的远端规则集准备；不保存、不生成材料 |
| `source` | 来源 NodeId，须存在于被评估的配置 |
| `target` | `node { nodeId }`；`address { address }`（Mesh 地址解析为节点）；`domain { name }`；`internet`（未命中任何规则的公网目的地） |
| `profile` | 可选出口方案，必须在来源允许的方案内；省略时按来源当前选择或默认方案 |

不带协议和端口：access 以允许的 traffic 列表返回，不替用户选一种。

## Response

`EvaluationResponse { requestId, revision, draft, evaluation }`。`revision` 是作为基准的当前版本，草稿时 `draft: true`。`evaluation` 的结构：

| 字段 | 说明 |
| --- | --- |
| `destination` | 解析后的目的：`node`；`address { address, class }`，class 为 `mesh`（池内未分配地址）、`private`、`public`；`domain { name }`（规范化后的名称）；`internet` |
| `profile` | `name`、决定它的 `assignment` pointer（内置 `mesh` 为 null）、`allowed`、`locked`、`requested`（是否由请求指定） |
| `access` | `allowed` 与 `grants[]`：每条带 `rule`（`/access/<i>`）、`traffic`、`reverse`（经 `bidirectional` 反向匹配）、`resolutionDependent`（见下）。没有 grant 即按 `defaults.access` 拒绝 |
| `route` | `kind` 为 `mesh`／`exit`／`direct`／`unmanaged`；`rule` 是决定它的元素 pointer（出口规则、方案 `fallback`、Mesh 路由），隐式决定时为 null（自动 Mesh 选路、`mesh` 方案不接管、`global:` 方案）；`profile` 是实际决定的方案（`auto` 已按源节点地区解析）；`ruleSet { key, entry }` 是命中的规则集条目；`targets`／`via` 是出口候选与必经点，Mesh 目的时 `targets` 为目的节点；`resolutionDependent` 列出因目的地类型而未参与判定的更早规则。整体为 null 表示没有走向可判：池内未分配的 Mesh 地址，或远端规则集还没有内容 |
| `targets[]` | 每个候选的 `problem`：`disabled`、`exitDisabled`、`notEnrolled`（没有可信密钥：未接入或凭据已吊销）、`noPath`（topology、routing 或承载能力下无路径）、`routingCycle`、`routingLimit`；以及该候选的 `paths` 数 |
| `paths[]` | 规划路径，主路径在前；`nodes` 为每一跳，`edges[]` 的 `topology` 是允许该边的 `/topology/<i>`，默认允许时为 null |
| `outcome` | `permitted`；`direct`；`unmanaged`；`unavailable { reason }`，reason 与执行判定相同：`inactiveNode`、`unauthorized`、`noPath`、`routingCycle`、`routingLimit`、`missingDependency` |

## 域名与地址的判定边界 {#resolution}

规划不知道域名会解析到什么地址，也不知道一个地址属于哪个域名，因此：

- 目的是域名时，按名称匹配 `domain`／`domainSuffix` 和规则集中的名称；按 `ipCIDR` 匹配的更早规则不参与判定，列入 `route.resolutionDependent`。目的是地址时相反。
- access 的 `domain:` 后缀授权对域名目的地是确定的；`autogroup:internet` 按地址集合语义同样确定（与[配置校验](/v2/configuration-reference/access#domains)一致，执行时仍按解析地址校验）；CIDR 授权对域名、`domain:` 授权对地址都是 `resolutionDependent`，不计入 `allowed`。
- `internet` 目的地只匹配 `autogroup:internet`，走向只看方案 fallback。

## 限制

- 即使 `access.allowed` 为 false，`targets` 和 `paths` 仍然计算，便于区分“没有授权”与“没有路径”；`outcome` 仍为 `unauthorized`。
- `auto` 方案按源节点声明的 `region` 解析，与执行同一函数（[自动方案](/v2/configuration-reference/routing#auto)）；没有具体源节点的方案级预览（如分割上限警告）没有地区，按 `fallback` 评估。
- 草稿缺少 listener 材料时返回与 validate 相同的准备需求（422），不以占位材料规划。
- 来源有效且 Access 允许、但没有完整路径且搜索发现显式边级承载无交集时，返回配置错误 `invalid_transport`（`params.reason: edge_transport_unavailable`），以 topology 指针和 `params.a`／`b` 定位边；语义见 [Topology](/v2/configuration-reference/topology#承载交集)。存在可行备选路径时不因其他失败边拒绝整个规划。
- 规划不读取 lease、报告或观测；节点是否在线不影响结果，`notEnrolled` 只表示没有可信密钥。
