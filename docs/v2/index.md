---
title: Meshway V2
description: Meshway V2 配置、状态与分发设计参考。
---

# Meshway V2

下一版配置与状态模型的设计文档，按公开参考格式逐项完善。**V2 仍在设计中，字段提案和功能目标不等于已经发布的运行契约。** 当前用法见[现行文档](/)。

## Configuration

[从完整配置示例开始](/v2/configuration-reference)。

| 部分 | 定义 |
| --- | --- |
| [Access](/v2/configuration-reference/access) | 谁可以访问什么 |
| [Topology](/v2/configuration-reference/topology) | 图上允许哪些连接和约束 |
| [Routing](/v2/configuration-reference/routing) | 获准业务在图上怎么走 |
| [Nodes](/v2/configuration-reference/nodes) | 节点静态设置及 listener／dialer 参数 |

Config 持久保存完整版本并支持回滚；Controller 编译、解析引用并产生完整的节点配置。

- [身份与操作权限](/v2/permissions)
- [按需执行许可：Controller 与 Edge 的分工](/v2/execution)

## State

[从 State 数据流开始](/v2/state-reference)。

Node 上报自身运行事实，Controller 聚合 NetworkState；Node 可按需 query 或 watch 相关 peer。实际 links／paths、socket 观测及测量和配置期望分开。

- [Capabilities](/v2/state-reference/capabilities)
- [按需连接与保活](/v2/state-reference/link-lifecycle)
- [查询、裁剪与订阅](/v2/state-reference/query)
- [Realtime Observation](/v2/state-reference/realtime)

## 分发与存储

- [NodeConfiguration 下发示例](/v2/configuration-reference/node-configuration)
- [Config／State 同步与回滚](/v2/configuration-reference/synchronization)
- [存储、查询与外部 API](/v2/configuration-reference/storage-api)
- [模型与讨论边界](/v2/network-configuration)
