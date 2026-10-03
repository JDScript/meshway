# Nodes

以已注册 Node ID 为 key 的静态设置草案。hostname 是配置的权威值；运行观测见 [State](/v2/state-reference)。

## Structure

```json
{
  "nodes": {
    "<node-id>": {
      "hostname": "backend-1",
      "remark": "开发后端",
      "identity": { "type": "tag", "tags": ["tag:backend"] },
      "enabled": true,
      "listeners": {},
      "exitEnabled": false
    }
  }
}
```

## Fields

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `hostname` | `string` | 首次接入时生成 | 网络内唯一的规范 DNS 名称 |
| `remark` | `string` | `""` | 自由备注，不影响身份、DNS 或授权 |
| `identity` | `NodeIdentity` | 接入时写入 | User／Tag 有效授权身份的唯一配置源 |
| `enabled` | `boolean` | `true` | 是否启用本节点业务执行；停用可随配置回滚 |
| `listeners` | `Record<string, Listener>` | `{}` | [本节点入站配置](/v2/configuration-reference/listeners) |
| `dialers` | `DialerSet` | 省略则继承 | 显式 `{}` 禁用主动建连；非空集合替换默认 |
| `exitEnabled` | `boolean` | `false` | 管理员允许此节点承担出口；不是执行就绪状态 |

## hostname

建议使用单个 DNS label：小写字母、数字及中间连字符，1–63 个字符，首尾为字母或数字。手工改名重名时拒绝保存；首次接入可规范化 OS hostname 并生成唯一后缀。

```text
hostname: backend-1
dns.domain: mesh.example
FQDN: backend-1.mesh.example
```

Node 查询投影此名称，不再有第二个独立可写的显示名。Agent 上报的 OS hostname 留在 State；自动跟随 OS hostname 的行为尚待确认。

## identity

采用互斥判别对象：

```json
{ "type": "user", "user": "alice@example.com" }
```

```json
{ "type": "tag", "tags": ["tag:backend"] }
```

| 分支 | 字段 | 类型 | 约束 |
| --- | --- | --- | --- |
| user | `type` | `"user"` | 固定值 |
| user | `user` | `string` | 本网络有效用户的可读邮箱，版本绑定稳定 UserId／MembershipId |
| tag | `type` | `"tag"` | 固定值 |
| tag | `tags` | `string[]` | 非空、不重复，引用 tagsOwners |

user 分支不接受 tags，tag 分支不接受 user。Node 数据库记录不保存另一份 owner／subject／tags。接入批准与身份切换负责验证谁有权写入此字段；Tag→User 或改绑其他 User 仍须设备完成相应用户登录，整份配置保存／回滚也不能绕过。

## enabled

`false` 撤回该节点的业务执行许可，停止依赖它的路径；凭据仍可认证以同步配置、报告停用进度并接收后续启用。改回 `true` 时重新校验身份、授权与路径，不恢复旧流资格。

永久接入吊销／密钥吊销属于认证记录，不能由 enabled=true 或配置回滚清除。当前状态可投影为“配置已启用，但凭据已吊销”，具体来源保留，不能把两者合成一个可写状态。

## 生命周期

建议注册事务创建默认 `nodes[id]` 配置项、写入批准的 identity 并生成配置版本。JSON 中新增一个 key 不会创建节点凭据；删除配置项停止业务发布，不等于永久吊销凭据，删除行为的管理体验需进一步确认。

Node 长期认证已选定本地持久密钥身份。接入批准、公钥与实体绑定走[注册流程](/v2/api/authentication#node-的三层凭据)，私钥不进入这份静态配置。

Tag 身份不继承 User／Group 权限，原创建用户离网后仍保留；最后一个 Tag 不能直接清空，须经设备用户登录完成身份转换。详见[有效身份](/v2/configuration-reference/identity#有效授权身份)。默认初始化不写空 dialers，以采用网络默认出站集合。

回滚恢复历史配置，仍校验 Node 凭据、用户成员关系和身份切换证明；不会复活已吊销接入。缺失 Node 引用需要显式解决。

`remark`、`exitEnabled` 及上述默认行为是本稿补充提案。
