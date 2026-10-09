# Dialers

声明节点可主动拨号的协议及其进阶选项，不承载组网、路径或访问策略。key 是协议名，不另起名称或填写 type。

## Structure

<!-- schema: DialerSet -->
```json
{
  "native_udp": {},
  "native_tcp": {},
  "vless": {}
}
```

只启用 VLESS 主动拨号可以写：

<!-- schema: DialerSet -->
```json
{ "vless": {} }
```

`vless: {}` 启用 VLESS 并使用其字段默认值；整个 `dialers: {}` 禁用主动建连。**省略 dialers 字段**才采用网络 defaults.dialers。

## Types

| key | 专用字段 |
| --- | --- |
| `native_udp` | [Native UDP](/v2/configuration-reference/dialers/native-udp) |
| `native_tcp` | [Native TCP](/v2/configuration-reference/dialers/native-tcp) |
| `vless` | [VLESS](/v2/configuration-reference/dialers/vless) |
| `trojan` | [Trojan](/v2/configuration-reference/dialers/trojan) |

普通使用只需列出启用的协议；`enabled: false` 可禁用集合中的某个协议。timeout、socket、TLS 等进阶选项见[通用 Dial 字段](/v2/configuration-reference/shared/dial)。

## 配置解析

Server 根据 access、topology、routing 和对端 listener，生成具体出站配置。目标地址、UUID／password、SNI、WS path 等从目标 listener 解析，不要求在每个发起节点上重复手填。

管理员可在 listener 配置中编辑这些字段；修改后，相应节点的入站／出站配置一起重新生成。完整例子见 [NodeConfiguration](/v2/configuration-reference/node-configuration#example)。

## Defaults

继承／覆盖只定义一套规则，见[字段复用与默认继承](/v2/configuration-reference/inheritance)。承载偏好由 `topology[].transports` 定义，完整候选排序规则见 [Topology 承载交集](/v2/configuration-reference/topology#承载交集)。

出站参数与实际通道状态分开。允许某种协议不表示已拨通，也不产生反向新流授权。承载交集、急切拨号与限定单条边协议的写法及限制见[承载与邻接模型](/v2/node/carriers)。
