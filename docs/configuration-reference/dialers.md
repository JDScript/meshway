# Dialers

位于 `nodes[id].dialers`。每项由[通用 Dial 字段](/configuration-reference/shared/dial)与协议字段组成，描述如何拨出，不要求配置每对节点的地址。

## Structure

```json
{
  "udp": { "type": "native_udp", "remark": "优先使用 UDP", "priority": 100 },
  "tcp": { "type": "native_tcp", "priority": 200 },
  "vless": {
    "type": "vless",
    "priority": 300,
    "streams": ["tcp", "websocket"]
  }
}
```

## Types

| type | 协议参考 | 当前能力 |
| --- | --- | --- |
| `native_udp` | [Native UDP](/configuration-reference/dialers/native-udp) | 平台托管 |
| `native_tcp` | [Native TCP](/configuration-reference/dialers/native-tcp) | 平台托管 |
| `vless` | [VLESS](/configuration-reference/dialers/vless) | TCP/TLS、WebSocket/TLS 已托管 |
| `trojan` | [Trojan](/configuration-reference/dialers/trojan) | TCP/TLS 独立库；尚未接入平台托管 |

## Defaults

- 节点 `dialers: {}` 使用 [defaults.dialers](/configuration-reference/defaults#dialers)。
- 非空对象替换默认集合；不和默认 UDP／TCP 隐式合并。
- 所有项 disabled 表示不主动拨号。dialer 不限制获准 listener 接收后形成的双向通道。
- 优先级数值较小者优先，同优先级按 key 稳定排序；不是负载均衡权重。
- 目前目标格式约定每种 type 至多一项；逐段覆盖和重复类型实例的规则还未定版。

Controller 从配置许可、对端 listener、两端能力和当前协调信息选择目标。server／port／凭据／信任绑定来自该获准目标，不在普通 dialer 里维护另一份手工 peer 清单。
