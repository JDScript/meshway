# Entity 与关系

**V2 数据模型提案。** 本页列持久身份、关联和字段归属；具体表名与列类型尚未定版。已确认的身份／权限语义继续有效，不从当前数据库表机械复制一套新实体。

## 主要对象

```text
User ──< Membership >── Network ──< Node
                            │          ├─ NodeKey
                            │          └─ AddressAllocation
                            └─ ConfigurationVersion

User 身份 Node ──→ 当前 Membership
Tag 身份 Node  ──→ 当前配置中的 tags
```

主要业务身份为 User 和 Node；Network 是隔离作用域，Membership 是用户与网络的权限关系。Tag、Group、Listener、Link、Path 不增加一套独立业务实体来源。

## User

Controller 全局账户。同一个 User 可以在多个网络拥有不同角色。

| 字段提案 | 含义 |
| --- | --- |
| `id` | 稳定 UserId，邮箱变更不改变 |
| `email` | 规范化且唯一的登录标识；不自动声称邮箱所有权已验证 |
| `status` | active／disabled；停用时撤销人类会话及相应用户身份许可 |
| `createdAt` | 账户创建时间 |

数据库另外保存 passwordHash、credentialVersion 等私有认证字段。它们不是 Console User DTO 的字段，更不进入 NetworkConfiguration。

邮箱引用在配置保存时绑定稳定 UserId。改邮箱更新当前可读引用并形成版本；旧历史仍指向原用户，不能重新解析到邮箱的新占用者。

## Network

所有节点、配置、地址、成员关系和观测的隔离边界。

| 字段提案 | 含义 |
| --- | --- |
| `id` | 稳定 NetworkId |
| `name` | 网络显示名称，建议属于实体元数据 |
| `createdBy` / `createdAt` | 创建来源 |
| `status` | 生命周期状态；精确删除／保留规则另定 |

创建网络时原子建立创建者的 Owner membership 和初始配置。Owner 不写成唯一的 `network.ownerId`，权限以有效 memberships 为准，可以存在多个 Owner；最后一个 Owner 保护仍保留。

`currentConfigurationRevision` 是存储中的配置指针，查询可投影返回；不提供绕过配置事务直接改指针的写入口。网络显示名若采用本提案归入实体元数据，配置回滚就不隐式改名。

## Membership

| 字段提案 | 含义 |
| --- | --- |
| `id` | 一次入网关系的稳定 MembershipId |
| `networkId` / `userId` | 关联本网络与全局用户 |
| `role` | owner／admin／member |
| `joinedAt` / `revokedAt` | 加入与离网时间 |

同一 `(networkId, userId)` 至多一条有效 membership。移除后再加入建立新 membership，不复活旧设备批准或已撤销身份。

角色权限沿用[操作权限](/v2/permissions)。网络成员加入处理已有账户，不通过成员操作创建 User 或替他人设置全局密码；精确加入交互待接口设计补齐。

## Node

每个 Node 是某一网络内的一次持久设备接入身份。同一物理设备保存多个隔离接入时，对应不同网络内的 Node；一次只激活一张网络。

| 字段提案 | 含义 |
| --- | --- |
| `id` / `networkId` | 稳定节点及其不可随意切换的网络归属 |
| `subject` | 有效授权身份：User membership 引用或 Tag 身份 |
| `createdBy` / `registeredAt` | 原创建用户或接入来源与时间，用于追溯 |
| `revokedAt` | 明确撤销，不因配置回滚清除 |

`subject` 结构示意：

```json
{
  "type": "user",
  "membershipId": "<membership-id>"
}
```

```json
{
  "type": "tag"
}
```

Tag 列表只保存在 `NetworkConfiguration.nodes[id].tags`。Node 的 subject 记录身份类别／用户绑定，**不再保存第二份可写 tags**。身份切换与配置提交同事务校验：User 身份不带 Tag；Tag 身份至少有一个有效 Tag；最后一个 Tag 须经设备重新用户登录后移除。

