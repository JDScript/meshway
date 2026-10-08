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
