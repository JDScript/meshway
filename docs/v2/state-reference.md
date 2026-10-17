---
title: State Reference
description: 节点状态上报、网络聚合、按需查询与订阅更新。
---

# State Reference

Node 上报自身事实，Server 按网络维护全局状态并按权限提供查询／订阅。精确字段见[生成契约](/v2/contracts/openapi.json)；运行能力范围见 [Server](/v2/server) 与 [Node](/v2/node)。

## 数据流

```text
Node A ── report ──┐
Node B ── report ──┼── Server: StateView
Node C ── report ──┘               │
                                  ├─ query → 请求者有权读取的最新状态
                                  └─ watch → 同一可见范围的增量更新
```

Server 维护网络 State，读取／订阅时根据身份、配置和实际执行需求产生视图，不预先为每个 Node 复制完整网络状态。

## Objects

| 对象 | 说明 |
| --- | --- |
| [NodeState](/v2/state-reference/node) | 节点报告的系统、能力、资源、连接及执行事实 |
| [StateView](/v2/state-reference/network) | Server 对可见 NodeState 的聚合、接收元数据及派生状态 |
| [PeerState](/v2/state-reference/query) | 某节点查询／订阅时可见的子集；不是第三份配置源 |
| [Realtime Observation](/v2/state-reference/realtime) | 用户显式请求的限时路径诊断与指标流；区别于订阅既有 State |

Config 定义期望和许可，State 定义当前事实。上报 State 不会修改 access／topology／routing；下行 State 也不能扩展 NodeConfiguration 中的授权。

## links

实际 listener socket 的 `localEndpoints` 上报与执行候选来源见[本机 listener 候选](/v2/state-reference/endpoints#本机-listener-候选)。字段省略表示未取得样本，空数组表示已采样但无合格地址。State 不创建或扩展执行权限。

实际认证通道的观测，按 peer 和通道标识组织；`acquisition` 保留真实连接获取来源，缺测为 null。topology 中允许一条边，不代表 State 中这条边已经建立。双方证据的聚合规则见[资源与连接](/v2/state-reference/resources#links)。

## paths

运行时选择／使用的业务路径，保留源、结果、时间和配置引用。routing 中的规划不是实测 trace；一条 link 可承载多个获准路径。详见[资源与连接](/v2/state-reference/resources#paths)。

## exitCapability

`NodeState.exitCapability` 是可省略的出口前置条件／安装事实；省略为未知。`exitEnabled` 仍只由管理员配置决定。

| 字段 | 含义 |
| --- | --- |
| `instanceId`／`netmap`／`roleGeneration` | 实际探测实例、所探测 netmap 与其出口角色的 `generation`；必须属于当前职责 |
| `families[]` | 角色的每个内层地址族一条（升序、不重复，只含角色的族；没有角色时为空）：`family`（`ipv4`／`ipv6`）、`status`、`installed` |
| `families[].status` | `{status: "probing"}`、`{status: "ready"}` 或 `{status: "unavailable", reason: ...}`；共享前提失败时各族同一原因 |
| `families[].installed` | 该族资源已安装；不得把就绪填成安装成功，为 true 须绑定当前 applied |
| `observedAt` | 原始探测时间；重放不刷新样本 |

原因枚举：`ipForwardDisabled`、`ipv6ForwardDisabled`、`ipv6NoEgress`、`ipv6RouterAdvertisementsIgnored`（三者为 IPv6 族自己的前提，见 [IPv6 出口](/v2/node/exit-design#ipv6)）、`forwardingProbeFailed`、`netAdminMissing`、`nftUnavailable`、`conntrackUnavailable`、`reversePathFilterConflict`、`tunUnavailable`、`firewallConflict`、`routeConflict`、`unsupportedFamily`、`ownershipConflict`。HTTP 使用同一模型的 camelCase 名称。出口 resolver 不可用不否定出口能力，不是出口能力原因，单独见 [exitDns](#exitdns)。

Server 在 applied 前接纳当前 role 的探测与失败；某族 `ready` 还要求 acquire 声明该族后端（`feature:exit-ipv4`／`feature:exit-ipv6` version 1）。能力投影最多存活至原观察后 60 秒与当前 instance lease 截止的交集，绑定同一源 revision／角色；未知、过期、旧实例或旧职责不授予出口可用资格。重启不恢复为 ready。路径检测显示 `exitUnavailable` 与新鲜具体原因，业务许可到期不长于能力预算；netmap 链（实现中）中就绪只是经 `peerFacts` 下发的选择事实（`roleGeneration`／`families`，源端按流的族选择该族 `ready` 且 `installed` 的出口），从不是编译输入；拒绝／不可用仍保留源端 capture。职责、capture 和业务许可见[出口联合许可](/v2/configuration-reference/routing#执行捕获与精确许可)。

## exitDns {#exitdns}

`NodeState.exitDns` 是出口 resolver 的可用性事实，仅在当前安装的 netmap 为 `serveExitResolver`（有源经本节点解析）时出现；省略为不适用或安装不是当前的。它不是编译输入，也不进入 `peerFacts`：源端不据此选择，出口不可用时源的查询收到 SERVFAIL。

| 字段 | 含义 |
| --- | --- |
| `netmap` | 所描述的已安装 netmap；必须是该节点当前 netmap，且其 DNS 方案为 `serveExitResolver`，否则整份报告被拒 |
| `status` | `{status: "serving"}` 或 `{status: "unavailable", reason: ...}` |

原因枚举（判定见[出口 resolver](/v2/node/dataplane#exit-resolver)）：`unobserved`（系统 resolver 尚未观察到，例如锚点未安装）、`noServers`、`localServer`（loopback／未指定／组播等本地地址，可能是回指 Meshway 的 stub）、`anchorServer`（本节点 DNS 锚点）、`meshServer`（Mesh 地址池内）、`exitRoutedServer`（本节点 egress 分割不是 Direct／Unmanaged）、`proxyStubMissing`（resolved 代理 stub `127.0.0.54:53` 未监听）。`dns.overrideLocal: true` 时出口用配置的 default resolver，状态为 `serving`。同一原因也是 IPC Status 的 `exit.dnsUnavailable`（`serving` 时为 null）。

## Reference

- [NodeState 字段](/v2/state-reference/node)
- [Capabilities](/v2/state-reference/capabilities)：transport:xxx 等命名空间能力。
- [Listeners、sockets、STUN 与 links](/v2/state-reference/resources)
- [Endpoint Candidates](/v2/state-reference/endpoints)：configured／interface／stun 来源与验证边界。
- [按需连接与保活](/v2/state-reference/link-lifecycle)
- [上报流程与新鲜度](/v2/state-reference/reporting)
- [Server 网络聚合](/v2/state-reference/network)
- [节点查询、裁剪与订阅](/v2/state-reference/query)
- [Realtime 路径观察](/v2/state-reference/realtime)：往返耗时、业务流量与探测未回应率。
- [观测与历史上报](/v2/state-reference/observations)
- [Config／State 同步关系](/v2/configuration-reference/deployment)
