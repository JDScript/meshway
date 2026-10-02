# Network 管理边界

一个 controller 维护**多张具有稳定 ID 与名称的隔离网络**，以 Tailscale tailnet 为产品边界。每张网络独立拥有成员、接入令牌 / App 兑换码、地址、DNS、tags、Access、Routing、连接配置、运行报告和 revision。Web 左上角切换网络，网络设置为 `/<network_id>/settings`；实例创建网络仍有独立权限边界。

邮箱密码账户、仅邀请加入、网络角色和设备归属已接入 v11，首批 Web 成员 / 邀请 / 角色 / 归属管理界面已部署；Apple 已支持保存多个隔离接入，一次激活一网，CLI 尚待交付。目标详情见[产品概念](/concepts#网络、用户与设备)。

## 已实现与尚缺的部分

| 项目 | 当前状态 |
| --- | --- |
| IPv4 地址池 | 首次启动默认 `100.96.0.0/16`，持久保存，可在初始化时显式指定 |
| IPv6 地址池 | 首次生成 ULA `/64`，持久保存；不代表 IPv6 TUN 已启用 |
| 节点地址 | 稳定分配，撤销保留记录，不自动复用 |
| 管理策略 | 同一网络内 Routing、Access、标签、连接配置共享 revision 与 CAS；不同网络互不推进版本 |
| Network ID / 名称 | v7 增量升级创建稳定 ID；名称通过管理员 API 与 Web 条件保存 |
| 多网络成员关系 | 已实现；一个节点身份不可变地属于一张网络，跨网络节点 / 标签引用拒绝 |
| 在线更换网段 | 尚未实现；已有库拒绝不同的地址池启动参数 |
| 节点 DNS 配置与分发 | 管理员配置 enabled / zone / TTL；节点通过独立认证 API 读取同 revision 的记录快照 |
| Split DNS / OS DNS 接管 | 尚未实现；本地节点 DNS 与系统解析器接入是不同步骤，见[DNS 设计](/dns) |

地址池初始化参数是 `meshway-controller serve --ipv4-prefix … --ipv6-prefix …`。它们不是日常在线设置：已有节点、TUN、系统路由和缓存都绑定了地址，改址必须有独立迁移流程，不能在设置页保存一个新 CIDR 后就宣称生效。

## 当前配置边界

`GET /api/v1/networks` 列出网络；`POST /api/v1/networks` 使用 `{ name }` 创建真实空网络。`GET/POST /api/v1/networks/{network_id}/network` 提供 ID、名称、只读地址池与 DNS 意图；改名和 DNS 修改使用该网络的 CAS。名称 / DNS 的实际变化增加 revision，规范化后无变化的保存不增加版本。没有网络删除 API。

节点 DNS 快照使用 NodeBearer 身份、稳定 network ID、网络 revision 和 canonical SHA-256 摘要；只包含当前网络的活动节点记录。节点显示名无效或规范化后重名时不产生短名 alias，完整节点 ID 的 canonical 名仍可用。DNS 记录不会赋予业务访问权限，AAAA 地址元数据也不代表 IPv6 TUN 已启用。

SQLite v6→v7 保留原身份、凭据、公钥、地址、路由、Access、tags 和旧 Agent 配置字节；新增关闭状态的 DNS 不改变既有 revision。此后 network 设置保存与其他配置共享冲突检查。

## 后续管理层方向

后续补齐 CLI 登录和多接入、显式改址，以及系统 DNS 集成与可观测应用状态。保存 DNS 意图、普通配置 ACK、解析器运行和实际解析结果分别呈现；设置页不能因保存成功就显示解析器“已生效”。

用户已要求的远端真实联测在 TUN / 自动组网和打洞完善后优先开展，见[Linux 跨地域验收](/linux-field-validation)。测试使用各主机内独立 Docker 网络命名空间，保留既有 SSH / 管理连接；核对管理地址、现有子网、端口及数据目录，避免与测试地址池重叠。尤其不能把正在使用的 `100.96.0.0/16` 管理地址当作 Meshway 自动接管的测试空间。

## 多网络 API 与默认配置

网络管理操作使用 `/api/v1/networks/{network_id}/…`，旧的未限定管理路径不再提供。服务端在根事务内验证用户 session、当前 membership 和 Read/Manage 权限；实例 bootstrap 仅保留独立兼容权限。节点 ID、selector、标签及连接两端只能引用该网络。Agent 的 URL 与请求 body 保持兼容：服务端从节点 bearer 或接入令牌推导网络，调用方不能用 query / body 指定另一张网络。外部 OIDC 尚待后续接入。

新网络默认 IPv4 池为 `100.96.0.0/16`，IPv6 为新随机 RFC 4193 ULA `/64`；地址只要求网络内唯一，因此不同网络可以同时分配 `100.96.0.1`。撤销地址仍保留、不复用。默认 DNS 关闭，zone 为 `net-<32 位 network ID>.meshway.test`，TTL 30。名称修改不改 stable ID 或 DNS zone。新网络从 revision 1 开始，成员 / 标签 / 连接报告为空；Access v2 以显式 default allow 表达成员全部允许，原网络策略原样保留。Routing v2 的初始 reject 与空手工连接图仍是当前实现，自动选路 / 默认连接模板在 P3a / P3b 接入，不能把创建网络描述成已经建立隧道。

## SQLite v8 → v9 无损迁移

升级在一个 `IMMEDIATE` 事务中完成所有表重建、数据复制和 `user_version` 推进。现有 network ID / 名称 / IPAM / DNS 原样作为第一张网络，不生成另一份身份、不推进 revision。节点、凭据摘要、注册 / 兑换绑定、公钥、tags、Access / Routing 及 JSONC 原文、连接配置、ACK、运行 epoch / lease / report 全部保留。任何复制、约束或 FK 校验失败都回滚 DDL 与行数据；迁移后开启 FK 约束。

固定建表语句位于 `apps/controller/src/store/schema_v9.sql`，逐表复制的原列清单位于 `store/networks.rs`，不解析或替换旧 `CREATE TABLE` 文本。持久表保留 `nodes`、`enrollment_tokens` 等清晰名称，带不可空 `network_id` 外键。单例配置为 `(network_id,id)` 主键；标签为 `(network_id,tag)`；IPv4 / IPv6 唯一约束包含网络。节点 ID、凭据摘要和身份公钥仍 controller 全局唯一，防止同一身份同时加入两张网络；复合外键拒绝跨网络令牌消费、节点标签与运行报告绑定。

## v9 → v10 承载托管与 v11 账户 / 历史

P2a在一个IMMEDIATE事务中添加 `managed_listeners`、
`managed_transport_capabilities`、`managed_enrollment_intents` 和注册令牌复合唯一索引；
固定DDL为 `apps/controller/src/store/schema_v10.sql`，失败回滚整份DDL/user_version。
已有表、身份、公钥、地址、JSONC、leases与网络revision不变，不把旧profiles自动换成新密钥。
可恢复的VLESS材料在SQLite内加密，仅节点专属接口最小下发；恢复须一并保存私有
`transport-material.key`。材料、初始接入意图、Agent执行与显式旧材料导入见
[托管承载对接](/managed-transports-api)。账户/ownership和延迟历史已由统一事务安装，见[账户迁移边界](/local-accounts#存储与迁移)；旧节点初始未认领，不自动创建账户。随后Mesh v3迁移保留旧源并要求显式CAS激活，见[执行合同](/p3a-mesh-execution)。

## 显式类型作用域

Store 没有“当前网络”，也没有 TEMP scope、过滤视图或作用域触发器。管理业务方法和内部查询显式接收已验证的 `NetworkId`，每条业务 SQL 显式绑定网络 ID。Agent、注册令牌与 App 兑换码通过全局唯一的凭据关联构造同一个类型；全局凭据 / 公钥唯一性检查单独存在。列出或创建网络不切换连接状态，并发 / 交错请求不会沿用上一请求的网络。
`NetworkId` 只证明网络存在 / 格式合法，不代表 Owner 权限；管理操作通过 `network_operation` 在同一根事务验证主体及网络角色，不能通过合法 ID 获得管理权限。

类型边界由 Rust 模块可见性保证：`NetworkId` 的字段及任意值构造器只在身份查找模块内可见，没有 `From<String>`、`Default` 或 `Deserialize`；API 模块无法访问 Store 的原始连接。调用 `store.nodes()` / `store.revision()` 等业务方法缺少 `&NetworkId` 会直接编译失败。正常调用形状为：

```rust
let network = store.network_id(&requested_id)?;
let nodes = store.nodes(&network)?;
```

版本判断仅在启动迁移链中，业务查询使用显式网络作用域。历史行为测试使用仅测试编译的单网络 fixture，向真实业务方法显式传入固定 ID；旧数据库由固定 v8 schema 构造，不引入生产版本兼容分支。

地址列在 v9 为 `NOT NULL`；迁移提交前及启动时检查全部节点（含撤销节点）的双栈地址、所属地址池、语义地址唯一性与 FK。固定 `identity_guards.sql` 只保护节点网络 / 凭据、公钥与已消费接入绑定不可变，不是 scope 或查询触发器；能力、联系、国家及 ACK 等已有合法更新继续允许。

Discovery 仍是按不可变全局 node ID 保存的有界内存记录：同进程网络切换不清空报告，展示仅遍历当前网络成员；controller 重启后重新等待上报，保持 v8 已有的非持久语义。

真实旧库只读副本迁移已验证原列/身份/地址/revision/名称/DNS保留与FK完整性。回归保留旧schema fixtures、失败整体回滚、跨网拒绝、相同地址独立分配、并发/交错与旧Agent库存兼容。接口检查`cargo test -p meshway-controller --tests`，Web选网/草稿手工验证；具体历史库/日志由Git和私有证据追溯。

## Web 网络与草稿边界

页面以 URL 的网络 ID 为作用域，Cookie 仅记住导航选择。每个 server action 重新校验真实登录凭据与渲染时的 sign-in marker，并显式携带**渲染时的网络 ID**，不使用可能已被其他标签页改写的选择 Cookie。Access / Routing / Connections 草稿逐网络保存，切换后恢复原 JSONC、注释、无效文本、未应用表单及 CAS 基准；另一张网络不继承草稿。空网络显示真实空清单，不补假节点或在线状态。

## Tailscale 对照与差异

- 与 tailnet 一致：网络是成员、地址、DNS 和策略边界；每网 IP 唯一；同一节点身份只能属于一张网络。Apple 多接入使用各自独立凭据并按活动网络隔离请求。
- tailnet DNS name 使用平台分配的域名；本地 Meshway 用 stable ID 生成独立 `.meshway.test` zone，不发布公共解析，因为现有产品是自托管开发控制面与显式本地 DNS，不能冒充 `ts.net` 公共服务。
- IPv4 采用已有 `100.96.0.0/16` 子池，保持节点规模与数据面界限；网络之间可重叠。IPv6 延续已有随机 ULA，不复用 Tailscale 专属前缀；只有地址元数据，不宣称 IPv6 TUN。
- 实例 bootstrap 留作初始化 / 恢复权限，日常使用本地账户及网络所有者 / 管理员 / 成员角色。承载与有序指定路径继续作为 Meshway 扩展，策略 v3 由独立阶段接入。
