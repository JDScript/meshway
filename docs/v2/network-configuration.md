---
description: Entity、Config、State 的职责与配置参考导航。
---

# 配置与状态模型

本页记录正在对齐的架构方向。字段与示例从 [Configuration Reference](/v2/configuration-reference) 阅读；运行状态见 [State Reference](/v2/state-reference)。具体新增字段、默认值、存储和消息形状仍为草案，不是已发布合同。

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
- State 有节点上报与节点读取两个方向：Controller 在网络内聚合一次，节点可 query 或 watch。裁剪按当前身份、access 及 topology／routing 的执行依赖进行，中转节点可见性不等于对其业务服务的访问权限。

## 阅读顺序 {#next}

1. [整网配置结构](/v2/configuration-reference)
2. [用户组与 Tag](/v2/configuration-reference/identity)、[访问权限](/v2/configuration-reference/access)、[组网规则](/v2/configuration-reference/topology)
3. [节点](/v2/configuration-reference/nodes)、[Listeners](/v2/configuration-reference/listeners)、[Dialers](/v2/configuration-reference/dialers)
4. [DNS](/v2/configuration-reference/dns)、[流量路由](/v2/configuration-reference/routing)、[规则集](/v2/configuration-reference/rule-sets)
5. [NodeConfiguration](/v2/configuration-reference/node-configuration)、[State](/v2/state-reference)、[同步与回滚](/v2/configuration-reference/synchronization)
6. [大配置查询与外部 API](/v2/configuration-reference/storage-api)

## 仍需确认 {#open-items}

模型骨架足以开始 API 草案；下面六类语义会影响写入、授权或下发合同，需要在相应接口定版前确认。

| 决策 | 需要明确的问题 |
| --- | --- |
| 身份与操作权限 | Tag 是否替代 User 身份；邮箱变更／复用与历史引用；设备拥有者、Tag owner、管理员各自能做什么 |
| 默认值与材料 | 省略／空对象／关闭的真值表；平台默认如何随版本固定；材料保留、显式修改、重新生成的区别 |
| Controller 与 Edge 分工 | 下发候选还是约束；新需求如何取得中转依赖；最终路径与反向建连如何由接收端独立验证 |
| 发布与恢复 | 配置／身份／材料依赖的一致发布引用；实例接管、saved/applied 失配、撤回与 rollback 的转换 |
| 出口联合授权 | 目的 access、允许方案／出口、exitEnabled、topology/routing 如何共同限制正常出口；direct／unmanaged 的范围 |
| 域名与 DNS | 可信域名/IP 关联由谁提供；共享 IP 能否区分；first-match 与出口 DNS 的解析、缓存及撤回规则 |

State 的 snapshot、删除、过期、reset 和真实测量方向继续沿用；具体预算、时序及操作权限在 API 草案中落定。初版完整配置按版本加载／缓存，不预先建设多套数据库查询投影。

参考 Tailscale 的 policy／NetworkMap、sing-box 的字段参考组织方式；不照搬它们所有协议、设备侧配置流程或历史兼容字段。现行实现与本轮设计分开，见[当前配置合同](/configuration)和[协议](/protocol)。
