# NodeConfiguration

Server 为一个 Node 编译的完整执行配置，包含实际协议参数、所需材料、可信身份及获准业务范围。它是网络配置和固定依赖的产物，不由管理员单独编辑。

Node 收到完整目标后独立校验、私有保存并安装。saved 不代表已安装；应用进度、实例和执行 lease 见[配置应用](/v2/configuration-reference/deployment)。运行能力范围见 [Node](/v2/node)。

## Structure

精确结构见[生成契约](/v2/contracts/openapi.json)中的 `NodeConfiguration`、`NodeConfigurationResponse` 与 `NodeConfigurationDelta`。查询响应区分 full／delta／unchanged；完整配置先形成，再计算增量。

| 字段 | 含义 |
| --- | --- |
| `networkId`／`nodeId` | 配置的唯一接收网络和节点 |
| `revision`／`publication`／`dependencyDigest` | 完整源版本、执行发布和编译依赖摘要 |
| `enabled`／`hostname`／`identity`／`addresses` | 本节点的确定配置值 |
| `selfKeys` | 本节点可信公钥绑定，不包含身份私钥 |
| `meshAddressPools` | 当前网络完整地址池的精确 capture 元数据，不是 Access 或 peer 可见性授权 |
| `listeners` | key → PreparedListener，本节点所需的完整监听参数 |
| `dialers` | 候选 key → DialCandidate，已解析的获准出站参数 |
| `peers` | NodeId → PeerConfiguration，必要对端的可信身份与配置 |
| `permits` | 需求 ID → PathPermit，绑定业务范围与完整路径的执行许可 |
| `localEgress` | 需求 ID → LocalEgressPermit，仅源节点的精确 Direct／Unmanaged 本机执行绑定 |
| `diagnostics` | 观察 ID → DiagnosticPermit，独立的短期测量许可 |
| `decisions` | 需求 ID → DemandDecision，获准、等待或不可用结果 |
| `dns` | 编译后的 DNS 执行内容 |
| `exitEnabled` | 本节点的配置出口资格，不表示转发已经就绪 |

diagnostics 不进入普通业务 permits。没有对应许可时不能发起或接收新的业务流。所有候选完整路径都须满足 access、topology 和 routing，见[按需执行许可](/v2/execution)。

## Capture 与本机 egress

`meshAddressPools` 精确保留同版 NetworkConfiguration 的完整网络地址池；它使 Node 能捕获尚未产生需求或尚未分配的 Mesh 地址，不等于下发全网 peer 库存，也不授予 Access。编译器把已分配 Mesh IP 绑定到 Node；池内未分配 IP 返回 `unavailable/noPath`，不落入 unmanaged。IPv4-mapped IPv6 使用对应 IPv4 的 Mesh 池分类，不能绕过该边界。

