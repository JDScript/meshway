---
description: NetworkConfiguration、节点状态与版本分发的逐项对齐稿。
---

# 配置与状态模型

直接查看配置格式与字段：[Configuration Reference](/configuration-reference)。

::: info 讨论稿：以用户给出的结构为基线
本页用于逐项对齐 Controller 的模型，不是当前已发布的 JSON Schema 或 HTTP 合同。
**基线**是本轮用户明确给出的方向；**补充建议／待确认**不视为已批准实现。
当前接口和实现仍见[配置分发](/configuration)、[执行合同](/mesh-execution)和[生成契约](/protocol)。
:::

## 1. Config 与 State

**Config 相对静态，类似 ROM；State 随运行频繁变化，类似 RAM。** Controller 管理期望，Node 产生运行事实。

| 内容 | 归属 | 变化来源 | 是否随配置回滚 |
| --- | --- | --- | --- |
| groups、Tag 授予权限、grants | Config | 管理员提交 | 是 |
| 节点 hostname、tags、listeners、dialers | Config | 管理员提交或明确的接入初始化 | 是 |
| 网络 DNS 设置 | Config | 管理员提交 | 是 |
| 当前实例、实际能力、绑定 socket、发现的 endpoint | State | 节点运行与观测 | 否 |
| 连接、选中的承载／路径、配置保存与执行进度 | State | 运行时 | 否 |
| 延迟、失败、最后联系时间 | State／观测历史 | 实际测量或事件 | 否 |
| Node／User 的身份、归属与撤销记录 | 持久实体状态 | 接入和明确的身份管理操作 | 否 |

ROM 是“受控提交、持久且可重建”的类比，不是禁止修改配置。RAM 是“实时且会失效”的类比，不意味着身份撤销、恢复凭据或需要保留的观测历史都必须丢失。**是否属于配置，最终还要看谁负责写入、是否代表期望，以及是否应当随回滚恢复。**

主要业务实体是 **Node 和 User**；Network 沿用已确认的隔离作用域。Tag、listener、dialer 等是配置中的值，不因此获得一套独立业务实体身份。配置子项可以有用于引用、差异更新和诊断的 key，但它不是另一份可独立写入的权威数据。

## 2. NetworkConfiguration 基础结构 {#network-configuration}

保留用户给出的结构和命名：

```jsonc
{
  "groups": {
    "group:dev": ["alice@example.com"]
  },
  "tagsOwners": {
    "tag:backend": ["group:dev", "alice@example.com"]
  },
  "grants": [],
  "nodes": {
    "<node-id>": {
      "hostname": "backend-1",
      "tags": ["tag:backend"],
      "listeners": {},
      "dialers": {}
    }
  },
  "dns": {}
}
```

这是结构示例，不是可以提交给现行 API 的完整配置，也不是新网络的默认授权模板。沿用已确认的 grants 语义，`grants: []` 不授予业务访问；新网络“成员全部允许”的模板应显式生成规则。

示例中的 `<node-id>` 对应用户草稿里的 `[uuid]`，表示稳定 Node 标识。**当前实现使用 64 字符十六进制 Node ID，并非标准 UUID；本轮没有决定重建节点身份或更换 ID 格式。**

### groups

- 用户集合的配置定义，例如 `group:dev`；Node 不需要成为 group 的成员记录，由其用户归属参与选择器匹配。
- 示例以邮箱引用 User。引用必须能解析到当前网络中的用户。
- **待确认**：邮箱变更时如何维护引用，以及是否支持嵌套 group。建议先保持直接用户集合，不引入递归 group。

### tagsOwners

- 定义哪些 Tag 可以使用，以及谁可以给节点赋予它们；值可引用用户或 group。
- **Tag 的定义／授予权限**与 **`nodes[id].tags` 的实际分配**都在配置里，但回答不同问题。
- Tailscale 使用的拼写是 `tagOwners`。这里先保留用户写下的 `tagsOwners`，最终统一选一个字段名，不同时保留两个 alias。
- **待确认**：空 owner 列表的含义、网络 Owner／Admin 的管理权限与委派赋 Tag 权限之间的关系。
- **待确认**：Tag 是叠加在用户归属上的授权标签，还是像 Tailscale 一样成为替代用户的身份。不能仅因为参考 Tailscale Schema 就自动更改设备归属语义。

### grants

- 谁可以访问什么；规则取并集、顺序无关、未匹配拒绝。
- grant 不创建连接，不指定实际选中的承载，也不证明目标可达。
- **补充建议**：以 Tailscale 的 `src`、`dst`、`ip` 为字段讨论起点，明确用户／group／Tag／Node 等选择器，以及协议、端口的表达。
- 当前 Meshway grants 字段并不等于完整 Tailscale grants；本稿尚未定下全部选择器和协议／端口能力。
- 已确认的 Mesh 必经节点和出口 first-match 规则仍需单独表达，不借用 Tailscale 的 `grants.via` 冒充有序多跳。

