# DNS

网络 DNS 配置草案。节点名称来自 `nodes[id].hostname`。

## Structure

```json
{
  "magicDNS": true,
  "domain": "mesh.example",
  "servers": {
    "public": { "type": "udp", "server": "1.1.1.1", "serverPort": 53 },
    "office": { "type": "udp", "server": "10.20.0.53", "serverPort": 53, "useWithExit": true }
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
| `useWithExit` | `boolean` | `false` | 字段提案：使用出口时仍保留此明确解析器及其 DNS 分流规则 |

## Behavior

- MagicDNS 网络后缀优先交给本地生成的 Node 记录，并自动位于搜索域最前。
- `routes` 采用最长 DNS 标签后缀匹配；`corp.example` 不匹配 `evilcorp.example`。
- 没有使用出口时，`overrideLocal: false` 只强制接管 MagicDNS／split DNS，其他查询保留系统解析。
- 没有使用出口时，`overrideLocal: true` 使用非空 default；空 default 则拒绝配置。未知解析器引用也拒绝。
- private nameserver 的可达路径和访问授权仍需存在，DNS 配置不会自动开放业务访问。

## 使用出口时

**已确认：普通 DNS 默认随有效方案的出口解析，只有明确配置的例外解析器保留。** MagicDNS 仍按节点记录处理；useWithExit=true 的解析器保留其明确的 split/default 用途，不因为用了出口就丢失该规则。

例外不是目的地址的全局放行；解析器流量仍须走明确、获准的网络路径。没有有效出口／解析依赖时报告不可用，不静默落回另一路公共解析器。

多出口方案必须在解析业务域名之前确定无循环的 DNS 路径：能由方案的默认出口唯一确定时直接使用；否则需要方案提供明确的 DNS 出口／路径。需要 DNS 结果才能确定的 IP 规则，不能反过来成为它自身唯一的解析器选择依据。具体方案字段在 API 阶段补齐，不能默认猜一个出口。

Controller／transport／协调的 bootstrap DNS 始终使用受保护系统通路，和业务 DNS 分开。

## 域名执行关联

解析所得地址集合由受控解析／验证流程建立并绑定发布、源范围、名称、地址、TTL；节点原始 State 不能自行扩大出口接受范围。正常分流保留域名上下文，同 IP 域名不具有严格服务隔离保证，见 [Access](/v2/configuration-reference/access#domains)。
