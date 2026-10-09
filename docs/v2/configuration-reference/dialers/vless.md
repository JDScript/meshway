# VLESS Dialer

位于 `nodes[id].dialers.vless`，用于选择 TCP/TLS 或 WebSocket/TLS 入口。

## Structure

通常只需：

<!-- schema: VlessDialer -->
```json
{}
```

需要限制封装或调整客户端参数时：

<!-- schema: VlessDialer -->
```json
{
  "remark": "仅使用 WS 入口",
  "stream": "websocket",
  "connectTimeout": "5s",
  "tls": { "trust": "managed" },
  "websocket": { "headers": {} }
}
```

## Fields

复用 [Dial Fields](/v2/configuration-reference/shared/dial) 的 TCP 子集，仅补充：

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `stream` | `"auto" \| "tcp" \| "websocket"` | `"auto"` | auto 使用目标 listener 的封装；显式值筛选兼容入口 |
| `tls` | [TLS dialer](/v2/configuration-reference/shared/tls#dialer-structure) | 托管信任 | 客户端验证选项 |
| `websocket` | [WS dialer](/v2/configuration-reference/shared/websocket#dialer-structure) | 省略 | WS 客户端选项，包含额外请求 headers |

省略 stream 不需要再列一份 `streams` 数组。指定 tcp 时不允许 websocket 配置；没有兼容入口时等待／报告不可用，不降低 TLS 或启用不允许的业务承载。

## 节点实际收到的字段

完整出站配置包含 endpoint、UUID、flow、TLS SNI／信任材料及 WS path／Host，来自管理员配置的目标 listener 或已持久化的生成值。

管理员在 listener 上修改 UUID／WS 等参数，Server 重新生成关联的拨号配置。数据来源见 [NodeConfiguration](/v2/configuration-reference/node-configuration#example)。

协议字段及限制见 [VLESS Listener](/v2/configuration-reference/listeners/vless#extensions)。

[sing-box VLESS outbound](https://sing-box.sagernet.org/configuration/outbound/vless/)
