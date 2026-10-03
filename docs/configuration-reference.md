---
title: Configuration Reference
description: Meshway 配置结构、字段及建议默认值。
---

# Configuration Reference

**设计草案，供逐项确认。** 文档按“结构 → 字段 → 示例”组织；新增字段和默认值均为建议，尚非现行 API／Agent 配置格式。当前实现见[协议](/protocol)。

Config 相对静态，完整版本保存到数据库；State 随运行变化。[两者分开发送](/configuration-reference/synchronization)。

## Structure

```jsonc
{
  "groups": {
    "group:dev": ["alice@example.com"]
  },
  "tagsOwners": {
    "tag:backend": ["group:dev"]
  },
  "grants": [
    {
      "id": "dev-to-backend",
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
      "listeners": {
        "mesh-udp": {
          "type": "native_udp",
          "listen": "0.0.0.0",
          "listenPort": 8421
        }
      },
      "dialers": {},
      "exitEnabled": false
    }
  },
  "dns": {
    "magicDNS": true,
    "domain": "mesh.example",
    "servers": {},
    "default": [],
    "routes": {},
    "searchDomains": [],
    "overrideLocal": false
  },
  "connections": [
    { "id": "default", "action": "allow", "a": "*", "b": "*" }
  ],
  "routing": {
    "mesh": [],
    "egress": { "rules": [], "profiles": {}, "assignments": [] }
  },
  "ruleSets": {}
}
```

`<node-id>` 引用已注册 Node。示例以 IPv4 业务说明格式，不表示当前实现已支持完整草案。

## Fields

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `groups` | `Record<string, string[]>` | [用户组](/configuration-reference/identity#groups) |
| `tagsOwners` | `Record<string, string[]>` | [Tag 定义及授予权限](/configuration-reference/identity#tagsowners) |
| `grants` | `Grant[]` | [业务访问授权](/configuration-reference/grants) |
| `nodes` | `Record<NodeId, NodeSettings>` | [节点静态设置](/configuration-reference/nodes) |
| `dns` | `DnsConfiguration` | [DNS](/configuration-reference/dns) |
| `connections` | `ConnectionRule[]` | [承载连接许可／阻断](/configuration-reference/routing#connections) |
| `routing` | `RoutingConfiguration` | [Mesh 与出口选路](/configuration-reference/routing) |
| `ruleSets` | `Record<string, RuleSet>` | [规则集](/configuration-reference/rule-sets) |

前六个字段沿用用户给出的基线；后三个是为已有连接／路由需求补充的字段提案。

## Reading

- 配置 listener：[监听器](/configuration-reference/listeners)；配置出站承载：[拨号器](/configuration-reference/dialers)。
- Agent 收到什么：[NodeConfiguration](/configuration-reference/node-configuration) 与 [State Reference](/state-reference)。
- 版本、回滚、增量：[同步与分发](/configuration-reference/synchronization)。
- 配置变大、查询和外部 API：[存储与查询](/configuration-reference/storage-api)。
