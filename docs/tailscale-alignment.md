# Tailscale 对照：借鉴边界与缺口

核对日期：2026-10-01。本文记录研究结论；实施次序与验收只维护在[后续里程碑](./milestones.md)。官方文档是产品语义依据，固定客户端源码用于核对实现；Headscale 是独立控制服务器，**不是官方控制面源码**。

## 已有基线

Meshway 已有单 control v7 network 的 stable ID、名称、不可变 IPAM、节点名称与 DNS intent API；管理员 tag catalog/assignment；端点 Access（node/tag/当前 network 的 `*`、有序 first-match allow/reject、显式双向）与 Routing 联合编译精确 NodeData 路径；E2E、mandatory via、Linux IPv4 TUN、UDP/TCP/VLESS TLS 及反向主动 dial、配置 lease 撤回。图形/JSONC 共享草稿、注释与 CAS，前端 spacing/draft 动画与 IA 整理已合并，`/network` 在主线推进。

Agent DNS 正在主线收口 fresh-lease P2：真实 loopback UDP/TCP A/AAAA/PTR、无递归；不代表 OS DNS、Fake-IP 或域名上下文已实现。

## 对照矩阵

### MagicDNS / DNS

| 官方语义与证据 | Meshway 差距及建议 |
| --- | --- |
| 每个 tailnet 有 DNS domain；machine short name 在 tailnet 唯一，冲突可加数字后缀；rename 同步改变 FQDN，搜索域使短名可用。[D1][D2] | 保留稳定 ID canonical（完整 64 字符十六进制 ID 分成两个 32 字符标签）与可变 alias；冲突 alias 按当前契约诊断/抑制，不引入静默自动改名。OS search domain 尚待集成。 |
| MagicDNS 指向 overlay 地址，不是公网 STUN endpoint；IPv4/IPv6 地址与 underlay endpoint 分开。[D3][S1] 公共 DNS 可以公开映射到私有 overlay IP，解析公开不等于可访问。 | AAAA 地址元数据不等于 IPv6 TUN。名称可见性与业务授权分开；按当前节点 DNS 快照回答，不能从管理员 inventory 补齐不可见记录。推荐新集成仍不发布公共 DNS。 |
| Quad100 `100.100.100.100:53`（IPv6 `fd7a:115c:a1e0::53`）是设备本地 stub/service，并非远端公共递归服务器；需要转发时才离开设备。[D4] | `127.0.0.1:5353` 第一段只是 nameservice。下一步推荐隔离 Linux 容器内 53 端口 adapter + OS resolver/search 集成，无须复制 Quad100 地址。 |
| restricted nameserver 是 split DNS；global nameserver 与 Override DNS servers 控制默认解析。搜索域补全不等于 DNS routing；`DNSConfig.Proxied` 是 MagicDNS 历史命名，不保证 proxy。[D3][S2] | 不把 DNS suffix 路由等同业务 Routing。集成层区分本地 zone、搜索域、外部上游和 override intent；首片保留原上游，外部查询才转发，不启用全局 override。 |
| exit node 默认承接外部 DNS，可按 nameserver 的 Use with exit node 覆盖。[D3][S3] DoH 上游需要 bootstrap；原 OS resolver 可能指回本机，必须防 self-loop。[S4] | 出口 DNS 等出口职责成熟再做。先排除自身监听及指回自身的 stub、保留接管前 base resolver；endpoint/control bootstrap 不依赖尚未建立的 Mesh DNS。 |
| proxy 正向：应用→本机 stub→本地记录/选定上游（或 exit）；响应反向返回查询者。PTR 是反向名称查询，并非“反向 dial”。[D3][D4][S3] | 建连方向与 DNS/业务方向分开：下一跳主动连接本机仍可承载已授权查询与响应；这不授予反向业务权限。DNS 回答不创建流授权，也不自动提供 IP 包的域名上下文。 |

### Tags / Policy schema

