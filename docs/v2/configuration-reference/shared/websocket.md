# WebSocket

VLESS WebSocket 参数。WS 封装可靠字节流，其外层仍为 TCP；不因承载 UDP 业务包就变成 UDP socket。

## Listener structure

<!-- schema: ListenerWebSocket -->
```json
{
  "path": "/meshway",
  "host": "edge.example.com",
  "responseHeaders": {}
}
```

## Dialer structure

<!-- schema: DialerWebSocket -->
```json
{
  "headers": {}
}
```

## Fields

| 字段 | 位置 | 默认 | 说明 |
| --- | --- | --- | --- |
| `path` | listener | `"/meshway"` | 精确路径，以 `/` 开头；无 query／fragment／空格／控制字符 |
| `host` | listener | TLS serverName | 精确 Host，不等于 TLS SNI 或外部 IP |
| `responseHeaders` | listener | `{}` | 服务端额外响应 header |
| `headers` | dialer | `{}` | 客户端额外请求 header |

dialer 的 path／Host 从所选 listener 下发，不在两端手工维护两份必须同步的配置。headers 不能覆盖 `Host`、`Connection`、`Upgrade`、`Sec-WebSocket-*` 等握手字段，以及 `Content-Length`／`Transfer-Encoding` 消息定界字段；这些名称按 ASCII 大小写不敏感检查。header 名称必须是合法 HTTP token，重复名称（包括不同大小写）拒绝；值允许水平制表符，拒绝其他控制字符和换行。不要通过普通配置 header 存储秘密。

header 值随配置受秘密保护；普通配置视图只返回安全摘要。支持范围由[承载目录](/v2/configuration-reference/transports)与节点 capability 共同约束。

## Early data

sing-box 的 `max_early_data` 和 `early_data_header_name` 分别定义早发 payload 大小及编码位置；Xray 常用 `Sec-WebSocket-Protocol`。这些字段不在 Meshway 配置中，不允许将 Node 握手数据放进 URL／header；未知 early-data 参数不能作为普通 WS 接受。

HTTPUpgrade、gRPC、HTTP/2、QUIC 是其他封装，不是本节 WebSocket 的别名。

[上游 WebSocket 字段](https://sing-box.sagernet.org/configuration/shared/v2ray-transport/#websocket)
