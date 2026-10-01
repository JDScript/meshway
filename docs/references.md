# 参考源码

研究基线记录于仓库的 `references/README.md`，拉取日期为 **2026-09-30**。本文链接固定到实际阅读的提交，适用于独立部署后的文档网站。

这些是开发分支快照，不代表稳定发行版，也不代表 Meshway 已复用或实现相同功能。参考 checkout 保持只读。

Tailscale 官方做法是 Meshway 常规产品基线，自定义 transport 与有序指定路径是已确认扩展。产品语义以[产品对齐](/product-alignment)为准，差异与当前缺口见[Tailscale 对照](/tailscale-alignment)；固定源码用于解释机制，不自动引入第二套需求。

## Tailscale：节点模型、控制分发与中继

基线：[`ab2380a6312858ccf95451787f33acc9c9fca066`](https://github.com/tailscale/tailscale/tree/ab2380a6312858ccf95451787f33acc9c9fca066)（`main`）。

- [`tailcfg/tailcfg.go`：Node 模型](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/tailcfg/tailcfg.go#L370)：节点、密钥、地址与 endpoint。
- [`MapResponse`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/tailcfg/tailcfg.go#L2038-L2136)：peer 全量 / 增量、DERP map、DNS 与 packet filters。
- [`wgengine/magicsock/relaymanager.go`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/wgengine/magicsock/relaymanager.go#L26-L80)：peer relay 分配、握手与探测。
- [`net/netcheck/netcheck.go`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/netcheck/netcheck.go#L90-L128)：网络报告和未知值；[`net/stun/stun.go`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/stun/stun.go#L154-L190)：真实源地址的 XOR-MAPPED 编码。M2 的具体交付与边界见 [Netcheck](/netcheck)。
- [`net/udprelay/server.go`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/udprelay/server.go#L802-L835)：依据 relay binding 转发隧道相关 UDP 包。

**对 Meshway 的启发：** 将控制分发与业务承载分开，研究节点清单、DNS 配置及邻接连通性。Peer relay 的隧道包中继不能直接等同于任意有序多跳路由产品模型。

## Headscale：自托管控制面与授权策略

基线：[`eeaac680bef26585c8cc1569f949e26d0d6aba80`](https://github.com/juanfont/headscale/tree/eeaac680bef26585c8cc1569f949e26d0d6aba80)（`main`）。

- [`README.md`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/README.md)：项目定位与支持范围。
- [`docs/ref/policy.md`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/docs/ref/policy.md#L3-L51)：ACL、Grants 及默认授权行为。
- [`ViaRoutesForPeer`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/policy/v2/policy.go#L1292-L1411)：按 via tags 计算路由 Include / Exclude。
- [`hscontrol/state`](https://github.com/juanfont/headscale/tree/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/state) 与 [`hscontrol/mapper`](https://github.com/juanfont/headscale/tree/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/mapper)：状态协调和客户端 map 分发。

**边界：** Headscale 是独立的开源 Tailscale 控制服务器实现，不是 Tailscale 官方控制面源码。这里的多个 via tags 表达候选路由器筛选，不是 `NYC → LAX → SHA` 的有序 hop 列表。

## Sing-box：规则输入、DNS 关联与承载接口

基线：[`927770c29f3e3698c711fc150e1066a4a78793a2`](https://github.com/SagerNet/sing-box/tree/927770c29f3e3698c711fc150e1066a4a78793a2)（`testing`）。

- [`option/rule.go`](https://github.com/SagerNet/sing-box/blob/927770c29f3e3698c711fc150e1066a4a78793a2/option/rule.go#L140-L185)：domain、suffix、keyword、regex、CIDR、rule-set 等匹配字段。
- [`route/route.go`](https://github.com/SagerNet/sing-box/blob/927770c29f3e3698c711fc150e1066a4a78793a2/route/route.go#L590-L608)：从 Fake-IP / DNS reverse mapping 恢复域名上下文。
- [`adapter/outbound.go`](https://github.com/SagerNet/sing-box/blob/927770c29f3e3698c711fc150e1066a4a78793a2/adapter/outbound.go#L15-L38) 与 [`adapter/endpoint.go`](https://github.com/SagerNet/sing-box/blob/927770c29f3e3698c711fc150e1066a4a78793a2/adapter/endpoint.go#L10-L21)：Outbound、FlowOutbound 与 Endpoint 接口。
- [`wireguard.md`](https://github.com/SagerNet/sing-box/blob/927770c29f3e3698c711fc150e1066a4a78793a2/docs/configuration/endpoint/wireguard.md) 与 [`masque-client.md`](https://github.com/SagerNet/sing-box/blob/927770c29f3e3698c711fc150e1066a4a78793a2/docs/configuration/endpoint/masque-client.md)：WireGuard 与该开发分支的 MASQUE CONNECT-IP endpoint。

**对 Meshway 的启发：** 域名规则依赖额外上下文；统一调度接口需要保留 stream、datagram 与 IP packet 的差异，不能仅列出协议名就声称可互换。

## Mihomo：分类与代理串联

基线：[`88dcbf7f1614a67c3b36b848ee3592dfa92ada36`](https://github.com/MetaCubeX/mihomo/tree/88dcbf7f1614a67c3b36b848ee3592dfa92ada36)（`Meta`）。

- [`docs/config.yaml`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/docs/config.yaml#L1992-L2026)：域名、CIDR 及逻辑规则组合。
- [`adapter/outbound/base.go`：dialer-proxy](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/base.go#L193-L218)：通过另一代理建立连接。
- [`proxy chain 记录`](https://github.com/MetaCubeX/mihomo/blob/88dcbf7f1614a67c3b36b848ee3592dfa92ada36/adapter/outbound/base.go#L245-L258)：代理链诊断信息。

**边界：** 代理链记录不等于操作系统 traceroute 的 IP hop。Meshway 需要独立设计真实探测、TTL / Hop Limit 与 ICMP 行为。

## 官方文档与协议

- [Tailscale Peer Relays](https://tailscale.com/docs/features/peer-relay)
- [Tailscale Route filtering with Via](https://tailscale.com/docs/features/access-control/grants/grants-via)
- [Tailscale Grants syntax](https://tailscale.com/docs/reference/syntax/grants)
- [MASQUE CONNECT-IP — RFC 9484](https://www.rfc-editor.org/rfc/rfc9484.html)

官方文档链接会随上游更新；需要可重复的源码结论时，以本页固定提交为准。