| 官方语义与证据 | Meshway 差距及建议 |
| --- | --- |
| `tagOwners` 可引用用户/组/tag；设备 tag identity 替代 user identity，不能同时存在。self-advertise 必须获得赋 tag 的权限，不是自报即生效。[D5] | 当前 tag 是管理员分配的节点授权属性，没有设备用户身份切换。继续此模型；未来 advertisement 只能产生待审批请求，enrollment/token 身份不能获得管理员权限。 |
| grants 是 additive union，ACL 网络规则仅 accept；更具体规则不覆盖较宽规则。[D6][D7] | 保留已确认 ordered first-match/reject 和有序 mandatory via。`grants.via` 是 router tag 候选筛选，不是 hop 序列；不能自动导入为等价规则。[S8] |
| 新 tailnet 默认模板 allow-all；显式配置则采用 deny-by-default；方向授权不等于对端可发起新连接。[D7] | 初始空策略 reject 已确定。新增成员/tag 只按现有规则匹配，不附赠权限；default allow 是显式策略选择。未知 group/不支持 selector 应报 schema 错误，不能转成 `*`。 |
| tagged device 不能在控制台移除最后一个 tag，须重新以用户身份认证；移除用户不删除独立 tagged device。[D5] 客户端 filter 的返回包处理按协议不同：TCP 非 SYN、UDP/SCTP reverse-flow 缓存、ICMP 响应。[S7] | Meshway 移除 tag 是属性撤回，节点 revoke 是身份撤销；须重新编译并终止旧代际。当前显式双向不是 stateful return，不照搬 Tailscale 的 TCP/ICMP shortcut。 |
| HuJSON 提供注释/尾逗号及 policy tests；schema 的语法容错不改变授权语义。[D6] | 借鉴测试断言与字段诊断，继续一份草稿+CAS；保留无效文本、注释和未保存编辑。已实现双 editor 不重做，未支持的 HuJSON 字段不可静默忽略。 |

### CLI / 连通性

| 官方语义与证据 | Meshway 差距及建议 |
| --- | --- |
| `tailscaled` 持续运行；CLI 经 LocalAPI 查询 daemon。`status --json/--peers`、`dns status/query`、`whois` 分别提供连接、解析配置/查询、IP 身份映射。[D8][S5] | 当前 `meshway-agent run` 是常驻进程，Docker `meshway` 是 UID launcher；`status` 仅保存状态，`ping` 仅认证邻接探测。推荐兼容命令风格与版本化输出，不承诺 Tailscale JSON 字段完全兼容；查询活 daemon，缓存明确标识 saved，停止时不伪造 live。 |
| `netcheck` 测 underlay UDP/STUN、映射随目的变化、地址与 relay latency；未测得是 unknown。[D8][S6] `ping` 分 DISCO/TSMP/ICMP/PeerAPI，各自覆盖不同层。[D9][S2] | 优先 human summary + JSON、稳定退出码；邻接 probe 不能冒充端到端 ICMP、真实 trace 或应用可达。whois 只描述身份，不能输出“已授权”。 |
| `up` 连接并必要时认证；`set` 只改指定偏好；`down` 暂断且保留身份；`login` 加入/登录；`logout` 使当前登录失效。[D8] | 后续 mutation 需定义生命周期与权限后再映射。不要把停止进程等同 logout/revoke，也不要将 enroll 偷换为网页登录。Rust 契约统一 IPC 类型与校验；已有私有 socket 上扩展 versioned enum，若选 HTTP 则由真实路由生成 OpenAPI/客户端，禁止另抄 DTO。 |
| STUN 获得候选 endpoint，DISCO 认证探测/交换；DERP 可先承载协商/流量，然后尝试 direct，再 peer relay，失败仍 DERP，持续重试；三种均保持 E2E。[D10][D11][S6] | endpoint 候选不等于已连接。传输主备仅作用于同一逻辑邻接；relayfallback 不能绕过 mandatory via。先本机 NAT fixture 验证，再考虑远端，测试矩阵见里程碑。 |

## 来源索引

以下官方 URL 均在核对日实际读取；页面自身 Last validated 日期不等于本项目验收日期。

