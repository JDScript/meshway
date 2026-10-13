# Transport

transport 承载 Meshway 节点之间的数据。协议凭据匹配与 Meshway 节点身份、业务授权分别校验；业务方向与建连方向独立。

## 协议与 Server 配置支持

| key | listener／dialer | 支持的声明 |
| --- | --- | --- |
| native_udp | [Listener](/v2/configuration-reference/listeners/native-udp)／[Dialer](/v2/configuration-reference/dialers/native-udp) | UDP 参数 |
| native_tcp | [Listener](/v2/configuration-reference/listeners/native-tcp)／[Dialer](/v2/configuration-reference/dialers/native-tcp) | TCP 参数 |
| vless | [Listener](/v2/configuration-reference/listeners/vless)／[Dialer](/v2/configuration-reference/dialers/vless) | TCP/TLS、WebSocket/TLS，空 flow、TCP CONNECT |
| trojan | [Listener](/v2/configuration-reference/listeners/trojan)／[Dialer](/v2/configuration-reference/dialers/trojan) | TCP/TLS、TCP CONNECT |

Server 接受并校验这些协议的类型化声明、TLS／WS 参数和材料。Node 运行支持由其实际 capability 与资源条件决定；Server 能保存配置不代表新 Node provider 已交付。

## 数据层次

```text
受保护的外层 socket → 协议所需 TLS／WebSocket／封装
  → Meshway 节点认证与端到端保护 → 获准的包流
```

TCP 业务可经 UDP 承载，UDP 业务也可经字节流承载。它们不代表 VLESS/Trojan 的原生 UDP command 或 TCP/UDP 外层打洞能力。多承载主备不能改变必经节点。

## 目录与 schema

getTransportCatalog 返回 protocol、outerSocket、requiredCapability 和 schema 引用名。listenerSchema／dialerSchema 指向 getConfigurationSchema 返回的 schemaJson 文档 `$defs`，例如 `#/$defs/VlessListener`，不是内嵌 schema 文本。

公共字段见 [Listen](/v2/configuration-reference/shared/listen)、[Dial](/v2/configuration-reference/shared/dial)、[TLS](/v2/configuration-reference/shared/tls)、[WebSocket](/v2/configuration-reference/shared/websocket)。默认／覆盖规则见[继承](/v2/configuration-reference/inheritance)。

## Socket 缓冲

listener／dialer 的 `socket.receiveBuffer`／`sendBuffer` 默认 0，表示由 provider 决定。Linux Native UDP 默认请求接收 7 MiB、发送 2 MiB，按承载速率设计而非内核缺省（约 208 KB）；请求先以特权方式绕过 `net.core.rmem_max`／`wmem_max`，没有 `CAP_NET_ADMIN` 时才受其截断。显式值覆盖默认值。完整规则见 [UDP 批量接口](/v2/node/transport#udp-批量接口)，宿主建议见[高吞吐宿主](/v2/install#high-throughput)。

## 边界

协议账户字段不创建 Meshway User。目标 endpoint 来自目标 listener，拨号端不维护另一份服务端凭据。underlay 的 mark／namespace 由资源 owner 管理，不通过协议参数任意覆盖。

Vision、REALITY、uTLS、ECH、mux、gRPC、HTTPUpgrade 和 QUIC 封装不在当前配置字段集合内。未知字段被拒绝，不能任意 JSON 透传。新 provider 的注册和代码装载见 [Node 插件](/v2/node/plugins)。
