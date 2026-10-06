---
title: Meshway V2
description: 总体结构、Server、Node 与共享契约。
---

# Meshway V2

Meshway 由 **Server（控制面）**、**Node（节点端）** 和 **Console（控制台）** 组成。常规能力以 Tailscale 为产品基线，扩展可插拔承载和按域名／IP 的有序路径路由。

```text
Console ── 用户操作／Config 草稿 ──► Server
        ◄─ 配置、实体与 State ──────┤
                                  │ 持钥接入、执行目标、State 同步
meshway CLI ── 本机 IPC ──► meshwayd ◄┘
                            │
                            └── 获准 transport／完整路径 ──► 其他 Node／出口
```

Server 管理账户、网络隔离、完整配置、授权和执行发布；Node 在获准集合内结合实时状态运行网络。Console 和 CLI 展示真实结果，不把保存配置、进程运行或心跳当作隧道已建立。

## 阅读顺序

| 章节 | 回答的问题 |
| --- | --- |
| [总体结构与 Crates](/v2/architecture) | 有哪些程序、库和依赖方向？ |
| [开发服务、测试与环境](/v2/development) | 如何运行、验证、更新、重置和清理开发环境？ |
| [运行日志](/v2/logging) | 日志如何配置与输出，包含哪些字段？ |
| [数据模型](/v2/entities) | Entity、Config、State 的权威来源分别是什么？ |
| [操作权限](/v2/permissions) | 各角色与 Node 凭据可以做什么？ |
| [按需执行许可](/v2/execution) | 需求、发布、lease 与原子刷新如何运作？ |
| [Server](/v2/server) | 控制面负责什么，当前能力与运行参数是什么？ |
| [HTTP 与事件契约](/v2/api) | Console／Node 如何认证、读写与同步？ |
| [存储与查询](/v2/storage) | Server 如何持久保存、提交和查询？ |
| [Node](/v2/node) | meshwayd／CLI、网络模式与组件如何组织？ |
| [Configuration](/v2/configuration-reference) | 配置结构、字段、继承与编辑规则是什么？ |
| [State](/v2/state-reference) | 节点上报什么，如何解释新鲜度与可见范围？ |
| [设计讨论](/v2/discussions/) | 包流、封装／加密与外部实现对照中的开放问题 |

字段参考保留结构、字段与示例；共同规则只在其所属章节定义。

## 命名与契约

已实现能力和限制见 [Server](/v2/server#当前能力) 与 [Node](/v2/node#当前范围)，Node 验收状态见[验收范围](/v2/node/validation)。字段的精确运行合同以 Rust 类型、实际路由及生成 OpenAPI／SDK 为准。

Server 应用包名是 `meshway-server`。协议中的 `controllerId`、`/api/v1` 与 `/agent-api/v1` 等名称按生成契约使用。
