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
- 已启用 listener 的 `listenPort` 在 Linux 默认临时端口范围 32768–60999 内时，配置照常接受，validate／apply 返回警告 `ephemeral_listener_port`（`params.port`、`first`、`last`，路径 `/nodes/<id>/listeners/<key>/listenPort`）：其他进程的出站 socket 可能短暂占用该端口，节点会重试绑定直到端口空闲，见[listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)。
- `listen` 是本机 bind；`endpoints` 是可选的额外公告地址，默认空数组。Node 自动上报实际 listener 的本机地址，接口 IP 变化通过 State 更新拨号候选；管理员只需为 NAT 公网地址／域名等补充公告。候选不证明可达，与协调端点的区别见[承载与邻接模型](/v2/node/carriers#两类建连机制)。
- 管理员可填写协议凭据／TLS 参数，省略的可生成字段由平台补全并保存；不创建独立代理账户清单。完整配置与分发见 [NodeConfiguration](/v2/configuration-reference/node-configuration)。
- `socket.receiveBuffer`／`sendBuffer` 为 0 时用 provider 默认值；Native UDP listener 默认接收 4 MiB、发送 2 MiB，有 `CAP_NET_ADMIN` 时不受 `net.core.rmem_max` 截断，实际值在 `meshway status` 的 SOCKET 表，见 [Socket 缓冲](/v2/configuration-reference/transports#socket-缓冲)。
- listener key 与授权 Tag 不同；改变 type 或替换资源须按新运行代际处理，不借用旧 socket。
