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
  "uuid": "11111111-1111-4111-8111-111111111111",
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
| `uuid` | UUID 字符串 | 平台生成并保存 | 管理员可明确填写；关联拨号端引用同一值 |
| `flow` | string | `""` | 当前只接受空 flow／空 addons |
| `tls` | [TLS listener](/configuration-reference/shared/tls#listener-structure) | 必填 | 平台托管 TLS |
| `websocket` | [WS listener](/configuration-reference/shared/websocket#listener-structure) | 由 WS 默认值生成 | 仅 stream=websocket 时允许 |

TCP/TLS 示例只需将 stream 改为 tcp 并移除 websocket。禁用 listener 使用 enabled=false，不以关闭 TLS 代替。

## 编译到节点的参数

下表是**完整 NodeConfiguration 中的协议参数**。它们不是另一类 State；管理员填写与平台生成只是两种取值来源。

| VLESS 字段 | 来源与约束 |
| --- | --- |
| UUID | 取本 listener 的显式配置或已保存生成值；进入本节点及获准拨号节点的配置 |
| flow／stream／TLS／WS | 取本 listener 配置，生成匹配的两端执行参数 |
| target | 固定的内部 Meshway 目标，精确验证；不由监听端任意转发到请求目标 |

协议实现固定使用 VLESS v0 TCP CONNECT；版本字节、command 和 response 是 wire 格式，不是另外一套用户配置。VLESS 响应存在不代表 Node 已认证。

普通代理 `users[]` 不是 Meshway 用户模型。UUID 只负责外层协议材料匹配，Node 公钥、连接许可和业务授权仍分别校验。

## sing-box 对照与扩展 {#extensions}

| 上游字段 | Meshway 对应 |
| --- | --- |
| `users[].uuid` | 本 listener 的 uuid；可编辑，默认由平台生成 |
| `users[].name` | 不创建独立代理用户；显示说明使用 remark |
| `users[].flow = xtls-rprx-vision` | 尚未实现；不能作为空 flow 接受 |
| `tls` | [TLS](/configuration-reference/shared/tls) |
| `transport` | 本版 stream + websocket；其他封装尚未实现 |
| `multiplex` | 尚未实现，不复用为多路径聚合 |

REALITY、Vision、gRPC、HTTPUpgrade、QUIC、XUDP 等需要独立协议实现和能力协商，当前不列为可执行组合。

[sing-box VLESS inbound](https://sing-box.sagernet.org/configuration/inbound/vless/)
