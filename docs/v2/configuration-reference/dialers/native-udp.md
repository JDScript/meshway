# Native UDP Dialer

**当前能力：平台托管。** 以下选项放在 `nodes[id].dialers.native_udp`，为下一版字段格式。

## Structure

```json
{
  "remark": "优先 UDP",
  "priority": 100,
  "connectTimeout": "5s"
}
```

## Fields

继承 [Dial Fields](/v2/configuration-reference/shared/dial) 的 UDP 子集。priority 缺省为 100；不接受 tcp／tls／websocket 字段。

目标参数的配置解析见 [NodeConfiguration](/v2/configuration-reference/node-configuration)。NAT 协调必须使用同一实际 socket 的观测和认证结果；STUN 映射与已认证连接是不同状态。

业务授权、必经节点与逻辑路径不会因 UDP 候选更快而改变。
