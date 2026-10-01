# 分类、授权与有序路径

**用户已确认的首版规则：域名仅支持 suffix，规则按 first-match 求值；无匹配时可配置 `direct` 或 `reject`，初始为 `reject`。** 这是对先前范围的修订：exact / prefix 当前不在首版要求中。

::: info 配置与路由切片：已实现并验证
纯 Rust 编译 / 预览、生成客户端及 Web 编辑 / 条件保存已通过行为测试和真实服务端到端验收。预览只生成逻辑计划，不授予 ACL 权限，不检查物理可达性，也不发送 trace 探测。证据见[开发路线](/development#配置与路由切片)。
:::

## 策略形状与匹配顺序

`RoutingPolicy` 包含 `default_action` 和有序 `rules`。每条规则包含唯一 `id`、来源选择器、目标范围、目标选择器和动作；Rust wire 类型位于 `crates/protocol/src/routing.rs`，纯编译器位于 `crates/policy`。

| 字段 | 语义 |
| --- | --- |
| `source.node_ids` | 来源节点 ID 列表；列表内 OR，空列表为任意来源节点 |
| `source.countries` | 来源国家列表；列表内 OR，空列表为任意国家，包括未知 |
| 两个来源维度 | AND；国家来自管理员维护的节点清单，预览请求不能自行覆盖来源国家 |
| `destination_scope` | `any`、`mesh` 或 `internet`，与其他条件 AND |
| `destination_any` | `node`、`domain_suffix`、`cidr`、`country` 选择器的 OR 列表；空列表为该 scope 内任意目标 |
| `action.kind` | `route` 或 `reject`；路由动作带有有序 `via` 与可空 `exit_node_id` |

按数组顺序寻找第一条完整匹配的规则，其动作即最终结果。命中拒绝规则就拒绝；命中规则但解析出循环等非法路径时也终止求值，不能跳过该规则继续寻找更宽松的路径。

没有规则匹配时使用 `default_action`：

- `reject`：拒绝，初始策略为 `{ "default_action": "reject", "rules": [] }`。
- `direct`：mesh 目标的逻辑路径为来源 → 目标；互联网目标由来源节点本地出网。

这里的 direct 是逻辑路由意图，并不证明已有直连传输。在纯预览中，未知域名或国家不会猜测补全，对应非空选择器不匹配，仍按其他明确条件与默认动作求值。托管 TUN 对缺少可信域名的真实包采用更保守的执行边界：不能确定一个相关域名规则是否改变结果时拒绝该路径，不能借缺失上下文跳过拒绝或必经节点。

## 域名、CIDR 与国家

### Suffix 的确定语义

域名先进行 IDNA 转换、ASCII 小写化和末尾点规范化，再按 DNS 标签边界匹配：

| 规则 `example.com` | 结果 |
| --- | --- |
| `example.com` | 匹配根域 |
| `a.example.com` | 匹配子域 |
| `A.Example.COM.` | 规范化后匹配 |
| `badexample.com` | 不匹配 |

后缀不是文本前缀、通配符或正则。规则和预览输入中的域名都必须是合法 DNS 名称；URL、字面 IP 不能伪装成域名。域名上下文如何从真实流量获取，仍见待实现的 [DNS 设计](/dns)。

### CIDR 与国家元数据

- CIDR 同时支持 IPv4 / IPv6；编译时将带主机位的输入规范化为网络前缀，例如 `192.0.2.7/24` → `192.0.2.0/24`。
- 国家值为两个 ASCII 字母，规范化为大写；节点 `country: null` 表示未设置，不代表推断到的地区。
- 来源国家和 mesh 目标国家从当前有效节点清单派生。
- 互联网 `destination_country` 是管理员在预览时提供的提示。编译器不做 GeoIP 查询，也不从域名、IP 或 `.cn` 自动推断国家。

域名后缀、国家标签和 IP 地理集合是不同输入，不能互相完整推导。

## `via` 是有序路径，`exit_node_id` 是出网职责

下面的策略意图是：美国来源访问 `example.cn` 及子域，经 LAX 中转、由 SHA 出网。`<LAX-ID>` / `<SHA-ID>` 必须替换为清单中的实际有效节点 ID，不是节点名称；这是策略对象，保存 API 还需要 `expected_revision`。

```json
{
  "default_action": "reject",
  "rules": [
    {
      "id": "us-to-example-cn",
      "source": { "node_ids": [], "countries": ["US"] },
      "destination_scope": "internet",
      "destination_any": [
        { "kind": "domain_suffix", "value": "example.cn" }
      ],
      "action": {
        "kind": "route",
        "via": ["<LAX-ID>"],
        "exit_node_id": "<SHA-ID>"
      }
    }
  ]
}
```

当来源为 NYC 时，计划路径为：

```text
NYC ──→ LAX ──→ SHA ──→ 外部目标
         via     exit
```

- `via` 必须按顺序经过；`[lax, tokyo]` 不是任选一个的候选集合。
- 显式 `exit_node_id` 只允许用于 `internet` scope；mesh 目标本身就是最终节点。
- `mesh` 路由可以有 `via`，但出口必须为 `null`；互联网路径有 `via` 时必须有显式出口。可能覆盖互联网的 `any` scope 不能用无出口的 `via` 规避这一限制。
- `route` 的 `via: []`、`exit_node_id: null` 表示直接 mesh 路径或本地互联网出口；`reject` 也必须使用空 `via` 和 `null` 出口。
- 结构性重复 hop、`via` 与出口重复、非法节点引用在策略编译时拒绝。来源 / 最终目标加入路径后产生的循环，在流预览时拒绝，绝不通过删除必经 hop “修复”。

来源节点 ID、目标节点选择器、`via` 与出口均须引用有效节点。编译成功仅证明类型、引用、规范化及逻辑约束通过；传输能力、ACL 与实际邻接可用性仍是后续数据面职责。

## 流预览与目标身份

`FlowQuery` 含必填 `source_node_id`，以及必须明确给值或 `null` 的 `destination_node_id`、`destination_ip`、`domain`、`destination_country`。节点 / 域名 / IP 至少提供一个目标；仅提供国家不是有效目标。

```json
{
  "source_node_id": "<NYC-ID>",
  "destination_node_id": null,
  "destination_ip": null,
  "domain": "www.example.cn",
  "destination_country": null
}
```

目标解析遵守以下约束：

1. **IPv4-mapped IPv6 是不支持的流输入**：`::ffff:100.96.0.1` 等点分形式及对应十六进制形式，在目标分类和规则求值前返回 `invalid`，不隐式规范化为 IPv4。无论映射地址已分配、池内未分配或属于外部目标，也无论是否同时指定节点，都不能落入默认 direct。
2. 显式 mesh 节点目标必须有效；若同时给 IP，必须是该节点已分配的原生 IPv4 或 IPv6；给国家时也必须与清单一致。
3. 字面 IP 属于有效节点时，解析为 mesh 目标，即使未提交节点 ID。
4. IP 落在 mesh 地址池内但没有有效归属时，返回非法目标；即使默认 `direct`，也不能退化为互联网直连。
5. 只给 mesh 节点目标而未给 IP 时，当前编译器以其 mesh IPv4 作为 CIDR 匹配上下文；可显式给该节点 IPv6 来预览 IPv6 条件。
6. 预览不发 DNS 查询，不验证域名与外部 IP 的真实解析关系，不做 GeoIP 查询。

预览结果区分有效 `route`、有效 `reject` 和 `invalid`；包含匹配规则、逻辑节点路径、出口与原因，非法输入带字段 / 规则诊断。**有意拒绝仍是有效决策**，不能把 `valid: true` 显示成“网络已连通”。

## 编辑、预览与条件保存

管理员读取保存的策略源文档，再提交草稿做纯预览。预览不写数据库、不增加配置 revision。保存带全局 `expected_revision` 做 compare-and-swap：

- 修订一致且策略有效才保存；无效策略返回 `422`，原策略保持不变。
- revision 过期返回 `409`，Web 保留草稿，重新读取最新状态后由管理员处理冲突。
- 注册、撤销或国家元数据变更也可能导致冲突，revision 并非只属于策略编辑器。

Web 的 `/routing` 提供引导式编辑器：suffix / CIDR / 节点 / 国家选择器、有序 hop、默认动作、只读网络前缀、当前草稿基准 revision，以及来源 / 目标预览选择。结果展示匹配规则、逻辑路径和诊断。保存基准只在成功保存或显式“重新读取”后推进；预览响应中的较新 revision 不自动改变 CAS 基准。

撤销可能使已保存策略的节点引用失效。此时保留原始策略供管理员修复，管理读取展示诊断；向节点分发的有效策略整体改为 deny-all（空规则、默认 `reject`），并携带 `policy_valid: false` 和诊断。不能跳过坏规则后继续匹配，尤其不能落入原策略的默认 direct。

编译器当前上限为 128 条规则、每条 8 个 `via`、每条合计 32 个来源 / 目标 selector，以及序列化策略 128 KiB；输入及规范化后大小都受限。配置大小与节点数量边界见[配置分发](/configuration#容量边界)，已通过的边界测试见[验收记录](/development#新切片验收清单)。

## 路由与托管数据面

路由匹配不授予访问权。[端点访问授权](/access-policy) 在 `/access` 管理 node/tag/`*`、默认 allow/reject 和有序规则；Routing 只决定路径。控制面联合编译出内部精确路径授权，供[托管 Mesh IPv4 数据面](/managed-dataplane) 执行。新策略默认拒绝，Routing 的 `direct` 不会自动放行流量。

本切片仅处理已注册节点之间的 IPv4 包。正反方向分别需要路由，授权可以使用一条双向 Access 规则；尚无隐式状态型回程许可。互联网出口、IPv6 和可信 DNS 上下文仍需后续接通。

必经 LAX 失效时，禁止偷偷改成 NYC 直连 SHA。**传输切换**只在相同逻辑邻接内换承载；**路径切换**需要独立、明确的政策。详见[多传输](/transports)。

托管模式在配置实质变更和有效期截止时撤回旧运行代际，旧 E2E 会话不能继续借缓存保有权限。三节点普通 ICMP / HTTP 与故障测试用于验证实际 IP 转发；它们仍不能冒充逐跳测量的 [mesh trace](/security-and-tracing#meshway-trace-必须测到实际转发)。NAT / conntrack、互联网出口和状态型回程策略不在这个切片内。
