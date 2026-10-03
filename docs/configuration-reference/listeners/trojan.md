# Trojan Listener

**当前能力：独立库实现 TCP CONNECT over TLS；尚未接入 Controller 托管目录／正常 Agent 装配。** 本页给出待实现的托管目标格式。

## Structure

```json
{
  "type": "trojan",
  "remark": "Trojan TLS 入口",
  "listen": "0.0.0.0",
  "listenPort": 443,
  "endpoints": [{ "host": "trojan.example.com", "port": 443 }],
  "stream": "tcp",
  "tls": { "mode": "managed", "serverName": "trojan.example.com" }
}
```

## Fields

继承 [Listen Fields](/configuration-reference/shared/listen) 的 TCP 部分。

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `type` | literal | 必填 | `trojan` |
| `stream` | literal | `"tcp"` | 当前库仅支持 TCP/TLS，不接受 websocket 等封装 |
| `tls` | [TLS listener](/configuration-reference/shared/tls#listener-structure) | 必填 | 托管目标使用平台材料 |

## Protocol fields

| 字段 | 来源与约束 |
| --- | --- |
| password | 托管目标由平台生成；库接收受保护的私有字节，不在普通配置中填写 |
| password hash | 按 Trojan 协议计算 SHA-224，仍属于秘密材料 |
| command | 固定 TCP CONNECT；UDP ASSOCIATE／mux 命令拒绝 |
| target | 固定内部目标，地址类型与端口精确匹配，不做任意代理转发 |

Trojan 没有成功响应头。请求写出或密码头解析完成都不等于 Meshway Node 已认证；必须继续执行节点握手。

## sing-box 对照与扩展

| 上游字段 | Meshway 范围 |
| --- | --- |
| `users[].password` | 平台托管材料目标，不提供代理账户列表 |
| `users[].name` | remark，不对应 User 实体 |
| `fallback` / `fallback_for_alpn` | 不实现未认证流量转发／Web fallback；不是 Meshway 中继兜底 |
| `tls` | [TLS](/configuration-reference/shared/tls) |
| `transport` | 库仅 TCP/TLS，WS／gRPC 等需后续实现 |
| `multiplex` | 未实现 |

[sing-box Trojan inbound](https://sing-box.sagernet.org/configuration/inbound/trojan/)
