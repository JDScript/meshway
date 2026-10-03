# Nodes

以已注册 Node ID 为 key 的静态设置草案。hostname 是配置的权威值；运行观测见 [State](/state-reference)。

## Structure

```json
{
  "nodes": {
    "<node-id>": {
      "hostname": "backend-1",
      "remark": "开发后端",
      "tags": ["tag:backend"],
      "listeners": {},
      "dialers": {},
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
| `tags` | `string[]` | `[]` | Tag 分配，引用 `tagsOwners` 中的定义 |
| `listeners` | `Record<string, Listener>` | `{}` | [本节点入站配置](/configuration-reference/listeners) |
| `dialers` | `Record<string, Dialer>` | `{}` | 空值使用 [defaults.dialers](/configuration-reference/defaults#dialers)；非空替换默认集合 |
| `exitEnabled` | `boolean` | `false` | 管理员允许此节点承担出口；不是执行就绪状态 |

## hostname

建议使用单个 DNS label：小写字母、数字及中间连字符，1–63 个字符，首尾为字母或数字。手工改名重名时拒绝保存；首次接入可规范化 OS hostname 并生成唯一后缀。

```text
hostname: backend-1
dns.domain: mesh.example
FQDN: backend-1.mesh.example
```

Node 查询投影此名称，不再有第二个独立可写的显示名。Agent 上报的 OS hostname 留在 State；自动跟随 OS hostname 的行为尚待确认。

## 生命周期

建议注册事务创建默认 `nodes[id]` 配置项并生成配置版本。JSON 中新增一个 key 不会创建节点凭据；删除配置项也不等于撤销身份。未提供有效节点配置时不发布可执行业务配置，删除行为的管理体验需进一步确认。

回滚恢复历史静态设置，仍校验 Node 是否有效；不会复活已撤销身份。缺失 Node 引用需要显式解决。

`remark`、`exitEnabled` 及上述默认行为是本稿补充提案。
