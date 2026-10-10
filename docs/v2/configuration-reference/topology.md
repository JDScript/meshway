# Topology · 组网规则

定义图上允许存在的直接邻接及其承载顺序，不代表业务访问授权或当前已连通。listeners／dialers 声明节点能力，不承载策略。规则使用 remark 描述；实际建立的通道见 [State.links](/v2/state-reference#links)。

## Structure

以下为 topology 字段的数组值，引用的 Tag／Node 须在本网络有效：

```json
[
  {
    "remark": "隔离开发与生产的直接承载",
    "action": "block",
    "a": "tag:dev",
    "b": "tag:prod"
  },
  {
    "remark": "允许开发节点连接中转节点",
    "action": "allow",
    "a": "tag:dev",
    "b": "node:<relay-id>",
    "transports": ["native_tcp", "vless"]
  }
]
```

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `remark` | `string` | `""` | 描述，不要求唯一，不接受 null |
| `action` | `"allow" \| "block"` | 必填 | 承载许可或阻断 |
| `a` / `b` | 节点选择器 | 必填 | 无方向的节点对，支持 Node／User／Group／Tag／`*` |
| `transports` | `string[]` | 省略＝不限制，采用平台顺序 | 有序的允许承载列表，越靠前越优先；元素为 `TransportProtocol` 的 wire 名：`native_udp`／`native_tcp`／`vless`／`trojan`；`[]` 表示没有允许的承载，不接受 null、未知值或重复协议 |
| `keepalive` | `boolean` | `false` | allow 规则匹配的边常驻：无业务也保持已认证邻接，不因空闲关闭；block 规则忽略。见[邻接的建立与常驻](#邻接的建立与常驻) |

重复协议在 validate／apply 时返回 IssueCode `invalid_transport`，`params.reason` 为 `repeated_transport`，`path` 为 `/topology/<i>/transports/<j>`，其中 `i` 为规则下标，`j` 为重复出现位置的下标。

## Matching

1. 任意匹配的 block 拒绝该直接逻辑邻接。
2. 没有 block，且存在匹配的 allow，则允许；取数组中首条匹配 allow 的 `transports` 作为承载集合与顺序，不合并后续规则。
3. 都不匹配时使用 [defaults.topology](/v2/configuration-reference/defaults#topology)。

不存在“后面的 allow 覆盖前面的 block”。允许／阻断结果与数组顺序无关；多个 allow 的承载约束按首条匹配决定。默认允许的边没有边级承载限制。block 始终阻断整条边，不用 `transports` 表达只阻断某个协议。诊断引用本配置版本中的 `/topology/<index>`。

## Behavior

- 许可不代表有 listener、端口可达、协议被双方支持或已经连接。
- 业务仍须 access，路径仍须遵守 routing；阻断一条直接边不等于禁止通过其他获准边中转。
- 连接双向复用与业务方向独立，允许 A/B 建连不代表 A/B 都能主动发起业务，见[承载与邻接模型](/v2/node/carriers)。
- 邻接按下节规则按需建立或常驻；不把连接规则变成需要管理员维护的运行 Link 实体。

### 邻接的建立与常驻

Node 按本机 netmap 判断每条获准边，两端对同一投影得到相同结论：

| 边 | 行为 |
| --- | --- |
| 双方都能拨出对方（各有获准承载） | **按需**：本机首包选路、中继收到经过本节点的握手或诊断需要下一跳时建立；无 interest 且无帧 120 秒后关闭 |
| 只有一方能拨出（另一方仅监听，或其 dialers 与对方 listeners 无获准承载） | **常驻**：能拨出的一方不等本机需求就建立并保持，仅监听的一方不因空闲关闭，并作为协调 UDP 穿透的目标参与 |
| 匹配 allow 规则的 `keepalive: true` | **常驻**，与上一行相同 |

只有一方能拨出的边是按需建立的唯一隐式例外：仅监听的节点没有通道向对端表达需求，若等对端本机产生需求，它作为中继或目标时永远等不到承载。常驻只决定是否持有通道，不授予业务、路径或出口权限；承载顺序、候选与认证规则不变。

## 与 routing 的关系

topology 决定哪些直接边存在及其承载顺序，routing 决定业务经过的路径与出口，access 决定业务权限。直接边隔离用 topology block／默认 deny，必经节点用 routing `via`，同一意图不在两处重复表达。设备结合当前 State 选择可用的实现。默认允许不意味着预先建立全连接图。

Server 按获准业务需求计算候选完整路径、材料与相关节点依赖，设备只在这份许可内选择；见[按需执行许可](/v2/execution)。不由设备提交任意中转 ID 后自动取得访问／可见资格。

## 承载交集

同一节点可对不同边使用不同协议，无需收窄节点级 dialers。下面是一条完整规则（具体网络中可将 `*` 换为 Node／Group／Tag 选择器）：

<!-- schema: TopologyRule -->
```json
{
  "remark": "这条无方向边优先 VLESS，其次 Native TCP",
  "action": "allow",
  "a": "*",
  "b": "*",
  "transports": ["vless", "native_tcp"]
}
```

此列表只允许 VLESS 与 Native TCP，并在相同端点来源内优先尝试 VLESS。

对每条实际边 A–B 分别计算两个拨号方向：

```text
A 拨 B = 首条匹配 allow 的 transports（省略则全集）
         ∩ A 启用的 dialers（省略则 defaults.dialers）
         ∩ B 启用的 listeners 协议
B 拨 A = 同一边级集合 ∩ B 的 dialers ∩ A 的 listeners 协议
```

VLESS stream、TLS 版本、运行能力与材料还须相容。只需一个拨号方向有可用候选，认证邻接便可双向承载数据；`transports` 的承载顺序不决定业务方向。`*`、Group、Tag 选择器展开后的每对节点独立求交集，不以其他节点的能力补齐该边。

承载顺序取首条匹配 allow 的 `transports` 列表顺序；该字段省略或边由默认允许时，使用固定的平台顺序：`native_udp` → `native_tcp` → `vless` → `trojan`。交集筛选不改变顺序。Server 将候选协议在该边顺序中的位置编译为 `DialCandidate.priority`，数值越小越优先。

对同一 peer，Manager 先尝试所有 `configured` 端点，再尝试 `local` 端点；端点来源的优先级跨承载生效。同一来源内按 `DialCandidate.priority`、listener key 稳定排序，即：configured 端点（边级承载顺序 → listener key）→ local 端点（边级承载顺序 → listener key）。这不是权重负载均衡。

两个方向都没有候选的边不会进入发布路径。递归 routing 的必经点不能删除，也不回退到后续 routing 规则；自动补入的中继仍须满足所有分段 routing 和边级交集。

validate／apply 对配置声明可证明的 routing 不可执行返回 `invalid_transport`，`params.reason` 为 `edge_transport_unavailable`，`path` 为 `/topology/<index>/transports`，`params.a`／`b` 为实际节点 ID，`params.routingRule` 定位 Mesh 路由规则。保留规则因依赖撤回失效时报告不可用依赖，允许撤销事务完成；运行库存／能力变化由按需编译返回 `unavailable/noPath`，不影响其他有效需求。已验证可行的自动中继路径不会因某条未使用的直接边无交集被拒绝。

Node 执行配置只收到交集内的 `dialers` 候选及路径 `dialCandidates`。节点级 listener 可以继续服务其他边，但不能用来绕过本边的许可；Manager 按已发布候选建连，不把节点级 dialers 或已编译 provider 当作兜底。
