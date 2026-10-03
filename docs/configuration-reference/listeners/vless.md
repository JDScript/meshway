# VLESS Listener

**当前能力：TCP/TLS 与 WebSocket/TLS 已托管。** 新字段布局尚未发布；高级项按[支持状态](/configuration-reference/transports)处理。

## Structure

```json
{
  "type": "vless",
  "remark": "VLESS over WebSocket",
  "listen": "0.0.0.0",
  "listenPort": 443,
  "endpoints": [{ "host": "edge.example.com", "port": 8443 }],
  "stream": "websocket",
  "flow": "",
  "tls": { "mode": "managed", "serverName": "edge.example.com" },
  "websocket": { "path": "/meshway", "host": "edge.example.com" }
}
```

## Fields

继承 [Listen Fields](/configuration-reference/shared/listen) 的 TCP 部分。

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `type` | literal | 必填 | `vless` |
| `stream` | `"tcp" \| "websocket"` | `"tcp"` | 外层封装，两者都使用 TCP socket |
| `flow` | string | `""` | 当前只接受空 flow／空 addons |
| `tls` | [TLS listener](/configuration-reference/shared/tls#listener-structure) | 必填 | 平台托管 TLS |
| `websocket` | [WS listener](/configuration-reference/shared/websocket#listener-structure) | 由 WS 默认值生成 | 仅 stream=websocket 时允许 |

TCP/TLS 示例只需将 stream 改为 tcp 并移除 websocket。禁用 listener 使用 enabled=false，不以关闭 TLS 代替。

## Protocol fields

| VLESS 字段 | 来源与约束 |
| --- | --- |
| version | 固定 VLESS v0 |
| UUID | 平台为 listener 生成并按授权下发，不写入 NetworkConfiguration |
| command | TCP CONNECT；后续承载 Meshway 包流 |
| target | 固定的内部 Meshway 目标，精确验证；不由监听端任意转发到请求目标 |
| response | VLESS 响应与后续 Noise 顺序由实现处理；响应存在不代表 Node 已认证 |

普通代理 `users[]` 不是 Meshway 用户模型。UUID 只负责外层协议材料匹配，Node 公钥、连接许可和业务授权仍分别校验。

## sing-box 对照与扩展

| 上游字段 | Meshway 对应 |
| --- | --- |
| `users[].uuid` | 平台私有材料 |
| `users[].name` | 不创建独立代理用户；显示说明使用 remark |
| `users[].flow = xtls-rprx-vision` | 尚未实现；不能作为空 flow 接受 |
| `tls` | [TLS](/configuration-reference/shared/tls) |
| `transport` | 本版 stream + websocket；其他封装尚未实现 |
| `multiplex` | 尚未实现，不复用为多路径聚合 |

REALITY、Vision、gRPC、HTTPUpgrade、QUIC、XUDP 等需要独立协议实现和能力协商，当前不列为可执行组合。

[sing-box VLESS inbound](https://sing-box.sagernet.org/configuration/inbound/vless/)
