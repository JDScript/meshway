# Listeners

位于 `nodes[id].listeners`，以节点内唯一 key 标识固定入站资源。每项由[通用 Listen 字段](/v2/configuration-reference/shared/listen)与协议字段组成。

## Structure

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

| type | 协议参考 | 当前能力 |
| --- | --- | --- |
| `native_udp` | [Native UDP](/v2/configuration-reference/listeners/native-udp) | 平台托管 |
| `native_tcp` | [Native TCP](/v2/configuration-reference/listeners/native-tcp) | 平台托管 |
| `vless` | [VLESS](/v2/configuration-reference/listeners/vless) | TCP/TLS、WebSocket/TLS 已托管 |
| `trojan` | [Trojan](/v2/configuration-reference/listeners/trojan) | TCP/TLS 独立库；尚未接入平台托管 |

以上页面描述下一版配置；“当前能力”不表示旧 API 接受新字段名。详见[支持状态](/v2/configuration-reference/transports)。

## Behavior

- 空对象不创建固定 listener；不禁止获准的出站／协调 socket。
- `enabled: false` 保留静态配置但不执行该 listener。
- listen 是本机 bind；endpoints 是明确公告的外部地址，实际可达性来自 State。
- 管理员可填写协议凭据／TLS 参数，省略的可生成字段由平台补全并保存；不创建独立代理账户清单。完整配置与分发见 [NodeConfiguration](/v2/configuration-reference/node-configuration)。
- listener key 与授权 Tag 不同；改变 type 或替换资源须按新运行代际处理，不借用旧 socket。
