# Netcheck：底层网络观测

M2 先交付独立 UDP 探测：从当前进程所在网络环境向明确指定的 STUN 观察点查询，记录对方实际看到的 IP / port，再比较不同目的地址下的映射。探测结果不创建节点身份、访问授权或连接候选。

## 执行方式

在要测量的节点网络环境内执行。下面使用文档保留地址，运行时替换为自己部署的两个观察点；不内置或自动选择公共 STUN 服务。

```sh
meshway-agent netcheck \
  --stun-server 192.0.2.10:3478 \
  --stun-server 192.0.2.11:3478 \
  --bind 0.0.0.0:0 \
  --timeout-ms 1000 --json
```

IPv6 底层探测显式使用 `--bind '[::]:0'` 和方括号 IPv6 server 地址；一次检查只处理同一地址族。CLI 不接受域名，以免把 DNS 引导问题混入 STUN 事务。

这条命令使用独立临时 socket，不需要 `--state` 或运行中的 daemon。`bound_socket` 是本次本地绑定地址，`mapped_endpoint` 是观察点看到的源地址。临时 socket 在命令结束后关闭：即使报告仍在观测有效窗口内，也不能把该地址直接填作现有 Agent 监听器的可达 endpoint。

## 结果如何解释

| 字段 | 证据含义 |
| --- | --- |
| `source=independent_udp_probe` | 当前 CLI 的临时 UDP 测量，不是 daemon 的承载 socket |
| `udp_observed` | 本次是否至少完成一笔有效 Binding 事务；false 只表示未观察到成功 |
| `mapping_stable_during_check` | 对第一个观察点前后两次测量是否一致；无法比较为 null |
| `mapping_varies_by_destination` | 同一 socket、不同观察点 IP 的结果是否不同；证据不足或检查期间变化为 null |
| `samples` | 每次实际事务的服务器、结果、映射地址、RTT 和耗时 |
| `measured_at` / `completed_at` / `valid_until` | 观测时间与保守的新鲜度窗口，不是 NAT 或通道的租约 |

两个相同 IP 上的不同端口不足以得出“映射是否随目的 IP 变化”。使用两个观察点时依次测 A、B、A；A 的复测发生变化时，优先报告时间上的不稳定，不把它误归类为目的地址造成的差异。部分观察点超时也不能填成 `mapping_varies_by_destination=false`。

`timeout`、`invalid_response`、`io_error`、`stun_error` 分别保留。服务器不响应、路径丢包、防火墙拦截都可能导致超时，单次 netcheck 无法唯一归因。IPv6 Binding 成功表示该次 IPv6 底层事务成功，不表示 Mesh IPv6 TUN 已实现。

JSON 采用单一 Rust 封闭类型，stdout 只输出结果。至少一次有效映射退出 `0`；全部失败退出 `1`；参数错误退出 `2`。界面通过本地命令入口引导诊断，当前没有 controller netcheck 报告 API。

## 协议与边界

STUN 按 [RFC 8489](https://www.rfc-editor.org/rfc/rfc8489.html) 的 Binding 头、随机 96-bit transaction ID、magic cookie 和 XOR-MAPPED-ADDRESS 解析。每个事务校验来源 endpoint、事务、消息边界、属性长度及地址族，使用独立截止时间和有界包大小。

STUN 服务提供的是地址观测。它没有证明 Meshway peer 身份，也没有证明另一个 peer 能向映射地址发起连接。`MappingVariesByDestIP` 一类属性也不应直接变成“已打洞”“对称 NAT”或“必须中继”的确定判定。接下来的连接仍需实际认证、优先级选择和撤回处理。

## 本机隔离验收

验收入口为仓库 `experiments/netcheck`。观察点依据真实 UDP `recvfrom()` 返回源地址，映射差异由独立 router 容器的 Linux SNAT 规则产生，不在 STUN 服务端编造映射结果。

已验收拓扑：client 只连接 LAN，router 连接 LAN / WAN，两个 STUN 观察点只连接 WAN。client 的默认路由和 router 的转发 / NAT 配置限定在本次独立 namespace；资源按本次 ownership 标签清理。

验收分别记录稳定映射、目的相关映射、受限入站、UDP 丢弃、部分观察点失败和两次检查之间的映射更换。实验设置与 netcheck 测量结果分别记录；某场景配置了受限入站，不代表 CLI 已自动检测出过滤类型。

2026-10-01 原生 ARM64 历史验收通过 7 场景，run `45c3ba2b4d9a4acdb5d6972890c0094e`，资源清理成功。受限入站使用保持打开的同一 socket 验证 B 未请求报文不送达，并核对 router drop counter。镜像 digest / 每次事务由 artifact 记录；重现入口为 `experiments/netcheck/run.py`，参数与构建见目录 README。

| 场景 | UDP 观察 | 目的相关变化 | A 复测稳定 |
| --- | --- | --- | --- |
| 稳定 SNAT | true | false | true |
| 按目的 IP 使用不同映射端口 | true | true | true |
| 受限入站实验 | true | false | true |
| 丢弃全部探测 UDP | false | null | null |
| 仅 B 不可达 | true | null | true |
| 更换映射前 / 后的两次检查 | 各自 true | 各自 false | 各自 true |

## 承载 socket 与候选生命周期

实际 managed UDP socket 的采集与有期限上报已经接入，绑定监听器 / outgoing candidate、节点身份、公开运行代际和独立有效期；Web 展示真实报告来源与失效状态，见[承载 endpoint 观测](/endpoint-discovery)。后续继续向获准的邻接分发候选、验证 peer 回包与身份，再进入自动打洞。

候选变化只影响同一逻辑邻接的承载选择。即使直连失败，也不能跳过 Routing 指定的必经节点或降低端到端加密。完成这些边界之后才进入 [M3 的打洞和同邻接 fallback](/milestones)。

## 对照依据

- [Tailscale CLI netcheck](https://tailscale.com/docs/reference/tailscale-cli#netcheck)：物理网络诊断、未知字段与 CLI 职责，官方文档核对于 2026-10-01。
- 固定研究基线 [`net/netcheck/netcheck.go:90–128`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/netcheck/netcheck.go#L90-L128)：UDP / 地址族观测与可空的目的相关映射属性；Meshway 不承诺相同 JSON。
- [`net/stun/stun.go:36–70,154–190`](https://github.com/tailscale/tailscale/blob/ab2380a6312858ccf95451787f33acc9c9fca066/net/stun/stun.go#L36-L70)：事务 ID、请求及根据实际观察地址构造答复的源码阅读入口。
- RFC 8489 §5、§6.3、§14.2：报文、接收校验和 XOR-MAPPED-ADDRESS；STUN 本身不是完整 NAT 穿透方案。