配置中的 Tag 身份与持久 Node subject 必须一致。修改配置、局部赋 Tag、设备重新登录都走同一身份转换检查；不能用整份 JSON 提交绕过设备登录要求。创建者离网不影响 Tag 节点，创建者信息也不继续授予管理或业务权限。

### NodeKey

数据库保存 Node 的公钥绑定、用途、代际和撤销记录。用途至少区分控制面认证与数据面密钥；私钥保留在设备。

Node ID 不等于公钥指纹。密钥轮换不自动新建 Node，但须受已选定的持钥／再授权流程约束；旧代际不能被历史配置复活。具体 NodeKey 字段取决于尚未定版的认证握手。

### AddressAllocation

Mesh 地址是持久分配记录，不是节点自报 State。记录 network、node、地址族、地址和分配生命周期；数据库保证网络内唯一和归属一致。

网络地址池／分配游标作为 IPAM 持久记录，NodeConfiguration 引用实际分配结果。地址池的可编辑范围与回收策略尚待定版，不把一次接口上报当作申请／更改 Mesh IP。

## ConfigurationVersion

每个网络一条线性配置历史，完整版本不可变。保存的内容包括：

| 内容 | 归属 |
| --- | --- |
| `networkId` / `revision` / `parentRevision` | 版本身份与前序 |
| source JSONC | 编辑原文与注释；JSONC 是配置源格式，不是另一份独立配置 |
| 规范化 NetworkConfiguration | 填充已确定默认值与生成参数后的完整有效值 |
| 绑定依据 | 稳定 User 引用、资源代际、材料／规则集版本及默认值版本 |
| 提交元数据 | actor、时间、请求关联、回滚来源和内容摘要 |

groups、tagsOwners、access、topology、routing、节点 hostname／remark／tags／listeners／dialers／exitEnabled、DNS、规则集定义都在版本内。表外不会再有另一份可直接更新的 Tag／Listener 业务配置。

每个历史版本必须固定能够重现其语义的依赖。一个源版本期间规则集可能更新多次，因此回滚还须明确其采用的历史依赖绑定／执行发布；不能仅凭源 revision 重新下载最新 URL。

## 字段归属速查

| 数据 | 唯一来源 |
| --- | --- |
| 用户邮箱、密码哈希、停用状态 | User／账户私有记录 |
| 网络显示名、成员角色、节点归属 | Entity／Membership |
| Node 控制面与数据面公钥、撤销代际 | NodeKey 持久绑定 |
| Mesh IPv4／IPv6 | AddressAllocation |
| 节点规范 hostname、Tag、listener 参数 | NetworkConfiguration |
| 实际 OS hostname、版本、能力、接口／STUN 候选、active links／paths | NodeState |
| 当前报告新鲜度、地区推导、全网连接聚合 | Controller NetworkState 派生视图 |
| NodeConfiguration | 由配置与身份／资源事实生成的执行产物 |
| 用户选择的 routing profile | 持久使用状态，不是管理员配置 |
| saved／applied 最新报告 | State；必要持久 ACK 另保存为回执，不能冒充当前执行 |
| 延迟历史、真实连接事件 | History，保留事件时间和来源 |

Console 的 Node 详情可以组合这些数据，但组合响应不创造第二个可写 Node 模型。State 的接口地址也不覆盖配置中的 declared endpoint 或 IPAM 分配。

## 数据库约束

- 网络内记录和关联均带 `networkId`，使用复合外键／唯一约束防止跨网绑定；不是只靠 handler 的筛选条件。
- UserId、NodeId、MembershipId 等不能互相代用；记录存在也不等于调用者有权限。
- 配置版本唯一、有效 membership 唯一、地址唯一、接入消费唯一等持久不变量在数据库保证。
- 配置中的完整 JSON 引用由提交服务校验，必要身份／资源绑定同事务保存；不为了实现外键把整棵配置拆成可独立修改的行。
- Node 撤销、身份转换、权限变化和受影响执行发布一起提交；回滚仅创建配置新版本。

完整持久记录清单、内存 State 和访问方式见[存储与访问](/v2/storage)。