- [D1 MagicDNS](https://tailscale.com/kb/1081/magicdns)、[D2 Machine names](https://tailscale.com/docs/concepts/machine-names)、[D3 DNS](https://tailscale.com/kb/1054/dns)、[D4 Quad100](https://tailscale.com/docs/reference/quad100)。
- [D5 Tags](https://tailscale.com/kb/1068/tags)、[D6 Policy/HuJSON](https://tailscale.com/docs/reference/syntax/policy-file)、[D7 Grants union](https://tailscale.com/docs/reference/syntax/grants) / [ACL 默认行为](https://tailscale.com/docs/features/access-control/acls)。
- [D8 CLI](https://tailscale.com/docs/reference/tailscale-cli)、[D9 Ping types](https://tailscale.com/docs/reference/ping-types)、[D10 Connection types](https://tailscale.com/kb/1257/connection-types)、[D11 Peer relay](https://tailscale.com/docs/features/peer-relay)。

源码固定于 Tailscale `ab2380a6312858ccf95451787f33acc9c9fca066`；行号均实际核对：

- S1 [`tailcfg/tailcfg.go:370–436`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/tailcfg/tailcfg.go#L370-L436)：FQDN/overlay address/endpoint/tag 分开。
- S2 同文件 [`1788–1888`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/tailcfg/tailcfg.go#L1788-L1888)：DNSConfig、PingType。
- S3 [`ipn/ipnlocal/node_backend.go:776–818,1435–1649`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/ipn/ipnlocal/node_backend.go#L1435-L1649)：可见节点索引、DNS 编译/exit。
- S4 [`net/dns/direct.go:447–483`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/dns/direct.go#L447-L483)、[`net/dns/resolver/forwarder.go:538–571`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/dns/resolver/forwarder.go#L538-L571)：self-loop/bootstrap。
- S5 [`client/local/local.go:782–810`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/client/local/local.go#L782-L810)、[`ipn/ipnstate/ipnstate.go:31–85`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/ipn/ipnstate/ipnstate.go#L31-L85)：daemon status。
- S6 [`net/netcheck/netcheck.go:91–150`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/netcheck/netcheck.go#L91-L150)、[`wgengine/magicsock/endpoint.go:1423–1458`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/wgengine/magicsock/endpoint.go#L1423-L1458)、[`relaymanager.go:26–80`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/wgengine/magicsock/relaymanager.go#L26-L80)。
- S7 [`wgengine/filter/filter.go:510–550,624–634`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/wgengine/filter/filter.go#L510-L550)：返回包边界。
- S8 Headscale `eeaac680bef26585c8cc1569f949e26d0d6aba80`，[`docs/ref/policy.md:3–51`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/docs/ref/policy.md#L3-L51)、[`hscontrol/policy/v2/policy.go:1292–1336`](https://github.com/juanfont/headscale/blob/eeaac680bef26585c8cc1569f949e26d0d6aba80/hscontrol/policy/v2/policy.go#L1292-L1336)：独立实现、默认行为与 via 路由筛选。

[D1]: https://tailscale.com/kb/1081/magicdns
[D2]: https://tailscale.com/docs/concepts/machine-names
[D3]: https://tailscale.com/kb/1054/dns
[D4]: https://tailscale.com/docs/reference/quad100
[D5]: https://tailscale.com/kb/1068/tags
[D6]: https://tailscale.com/docs/reference/syntax/policy-file
[D7]: https://tailscale.com/docs/reference/syntax/grants
[D8]: https://tailscale.com/docs/reference/tailscale-cli
[D9]: https://tailscale.com/docs/reference/ping-types
[D10]: https://tailscale.com/kb/1257/connection-types
[D11]: https://tailscale.com/docs/features/peer-relay
[S1]: #来源索引
[S2]: #来源索引
[S3]: #来源索引
[S4]: #来源索引
[S5]: #来源索引
[S6]: #来源索引
[S7]: #来源索引
[S8]: #来源索引
