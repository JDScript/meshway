# TLS

用于 VLESS／Trojan 外层 TCP 的配置字段。TLS 验证通过后仍需 Meshway 节点认证。

## Listener structure

<!-- schema: ListenerTls -->
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

<!-- schema: DialerTls -->
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
| `mode` | listener | `"managed"` | managed 由平台生成；provided 使用管理员提交的证书／私钥 |
| `serverName` | listener | 必填 | 配置的 TLS 名称，决定证书 SAN 及目标拨号验证名 |
| `certificate` | listener | provided 输入必填；managed 首次省略时生成 | PEM 证书链，叶证书在前；规范化版本保存完整值 |
| `privateKey` | listener | provided 输入必填；managed 首次省略时生成 | 匹配的 PEM 私钥；规范化版本加密保存完整值，读取只显示[保留标记](/v2/configuration-reference/editing#secret-retained-marker) `{"retained": true}` |
| `trust` | dialer | `"managed"` | 使用 Server 为目标绑定的材料、信任根与指纹 |
| `minVersion` / `maxVersion` | 两侧 | `"1.2"` / `"1.3"` | 仅这两个版本，min 不大于 max |
| `alpn` | 两侧 | `[]` | 有序协议名；WS 只允许兼容 HTTP/1.1 的协商 |
| `handshakeTimeout` | 两侧 | `"5s"` | 不超过总体建连／运行 lease 的剩余预算 |

`serverName`、公告 endpoint 的 host、WebSocket Host 是三种不同的值。管理员在 listener 上设置，dialer 默认继承经验证的目标参数，不需要维护第二份必须保持一致的值。

VLESS WebSocket 外层为 HTTP/1.1 Upgrade；不允许配置 h2 却执行 HTTP/1.1 握手。空 ALPN 的协议缺省由承载实现处理。

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

PEM 是示意占位，不能作为实际证书提交。Server 校验证书链、名称、有效期和私钥匹配；输入是平台配置，不要求设备自行读取某个本地文件。

Server 与 Node provider 共用 `meshway_transport_stream::tls` 的材料验证。证书链只接受证书 PEM block 与外部空白，拒绝私钥／未知 block、嵌套起始标记和被解析器跳过的文本；避免将隐藏在证书字段中的材料分发给 dialer。客户端使用下发链的信任根并精确匹配叶证书，同时保留名称、有效期、链及握手签名验证。当前两端关闭 TLS 会话恢复与 tickets，每条新连接重新验证。

**材料仍是配置数据。** 数据库中加密存储，并按配置版本绑定；包含敏感输入的原始 JSONC 也需要同样保护。获准管理员的读写入口与普通成员视图区分，日志不回显材料；不以存储保护为理由禁止管理员配置。

managed 的生成结果也保存在完整配置，省略时沿用该 listener 已有值；不另建一套材料版本记录。协议材料永久吊销使用独立的拒绝记录，保存／回滚／发布均检查。

完整 NodeConfiguration 给 listener 所属节点提供所需私钥，给拨号节点提供验证材料，不向拨号节点复制服务端私钥。具体下发结构见 [NodeConfiguration](/v2/configuration-reference/node-configuration#sources)。

默认平台自签证书使用精确信任，不标成公网 CA 签发。管理端可读取公有摘要；State 才报告实际加载／验证的指纹。证书过期、SNI 不匹配、材料版本错误均拒绝，不能 fallback 到明文或跳过校验。

## 上游扩展

| sing-box 字段 | Meshway 范围 |
| --- | --- |
| `certificate` / `key` | 管理员提供或平台生成，均为配置材料 |
| `certificate_path` / `key_path` | 不依赖设备手工文件；在平台配置中提交内容 |
| `certificate_sha256` 等 pin | 从所配置证书派生并下发，不替代独立的 Meshway Node 身份 |
| `insecure` / `disable_sni` | 不提供关闭验证的配置入口 |
| `cipher_suites` / `curve_preferences` | 当前使用受支持库默认，不承诺完整透传 |
| `certificate_provider` / ACME | 不在配置字段集合中 |
| mTLS / client certificate | 不在配置字段集合中；不能代替 Meshway Node 身份 |
| uTLS、REALITY、ECH、fragment、kernel TLS | 不在配置字段集合中 |

[上游 TLS](https://sing-box.sagernet.org/configuration/shared/tls/)
