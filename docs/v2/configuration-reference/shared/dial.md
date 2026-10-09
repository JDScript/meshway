# Dial Fields

`nodes[id].dialers.<协议名>` 内的通用选项；协议由 key 确定，不重复填写 type。具体目标配置由 Server 解析，见[下发参数](/v2/configuration-reference/node-configuration#example)。

## Structure

```json
{
  "enabled": true,
  "remark": "",
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

以上为通用字段节选；TCP 字段只适用于外层 TCP 类型。Node 安装时须校验平台支持和资源预算。

## Fields

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | 是否主动使用本拨号器 |
| `remark` | `string` | `""` | 描述，不是资源 key |
| `connectTimeout` | duration | `"5s"` | 建立承载的总预算，受剩余活动 lease 进一步限制 |
| `bindInterface` | string | `""` | 当前 underlay 内的指定接口；空值由平台选择 |
| `localAddress` | IP 字符串 | `""` | 本地出站地址；空值自动选择，不能绑定 Mesh 地址来绕过保护 |
| `socket` | object | 系统值 | 与 [Listen](/v2/configuration-reference/shared/listen#fields) 相同的 buffer 字段 |
| `tcp` | object | 同 Listen | [TCP socket 设置](/v2/configuration-reference/shared/listen#tcp) |

总建立预算覆盖外层解析／建连与所需 TLS／协议握手；TLS 子预算不能延长它。失败后的重试、候选切换属于有界连接调度，不把每次重试累加成无限 deadline。

## Target binding

Server 为一次获准拨号提供 peer 身份、目标 listener／协调资源、公告 endpoint、凭据及信任绑定。它们都是完整 NodeConfiguration 的组成部分；字段来源统一见 [NodeConfiguration](/v2/configuration-reference/node-configuration#sources)。

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
