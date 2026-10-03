# Groups 与 Tags

配置草案；类型、默认值和限制供本轮确认。

## Structure

```json
{
  "groups": {
    "group:dev": ["alice@example.com", "bob@example.com"]
  },
  "tagsOwners": {
    "tag:backend": ["group:dev", "alice@example.com"]
  }
}
```

## groups

**类型：** `Record<string, string[]>` · **建议默认：** `{}`

| 部分 | 说明 |
| --- | --- |
| key | 以 `group:` 开头的组名 |
| value | 本网络 User 的邮箱列表；按集合处理 |

建议首版只支持直接用户成员，不允许 group 嵌套。组成员通过 User 的设备归属匹配节点，不维护第二份“组里的节点列表”。

未知用户／组引用在保存时报告。邮箱变更时如何更新引用仍需确认，不能静默匹配到另一个用户。

## tagsOwners

**类型：** `Record<string, string[]>` · **建议默认：** `{}`

| 部分 | 说明 |
| --- | --- |
| key | `tag:` 开头的标签名，如 `tag:backend` |
| value | 有权给节点赋予该标签的用户邮箱或 `group:` |

Tag 在这里定义，在 `nodes[id].tags` 中分配。没有独立 Tag ID 或可单独修改的 Tag 数据源。

建议空 owner 列表只允许网络 Owner／Admin 赋予该标签；这是待确认的权限规则。编辑整个配置仍须检查网络管理权限，不能因为是某 Tag 的 owner 就修改所有配置。

保留用户选用的 `tagsOwners` 拼写；Tailscale 的字段是 `tagOwners`，本稿不同时提供两个 alias。

## Selectors

以下是 access、topology 和 routing 可复用的选择器提案；各字段可接受的子集由所属规则定义。

| 值 | 含义 |
| --- | --- |
| `*` | 本网络中的所有节点，不是任意公网目的地址 |
| `alice@example.com` | 该用户拥有的节点 |
| `group:dev` | 组内用户拥有的节点 |
| `tag:backend` | 配置中带此 Tag 的节点 |
| `node:<node-id>` | 指定 Node |
| `autogroup:internet` | 公网目的地，仅用于业务目的授权 |

Tag 是否像 Tailscale 一样替代用户身份，还是作为用户设备上的授权标签，**尚待确认**；不会因添加 `tags` 字段自动解除 User 归属。
