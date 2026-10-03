# Dialers

出站承载配置草案。位于 `nodes[id].dialers`；它约束如何建连，不要求手工列出每一个 peer。

## Structure

```json
{
  "udp": { "type": "native_udp", "priority": 100 },
  "tcp": { "type": "native_tcp", "priority": 200 },
  "vless": {
    "type": "vless",
    "priority": 300,
    "streams": ["tcp", "websocket"]
  }
}
```

## Fields

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `type` | `string` enum | 必填 | `native_udp`、`native_tcp`、`vless` |
| `enabled` | `boolean` | `true` | 是否允许使用此项 |
| `priority` | `integer` | 必填 | 0–65535，较小值优先 |
| `streams` | `string[]` | `["tcp", "websocket"]` | 仅 VLESS，可用流封装；仍取决于目标 listener 和两端实际能力 |

建议每节点每种 type 至多一项；同优先级时按 key 稳定排序。它是主备优先级，不是负载均衡权重。

## Defaults

建议语义：

- `dialers: {}` 使用平台默认：Native UDP 100、Native TCP 200、VLESS 300。
- 非空对象替换平台默认允许集合；仅写 VLESS 就不能偷偷使用 Native UDP 业务承载。
- 所有条目 `enabled: false` 表示禁用主动业务拨号。

这些是待确认默认值。默认集合也必须与 Agent 实际编译能力、目标 listener、网络连接约束取交集。

## 平台提供的内容

目标 Node、可用 listener／endpoint、身份公钥、VLESS 凭据、TLS 信任与协调参数由 Controller 提供。拨号器不持有另一套手填 peer 清单，也不允许关闭身份／证书校验。

STUN／协调用途与业务承载分别控制，不将允许一次发现探测解释为允许 UDP 业务流量。

用户已确认逐段约束覆盖需求；本稿先定义节点级允许集合和优先级，**逐段覆盖的字段与冲突规则需下一轮单独确认**。
