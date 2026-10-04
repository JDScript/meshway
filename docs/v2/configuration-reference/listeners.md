# Listeners

位于 `nodes[id].listeners`，以节点内唯一 key 标识固定入站资源。每项由[通用 Listen 字段](/v2/configuration-reference/shared/listen)与协议字段组成。

## Structure

以下为单个节点的 listeners 字段值；省略的自动材料在保存时补全：

```json
{
  "public-ws": {
    "type": "vless",
    "remark": "公网 WebSocket 承载",
    "listen": "0.0.0.0",
    "listenPort": 443,
    "endpoints": [{ "host": "edge.example.com", "port": 443 }],
    "stream": "websocket",
    "tls": { "serverName": "edge.example.com" },
    "websocket": { "path": "/meshway", "host": "edge.example.com" }
  }
}
```

## Types

| type | 协议参考 | 外层 |
| --- | --- | --- |
| `native_udp` | [Native UDP](/v2/configuration-reference/listeners/native-udp) | UDP |
| `native_tcp` | [Native TCP](/v2/configuration-reference/listeners/native-tcp) | TCP |
| `vless` | [VLESS](/v2/configuration-reference/listeners/vless) | TCP/TLS、WebSocket/TLS |
| `trojan` | [Trojan](/v2/configuration-reference/listeners/trojan) | TCP/TLS |

Server 目录与 Node 能力共同约束可选承载，见[承载](/v2/configuration-reference/transports)。配置保存不表示节点已创建监听资源。

## Behavior

- 空对象不创建固定 listener；不禁止获准的出站／协调 socket。
- `enabled: false` 保留静态配置但不执行该 listener。
- listen 是本机 bind；endpoints 是明确公告的外部地址，实际可达性来自 State。
- 管理员可填写协议凭据／TLS 参数，省略的可生成字段由平台补全并保存；不创建独立代理账户清单。完整配置与分发见 [NodeConfiguration](/v2/configuration-reference/node-configuration)。
- listener key 与授权 Tag 不同；改变 type 或替换资源须按新运行代际处理，不借用旧 socket。