`LocalEgressPermit` 包含 `demandId`、`source`、`target`、`protocol`、`port`、`profile`、`action`（`direct`／`unmanaged`）和 `expiresAt`，仅发布到 source。它是实际本机路由的精确证据，不是 Mesh PathPermit，也不授予中转／出口资格。消费方仍检查 publication、当前执行快照和完整业务绑定；DNS 的密封路由消费见 [resolver 路由](/v2/node/dataplane#service-与-resolver-路由)。

`decisions` 中单独的 `direct`／`unmanaged` 或需求 HTTP 回应仅为反馈，永不直接变成 SocketPermission。当前 daemon 仅接受由 DNS owner 持有的 resolver localEgress（见 [resolver 路由](/v2/node/dataplane#service-与-resolver-路由)）；其他非空 localEgress 按[安装门槛](/v2/node/runtime#安装范围与状态)拒绝。

## DNS 执行内容

`DnsExecution` 包含 `settings`、按源 Access 裁剪的 `meshHosts`、可信 `associations`、当前 `profile` 和 `egress`。`egress` 的判别联合为：

| 值 | 含义 |
| --- | --- |
| `{"type":"noExit"}` | 普通查询用本机系统 resolver：没有经出口的名称规则，fallback 也不经出口（只有 IP 规则的出口不影响 DNS）；不授予上游 socket |
| `{"type":"unavailable","reason":"exitPlanRequired"}` | 名称规则或出口 fallback 不止一个出口，没有单一出口方案（保存时即拒绝；节点缺规则集 artifact 时也会出现），见 [DNS · 使用出口时](/v2/configuration-reference/dns#使用出口时) |
| `viaExit` | `exit`、`service: DnsServicePermit`、`generation` 指定单出口 resolver；仍需真实 Mesh 适配才能安装 |
| `serveExitResolver` | `permit: DnsServicePermit[]` 为该出口接受的源许可；不自行认证 socket 来访者 |

`unavailable.reason` 还可为 `exitUnavailable`、`noPath` 或 `unauthorized`。完整字段、选择、联合许可和当前接线限制统一见 [使用出口时](/v2/configuration-reference/dns#使用出口时)。

出口可能性保守检查公共规则、profile 与 auto 候选中的全部出口选择，包括业务 first-match 中被遮蔽的分支；不靠一次匹配结果推定无出口。多出口的显式无循环计划尚未有配置字段，产品规则见 [DNS](/v2/configuration-reference/dns#使用出口时)。

## 参数来源 {#sources}

| 参数 | NetworkConfiguration 或可信依赖中的来源 | 下发边界 |
| --- | --- | --- |
| 本机 bind／监听端口 | 本节点 listener | 本机监听参数 |
| 目标 host／port 与 source | 对端可选 listener.endpoints（configured）与 State 本机地址（local） | 获准出站候选，configured 优先 |
| VLESS UUID／Trojan password | listener 显式值或已保存的生成值 | 监听端与获准拨号端使用相同材料 |
| TLS 证书／私钥 | 管理员提供或平台生成 | 私钥只发给监听端；拨号端取得验证材料 |
| SNI、WS path／Host | 对端 listener 设置 | 相容的客户端参数 |
| 拨号优先级／timeout／额外 headers | 本节点 dialers 或 defaults | 本地拨号参数 |
| peer 公钥 | 已认证 Node 的公钥绑定 | 固定用途与代际 |
| Mesh 地址 | 同版 nodes[id].addresses | 地址准入、DNS 和路径使用同一绑定 |
| 业务许可／路径／DNS | 完整配置及已验证的执行依赖 | 仅本节点所需范围 |

管理员修改协议参数后，Server 重新生成受影响的切片。只读查询／重发不重新生成密码；完整源版本保存确定材料，发布固定运行依赖。

## 示例：A 拨 B 的 VLESS listener {#example}

配置节选。A、B 必须已注册，且存在允许该业务与路径的 access／topology／routing：

```json
{
  "nodes": {
    "<node-a>": {
      "dialers": { "vless": {} }
    },
    "<node-b>": {
      "listeners": {
        "public-ws": {
          "type": "vless",
          "listen": "0.0.0.0",
          "listenPort": 443,
          "endpoints": [{ "host": "edge.example.com", "port": 8443 }],
          "stream": "websocket",
          "tls": { "serverName": "edge.example.com" },
          "websocket": { "path": "/meshway", "host": "edge.example.com" }
        }
      }
    }
  }
}
```

Server 补全并保存 B 的 UUID／TLS 材料，再向 A 编译对应的 DialCandidate。A 取得目标入口、UUID、TLS 验证材料和 WS path／Host；B 取得完整监听参数及服务端私钥。A 不取得 B 的私钥。

源配置的 dialers 以协议为 key；执行配置的 dialers 以具体候选为 key。候选由 Server 编译，不是需要管理员维护的连接表。两份切片属于一致发布，各自 digest 不必相同。

## Config 与 State

管理员声明的额外 endpoint 和协议参数属于 Config。本机地址、STUN 映射、实际通道和 RTT 属于 State。Server 把当前 listener 的 local 地址加入已获准 DialCandidate，流式承载与 Native UDP 均可使用；STUN 继续走 Native UDP 协调穿透。动态地址只改变已获准 peer 的到达方式，不能替换身份、UUID、TLS 信任或业务许可；字段和上限见[本机 listener 候选](/v2/state-reference/endpoints#本机-listener-候选)。

Mesh 地址修改属于 Config，不能用 endpoint State 改写。接收方须校验对应发布中的源／目的地址绑定；同一 IP 转交其他 Node 不继承旧会话资格，见[改址](/v2/configuration-reference/addressing#改址与安全切换)。
