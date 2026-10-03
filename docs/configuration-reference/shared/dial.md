# Dial Fields

所有 dialer 的共享目标字段。拨号目标由 Controller 的授权投影给出，不要求在配置中重复填写 server。

## Structure

```json
{
  "enabled": true,
  "remark": "",
  "priority": 200,
  "connectTimeout": "5s",
  "bindInterface": "",
  "localAddress": "",
  "socket": { "receiveBuffer": 0, "sendBuffer": 0 },
  "tcp": {
    "noDelay": true,
    "keepAlive": { "enabled": true, "idle": "5m", "interval": "75s", "count": 3 }
  }
}
```

TCP 字段只适用于外层 TCP 类型。除 enabled／priority 对应现有语义外，新增调优字段仍需执行接线。

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | 是否主动使用本拨号器 |
| `remark` | `string` | `""` | 描述，不是资源 key |
| `priority` | integer | 由协议默认值提供 | 0–65535，越小越优先 |
| `connectTimeout` | duration | `"5s"` | 建立承载的总预算，受剩余活动 lease 进一步限制 |
| `bindInterface` | string | `""` | 当前 underlay 内的指定接口；空值由平台选择 |
| `localAddress` | IP 字符串 | `""` | 本地出站地址；空值自动选择，不能绑定 Mesh 地址来绕过保护 |
| `socket` | object | 系统值 | 与 [Listen](/configuration-reference/shared/listen#fields) 相同的 buffer 字段 |
| `tcp` | object | 同 Listen | [TCP socket 设置](/configuration-reference/shared/listen#tcp) |

总建立预算覆盖外层解析／建连与所需 TLS／协议握手；TLS 子预算不能延长它。失败后的重试、候选切换属于有界连接调度，不把每次重试累加成无限 deadline。

## Target binding

Controller 为一次获准拨号提供 peer 身份、目标 listener／协调资源、候选 endpoint、凭据引用和信任绑定。这些不同于可编辑的 dialer 默认值。

DNS 解析使用受保护的 bootstrap resolver；不允许把业务 DNS／出口规则反过来截获自身承载建连。选定本地接口或地址仍接受相同的 socket purpose／generation 校验。

## sing-box 对照

| 上游字段 | 对应／范围 |
| --- | --- |
| `connect_timeout` | `connectTimeout` |
| `bind_interface` | `bindInterface` |
| `inet4_bind_address` / `inet6_bind_address` | `localAddress`，具体候选需匹配地址族 |
| TCP keepalive | `tcp.keepAlive` |
| `domain_resolver` | 平台保护通路的解析器，不直接引用任意业务 DNS |
| `routing_mark` / `netns` | 由 underlay owner 分配，不开放直接覆盖 |
| `detour` | 不引入另一套绕行图，交给 Meshway 路由 |
| MPTCP、TFO、bind_address_no_port、network_strategy 等 | 未纳入本版公开字段；需要时按平台能力单独扩展 |

[上游 Dial Fields](https://sing-box.sagernet.org/configuration/shared/dial/)
