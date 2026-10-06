# 数据模型与权威来源

## Entity、Config、State

| 层 | 权威内容 | 生命周期 |
| --- | --- | --- |
| Entity／认证记录 | User、Network、Membership、Node 接入、公钥与永久吊销 | 持久身份，不因配置回滚复活 |
| NetworkConfiguration | 管理期望、identity／enabled、地址、协议参数、策略与 DNS | 完整版本保存，恢复形成新 revision |
| NodeConfiguration | 从配置与可信依赖编译的节点参数／执行许可 | 由发布和 digest 绑定，不独立编辑 |
| State | 真实能力、资源、连接、安装结果与观测 | 有来源、实例、序号和期限；不回滚旧运行事实 |

Config 类似 ROM，State 类似 RAM，分别下发；观测历史和恢复依据可以持久化，但不混入管理员配置历史。

```text
User ──< Membership >── Network ──< Node
                            └─< ConfigurationVersion
                                  └─ nodes[NodeId]
                                       ├─ identity: user | tag
                                       ├─ enabled / addresses
                                       └─ hostname / listeners / ...
```

## 配置模型 {#configuration}

`access` 定义业务端点权限，`topology` 定义允许的连接图与建连约束，`routing` 定义获准业务的路径。运行中的 links／paths 属于 State。完整字段见[Configuration](/v2/configuration-reference)，执行分工见[按需许可](/v2/execution)。

Node 的有效身份唯一位于 `nodes[id].identity`，格式见 [NodeIdentity](/v2/configuration-reference/nodes#identity)。可读邮箱引用按用途绑定稳定 UserId／MembershipId；User／Tag 身份的授权与改绑规则见[有效授权身份](/v2/configuration-reference/identity#有效授权身份)。

## 字段归属速查 {#字段归属速查}

| 内容 | 权威来源 |
| --- | --- |
| User 邮箱、密码哈希、账户停用 | 私有 User 记录 |
| Network 名称、成员角色／离网 | Network／Membership |
| Node ID、网络绑定、公钥、永久接入吊销 | Node／NodeKey |
| Node identity、enabled、规范 hostname、listeners | 完整 Configuration |
| 地址池与期望分配 | addressPools／nodes[id].addresses |
| 旧地址安全复用下界 | AddressRetirement；不是第二份当前分配 |
| 实际 OS hostname、socket、link、path、applied | 有期限的 NodeState |
| 用户选择的方案 | RoutingSelection；实际生效在 State |
| 测量／管理历史 | ObservationEvent／ActivityEvent，分开记录 |

Tag、Group、Listener 是配置内容，Link、Path 是运行记录。索引、缓存和 API 投影没有独立写入权，不另建同名实体。

## 停用、删除与恢复

`enabled=false` 停止业务但保留获准控制同步，配置恢复可重新启用。永久删除 Node 同事务吊销接入、移除配置并保留必要退出／历史依据；重新接入建立新身份，不提供回收站。某把密钥永久吊销后也不能由旧配置恢复。

协议凭据随完整配置保存，永久失效依据独立存放；listener 创建代际和资源绑定固定在版本中。改址不改变 Node ID／密钥，旧地址复用仍须等待许可及受管缓存退出。详见[身份](/v2/configuration-reference/identity)、[地址](/v2/configuration-reference/addressing)和[权限](/v2/permissions)。
