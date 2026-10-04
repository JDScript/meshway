# Trojan Dialer

位于 `nodes[id].dialers.trojan`。**新 Controller 已支持声明、校验与编译；实际 Agent TCP/TLS 承载装配及部署验证独立跟踪。** 执行仅使用 Controller 配置与节点 capability 的交集，目录存在不代表隧道已验证。

## Structure

```json
{
  "remark": "Trojan 备用承载",
  "priority": 400,
  "tls": { "trust": "managed" }
}
```

## Fields

复用 [Dial Fields](/v2/configuration-reference/shared/dial) 的 TCP 子集。priority 默认 400；TLS 使用[TLS 客户端选项](/v2/configuration-reference/shared/tls#dialer-structure)。当前只支持 TCP，因此不要求再填写一个只能为 tcp 的 streams 数组。

## 节点实际收到的字段

完整出站配置包含目标 endpoint、password、TLS 信任和 CONNECT target。password 可以由管理员在目标 listener 中指定，或由平台首次生成并持久保存，然后编译到获准节点的配置中。

其存储、分发和引用规则统一见 [NodeConfiguration](/v2/configuration-reference/node-configuration#sources)；不会为每个拨号端维护另一份密码源。

协议能力及上游扩展范围统一见 [Trojan Listener](/v2/configuration-reference/listeners/trojan#extensions)。

[sing-box Trojan outbound](https://sing-box.sagernet.org/configuration/outbound/trojan/)
