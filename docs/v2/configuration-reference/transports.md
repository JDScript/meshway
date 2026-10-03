# Transport 与支持状态

## 文档版本

本配置参考面向开发中的下一版 `NetworkConfiguration`。采用公开文档的结构、字段和默认值写法；在新契约发布前，示例不能直接交给现行 API。

| 状态 | 含义 |
| --- | --- |
| 平台托管 | 当前 Controller／Agent 已接通这项承载能力；现行字段见[托管 API](/managed-transports-api) |
| 独立库 | 协议库有实现，但目录、材料或正常 Agent 执行尚未托管 |
| 目标字段 | 本参考已定义格式，执行接线尚未完成；不允许保存后静默忽略 |
| 未纳入 | 上游有此项，Meshway 当前没有对应公开配置能力 |

## 协议层次

```text
受保护的外层 socket
  → 可选 TLS／WebSocket
  → VLESS、Trojan 或原生承载
  → Meshway 节点认证与加密
  → 获准的 Mesh 包流
```

当前 VLESS 使用 TCP → TLS → 可选 WebSocket → VLESS → Noise。Trojan 独立库使用 TCP → TLS → Trojan，再交给 Meshway 认证层。

业务 UDP 包可以在可靠字节流中传输；这不等于 VLESS/Trojan 的原生 UDP command，也不等于外层 UDP 打洞。协议 UUID/password 不替代 Node 身份。

## 参考范围

基础协议与配置模型如下：

| 协议 | listener | dialer | 公开字段范围 |
| --- | --- | --- | --- |
| Native UDP | 已托管 | 已托管 | [Listen](/v2/configuration-reference/shared/listen)／[Dial](/v2/configuration-reference/shared/dial) 的 UDP 子集 |
| Native TCP | 已托管 | 已托管 | TCP 子集 |
| VLESS | TCP/TLS、WS/TLS 已托管 | TCP/TLS、WS/TLS 已托管 | 空 flow、TCP CONNECT、平台凭据、TLS、WS |
| Trojan | 库级 TCP/TLS | 库级 TCP/TLS | TCP CONNECT、平台凭据目标、TLS；托管待实现 |

通用 socket、TLS 版本／ALPN、WS 自定义 headers 等高级设置是目标字段，不因底层库可配置就宣称当前控制面已支持。

本轮参考 sing-box 固定 checkout `927770c29f3e3698c711fc150e1066a4a78793a2` 的 listen／dial／TLS、VLESS、Trojan 字段。采用其通用／专用字段分层；字段命名保持本参考的 camelCase。

## 不直接照搬的字段

| 上游字段／能力 | Meshway 处理 |
| --- | --- |
| `tag` | listener 资源 key；dialers 按协议 key 选择，不是授权 Tag |
| `users`、客户端 UUID/password | 协议凭据可在 listener 配置，默认生成并保存；拨号配置引用目标值，不创建代理账户实体 |
| `server` / `server_port` | 从可编辑的目标 listener endpoints 解析到出站配置，避免双端重复填写 |
| `routing_mark` / `netns` | underlay owner 的受限保护机制，不开放任意覆盖 |
| `detour` | 不可绕过 Meshway 逻辑路径；使用连接／路由配置 |
| sniff／旧 domain_strategy | 不将上游已废弃字段搬进新配置 |
| REALITY、Vision、uTLS、ECH、mux、gRPC、HTTPUpgrade、QUIC 封装 | 在专页说明对应关系，但当前未纳入执行合同 |

需要扩展时按协议逐项增加强类型字段和能力协商，不通过任意 JSON 透传到运行时。

共用字段和配置继承集中见[字段复用与默认继承](/v2/configuration-reference/inheritance)，完整协议值如何下发见 [NodeConfiguration](/v2/configuration-reference/node-configuration)。
