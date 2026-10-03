---
title: Configuration Reference
description: NetworkConfiguration 配置示例与字段参考。
---

# Configuration Reference

配置格式草案。我们直接在本页逐项确认字段；尚非现行 API 格式。

## NetworkConfiguration

```jsonc
{
  "groups": {
    "group:dev": ["alice@example.com"]
  },
  "tagsOwners": {
    "tag:backend": ["group:dev", "alice@example.com"]
  },
  "grants": [
    {
      "src": ["group:dev"],
      "dst": ["tag:backend"],
      "ip": ["tcp:443"]
    }
  ],
  "nodes": {
    "<node-id>": {
      "hostname": "backend-1",
      "remark": "开发后端",
      "tags": ["tag:backend"],
      "listeners": {},
      "dialers": {}
    }
  },
  "dns": {}
}
```

`<node-id>` 是已注册节点的稳定 ID。`grants.ip` 与 `nodes.*.remark` 为本稿补充建议。

## groups

**类型：** `Record<string, string[]>`

用户组。key 以 `group:` 开头，value 是用户邮箱列表。

```json
"groups": {
  "group:dev": ["alice@example.com", "bob@example.com"]
}
```

## tagsOwners

**类型：** `Record<string, string[]>`

定义 Tag 及有权赋予它的用户／用户组。key 以 `tag:` 开头。

```json
"tagsOwners": {
  "tag:backend": ["group:dev", "alice@example.com"]
}
```

节点实际使用的 Tag 写在 `nodes.<node-id>.tags`。

## grants

**类型：** `Grant[]`

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `src` | `string[]` | 发起访问的用户、组、Tag 等选择器 |
| `dst` | `string[]` | 允许访问的目标选择器 |
| `ip` | `string[]` | 协议／端口，如 `*`、`tcp:443`、`udp:53` |

规则只表达允许，取并集、与顺序无关。未匹配的访问拒绝；`grants: []` 不允许业务访问。

## nodes

**类型：** `Record<NodeId, NodeSettings>`

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `hostname` | `string` | 网络内唯一名称，也是节点的 DNS 名称 |
| `remark` | `string` | 备注，不影响 DNS 或身份 |
| `tags` | `string[]` | 分配给节点的 Tag，如 `tag:backend` |
| `listeners` | `Record<string, Listener>` | 本节点的入站承载配置 |
| `dialers` | `Record<string, Dialer>` | 本节点的出站承载配置 |

### listeners

按 listener 名称定义协议、监听地址／端口及相关参数。`Listener` 的具体字段待下一步展开。

### dialers

按 dialer 名称定义出站承载及相关参数。`Dialer` 的具体字段、自动选择与覆盖规则待下一步展开。

`listeners: {}`、`dialers: {}` 在示例中仅占位，空对象的默认行为尚未定稿。

## dns

**类型：** `DnsConfiguration`

网络 DNS 设置。具体字段待展开：MagicDNS、网络后缀、全局 nameserver、split DNS 和 search domains。

---

[配置与状态的设计说明](/network-configuration)
