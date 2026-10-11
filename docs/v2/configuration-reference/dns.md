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
- MagicDNS 的 `netmap.dns.meshHosts` 按源 Node 的 Access 权限裁剪：允许至少一种业务协议／端口访问的节点，其名称／地址可见；不要求目标在线或已有连接。Node 在租约内完全本地解析，未知与不可见名称均返回 REFUSED。
- 停用或暂缺执行密钥不影响身份级名称可见性；永久删除／吊销 Node、失效的成员身份不再可见。通过有效 Node 认证的节点可查询自己的当前名称／地址，无需额外自访问规则。名称可见不授予业务访问或执行资格。
- 改址时 DNS、地址准入与路由按同一 netmap 更新；某地址族为 null 则不生成对应 A／AAAA。旧地址复用仍等待受管 DNS 与执行退出窗口，不能只更新 DNS 就宣布切换完成。
- `routes` 采用最长 DNS 标签后缀匹配；`corp.example` 不匹配 `evilcorp.example`。
- 没有使用出口时，`overrideLocal: false` 只强制接管 MagicDNS／split DNS，其他查询保留系统解析。
- 没有使用出口时，`overrideLocal: true` 使用非空 default；空 default 则拒绝配置。未知解析器引用也拒绝。
- private nameserver 的可达路径和访问授权仍需存在，DNS 配置不会自动开放业务访问。

## Node 精确名称查询

名称记录随 `POST /agent-api/v1/sync` 的签名 netmap 下发；查询本身不调用 Server。Node 在有效 netmap 内按 DNS 标签比较完整名称，并保留请求中的大小写。配置名称使用 IDNA 规范化；Node hostname 与网络 domain 拼接后的完整名称须满足 253 ASCII bytes 上限。

记录中的 IPv4／IPv6 分别可空：已知名称缺少所问地址族返回 NODATA，未知与不可见名称返回 REFUSED。`magicDNS: false` 时按 split／default 规则处理。名称可见性不替代业务准入；租约到期或本机 authority 撤销后，旧记录也不能回答查询。租约和撤销交付边界见 [Netmap](/v2/node/netmap-design)。

## 使用出口时

单出口编译通过 `DnsPlan::ViaExit { service }` 指定普通查询的 Mesh resolver。只要当前方案的 fallback 或 CIDR 规则使用同一个出口及同一组有序 via，普通名称在解析前即选择该出口，不先查询客户端上游再根据答案选择路径。MagicDNS 仍按本地记录处理；`useWithExit: true` 的解析器保留最长后缀 split／default 用途。

`DnsService` 绑定源节点及 Mesh 源地址、出口及其 Mesh IPv4 地址（只作绑定，出口不在该地址监听 DNS）、UDP／TCP 传输、完整有序路径、源 profile／capture generation、出口 role generation、并发和查询时间预算。执行截止来自 netmap 租约；instance 和 State 就绪事实不签发 DNS 权限。编译要求出口已启用并声明能力、路径可行且源有外部 Access 资格。缺少出口能力／路径／资格分别返回 `exitUnavailable`／`noPath`／`unauthorized`；没有无环单出口方案返回 `exitPlanRequired`。

例外不是目的地址的全局放行；解析器流量仍须走明确、获准的网络路径。没有有效出口／解析依赖时报告不可用，不静默落回另一路公共解析器。

存在歧义的多出口、不同 via、auto 和域名依赖方案继续要求**显式、无循环的 DNS plan**；首版单出口编译不推测这些依赖。出口自己的 egress 必须证明无出口，避免 resolver 再指回 Mesh。该显式 plan 的用户配置字段尚未实现。

客户端 owner 将普通查询生成独立 `MeshResolverRoute`；出口能力事实不是 ready、出口不可达或出口回 SERVFAIL 时查询失败，不进入客户端系统上游重试队列。UDP 截断只可使用同一个 service 中独立允许的 TCP；旧查询、过期回答和切换前的输出保留原 ticket／scope 截止。

`ServeExitResolver { services }` 只接受唯一 SecureNode 验证的 DNS-purpose circuit 请求，源身份和完整路径来自不透明的认证请求对象。请求／回复复用 Noise 与逐跳 MAC，回复沿原路径返回；仅有匹配的源 IP／端口不能进入出口 resolver。出口用**出口机器自己的系统 resolver** 解析：systemd-resolved 后端经其代理 stub（需 systemd ≥ 248、`DNSStubListener=yes`）保留按链路／后缀选上游，显式文件后端使用安装前记录的原 nameserver。`overrideLocal: true` 时出口改用配置的 `default` resolver 直接作上游，两种后端相同。未知本地 stub、Mesh 地址或经出口路由的上游使出口查询不可用（只对源回 SERVFAIL 并报告原因，不影响出口自身 DNS），不以公共 DNS 代替。回到出口自身 Meshway DNS 链路的名称（出口锚点已安装的域，例如 MagicDNS 域与出口的 split 后缀）也回 SERVFAIL：出口模式下要用 split 后缀，就在该解析器上标 `useWithExit`，由源节点自己解析。组件与不可用条件见 [出口 resolver](/v2/node/dataplane#exit-resolver)。

| 查询 | 走向 |
| --- | --- |
| MagicDNS 域内名称 | 源节点本地回答，不离开源节点 |
| `useWithExit: true` 解析器的 split 后缀 | 该解析器，按其地址在源节点 egress 分割中的 Direct／Unmanaged 从本机发出；地址被出口路由时 DNS 安装失败，阻断 applied |
| 其他名称（含不带 `useWithExit` 的 split） | 经完整获准路径交给出口的系统 resolver（出口 `overrideLocal: true` 时为其 default resolver）；落在出口自身 Meshway DNS 域内的名称回 SERVFAIL |

IPv4 首版：A 与 AAAA 都原样返回出口 resolver 的真实回答，AAAA 不伪造 NXDOMAIN；受管 IPv6 包仍被源端捕获与准入拒绝。

每条 DNS 消息上限为 4096 bytes，超限请求或回答失败；并发总上限 128，单查询截止不超过 5 秒和原 netmap／请求 guard。相同内容续租不延长已接纳查询的截止，内容变更取消旧查询。缺少适配、必需解析器或系统后端时阻断 applied。系统后端与出口组合的真实验收状态见 [验证范围](/v2/node/validation)，不同组合不能互相替代。

Server 通信／transport／协调的 bootstrap DNS 必须使用受保护系统通路，和业务 DNS 分开。运行实现范围见 [Node](/v2/node)。

Node 的 TTL 0 MagicDNS、REFUSED／NODATA、loopback Service、路由证据及宿主 DNS 安装限制统一见 [DNS 查询 owner](/v2/node/dataplane#dns-查询-owner)。

## 域名执行关联

经出口 resolver 的回答只用于客户端解析，不自动创建 `TrustedDnsAssociation`，也不授予解析地址的业务访问。关联 producer 属于后续 X6：受控解析／验证流程须绑定发布、源范围、名称、地址和 TTL；节点原始 State 不能自行扩大出口接受范围。同 IP 域名不具有严格服务隔离保证，见 [Access](/v2/configuration-reference/access#domains)。
