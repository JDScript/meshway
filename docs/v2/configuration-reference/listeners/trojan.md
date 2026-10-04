# Trojan Listener

Trojan TCP CONNECT over TLS 监听配置。执行按 Server 允许协议与节点 capability 的交集选择；目录声明不表示隧道可用，运行范围见 [Node](/v2/node)。

## Structure

<!-- schema: Listener -->
```json
{
  "type": "trojan",
  "remark": "Trojan TLS 入口",
  "listen": "0.0.0.0",
  "listenPort": 443,
  "endpoints": [{ "host": "trojan.example.com", "port": 443 }],
  "stream": "tcp",
  "password": "example-password",
  "tls": { "mode": "managed", "serverName": "trojan.example.com" }
}
```

## Fields

继承 [Listen Fields](/v2/configuration-reference/shared/listen) 的 TCP 部分。

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `type` | literal | 必填 | `trojan` |
| `stream` | literal | `"tcp"` | 只接受 TCP/TLS，不接受 websocket 等封装 |
| `password` | 非空字符串 | 平台生成并保存 | 管理员可指定；字节值不做 trim 或大小写归一化 |
| `tls` | [TLS listener](/v2/configuration-reference/shared/tls#listener-structure) | 必填 | 托管目标使用平台材料 |

## 编译到节点的参数

这些参数属于完整 NodeConfiguration，来源为管理员配置或持久化生成值。

| 字段 | 来源与约束 |
| --- | --- |
| password | listener 显式值优先；省略时生成并保存；下发给监听端和获准拨号端 |
| TLS／stream | 根据本 listener 参数生成兼容的两端执行配置 |
| target | 固定内部目标，地址类型与端口精确匹配，不做任意代理转发 |

wire 中的 SHA-224 由配置 password 计算，不单独编辑；command 固定 TCP CONNECT，UDP ASSOCIATE／mux 拒绝。Trojan 没有成功响应头。请求写出或密码头解析完成都不等于 Meshway Node 已认证；必须继续执行节点握手。

## sing-box 对照与扩展 {#extensions}

| 上游字段 | Meshway 范围 |
| --- | --- |
| `users[].password` | listener 的 password；可编辑，没有独立代理账户列表 |
| `users[].name` | remark，不对应 User 实体 |
| `fallback` / `fallback_for_alpn` | 不实现未认证流量转发／Web fallback；不是 Meshway 中继兜底 |
| `tls` | [TLS](/v2/configuration-reference/shared/tls) |
| `transport` | 只接受 TCP/TLS |
| `multiplex` | 不在配置字段集合中 |

[sing-box Trojan inbound](https://sing-box.sagernet.org/configuration/inbound/trojan/)
