# WebSocket

VLESS WebSocket 的目标参数。WS 封装可靠字节流，其外层仍为 TCP；不因承载 UDP 业务包就变成 UDP socket。

## Listener structure

```json
{
  "path": "/meshway",
  "host": "edge.example.com",
  "responseHeaders": {}
}
```

## Dialer structure

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
| `responseHeaders` | listener | `{}` | 目标扩展：服务端额外响应 header |
| `headers` | dialer | `{}` | 目标扩展：客户端额外请求 header |

dialer 的 path／Host 从所选 listener 下发，不在两端手工维护两份必须同步的配置。headers 不能覆盖 `Host`、`Connection`、`Upgrade`、`Sec-WebSocket-*` 等握手字段，以及 `Content-Length`／`Transfer-Encoding` 消息定界字段；这些名称按 ASCII 大小写不敏感检查。header 名称必须是合法 HTTP token，重复名称（包括不同大小写）拒绝；值允许水平制表符，拒绝其他控制字符和换行。不要通过普通配置 header 存储秘密。

目前平台已托管 path／Host；自定义 header 尚未托管。只有承载目录明确支持时才接受扩展字段。

## Early data

sing-box 的 `max_early_data` 和 `early_data_header_name` 分别定义早发 payload 大小及编码位置；Xray 常用 `Sec-WebSocket-Protocol`。Meshway 尚未实现该模式，不允许将 Node 握手数据偷塞进 URL／header；不把未知 early-data 参数静默当作普通 WS。

HTTPUpgrade、gRPC、HTTP/2、QUIC 是其他封装，不是本节 WebSocket 的别名。

[上游 WebSocket 字段](https://sing-box.sagernet.org/configuration/shared/v2ray-transport/#websocket)
