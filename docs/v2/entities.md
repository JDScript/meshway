# 数据模型

按存放位置直接阅读字段：

| 位置 | 字段清单 |
| --- | --- |
| 持久数据库 | [User、Network、Node、会话、配置版本、吊销、执行发布与历史](/v2/storage/database) |
| Controller 内存 | [配置缓存、NodeState、NetworkState、订阅、任务与临时状态](/v2/storage/memory) |
| 文件／客户端 | [部署参数、私有密钥、Agent 缓存与 Console 草稿](/v2/storage#local) |

字段页是下一版逻辑结构提案，尚未实施 SQL／API。相同命名类型只有一个定义；API 视图和数据库私有记录分别生成或映射。

## 关系

```text
User ──< Membership >── Network ──< Node
                            └─< ConfigurationVersion
                                  └─ nodes[NodeId]
                                       ├─ identity: User | Tag
                                       ├─ enabled
                                       └─ hostname / listeners / ...
```

Node 记录保存持久接入与网络绑定；**有效 User／Tag 身份和业务启停唯一来自当前 Configuration**。Node 表不再保存 owner、subject、tags 或 enabled 的可写副本。

## NodeIdentity

```json
{ "type": "user", "user": "alice@example.com" }
```

```json
{ "type": "tag", "tags": ["tag:backend"] }
```

该对象位于 `nodes[id].identity`。可读用户引用在配置版本内绑定稳定 UserId／MembershipId；历史引用不会因邮箱复用改指他人。Tag→User 和改绑 User 需要设备登录证明，完整配置保存／回滚同样检查。

## 停用与吊销

| 操作 | 权威字段 | 恢复 |
| --- | --- | --- |
| 停用业务 | `nodes[id].enabled=false` | 新配置／回滚可启用；重新发布许可 |
| 永久吊销整个接入 | `Node.credentialsRevokedAt` | 旧 Node 凭据不可恢复，重新接入建立新身份 |
| 吊销某把密钥 | `NodeKey.revokedAt` | 该密钥不可复活；其他有效凭据下按受控流程换钥 |

enabled=false 的设备仍可认证以同步配置、报告停止进度。enabled=true 也不能覆盖凭据吊销、失效 membership 或 access 拒绝。

## 字段归属速查 {#字段归属速查}

| 字段 | 权威位置 |
| --- | --- |
| User email／passwordHash／disabledAt | 数据库 User |
| Network name、Membership role／revokedAt | 数据库 Network／Membership |
| Node ID／networkId／公钥／永久接入吊销 | 数据库 Node／NodeKey |
| Node identity／enabled／hostname／listener | NetworkConfiguration |
| Mesh 地址 | 数据库 Node；地址池位于 Network（固定地址模型提案） |
| OS hostname／版本／能力／socket／link／path／applied | 内存 NodeState |
| 用户请求的 routing profile | 数据库 RoutingSelection；实际 applied 在 State |
| 真实测量历史 | 数据库 ObservationEvent |

Tag、Group、Listener 是配置内容；Link、Path 是运行记录。SQL 索引和查询投影不产生第二份业务配置源。

协议凭据随完整配置版本保存，永久拒绝依据保存在 CredentialRevocation。listener 创建代际嵌入版本绑定，编译后的节点配置嵌入执行发布；嵌套值不另建立一套业务实体。
