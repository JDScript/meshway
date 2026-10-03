---
description: Entity、Config、State 的职责与配置参考导航。
---

# 配置与状态模型

本页记录正在对齐的架构方向。字段与示例从 [Configuration Reference](/configuration-reference) 阅读；运行状态见 [State Reference](/state-reference)。具体新增字段、默认值、存储和消息形状仍为草案，不是已发布合同。

## Entity / Config / State

| 层 | 职责 | 生命周期 |
| --- | --- | --- |
| Node / User | 持久身份、归属、撤销 | 不因配置回滚而复活 |
| NetworkConfiguration | 管理员期望；groups、tagsOwners、access、topology、routing、nodes、DNS 等 | 相对静态，数据库完整版本，可回滚 |
| NodeConfiguration | 按节点生成的执行许可／参数 | 从配置与一致实体事实派生，不独立编辑 |
| State | 实际能力、endpoint、通道、使用选择和观测 | 频繁变化，有实例、序号和有效期 |

**Config 类似 ROM，State 类似 RAM，分别下发。** 身份、必要恢复记录和需要查询的观测历史可以持久化，但不能将其混入管理员配置历史。

## 已确认的设计方向 {#network-configuration}

- 主要业务实体 Node／User；Network 保持既有隔离作用域。
- 配置命名统一为 `access`（访问权限）、`topology`（组网规则）、`routing`（流量路由）；保留 `groups`、`tagsOwners`、`nodes[id]`、`dns` 等基础部分。
- access 定义业务端点权限；topology 定义允许的图；routing 定义业务在图上的路径约束。实际 links／paths 属于 State。
- `nodes[id].hostname` 是规范 DNS 名称的权威来源；运行上报的 OS hostname 是另一项事实。
- 共享 configuration crate，提供类型／Schema、校验、编译／切片及增量处理；HTTP、数据库和运行资源 owner 各自分责。
- Config 实际入库并保留完整版本，回滚是历史配置作为新提交，不倒退版本号。
- Config 与 State 是不同的下发载荷和版本语义；状态不会扩大配置许可。

## 阅读顺序 {#next}

1. [整网配置结构](/configuration-reference)
2. [用户组与 Tag](/configuration-reference/identity)、[访问权限](/configuration-reference/access)、[组网规则](/configuration-reference/topology)
3. [节点](/configuration-reference/nodes)、[Listeners](/configuration-reference/listeners)、[Dialers](/configuration-reference/dialers)
4. [DNS](/configuration-reference/dns)、[流量路由](/configuration-reference/routing)、[规则集](/configuration-reference/rule-sets)
5. [NodeConfiguration](/configuration-reference/node-configuration)、[State](/state-reference)、[同步与回滚](/configuration-reference/synchronization)
6. [大配置查询与外部 API](/configuration-reference/storage-api)

## 仍需确认 {#open-items}

字段参考按未来公开文档维护，未发布格式和协议能力如实标注。access／topology 规则不使用 ID，remark 按非 null 字符串、默认空字符串定义；defaults 的兜底范围、Tag 身份语义、逐段覆盖、DNS 出口行为及历史依赖继续逐项确认。初版完整配置按版本加载／缓存，不预先建设多套数据库查询投影。

参考 Tailscale 的 policy／NetworkMap、sing-box 的字段参考组织方式；不照搬它们所有协议、设备侧配置流程或历史兼容字段。现行实现与本轮设计分开，见[当前配置合同](/configuration)和[协议](/protocol)。
