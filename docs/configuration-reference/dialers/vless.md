# VLESS Dialer

位于 `nodes[id].dialers.vless`。**当前承载支持 TCP/TLS、WebSocket/TLS；下述新字段布局尚未发布。**

## Structure

通常只需：

```json
{ "priority": 300 }
```

需要限制封装或调整客户端参数时：

```json
{
  "remark": "仅使用 WS 入口",
  "priority": 300,
  "stream": "websocket",
  "connectTimeout": "5s",
  "tls": { "trust": "managed" },
  "websocket": { "headers": {} }
}
```

## Fields

复用 [Dial Fields](/configuration-reference/shared/dial) 的 TCP 子集，仅补充：

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `priority` | integer | `300` | 协议默认优先级 |
| `stream` | `"auto" \| "tcp" \| "websocket"` | `"auto"` | auto 使用目标 listener 的封装；显式值筛选兼容入口 |
| `tls` | [TLS dialer](/configuration-reference/shared/tls#dialer-structure) | 托管信任 | 客户端验证选项 |
| `websocket` | [WS dialer](/configuration-reference/shared/websocket#dialer-structure) | `{}` | WS 客户端选项；自定义 headers 尚未托管 |

省略 stream 不需要再列一份 `streams` 数组。指定 tcp 时不允许 websocket 配置；没有兼容入口时等待／报告不可用，不降低 TLS 或启用不允许的业务承载。

## 节点实际收到的字段

完整出站配置仍包含 endpoint、UUID、flow、TLS SNI／信任材料及 WS path／Host。它们来自管理员配置的目标 listener 或已持久化的生成值，**不是不属于 configuration**。

为避免双端重复填写，管理员在 listener 上修改 UUID／WS 等参数，Controller 重新生成关联的拨号配置。数据来源和完整例子只在 [NodeConfiguration](/configuration-reference/node-configuration#example) 定义。

协议能力及未实现扩展统一见 [VLESS Listener](/configuration-reference/listeners/vless#extensions)。

[sing-box VLESS outbound](https://sing-box.sagernet.org/configuration/outbound/vless/)
