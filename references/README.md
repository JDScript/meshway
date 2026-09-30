# 网络产品参考源码

拉取日期：2026-09-30。各目录是独立 Git checkout，初次使用 shallow clone；下表记录本次研究实际使用的提交。分支 HEAD 会继续变化，提交 SHA 是本次结论的基线。

| 目录 | 上游 | 分支 | 提交 |
| --- | --- | --- | --- |
| `tailscale/` | <https://github.com/tailscale/tailscale> | `main` | `ab2380a6312858ccf95451787f33acc9c9fca066` |
| `headscale/` | <https://github.com/juanfont/headscale> | `main` | `eeaac680bef26585c8cc1569f949e26d0d6aba80` |
| `sing-box/` | <https://github.com/SagerNet/sing-box> | `testing` | `927770c29f3e3698c711fc150e1066a4a78793a2` |
| `mihomo/` | <https://github.com/MetaCubeX/mihomo> | `Meta` | `88dcbf7f1614a67c3b36b848ee3592dfa92ada36` |

Mihomo 的默认 `main` 在拉取时停留于 2025-02-21，因此额外 fetch 并 checkout 当前更新的 `Meta` 分支。Sing-box 使用上游默认的 `testing` 分支。以上是开发分支快照，不代表稳定发行版。

## 源码阅读入口

以下行号均对应上表提交。

### Tailscale：客户端、控制协议、连通性与中继

- [`tailcfg/tailcfg.go`](tailscale/tailcfg/tailcfg.go):370：Node、密钥、地址及 endpoint 模型。
- [`tailcfg/tailcfg.go`](tailscale/tailcfg/tailcfg.go):2038–2136：MapResponse 中的 peer 全量/增量、DERP map、DNS 和 packet filters。
- [`wgengine/magicsock/relaymanager.go`](tailscale/wgengine/magicsock/relaymanager.go):26–80：peer relay endpoint 分配、握手与探测管理。
- [`net/udprelay/server.go`](tailscale/net/udprelay/server.go):4–6、802–835：peer relay 转发 Disco/WireGuard UDP 包，按 Geneve VNI 找到 relay binding。
- [`wgengine/magicsock/`](tailscale/wgengine/magicsock/)：直连 endpoint 探测、选择和 relay 集成。

### Headscale：自托管控制面与 Policy

- [`README.md`](headscale/README.md):5、25–44：独立的 Tailscale 控制服务器实现及项目范围。
- [`docs/ref/policy.md`](headscale/docs/ref/policy.md):3–51：HuJSON、ACL、Grants、默认授权行为。
- [`hscontrol/policy/v2/types.go`](headscale/hscontrol/policy/v2/types.go):1834、1934：Grant 与 Policy 的输入类型。
- [`hscontrol/policy/v2/policy.go`](headscale/hscontrol/policy/v2/policy.go):1292–1411：`ViaRoutesForPeer` 根据 via tags 生成路由 Include/Exclude；多个 via tags 是候选匹配，不是有序 hop 列表。
- [`hscontrol/state/`](headscale/hscontrol/state/)、[`hscontrol/mapper/`](headscale/hscontrol/mapper/)：状态协调与客户端 map 分发。

### Sing-box：分类规则、DNS 关联与协议接口

- [`option/rule.go`](sing-box/option/rule.go):140–185：domain、suffix、keyword、regex、CIDR、端口、rule-set 等匹配字段。
- [`route/route.go`](sing-box/route/route.go):590–608：从 Fake-IP / DNS reverse mapping 恢复域名上下文。
- [`adapter/outbound.go`](sing-box/adapter/outbound.go):15–38、[`adapter/endpoint.go`](sing-box/adapter/endpoint.go):10–21：Outbound、FlowOutbound 与 Endpoint 接口。
- [`docs/configuration/endpoint/wireguard.md`](sing-box/docs/configuration/endpoint/wireguard.md)：WireGuard endpoint。
- [`docs/configuration/endpoint/masque-client.md`](sing-box/docs/configuration/endpoint/masque-client.md):3–5：当前开发分支中的 MASQUE CONNECT-IP endpoint。

### Mihomo：分流规则与代理串联

- [`docs/config.yaml`](mihomo/docs/config.yaml):1992–2026：DOMAIN-REGEX、DOMAIN-SUFFIX、DOMAIN-KEYWORD、DOMAIN-WILDCARD、IP-CIDR 及逻辑子规则。
- [`adapter/outbound/base.go`](mihomo/adapter/outbound/base.go):193–218：`dialer-proxy` 通过另一个代理建立连接。
- [`adapter/outbound/base.go`](mihomo/adapter/outbound/base.go):245–258：连接的 proxy chain 记录。

## 本次研究确认的边界

1. Headscale 是独立的开源控制服务器实现，不是 Tailscale 官方控制面源码。
2. Tailscale peer relay 是隧道包中继；`grants.via` 是出口/子网路由器/app connector 的路由筛选。二者都不能直接当作任意有序的 `NYC → LAX → SHA` 路径配置。
3. 域名规则需要 DNS、Fake-IP 或显式代理等输入提供域名上下文；普通 IP 包本身没有域名。
4. WireGuard、MASQUE CONNECT-IP、MASQUE CONNECT-UDP 和 VMess/VLESS 类代理的承载能力不同，需要显式区分 IP packet、datagram 和 stream。
5. Proxy chain 的诊断记录不等于操作系统 traceroute 的 IP hop；显示 mesh 中间节点需要转发层处理 TTL/Hop Limit 和 ICMP，或专门设计 overlay 到 IP 的诊断映射。

## 官方文档

- [Tailscale Peer Relays](https://tailscale.com/docs/features/peer-relay)
- [Tailscale Route filtering with Via](https://tailscale.com/docs/features/access-control/grants/grants-via)
- [Tailscale Grants syntax](https://tailscale.com/docs/reference/syntax/grants)
- [MASQUE CONNECT-IP — RFC 9484](https://www.rfc-editor.org/rfc/rfc9484.html)

此文件记录参考版本与已核对的源码入口；产品模型和 Rust 实现方案仍属于本次设计讨论。
