# Network 管理边界

一个 controller 维护**多张具有稳定 ID 与名称的隔离网络**，以 Tailscale tailnet 为产品边界。每张网络独立拥有成员、接入令牌 / App 兑换码、地址、DNS、tags、Access、Routing、连接配置、运行报告和 revision。Web 左上角可切换 / 创建网络，设置入口仍为 `/network`。

**已确认目标**是一个 controller 管理多张隔离网络：成员、地址、DNS、连接、授权、路由与报告均限定所选网络，设备一次只接入一张、可保存多个接入并切换。邮箱密码账户、仅邀请加入及设备归属与此边界一起推进；当前单网络实现须无损迁移，不能仅用前端切换代替服务端隔离。目标详情见[产品概念](/concepts#网络、用户与设备)，本页以下 API 均描述当前实现。

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

后续继续提供显式改址流程、用户 / Owner 网络成员权限、客户端多接入切换，以及系统 DNS 集成与可观测应用状态。保存 DNS 意图、普通配置 ACK、解析器运行和实际解析结果分别呈现；设置页不能因保存成功就显示解析器“已生效”。

用户已要求的远端真实联测留待 TUN 和打洞完善之后开展。测试应使用各主机内独立 Docker 网络命名空间，保留既有 SSH / 管理连接；规划阶段核对管理地址、现有子网、端口及数据目录，避免与测试地址池重叠。尤其不能把正在使用的 `100.96.0.0/16` 管理地址当作 Meshway 自动接管的测试空间。

## 多网络 API 与默认配置

所有其他管理员操作使用 `/api/v1/networks/{network_id}/…`，旧的未限定管理路径不再提供。服务端先验证全局管理员，再选择存在的网络；节点 ID、selector、标签及连接两端仅能引用该网络。Agent 的现有 URL 与请求 body 保持兼容：服务端从节点 bearer 或接入令牌推导网络，调用方不能用 query / body 指定另一张网络。管理员仍是现有全局管理员；网络成员权限、用户 / Owner 与 OIDC 属于后续独立接入阶段。

新网络默认 IPv4 池为 `100.96.0.0/16`，IPv6 为新随机 RFC 4193 ULA `/64`；地址只要求网络内唯一，因此不同网络可以同时分配 `100.96.0.1`。撤销地址仍保留、不复用。默认 DNS 关闭，zone 为 `net-<32 位 network ID>.meshway.test`，TTL 30。名称修改不改 stable ID 或 DNS zone。新网络从 revision 1 开始，成员 / 标签 / 连接报告为空；Access v2 以显式 default allow 表达成员全部允许，原网络策略原样保留。Routing v2 的初始 reject 与空手工连接图仍是当前实现，自动选路 / 默认连接模板在 P3a / P3b 接入，不能把创建网络描述成已经建立隧道。

## SQLite v8 → v9 无损迁移

升级在一个 `IMMEDIATE` 事务中完成所有表重建、数据复制和 `user_version` 推进。现有 network ID / 名称 / IPAM / DNS 原样作为第一张网络，不生成另一份身份、不推进 revision。节点、凭据摘要、注册 / 兑换绑定、公钥、tags、Access / Routing 及 JSONC 原文、连接配置、ACK、运行 epoch / lease / report 全部保留。任何复制、约束或 FK 校验失败都回滚 DDL 与行数据；迁移后开启 FK 约束。

持久表使用 `<业务表>_scoped` 并含不可空 `network_id` 外键。单例配置改为 `(network_id,id)` 主键；标签为 `(network_id,tag)`；IPv4 / IPv6 唯一约束包含网络。节点 ID、凭据摘要和身份公钥仍 controller 全局唯一，防止同一个身份同时加入两张网络。节点标签、令牌消费、运行报告等有复合外键，数据库也拒绝跨网络绑定。

Store 的单连接 / mutex 内通过连接私有 TEMP scope、过滤视图及写触发器统一限定所有现有编译与业务 SQL；不能绕过 scope 访问物理表。物理表访问仅用于迁移、创建网络及全局唯一凭据定位。视图写入的 SQLite `changes()` 不代表行数，因此撤销以读取的原状态决定是否推进 revision；UPSERT 改为事务内差量更新。新业务表需加入 scope 表清单并补隔离测试。

Discovery 仍是按不可变全局 node ID 保存的有界内存记录：同进程网络切换不清空报告，展示仅遍历当前网络成员；controller 重启后重新等待上报，保持 v8 已有的非持久语义。

2026-10-01 已用 SQLite backup API 对运行库做只读复制演练：14 张旧表的所有原列逐行一致，4 个节点、revision 42、ID `baf5c8095f41b92ea539a10456da64e3`、IPv4 `100.96.0.0/16`、IPv6 `fd73:96a6:cf5e::/64`、名称与 DNS 不变，FK 校验通过。副本在私有临时目录迁移后删除；live 服务未升级。测试另覆盖完整 v8 fixture、迁移失败整体回滚、跨网络拒绝、相同地址独立分配与旧 Agent 快照字节兼容。

## Web 网络与草稿边界

网络选择存于 HttpOnly cookie；每页按所选网络读取生成接口，每个 server action 重新检查管理员身份。mutation 显式携带**渲染时的网络 ID**，不临时读取可能已被其他标签页改写的选择 cookie。Access / Routing / Connections 的草稿注册表逐网络保存，切换后恢复原 JSONC、注释、无效文本、未应用表单及 CAS 基准；另一张网络不继承这份草稿。空网络显示真实空清单，不补假节点或在线状态。

## Tailscale 对照与差异

- 与 tailnet 一致：网络是成员、地址、DNS 和策略边界；每网 IP 唯一；同一节点身份只能属于一张网络，客户端保存 / 切换多个接入留待客户端生命周期实现。
- tailnet DNS name 使用平台分配的域名；本地 Meshway 用 stable ID 生成独立 `.meshway.test` zone，不发布公共解析，因为现有产品是自托管开发控制面与显式本地 DNS，不能冒充 `ts.net` 公共服务。
- IPv4 采用已有 `100.96.0.0/16` 子池，保持节点规模与数据面界限；网络之间可重叠。IPv6 延续已有随机 ULA，不复用 Tailscale 专属前缀；只有地址元数据，不宣称 IPv6 TUN。
- 全局管理员是本阶段兼容选择，不等同于 Tailscale 的网络用户 / 角色体系。承载与有序指定路径继续作为 Meshway 扩展，策略 v3 由独立阶段接入。
