# Connections

控制哪些节点之间可以建立承载连接，不代表业务访问授权。下一版格式使用 remark，不使用规则 ID。

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
3. 都不匹配时使用 [defaults.connections](/configuration-reference/defaults#connections)。

不存在“后面的 allow 覆盖前面的 block”。多个规则的数组顺序无关；诊断引用本配置版本中的 `/connections/<index>`。

## Behavior

- 许可不代表有 listener、端口可达、协议被双方支持或已经连接。
- 业务仍须 grants，路径仍须遵守必经节点；阻断一条直接边不等于禁止通过其他获准边中转。
- 连接双向复用与业务方向独立，允许 A/B 建连不代表 A/B 都能主动发起业务。
- 实际拨号按需求产生；不把连接规则变成需要管理员维护的运行 Link 实体。
