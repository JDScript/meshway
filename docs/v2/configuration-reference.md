---
title: Configuration Reference
description: Meshway 网络配置、默认行为和协议字段参考。
---

# Configuration Reference

本参考描述**开发中的下一版配置格式**，按公开文档维护。尚未发布的字段不会因此成为现行 API；可用范围见[承载与支持状态](/v2/configuration-reference/transports)，现行接口见[协议](/protocol)。

Config 相对静态，完整版本保存到数据库；State 随运行变化。[两者分开发送](/v2/configuration-reference/deployment)。

| 配置 | 回答的问题 |
| --- | --- |
| `access` · 访问权限 | 谁可以访问什么？ |
| `topology` · 组网规则 | 图上允许存在哪些边，如何约束建连？ |
| `routing` · 流量路由 | 获准的业务在图上经过哪里？ |

topology 是允许的图，不是当前连接状况；实际通道和路径分别由 [State.links / State.paths](/v2/state-reference) 表达。

## Structure

```jsonc
{
  "addressPools": {
    "ipv4": ["100.80.0.0/16"],
    "ipv6": ["fd42:7a65:8800::/48"]
  },
  "defaults": {
    "access": "deny",
    "topology": "allow"
  },
  "groups": {
    "group:dev": ["alice@example.com"]
  },
  "tagsOwners": {
    "tag:backend": ["group:dev"]
  },
  "access": [
    {
      "remark": "开发组访问后端 HTTPS",
      "src": ["group:dev"],
      "dst": ["tag:backend"],
      "ip": ["tcp:443"]
    }
  ],
  "nodes": {
    "<node-id>": {
      "hostname": "backend-1",
      "remark": "开发后端",
      "identity": { "type": "tag", "tags": ["tag:backend"] },
      "enabled": true,
      "addresses": {
        "ipv4": "100.80.0.10",
        "ipv6": "fd42:7a65:8800::10"
      },
      "listeners": {
        "mesh-udp": {
          "type": "native_udp",
          "listen": "0.0.0.0",
          "listenPort": 8421
        }
      },
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
  "topology": [],
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
| `addressPools` | `AddressPools` | [可修改的 Mesh 地址段与分配规则](/v2/configuration-reference/addressing) |
| `defaults` | `Defaults` | [未命中规则时的行为与默认出站配置](/v2/configuration-reference/defaults) |
| `groups` | `Record<string, string[]>` | [用户组](/v2/configuration-reference/identity#groups) |
| `tagsOwners` | `Record<string, string[]>` | [Tag 定义及授予权限](/v2/configuration-reference/identity#tagsowners) |
| `access` | `AccessRule[]` | [访问权限](/v2/configuration-reference/access) |
| `nodes` | `Record<NodeId, NodeSettings>` | [节点静态设置](/v2/configuration-reference/nodes) |
| `dns` | `DnsConfiguration` | [DNS](/v2/configuration-reference/dns) |
| `topology` | `TopologyRule[]` | [组网规则](/v2/configuration-reference/topology) |
| `routing` | `RoutingConfiguration` | [流量路由](/v2/configuration-reference/routing) |
| `ruleSets` | `Record<string, RuleSet>` | [规则集](/v2/configuration-reference/rule-sets) |

## Conventions

- `remark` 是描述文本，不是标识；可重复、可为 `""`，不能为 `null`，省略时规范化为 `""`。
- access／topology／Mesh 路由／出口规则不包含 `id`，统一使用 remark。诊断使用本版本的字段路径，例如 `/access/0`；修改使用配置 revision 保证基准一致。
- listener key 标识节点内入站资源；dialers 的 key 直接是协议名。两者都不是 `tag:backend` 这类授权 Tag。
- 对象字段的缺省、空数组和 `null` 只按所属字段解释，不自动互换。

## Reading

- 网络改段、Node 显式改址／重分配：[Addressing](/v2/configuration-reference/addressing)。
- 配置 listener：[监听器](/v2/configuration-reference/listeners)；配置出站承载：[拨号器](/v2/configuration-reference/dialers)。
- 通用字段：[Listen](/v2/configuration-reference/shared/listen)、[Dial](/v2/configuration-reference/shared/dial)、[TLS](/v2/configuration-reference/shared/tls)、[WebSocket](/v2/configuration-reference/shared/websocket)。
- 字段怎样复用、默认值怎样覆盖：[字段复用与默认继承](/v2/configuration-reference/inheritance)。
- 整网草稿、强类型操作、应用前数量／对照与严格 CAS：[编辑与应用对照](/v2/configuration-reference/editing)。
- Agent 收到什么：[NodeConfiguration](/v2/configuration-reference/node-configuration) 与 [State Reference](/v2/state-reference)。
- 版本、回滚、增量：[同步与应用](/v2/configuration-reference/deployment)。
- 配置变大、查询和外部 API：[存储与查询](/v2/storage)。
