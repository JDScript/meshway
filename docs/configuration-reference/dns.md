# DNS

网络 DNS 配置草案。节点名称来自 `nodes[id].hostname`。

## Structure

```json
{
  "magicDNS": true,
  "domain": "mesh.example",
  "servers": {
    "public": { "type": "udp", "server": "1.1.1.1", "serverPort": 53 },
    "office": { "type": "udp", "server": "10.20.0.53", "serverPort": 53 }
  },
  "default": ["public"],
  "routes": { "corp.example": ["office"] },
  "searchDomains": ["corp.example"],
  "overrideLocal": false
}
```

## Fields

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `magicDNS` | `boolean` | `true` | 解析本网络 Node 名称 |
| `domain` | `string` | 平台为网络分配 | 网络 DNS 后缀；必须可规范化且无冲突 |
| `servers` | `Record<string, DnsServer>` | `{}` | 可引用的上游解析器 |
| `default` | `string[]` | `[]` | 默认解析器 key 列表 |
| `routes` | `Record<string, string[]>` | `{}` | DNS 后缀到解析器 key 列表的映射 |
| `searchDomains` | `string[]` | `[]` | 附加搜索后缀，保留顺序 |
| `overrideLocal` | `boolean` | `false` | 是否接管普通域名解析并使用 default |

### DnsServer

本稿先定义 UDP／TCP，TLS／HTTPS 的具体字段后续按实际实现补充。

| 字段 | 类型 | 建议默认 | 说明 |
| --- | --- | --- | --- |
| `type` | `"udp" \| "tcp"` | 必填 | 上游 DNS 传输 |
| `server` | IP 字符串 | 必填 | 避免解析器自身的引导依赖 |
| `serverPort` | `integer` | `53` | 1–65535 |

## Behavior

- MagicDNS 网络后缀优先交给本地生成的 Node 记录，并自动位于搜索域最前。
- `routes` 采用最长 DNS 标签后缀匹配；`corp.example` 不匹配 `evilcorp.example`。
- `overrideLocal: false` 时只强制接管 MagicDNS／split DNS，其他查询保留系统解析。
- `overrideLocal: true` 时使用非空 default；空 default 则拒绝配置。未知解析器引用也拒绝。
- private nameserver 的可达路径和访问授权仍需存在，DNS 配置不会自动开放业务访问。

这是业务 DNS。Controller／transport／协调的 bootstrap DNS 使用受保护系统通路。出口是否代理普通 DNS、哪些解析器在出口方案中保留，需要与域名路由一起确认，不能默许解析泄漏到另一条路径。