### nodes

键引用实际存在的 Node，值是该节点的静态期望。公钥、在线状态、观测 endpoint 和 RTT 不放在这里。

| 字段 | 期望语义 | 仍需确定 |
| --- | --- | --- |
| `hostname` | 网络内唯一的规范节点名称，直接生成 MagicDNS 名称 | 合法字符、首次生成与冲突处理、是否自动跟随 OS hostname |
| `tags` | 本节点被赋予的 Tag 集合 | 分配权限与身份语义 |
| `listeners` | 平台管理的入站承载配置，以稳定 key 引用 | bind、公告 endpoint、TLS／WS 参数和材料引用的具体 Schema |
| `dialers` | 平台管理的出站承载配置或约束，以稳定 key 引用 | 是协议配置模板还是指定对端覆盖；默认自动选择和覆盖规则 |

**hostname 的唯一权威在此配置中。** Node 查询可以投影这个名称，但不再维护另一份可独立编辑的 `Node.name`。Agent 报告的 OS hostname 属于 State，可以作为初次命名的输入，不能未经定义就覆盖已保存名称。

```text
nodes[id].hostname + 网络 DNS 后缀 → 节点 FQDN
```

用户已要求可以写备注。**补充建议**：增加 `nodes[id].remark`，用于自由描述，不参与 DNS、选择器或身份判断，并与节点设置一起保存版本；字段位置尚待确认。

关于 `dialers`，建议保留按需自动组网：它表达允许的出站承载、参数和高级约束，**不要求管理员预先为每对节点填写连接记录**。发现的 NAT 地址、临时认证 attempt 和实际通道仍由运行时管理。

**补充建议，空值语义待确认：**

- `listeners: {}` 表示没有配置固定入站 listener；不等于禁止通过真实出站 socket 参与获准协调。
- `dialers: {}` 表示使用平台定义的自动出站默认值，而非关闭所有出站。
- 禁止某种承载或关闭出站应使用明确约束，不能同时让空对象表示“自动”和“禁止”。

**待确认：** `nodes` 是全部已接入节点的静态配置，还是只包含覆盖项？由于 hostname 也在其中，建议接入事务为新 Node 创建默认配置项并保存一次配置版本；节点存在性与身份仍以 Node 实体为准，不能仅添加一个 JSON key 就注册节点。

### dns

- 参考 Tailscale：MagicDNS、全局 nameserver、按域名后缀的 split DNS、search domains 等。
- 自动节点记录由 `nodes[id].hostname` 和已分配的 Mesh 地址生成，不再并行维护另一份手工节点 DNS 名称。
- **待确认**：网络 DNS 后缀、覆盖系统 DNS 的默认行为，以及与出口／域名分流的关系。
- 这里管理业务 DNS。Controller、承载、协调探测所需的 bootstrap DNS 仍遵守系统保护通路，不由业务 DNS 规则任意截断或放行。

## 3. 这份基础结构还缺哪些内容 {#open-items}

下面是既有产品要求需要找到归属的位置，**不代表现在已经决定新增同名顶层字段**。

| 需要表达的内容 | 为什么基础结构还不够 | 对齐方向 |
| --- | --- | --- |
| 连接 allow／block | grants 是业务访问授权，listener／dialer 是承载设置，都不能代替连接约束 | 独立连接规则；block 优先、与规则顺序无关 |
| Mesh 内有序必经路径 | 有 grant 并不说明必须经过哪个节点 | 独立的有序 first-match 路由规则 |
| 出口公共规则、方案、兜底和分配 | 需要域名／IP 分流、指定出口与管理员允许／锁定范围 | 配置中定义方案与授权范围；当前使用选择属于 State |
| 节点出口资格 | 节点必须由管理员开启“可作为出口” | 建议作为 `nodes[id]` 的静态设置，具体字段待定 |
| IP／域名规则集 | 路由需要引用 Controller 管理的数据集 | 定义来源与更新策略；发布时绑定实际数据集版本 |
| 默认承载及覆盖 | 全网默认优先级、逐节点／逐段覆盖不能互相冲突 | 与 listeners／dialers 一起确定默认和覆盖规则 |
| 配置断言 | 希望保存时就能验证“某用户可访问／不可访问某目标” | 可参考 Tailscale `tests`，作为待确认能力 |

默认值、继承和覆盖规则须明确后再定 Schema。特别是路由的 first-match 顺序不能套用 grants 的并集语义。

## 4. Config 的持久化与回滚

**已确认方向：NetworkConfiguration 实际保存到数据库，每次提交保留完整版本，并可回滚到历史配置。**

版本记录与可编辑文档分开；记录至少需要网络作用域、revision、完整配置、原始 JSONC、提交时间和操作者。配置摘要用于一致性核对，不能替代身份认证。

