# Listeners

入站承载配置草案。位于 `nodes[id].listeners`，key 是该节点内稳定的 listener 名称。

## Structure

```json
{
  "mesh-udp": {
    "type": "native_udp",
    "enabled": true,
    "listen": "0.0.0.0",
    "listenPort": 8421,
    "endpoints": []
  },
  "mesh-ws": {
    "type": "vless",
    "listen": "0.0.0.0",
    "listenPort": 443,
    "endpoints": [{ "host": "edge.example.com", "port": 443 }],
    "stream": "websocket",
    "tls": { "serverName": "edge.example.com" },
    "websocket": { "path": "/meshway", "host": "edge.example.com" }
  }
}
```

## Shared fields

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `type` | `string` enum | 必填 | `native_udp`、`native_tcp`、`vless` |
| `enabled` | `boolean` | `true` | 是否启用这项期望 |
| `listen` | IP 字符串 | `"0.0.0.0"` | 本机绑定地址；不是域名 |
| `listenPort` | `integer` | 必填 | 1–65535；表单可建议端口，实际占用仍由 Agent 校验 |
| `endpoints` | `Endpoint[]` | `[]` | 管理员明确提供的对外地址；元素为 `{ host, port }` |

`listen` 与 `endpoints` 分开：绑定 `0.0.0.0` 不代表对方能访问。空 endpoints 不保证可达；可使用的发现结果属于 State。UDP 映射不能充当 TCP／VLESS endpoint 证明。

## VLESS fields

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `stream` | `"tcp" \| "websocket"` | `"tcp"` | 外层流封装，两者的实际 socket 都是 TCP |
| `tls.serverName` | `string` | 必填 | 平台证书及拨号验证使用的名称 |
| `websocket.path` | `string` | `"/meshway"` | 仅 websocket；以 `/` 开头 |
| `websocket.host` | `string` | TLS serverName | 仅 websocket；HTTP Host |

本稿 VLESS 固定使用 TLS，UUID／证书／私钥由平台生成并按节点权限下发，不放在普通配置文档里。native 类型不接受 VLESS 专用字段；TCP 模式不接受 websocket 字段。

## Behavior

- `listeners: {}`：建议表示没有固定入站 listener，节点仍可主动拨号和使用获准的协调出站 socket。
- 配置更新必须经统一 CAS 提交、重新编译并撤退旧运行代际。
- 本页只定义 Meshway 内部承载，不提供普通代理客户端账户入口。
- 其他协议的表单来自后续统一承载目录，不能仅填任意 `type` 字符串就运行插件。
