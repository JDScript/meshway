# TLS

用于 VLESS／Trojan 外层 TCP 的目标字段。TLS 验证通过后仍需 Meshway 节点认证。

## Listener structure

```json
{
  "mode": "managed",
  "serverName": "edge.example.com",
  "minVersion": "1.2",
  "maxVersion": "1.3",
  "alpn": [],
  "handshakeTimeout": "5s"
}
```

## Dialer structure

```json
{
  "trust": "managed",
  "minVersion": "1.2",
  "maxVersion": "1.3",
  "alpn": [],
  "handshakeTimeout": "5s"
}
```

## Fields

| 字段 | 位置 | 默认 | 说明 |
| --- | --- | --- | --- |
| `mode` | listener | `"managed"` | 平台生成和保存证书／私钥；本版唯一模式 |
| `serverName` | listener | 必填 | 配置的 TLS 名称，决定证书 SAN 及目标拨号验证名 |
| `trust` | dialer | `"managed"` | 使用 Controller 为目标绑定的材料、信任根与指纹 |
| `minVersion` / `maxVersion` | 两侧 | `"1.2"` / `"1.3"` | 仅这两个版本，min 不大于 max；配置覆盖为目标能力 |
| `alpn` | 两侧 | `[]` | 目标字段：有序协议名；WS 只允许兼容 HTTP/1.1 的协商 |
| `handshakeTimeout` | 两侧 | `"5s"` | 目标字段；不超过总体建连／运行 lease 的剩余预算 |

`serverName`、公告 endpoint 的 host、WebSocket Host 是三种不同的值。dialer 使用目标 listener 下发的 SNI，不自行猜测一个名称覆盖证书绑定。

VLESS WebSocket 外层为 HTTP/1.1 Upgrade；本稿不允许配置 h2 却仍执行 HTTP/1.1 握手。空 ALPN 的具体协议缺省由已验证的承载实现处理。

## Materials

普通 NetworkConfiguration 不携带 PEM／私钥、任意 CA 路径或原始证书 pin。平台内部材料包含证书链、私钥、有效期与指纹，并绑定网络／节点／listener／材料版本。

默认平台自签证书使用精确信任，不标成公网 CA 签发。管理端可读取公有摘要；State 才报告实际加载／验证的指纹。证书过期、SNI 不匹配、材料版本错误均拒绝，不能 fallback 到明文或跳过校验。

## 上游扩展

| sing-box 字段 | Meshway 范围 |
| --- | --- |
| `certificate` / `certificate_path` / `key` / `key_path` | 平台材料管理，不接受设备本地路径 |
| `certificate_sha256` 等 pin | 由目标材料生成，不是用户随意替换的对端身份 |
| `insecure` / `disable_sni` | 不提供关闭验证的配置入口 |
| `cipher_suites` / `curve_preferences` | 当前使用受支持库默认，不承诺完整透传 |
| `certificate_provider` / ACME | 后续平台签发管理，不复制节点侧 provider 配置 |
| mTLS / client certificate | 未纳入本版；不能代替 Meshway Node 身份 |
| uTLS、REALITY、ECH、fragment、kernel TLS | 尚未实现或未接托管，不在当前可执行字段集合 |

[上游 TLS](https://sing-box.sagernet.org/configuration/shared/tls/)
