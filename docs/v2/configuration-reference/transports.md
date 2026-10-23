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

listener／dialer 的 `socket.receiveBuffer`／`sendBuffer` 默认 0，表示由 provider 决定。Linux Native UDP 默认请求接收 4 MiB、发送 2 MiB，按承载速率设计而非内核缺省（约 208 KB）；请求先以特权方式绕过 `net.core.rmem_max`／`wmem_max`，没有 `CAP_NET_ADMIN` 时才受其截断。显式值覆盖默认值。完整规则见 [UDP 批量接口](/v2/node/transport#udp-批量接口)，宿主建议见[高吞吐宿主](/v2/install#high-throughput)。

## 承载选择 {#carrier-choice}

Native TCP、VLESS 与 Trojan 用一条外层 TCP 承载整条邻接。在有丢包或会短暂中断的长路径上，内层 TCP 经它运行会出现数百毫秒到数秒的排队延迟和一到数秒的 0 吞吐（TCP 套 TCP），Native UDP 没有这一问题；实测与原因见[流式承载在有损路径上](/v2/node/performance#stream-carriers-lossy)。UDP 可达的边应由 Native UDP 承载，流式承载用于 UDP 被阻断或只能经 TCP／HTTPS 代理到达的边。

一条边的协议由以下规则决定，`transports` 的顺序本身不保证 Native UDP 获选：

- `transports` 的顺序只决定拨号方尝试候选的顺序，而且端点来源优先于它：configured 端点（任何承载）先于 `local` 端点（[候选排序](/v2/configuration-reference/topology#承载交集)）。
- 两端各自能拨对方时（例如 A 拨 B 的 Native UDP listener、B 拨 A 的 VLESS listener）先完成认证的通道获选；已有不同协议的健康通道时，新通道不替换它（[拨号与接受](/v2/node/carriers#拨号与接受)）。
- 例外：选定的是流式通道，而 `transports` 中本节点可拨的 Native UDP 候选（configured 或 `local` 端点）排在它之前时，拨号方在流式通道旁边按候选退避（1 秒起、上限 60 秒）重新获取 Native UDP，成功后替换流式通道，已建立的流保持原路径。因此一次暂态（UDP 短暂不通时流式先获选）不会让这条边永久停在流式承载上；UDP 恢复后至多约一分钟回到 Native UDP。流式承载之间不替换，协调穿透得到的 Native UDP 也不重新获取（[候选节奏](/v2/node/connectivity#candidate-pacing)）。

需要 UDP 不可达时的退路，把流式承载写在 `native_udp` 之后即可：UDP 可获取时边会回到 Native UDP。只有在宁可没有承载也不要流式承载时，才在通配规则之前为这条边写一条只允许 `native_udp` 的 allow 规则（首条匹配的 allow 决定承载集合）；剩余的竞争只在两端都能拨对方、且各自先获得不同协议的通道时出现。

## 边界

协议账户字段不创建 Meshway User。目标 endpoint 来自目标 listener，拨号端不维护另一份服务端凭据。underlay 的 mark／namespace 由资源 owner 管理，不通过协议参数任意覆盖。

Vision、REALITY、uTLS、ECH、mux、gRPC、HTTPUpgrade 和 QUIC 封装不在当前配置字段集合内。未知字段被拒绝，不能任意 JSON 透传。新 provider 的注册和代码装载见 [Node 插件](/v2/node/plugins)。
