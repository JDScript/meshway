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
| Network ID / 名称 | 创建网络时分配稳定 ID；名称通过管理员 API 与 Web 条件保存 |
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

新网络默认 IPv4 池为 `100.96.0.0/16`，IPv6 为新随机 RFC 4193 ULA `/64`；地址只要求网络内唯一，因此不同网络可以同时分配 `100.96.0.1`。撤销地址仍保留、不复用。默认 DNS 关闭，zone 为 `net-<32 位 network ID>.meshway.test`，TTL 30。名称修改不改 stable ID 或 DNS zone。新网络从 revision 1 开始，成员 / 标签 / 连接报告为空；唯一 policy 使用默认连接 allow 与成员 grants，Mesh 路由为空时自动沿允许候选图选路；候选自动发现/建连仍待完成，创建网络不表示已建立隧道。

## 当前 SQLite schema

`apps/controller/src/store/schema/schema.sql` 是唯一建表权威来源。新库在一个 IMMEDIATE 事务内建立完整表/约束、初始网络、默认 policy 和 user_version；初始化失败回滚，不保留半套可服务 schema。

普通启动只接受源码定义的当前 schema，比较完整表/索引/trigger 定义、FK、双栈地址与网络配置完整性，并绑定原 admin.token 与私有材料 key。未知、损坏、缺失或弱化结构明确拒绝，不修复或异常清库。开发数据重置由部署者显式操作，不能仅重编 binary 就声称已部署。

持久表使用正常领域名，网络外键不可空，IPv4/IPv6 在网络内唯一。节点ID、凭据摘要、公钥全局唯一；复合外键与不可换绑 guard 拒绝跨网标签、令牌消费与报告绑定。所有当代角色权限、CAS、接入幂等、材料保护和预算失败回滚继续有效。

## 显式类型作用域

Store 没有“当前网络”，也没有 TEMP scope、过滤视图或作用域触发器。管理业务方法和内部查询显式接收已验证的 `NetworkId`，每条业务 SQL 显式绑定网络 ID。Agent、注册令牌与 App 兑换码通过全局唯一的凭据关联构造同一个类型；全局凭据 / 公钥唯一性检查单独存在。列出或创建网络不切换连接状态，并发 / 交错请求不会沿用上一请求的网络。
`NetworkId` 只证明网络存在 / 格式合法，不代表 Owner 权限；管理操作通过 `network_operation` 在同一根事务验证主体及网络角色，不能通过合法 ID 获得管理权限。

类型边界由 Rust 模块可见性保证：`NetworkId` 的字段及任意值构造器只在身份查找模块内可见，没有 `From<String>`、`Default` 或 `Deserialize`；API 模块无法访问 Store 的原始连接。调用 `store.nodes()` / `store.revision()` 等业务方法缺少 `&NetworkId` 会直接编译失败。正常调用形状为：

```rust
let network = store.network_id(&requested_id)?;
let nodes = store.nodes(&network)?;
```

版本判断只在启动 schema gate；业务方法不含版本兼容分支。启动核对全部节点（含已撤销节点）的双栈地址、地址池、语义唯一性与FK；身份guard只保护节点/公钥/已消费接入绑定，不作为查询scope。

Discovery 仍是按不可变全局 node ID 保存的有界内存记录：同进程网络切换不清空报告，展示仅遍历当前网络成员；controller 重启后重新等待上报，保持 v8 已有的非持久语义。

## Console 网络与草稿边界

管理请求显式携带所选网络ID。每次mutation由Controller复核登录身份和当前网络角色。缓存与JSONC草稿按网络隔离；切换视图不覆盖注释、无效文本、未保存修改或CAS基准。会话存储与标签页变化按[SPA合同](/spa-api-audit)处理，空网显示真实空清单。

## Tailscale 对照与差异

- 与 tailnet 一致：网络是成员、地址、DNS 和策略边界；每网 IP 唯一；同一节点身份只能属于一张网络。Apple 多接入使用各自独立凭据并按活动网络隔离请求。
- tailnet DNS name 使用平台分配的域名；本地 Meshway 用 stable ID 生成独立 `.meshway.test` zone，不发布公共解析，因为现有产品是自托管开发控制面与显式本地 DNS，不能冒充 `ts.net` 公共服务。
- IPv4 采用已有 `100.96.0.0/16` 子池，保持节点规模与数据面界限；网络之间可重叠。IPv6 延续已有随机 ULA，不复用 Tailscale 专属前缀；只有地址元数据，不宣称 IPv6 TUN。
- 实例 bootstrap 留作初始化 / 恢复权限，日常使用本地账户及网络所有者 / 管理员 / 成员角色。承载与有序指定路径继续作为 Meshway 扩展，统一使用当前四层 policy。
