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
- 带 Tag 的节点使用 Tag 授权身份，不再继承原 User／Group 的 access 权限；创建者可保留用于追溯。
- 原创建用户离网或停用不自动撤销 Tag 身份节点，节点继续按标签策略运行，管理员可独立撤销。
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
7. [API 分区](/v2/api)、[注册与认证](/v2/api/authentication)、[SSE 与初始同步](/v2/api/events)

## 决策与接口定版 {#open-items}

本轮问答已确认下列产品语义，可以作为 API 草案的输入；新接口尚未实现。引用／字段编码、预算和恢复时序按这些边界继续落实，不把未实现的格式标成已发布。

| 决策 | 已确认结论与规范 |
| --- | --- |
| 身份与操作权限 | Tag 替代 User，原用户离网不撤 Tag 节点；最后一个 Tag 须重新用户登录后移除；邮箱引用跟随稳定 User ID。[身份](/v2/configuration-reference/identity)、[权限矩阵](/v2/permissions) |
| 默认值与材料 | 默认拒绝＋显式 Mesh 放行模板；缺省继承、空集合禁用；删除后重建生成新自动材料。[Defaults](/v2/configuration-reference/defaults)、[继承](/v2/configuration-reference/inheritance) |
| Controller 与 Edge 分工 | 按合法需求计算有界候选／完整路径和依赖，设备结合 State 选择；各接收端独立验证。[执行许可](/v2/execution) |
| 发布与恢复 | 同步绑定源／身份／材料等依赖；回滚遇缺失／吊销资源失败，要求替代，不静默换新。[同步](/v2/configuration-reference/synchronization#publication) |
| 出口联合授权 | access 管目的，允许方案与 routing 决定出口；allowed 并集，default/locked 冲突拒绝；记住有效用户选择；本机 direct 不套 Mesh access。[Routing](/v2/configuration-reference/routing) |
| 域名与 DNS | 域名授权按解析地址集合，接受共享 IP 限制；普通 DNS 随方案出口，明确例外保留，bootstrap 独立。[Access](/v2/configuration-reference/access#domains)、[DNS](/v2/configuration-reference/dns) |

普通成员 State／History 只读相关范围，主动诊断仅源于自己的 User 身份设备；Owner／Admin 管理本网络。State snapshot、删除、过期、reset 和真实测量方向继续沿用。初版完整配置按版本加载／缓存，不预先建设多套数据库查询投影。

API 本轮新增决定：默认开放账户注册且可由实例配置关闭，不提供邀请注册；登录用户可创建网络并成为 Owner。用户沿用不透明会话，Node 使用持久密钥身份；Console 首次 SSE 全量同步当前可见 Config／State，再接收增量与配置变更通知。User／Node API 分区，精确路由、Node 持钥证明握手和流式生成方案见 [API 草案](/v2/api)。

参考 Tailscale 的 policy／NetworkMap、sing-box 的字段参考组织方式；不照搬它们所有协议、设备侧配置流程或历史兼容字段。现行实现与本轮设计分开，见[当前配置合同](/configuration)和[协议](/protocol)。
