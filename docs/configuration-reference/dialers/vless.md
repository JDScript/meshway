# VLESS Dialer

**当前能力：TCP/TLS、WebSocket/TLS 已托管。** 以下为下一版字段格式。

## Structure

```json
{
  "type": "vless",
  "remark": "允许 VLESS TCP 和 WS",
  "priority": 300,
  "connectTimeout": "5s",
  "streams": ["tcp", "websocket"],
  "flow": "",
  "tls": { "trust": "managed" },
  "websocket": { "headers": {} }
}
```

## Fields

继承 [Dial Fields](/configuration-reference/shared/dial) 的 TCP 子集。

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `type` | literal | 必填 | `vless` |
| `priority` | integer | `300` | 承载优先级 |
| `streams` | enum 数组 | `["tcp", "websocket"]` | 允许的目标 listener 封装；每次按已选 listener 确定，不做明文降级 |
| `flow` | string | `""` | 当前仅空 flow |
| `tls` | [TLS dialer](/configuration-reference/shared/tls#dialer-structure) | 托管信任 | 目标材料固定 SNI、证书链和指纹 |
| `websocket` | [WS dialer](/configuration-reference/shared/websocket#dialer-structure) | `{}` | WS 模式额外选项；headers 为待接线扩展 |

streams 不可为空或重复；不包含 websocket 时不能指定 websocket 配置。streams 的集合不是实际建连顺序；端点选择与优先级由当前获准候选决定。

## Controller-provided fields

| 上游字段 | 下发来源 |
| --- | --- |
| `server` / `server_port` | 选中的 listener endpoint 或获准协调目标 |
| `uuid` | 目标 listener 的私有材料 |
| TLS `server_name` / CA / pin | 同版本目标材料 |
| transport path / Host | listener 的 WS 参数 |
| CONNECT target | Meshway 固定内部目标，不是应用访问目的地 |

这些字段不在普通 dialer 中重复手填。外层为 TCP CONNECT，但里面可以承载获准的 TCP／UDP／ICMP Mesh 包；不能将上游 `network: udp` 或 `packet_encoding: xudp` 当作本模式。

## 未实现扩展

Vision、原生 UDP command、packetaddr／XUDP、multiplex、REALITY、gRPC／HTTPUpgrade／QUIC 均未接入当前执行。只允许 VLESS 时，不得切到被排除的 Native UDP 业务承载。

[sing-box VLESS outbound](https://sing-box.sagernet.org/configuration/outbound/vless/)