```text
编辑草稿
  → Schema 校验
  → 当前网络／Node／User 引用与语义校验
  → 编译、检查执行预算
  → CAS 检查与事务提交
  → 发布对应节点视图
```

原子提交同时处理受影响的配置和必要的身份／材料绑定，不能依次保存几个独立配置源后再补版本。编译若在事务外执行，提交时仍需核对所用配置与实体输入未发生变化。

回滚选择历史完整文档，作为新的提交生成新 revision，并按当前有效身份与资源条件验证。回滚不会倒退 revision、复活撤销节点、恢复旧通道，或把历史 endpoint／延迟当作当前事实。遇到已不存在或无权使用的引用应给出明确诊断。

只有 URL 的规则集配置不足以重建历史发布，还需保留所用内容版本／摘要等依赖。凭据材料单独加密保存并与发布绑定，普通配置历史不返回私钥。历史保留和材料轮换／回滚的具体规则尚待对齐，不能把“只存最新配置”当作完成回滚能力。

## 5. NodeConfiguration 与增量分发

**已确认方向：**共享 `configuration` crate，供 Controller 编译／切片，也供 Agent 校验和处理更新。具体模块与字段仍是待定设计，不表示该 crate 已经实现。

```text
NetworkConfiguration（持久期望） + Node／User 事实 + 当前必要 State
                              │
                    编译并生成逐节点视图
                              │
                     NodeConfiguration
                              │
                    全量或 node-specific delta
```

NodeConfiguration 是完整的节点执行视图，不是把网络 JSON 删掉几个字段后的原文。它需要包含本节点设置、相关 peers／公钥、编译后的授权／路径、DNS 及本节点所需材料；它不可成为第二份可独立编辑的配置。

**补充建议：**持久配置 revision 与节点视图的更新序号分开。配置 revision 负责管理员历史和回滚；节点上线、endpoint 变化等可以更新节点视图，而不产生一版管理员配置。具体序号、基准和有效期合同尚未定版。

Delta 应基于同一节点的前后完整视图生成，明确增删／替换语义和基准。Agent 在临时副本上还原、校验完整目标后再保存和切换；基准不匹配时获取全量。路由顺序、删除与空值必须准确表达，不将半份补丁直接应用到运行对象。

配置已保存 ACK、执行完成和真实业务连通是不同状态。Controller 的原子提交、单节点的原子保存，也不等于所有节点在同一瞬间切换。

## 6. State 与可观测性还需对齐

State 以当前运行事实为中心，主要在内存中维护并带有效期；需要恢复或历史查询的部分可以单独持久化。读取旧记录不能把它重新标记为新鲜。

建议逐项定义来源、时钟、失效条件与是否保留历史，而不是给 Node 一个涵盖所有情况的 `online / offline / error`：

- 控制面联系状态；
- 当前实例与实际能力；
- 配置保存、执行和 lease；
- 通道与实际路径；
- 邻接／端到端探测及失败原因。

当前[延迟历史](/latency-history)的真实链路已存在，但尚未完成统一观测模型：5 秒连接健康探测与 30 秒历史采样分别发起探测；底层探测超时会退休对应通道；历史耗时包含排队和内部重试。无 listener 的协调通道关联、缺口原因及端到端路径测量仍需处理。

**补充建议：**统一测量来源，让健康判断、当前状态和历史消费真实事件；历史记录者不另外拥有通道生命周期。该方向待进一步确认，不在本文伪报为已实现。

## 7. 下一轮对齐顺序 {#next}

1. **节点静态设置**：hostname／remark 的权威来源、`nodes` 是否完整覆盖已接入节点。
2. **listeners／dialers**：字段、默认值、覆盖规则和空配置的确切含义。
3. **groups／tagsOwners／grants**：引用、赋 Tag 权限、Tag 身份和协议／端口范围。
4. **连接与路由**：在基础结构中加入已确认的四层语义，决定具体字段位置。
5. **DNS**：默认行为、后缀、split DNS 与出口的关系。
6. **版本与 State**：回滚依赖、节点视图增量和统一观测事件。

我们参考 Tailscale 的选择器、policy 和 NetworkMap 思路，而不是复制它所有功能、历史字段或设备侧配置流程。Meshway 的平台托管承载与有序 routing 仍是独立扩展。

## 参考

- [Tailscale policy 语法](https://tailscale.com/kb/1337/policy-syntax)：groups、tagOwners、grants、nodeAttrs 与 tests。
- [Tailscale grants](https://tailscale.com/docs/reference/syntax/grants)：授权并集及协议／端口语义。
- [Tailscale machine names](https://tailscale.com/kb/1098/machine-names) 与 [DNS 设置](https://tailscale.com/docs/reference/dns-in-tailscale)。
- [Tailscale MapResponse 源码](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/tailcfg/tailcfg.go#L1990-L2136)：面向节点的全量视图与语义增量，不作为 Meshway 已发布合同。
- [已确认产品要求](/product-alignment)：四层策略、托管配置及当前产品边界。
