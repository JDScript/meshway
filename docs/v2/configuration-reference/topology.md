# Topology · 组网规则

定义图上允许存在的直接邻接及建连约束，不代表业务访问授权或当前已连通。规则使用 remark 描述；实际建立的通道见 [State.links](/v2/state-reference#links)。

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
| `transports` | `string[]` | 省略＝不限制 | 元素为 `TransportProtocol` 的 wire 名：`native_udp`／`native_tcp`／`vless`／`trojan`；`[]` 表示没有允许的承载，不接受 null 或未知值 |

## Matching

1. 任意匹配的 block 拒绝该直接逻辑邻接。
2. 没有 block，且存在匹配的 allow，则允许；取数组中首条匹配 allow 的 `transports`，不合并后续规则。
3. 都不匹配时使用 [defaults.topology](/v2/configuration-reference/defaults#topology)。

不存在“后面的 allow 覆盖前面的 block”。允许／阻断结果与数组顺序无关；多个 allow 的承载约束按首条匹配决定。默认允许的边没有边级承载限制。block 始终阻断整条边，不用 `transports` 表达只阻断某个协议。诊断引用本配置版本中的 `/topology/<index>`。

## Behavior

- 许可不代表有 listener、端口可达、协议被双方支持或已经连接。
- 业务仍须 access，路径仍须遵守 routing；阻断一条直接边不等于禁止通过其他获准边中转。
- 连接双向复用与业务方向独立，允许 A/B 建连不代表 A/B 都能主动发起业务，见[承载与邻接模型](/v2/node/carriers)。
- 拨号由 Server 按业务需求发布的执行配置触发，Node 收到候选即建立邻接、不等首包（见[承载与邻接模型](/v2/node/carriers)）；不把连接规则变成需要管理员维护的运行 Link 实体。

## 与 routing 的关系

topology 决定可选图，routing 为业务在图上的路径增加约束，设备结合当前 State 选择可用的实现。默认允许不意味着预先建立全连接图。

Server 按获准业务需求计算候选完整路径、材料与相关节点依赖，设备只在这份许可内选择；见[按需执行许可](/v2/execution)。不由设备提交任意中转 ID 后自动取得访问／可见资格。

## 承载交集

同一节点可对不同边使用不同协议，无需收窄节点级 dialers。下面是一条完整规则（具体网络中可将 `*` 换为 Node／Group／Tag 选择器）：

<!-- schema: TopologyRule -->
```json
{
  "remark": "这条无方向边只允许 VLESS",
  "action": "allow",
  "a": "*",
  "b": "*",
  "transports": ["vless"]
}
```

对每条实际边 A–B 分别计算两个拨号方向：

```text
A 拨 B = 首条匹配 allow 的 transports（省略则全集）
         ∩ A 启用的 dialers（省略则 defaults.dialers）
         ∩ B 启用的 listeners 协议
B 拨 A = 同一边级集合 ∩ B 的 dialers ∩ A 的 listeners 协议
```

VLESS stream、TLS 版本、运行能力与材料还须相容。只需一个拨号方向有可用候选，认证邻接便可双向承载数据；`transports` 不决定业务方向或拨号优先级。`*`、Group、Tag 选择器展开后的每对节点独立求交集，不以其他节点的能力补齐该边。

两个方向都没有候选的边不会进入发布路径。递归 routing 的必经点不能删除，也不回退到后续 routing 规则；自动补入的中继仍须满足所有分段 routing 和边级交集。

validate／apply 对配置声明可证明的 routing 不可执行返回 `invalid_transport`，`params.reason` 为 `edge_transport_unavailable`，`path` 为 `/topology/<index>/transports`，`params.a`／`b` 为实际节点 ID，`params.routingRule` 定位 Mesh 路由规则。保留规则因依赖撤回失效时报告不可用依赖，允许撤销事务完成；运行库存／能力变化由按需编译返回 `unavailable/noPath`，不影响其他有效需求。已验证可行的自动中继路径不会因某条未使用的直接边无交集被拒绝。

Node 执行配置只收到交集内的 `dialers` 候选及路径 `dialCandidates`。节点级 listener 可以继续服务其他边，但不能用来绕过本边的许可；Manager 按已发布候选建连，不把节点级 dialers 或已编译 provider 当作兜底。
