---
title: State Reference
description: 节点状态上报、网络聚合、按需查询与订阅更新。
---

# State Reference

本参考描述**下一版 State 模型**。Node 上报自身事实；Controller 按网络维护全局状态；Node 按需读取相关 peer 状态，也可订阅变化。具体字段及 API 为设计提案，尚未发布。

## 数据流

```text
Node A ── report ──┐
Node B ── report ──┼── Controller: NetworkState
Node C ── report ──┘               │
                                  ├─ query → 请求者有权读取的最新状态
                                  └─ watch → 同一可见范围的增量更新
```

Controller 维护一份网络 State，**不必预先给每个 Node 复制一份 State**。读取／订阅时根据身份、配置和实际执行需求产生视图。

## Objects

| 对象 | 说明 |
| --- | --- |
| [NodeState](/state-reference/node) | 节点报告的系统、能力、资源、连接及执行事实 |
| [NetworkState](/state-reference/network) | Controller 对全网 NodeState 的聚合、接收元数据及派生状态 |
| [PeerStateView](/state-reference/query) | 某节点查询／订阅时可见的子集；不是第三份配置源 |

Config 定义期望和许可，State 定义当前事实。上报 State 不会修改 access／topology／routing；下行 State 也不能扩展 NodeConfiguration 中的授权。

## links

实际认证通道的观测，按 peer 和通道标识组织。topology 中允许一条边，不代表 State 中这条边已经建立。双方证据的聚合规则见[资源与连接](/state-reference/resources#links)。

## paths

运行时选择／使用的业务路径，保留源、结果、时间和配置引用。routing 中的规划不是实测 trace；一条 link 可承载多个获准路径。详见[资源与连接](/state-reference/resources#paths)。

## Reference

- [NodeState 字段](/state-reference/node)
- [Listeners、sockets、STUN 与 links](/state-reference/resources)
- [上报流程与新鲜度](/state-reference/reporting)
- [Controller 网络聚合](/state-reference/network)
- [节点查询、裁剪与订阅](/state-reference/query)
- [延迟与观测事件](/state-reference/observations)
- [Config／State 同步关系](/configuration-reference/synchronization)
