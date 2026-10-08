# Dialers

定义节点允许使用哪些出站承载及其优先级。key 是协议名，不另起名称或填写 type。

## Structure

<!-- schema: DialerSet -->
```json
{
  "native_udp": { "priority": 100 },
  "native_tcp": { "priority": 200 },
  "vless": { "priority": 300 }
}
```

只允许 VLESS 可以写：

<!-- schema: DialerSet -->
```json
{ "vless": {} }
```

`vless: {}` 启用 VLESS 并使用其字段默认值；整个 `dialers: {}` 禁用主动建连。**省略 dialers 字段**才采用网络 defaults.dialers。

## Types

| key | 默认 priority | 专用字段 |
| --- | --- | --- |
| `native_udp` | 100 | [Native UDP](/v2/configuration-reference/dialers/native-udp) |
| `native_tcp` | 200 | [Native TCP](/v2/configuration-reference/dialers/native-tcp) |
| `vless` | 300 | [VLESS](/v2/configuration-reference/dialers/vless) |
| `trojan` | 400 | [Trojan](/v2/configuration-reference/dialers/trojan) |

普通使用只需要协议与 priority。timeout、socket、TLS 等进阶选项见[通用 Dial 字段](/v2/configuration-reference/shared/dial)。

## 配置解析

Server 根据 access、topology、routing 和对端 listener，生成具体出站配置。目标地址、UUID／password、SNI、WS path 等从目标 listener 解析，不要求在每个发起节点上重复手填。

管理员可在 listener 配置中编辑这些字段；修改后，相应节点的入站／出站配置一起重新生成。完整例子见 [NodeConfiguration](/v2/configuration-reference/node-configuration#example)。

## Defaults

继承／覆盖只定义一套规则，见[字段复用与默认继承](/v2/configuration-reference/inheritance)。相同端点来源内，priority 越小越优先，同优先级按候选 key 稳定排序；同一 peer 的 configured 地址整体先于 local 地址，不做权重负载均衡。

出站参数与实际通道状态分开。允许某种协议不表示已拨通，也不产生反向新流授权。承载交集、急切拨号与限定单条边协议的写法及限制见[承载与邻接模型](/v2/node/carriers)。
