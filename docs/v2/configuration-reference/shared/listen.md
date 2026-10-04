# Listen Fields

所有 listener 共享的字段。Server 校验配置值，Node 安装时还须检查平台支持、本机资源和系统限制。

## Structure

```json
{
  "enabled": true,
  "remark": "",
  "listen": "0.0.0.0",
  "listenPort": 8421,
  "endpoints": [],
  "bindInterface": "",
  "socket": { "receiveBuffer": 0, "sendBuffer": 0 },
  "tcp": {
    "noDelay": true,
    "keepAlive": { "enabled": true, "idle": "5m", "interval": "75s", "count": 3 }
  }
}
```

这是共享结构，不是 UDP listener 的完整示例：`tcp` 只允许出现在外层 TCP 类型中。

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | 是否创建固定监听资源 |
| `remark` | `string` | `""` | 描述，不能为 null |
| `listen` | IP 字符串 | `"0.0.0.0"` | IPv4／IPv6 bind 地址，不是域名 |
| `listenPort` | `integer` | 必填 | 1–65535；端口冲突在执行时明确报告 |
| `endpoints` | `Endpoint[]` | `[]` | 明确公告的外部 host／port，可与 bind 不同 |
| `bindInterface` | `string` | `""` | 空值由平台选择，否则在当前受控网络环境中校验接口 |
| `socket.receiveBuffer` | `integer` | `0` | 字节；0 使用系统值 |
| `socket.sendBuffer` | `integer` | `0` | 字节；0 使用系统值，不保证内核接受请求大小 |

非零 buffer 受平台内存预算及系统限制，不能通过配置无限放大。接口绑定不能绕过用途、namespace 和路由保护。

### Endpoint

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `host` | `string` | IP 或 DNS 名称，不含 URL scheme／路径 |
| `port` | `integer` | 1–65535，外部实际端口 |

公告地址不等于实测可达。STUN 候选来自 State，且须关联实际 socket；UDP 映射不推导 TCP 可达性。

这份显式地址在候选视图中标记为 configured，可与 interface／stun 候选一起使用；详见[Endpoint Candidates](/v2/state-reference/endpoints)。来源优先级不等于强制只使用此地址。

### TCP

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `tcp.noDelay` | `boolean` | `true` | TCP_NODELAY |
| `tcp.keepAlive.enabled` | `boolean` | `true` | 操作系统 TCP keepalive |
| `tcp.keepAlive.idle` | duration | `"5m"` | 首次 keepalive 前空闲时间 |
| `tcp.keepAlive.interval` | duration | `"75s"` | keepalive 间隔 |
| `tcp.keepAlive.count` | integer | `3` | 探测次数，须大于 0 |

keepalive 不是 Meshway 已认证邻接探测，也不决定业务授权或 lease。duration 采用无符号、无前导零的十进制正整数加 `ms`、`s`、`m`、`h`，换算为毫秒后须在 `1..=u64::MAX` 内；保留输入单位，不接受小数或组合单位。这个范围仅保证无损表示，实际参数还受资源预算和 lease 限制。平台不支持指定选项时应拒绝或报告不支持，不能静默丢掉。

## sing-box 对照

`listen`／`listen_port`、接口绑定、TCP keepalive 在此有对应字段。`routing_mark`、`netns` 由平台持有；`reuse_addr`、TCP Fast Open／MPTCP、UDP fragmentation 未纳入本版通用配置。`udp_timeout` 是代理 UDP 映射语义，不能套用到 Meshway 流资格或配置 lease。

[上游 Listen Fields](https://sing-box.sagernet.org/configuration/shared/listen/)
