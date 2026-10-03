# Trojan Dialer

**当前能力：TCP/TLS 独立协议库；平台托管尚未完成。** 本页是托管目标字段参考。

## Structure

```json
{
  "type": "trojan",
  "remark": "Trojan 备用承载",
  "priority": 400,
  "connectTimeout": "5s",
  "streams": ["tcp"],
  "tls": { "trust": "managed" }
}
```

## Fields

继承 [Dial Fields](/configuration-reference/shared/dial) 的 TCP 子集。

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `type` | literal | 必填 | `trojan` |
| `priority` | integer | `400` | 目标格式的协议默认值，不自动加入平台默认允许集合 |
| `streams` | enum 数组 | `["tcp"]` | 当前库只支持 TCP/TLS |
| `tls` | [TLS dialer](/configuration-reference/shared/tls#dialer-structure) | 托管信任 | 校验证书、SNI、有效期与版本绑定 |

## Controller-provided fields

`server`／`server_port`、password、TLS 信任和内部 CONNECT target 都来自获准目标的同版本投影。password 与其 SHA-224 表示均不进入普通配置／日志。

库的 TCP CONNECT 字节流完成外层握手后，还要运行 Meshway 节点认证。Trojan 无成功响应头，不把“请求 flush 成功”上报成 peer 已认证。

## sing-box 对照

- `network`：当前固定 TCP CONNECT，不实现上游 UDP ASSOCIATE。
- `multiplex`、WebSocket、gRPC、HTTPUpgrade、QUIC：未实现。
- `fallback` 是上游服务端的未认证流量处理，不是此 dialer 的备选 Mesh 路径。
- 自定义 server/password 的普通代理客户端模式不属于当前 Meshway 产品入口。

[sing-box Trojan outbound](https://sing-box.sagernet.org/configuration/outbound/trojan/)
