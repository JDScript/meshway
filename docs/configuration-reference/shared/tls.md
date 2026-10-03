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
| `mode` | listener | `"managed"` | managed 由平台生成；provided 使用管理员提交的证书／私钥，后者为新增目标能力 |
| `serverName` | listener | 必填 | 配置的 TLS 名称，决定证书 SAN 及目标拨号验证名 |
| `certificate` | listener | mode=provided 时必填 | PEM 证书链，叶证书在前 |
| `privateKey` | listener | mode=provided 时必填 | 与证书匹配的 PEM 私钥 |
| `trust` | dialer | `"managed"` | 使用 Controller 为目标绑定的材料、信任根与指纹 |
| `minVersion` / `maxVersion` | 两侧 | `"1.2"` / `"1.3"` | 仅这两个版本，min 不大于 max；配置覆盖为目标能力 |
| `alpn` | 两侧 | `[]` | 目标字段：有序协议名；WS 只允许兼容 HTTP/1.1 的协商 |
| `handshakeTimeout` | 两侧 | `"5s"` | 目标字段；不超过总体建连／运行 lease 的剩余预算 |

`serverName`、公告 endpoint 的 host、WebSocket Host 是三种不同的值。管理员在 listener 上设置，dialer 默认继承经验证的目标参数，不需要维护第二份必须保持一致的值。

VLESS WebSocket 外层为 HTTP/1.1 Upgrade；本稿不允许配置 h2 却仍执行 HTTP/1.1 握手。空 ALPN 的具体协议缺省由已验证的承载实现处理。

## Materials

管理员可以明确提供证书与私钥，也可以使用 managed 生成。provided 示例：

```json
{
  "mode": "provided",
  "serverName": "edge.example.com",
  "certificate": "<PEM certificate chain>",
  "privateKey": "<PEM private key>"
}
```

PEM 是示意占位。证书链、名称、有效期和私钥匹配须校验；输入是平台配置，不是让设备自行读取某个本地文件。当前托管 API 尚未提供这个管理员输入能力。

**材料仍是配置数据。** 数据库中加密存储，并按配置版本绑定；包含敏感输入的原始 JSONC 也需要同样保护。获准管理员的读写入口与普通成员视图区分，日志不回显材料；不以存储保护为理由禁止管理员配置。

完整 NodeConfiguration 给 listener 所属节点提供所需私钥，给拨号节点提供验证材料，不向拨号节点复制服务端私钥。具体下发结构见 [NodeConfiguration](/configuration-reference/node-configuration#sources)。

默认平台自签证书使用精确信任，不标成公网 CA 签发。管理端可读取公有摘要；State 才报告实际加载／验证的指纹。证书过期、SNI 不匹配、材料版本错误均拒绝，不能 fallback 到明文或跳过校验。

## 上游扩展

| sing-box 字段 | Meshway 范围 |
| --- | --- |
| `certificate` / `key` | 管理员提供或平台生成，均为配置材料 |
| `certificate_path` / `key_path` | 不依赖设备手工文件；在平台配置中提交内容或后续受控材料引用 |
| `certificate_sha256` 等 pin | 从所配置证书派生并下发，不替代独立的 Meshway Node 身份 |
| `insecure` / `disable_sni` | 不提供关闭验证的配置入口 |
| `cipher_suites` / `curve_preferences` | 当前使用受支持库默认，不承诺完整透传 |
| `certificate_provider` / ACME | 后续平台签发管理，不复制节点侧 provider 配置 |
| mTLS / client certificate | 未纳入本版；不能代替 Meshway Node 身份 |
| uTLS、REALITY、ECH、fragment、kernel TLS | 尚未实现或未接托管，不在当前可执行字段集合 |

[上游 TLS](https://sing-box.sagernet.org/configuration/shared/tls/)
