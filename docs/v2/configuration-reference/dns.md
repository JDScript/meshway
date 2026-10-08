# DNS

网络 DNS 配置。节点名称来自 `nodes[id].hostname`，Mesh 记录地址来自同版 `nodes[id].addresses`。

## Structure

<!-- schema: DnsConfiguration -->
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

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `magicDNS` | `boolean` | `true` | 解析本网络 Node 名称 |
| `domain` | `string` | 平台为网络分配 | 网络 DNS 后缀；必须可规范化且无冲突 |
| `servers` | `Record<string, DnsServer>` | `{}` | 可引用的上游解析器 |
| `default` | `string[]` | `[]` | 默认解析器 key 列表 |
| `routes` | `Record<string, string[]>` | `{}` | DNS 后缀到解析器 key 列表的映射 |
| `searchDomains` | `string[]` | `[]` | 附加搜索后缀，保留顺序 |
| `overrideLocal` | `boolean` | `false` | 是否接管普通域名解析并使用 default |

### DnsServer

解析器传输支持 UDP／TCP。

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `type` | `"udp" \| "tcp"` | 必填 | 上游 DNS 传输 |
| `server` | IP 字符串 | 必填 | 避免解析器自身的引导依赖 |
| `serverPort` | `integer` | `53` | 1–65535 |
| `useWithExit` | `boolean` | `false` | 使用出口时仍保留此明确解析器及其 DNS 分流规则 |

## Behavior

- MagicDNS 网络后缀优先交给本地生成的 Node 记录，并自动位于搜索域最前。
- MagicDNS 预需求发现按请求源 Node 的 Access 权限裁剪：当前配置允许至少一种业务协议／端口访问的节点，其名称／地址可见；不要求目标在线、已有连接或已有 demand。精确名称查询见下节；当前执行发布仍只包含部分路径 peer，不能因名称缺席就给出权威 NXDOMAIN。
- 停用或暂缺执行密钥不影响身份级名称可见性；永久删除／吊销 Node、失效的成员身份不再可见。通过有效 Node 认证的节点可查询自己的当前名称／地址，无需额外自访问规则。名称可见不授予业务访问或执行资格。
- 改址时 DNS、地址准入与路由按同一发布更新；某地址族为 null 则不生成对应 A／AAAA。旧地址复用仍等待受管 DNS 与执行退出窗口，不能只更新 DNS 就宣布切换完成。
- `routes` 采用最长 DNS 标签后缀匹配；`corp.example` 不匹配 `evilcorp.example`。
- 没有使用出口时，`overrideLocal: false` 只强制接管 MagicDNS／split DNS，其他查询保留系统解析。
- 没有使用出口时，`overrideLocal: true` 使用非空 default；空 default 则拒绝配置。未知解析器引用也拒绝。
- private nameserver 的可达路径和访问授权仍需存在，DNS 配置不会自动开放业务访问。

## Node 精确名称查询

`POST /agent-api/v1/dns/query`（生成操作 `queryNodeDns`）使用 Node bearer。请求的 network／source 从已认证身份取得；用户管理 token 不能替代 Node 凭据。

<!-- schema: NodeDnsQueryRequest -->
```json
{"name":"node.mesh.example"}
```

名称必须是精确 FQDN，支持大小写／IDNA 规范化及一个末尾根点；原始 UTF-8 上限 1024 bytes，规范化名称上限 253 ASCII bytes。Node hostname 与网络 domain 拼接后的完整名称也须满足此上限。没有通配、搜索或批量枚举。

| 响应字段 | 含义 |
| --- | --- |
| `requestId` | Server 请求关联 ID |
| `networkId`／`sourceNodeId` | 本次认证所得网络与源节点 |
| `configurationRevision`／`asOf` | 最终读取事务中的配置版本与查询时点 |
| `record` | 必填可空；可见时包含 `nodeId`、规范化 `name`、`addresses` |

未知名称和无权访问的名称都返回 HTTP 200、`record: null`；`magicDNS: false` 同样不返回记录。可见记录某个地址族为 null 时保留该 null，不与不可见记录混同。

读取在同一事务中检查当前 Node 凭据、永久吊销、成员／身份、配置绑定和 Access。它不要求取得 instance，不依赖 State 或现有路径。响应是配置快照，不包含 TTL、租约、安装证明或外部域名的可信 DNS 关联；配置 revision 不代替当前主体资格检查。

## 使用出口时

单出口编译通过 `DnsEgress::ViaExit { exit, service, generation }` 指定普通查询的 Mesh resolver。只要当前方案的 fallback 或 CIDR 规则使用同一个出口及同一组有序 via，普通名称在解析前即选择该出口，不先查询客户端上游再根据答案选择路径。MagicDNS 仍按本地记录／精确发现处理；`useWithExit: true` 的解析器保留最长后缀 split／default 用途。

`DnsServicePermit` 绑定源节点／instance 及 Mesh 源地址、出口 Mesh IPv4 地址的 UDP／TCP 53、完整有序路径、源 profile／capture generation、出口 instance／role、并发／查询时间预算和到期；到期取源 instance 与出口 ready 观测期限的交集。联合编译要求出口已启用、能力观测仍 ready、路径可行且源有外部 Access 资格。缺少 ready／路径／资格分别返回 `exitUnavailable`／`noPath`／`unauthorized`；没有无环单出口方案返回 `exitPlanRequired`。

例外不是目的地址的全局放行；解析器流量仍须走明确、获准的网络路径。没有有效出口／解析依赖时报告不可用，不静默落回另一路公共解析器。

存在歧义的多出口、不同 via、auto 和域名依赖方案继续要求**显式、无循环的 DNS plan**；首版单出口编译不推测这些依赖。出口自己的 egress 必须证明无出口，避免 resolver 再指回 Mesh。该显式 plan 的用户配置字段尚未实现。

客户端 owner 将普通查询生成独立 `MeshResolverRoute`，出口不可达时失败，不进入客户端系统上游重试队列。UDP 截断只可使用同一个 service permit 中独立允许的 TCP；旧查询、过期回答和切换前的输出保留原 ticket／scope 截止。

**运行接线限制：**当前 `ServeExitResolver { permit }` 发布接受的源许可，但生产 SecureNode DNS-purpose ingress／原路径回复、受保护出口上游适配和 Mesh listener 尚未接通。`OwnerDispatcher::validate_scope` 对缺少真实适配的计划拒绝安装，不能据上述编译结果报告 applied。`exit/resolved` 与 `exit/resolv-conf` 尚需整组验收；这两个系统后端的通过状态不得相互替代。

Server 通信／transport／协调的 bootstrap DNS 必须使用受保护系统通路，和业务 DNS 分开。运行实现范围见 [Node](/v2/node)。

Node 的 TTL 0 MagicDNS、REFUSED／NODATA、loopback Service、路由证据及宿主 DNS 安装限制统一见 [DNS 查询 owner](/v2/node/dataplane#dns-查询-owner)。

## 域名执行关联

经出口 resolver 的回答只用于客户端解析，不自动创建 `TrustedDnsAssociation`，也不授予解析地址的业务访问。关联 producer 属于后续 X6：受控解析／验证流程须绑定发布、源范围、名称、地址和 TTL；节点原始 State 不能自行扩大出口接受范围。同 IP 域名不具有严格服务隔离保证，见 [Access](/v2/configuration-reference/access#domains)。
