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
| [Addressing](/v2/configuration-reference/addressing) | Mesh 地址段、节点地址与安全重分配 |

Config 持久保存完整版本并支持回滚；Controller 编译、解析引用并产生完整的节点配置。

- [身份与操作权限](/v2/permissions)
- [按需执行许可：Controller 与 Edge 的分工](/v2/execution)

## State

[从 State 数据流开始](/v2/state-reference)。

Node 上报自身运行事实，Controller 聚合 NetworkState；Node 可按需 query 或 watch 相关 peer。实际 links／paths、socket 观测及测量和配置期望分开。

- [Capabilities](/v2/state-reference/capabilities)
- [按需连接与保活](/v2/state-reference/link-lifecycle)
- [查询、裁剪与订阅](/v2/state-reference/query)
- [GeoIP／ASN 与节点位置](/v2/state-reference/ip-info)
- [Realtime Observation](/v2/state-reference/realtime)

## 分发与存储

- [NodeConfiguration 下发示例](/v2/configuration-reference/node-configuration)
- [配置应用与逐节点进度](/v2/configuration-reference/deployment)
- [整网草稿与应用前对照](/v2/configuration-reference/editing)
- [Config／State 同步与回滚](/v2/configuration-reference/synchronization)
- [数据模型与权威来源](/v2/entities)
- [持久数据库：逐字段](/v2/storage/database)
- [Controller 内存：逐字段](/v2/storage/memory)
- [读写、保留与本地文件](/v2/storage)
- [存储、查询与外部 API](/v2/configuration-reference/storage-api)
- [模型与讨论边界](/v2/network-configuration)

## API

- [API 分区与操作索引](/v2/api)
- [用户注册、网络创建与 Node 密钥认证](/v2/api/authentication)
- [SSE：首次全量、后续变化与生成客户端](/v2/api/events)
- [配置版本与操作历史](/v2/api/history)
- [Actions 与 Problem](/v2/api/problems)

## 工程结构

- [技术栈、Workspace 布局与 Shared crates](/v2/architecture)

## 新 Agent

- [Daemon、CLI 与组件职责](/v2/agent)
- [Userspace／系统 TUN、运行生命周期与本机 IPC](/v2/agent/runtime)
- [启动时动态插件、整体交付与 capabilities](/v2/agent/plugins)
- [Crates 与目录提案](/v2/agent/structure)
