# Topology · 组网规则

定义图上允许存在的直接邻接及建连约束，不代表业务访问授权或当前已连通。下一版格式使用 remark，不使用规则 ID；实际建立的通道见 [State.links](/state-reference#links)。

## Structure

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
    "b": "node:<relay-id>"
  }
]
```

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `remark` | `string` | `""` | 描述，不要求唯一，不接受 null |
| `action` | `"allow" \| "block"` | 必填 | 承载许可或阻断 |
| `a` / `b` | 节点选择器 | 必填 | 无方向的节点对，支持 Node／User／Group／Tag／`*` |

## Matching

1. 任意匹配的 block 拒绝该直接逻辑邻接。
2. 没有 block，且存在匹配的 allow，则允许。
3. 都不匹配时使用 [defaults.topology](/configuration-reference/defaults#topology)。

不存在“后面的 allow 覆盖前面的 block”。多个规则的数组顺序无关；诊断引用本配置版本中的 `/topology/<index>`。

## Behavior

- 许可不代表有 listener、端口可达、协议被双方支持或已经连接。
- 业务仍须 access，路径仍须遵守 routing；阻断一条直接边不等于禁止通过其他获准边中转。
- 连接双向复用与业务方向独立，允许 A/B 建连不代表 A/B 都能主动发起业务。
- 实际拨号按需求产生；不把连接规则变成需要管理员维护的运行 Link 实体。

## 与 routing 的关系

topology 决定可选图，routing 为业务在图上的路径增加约束，设备结合当前 State 选择可用的实现。默认允许不意味着预先建立全连接图。

固定某段承载、固定拨号方向与无业务时仍维持邻接是不同要求；这些高级字段继续单独对齐，不仅凭新增 topology 名称推定其已实现。
