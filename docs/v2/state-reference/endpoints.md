# Endpoint Candidates

配置声明、接口地址和 STUN 结果统一为 endpoint 候选，用 `origin.source` 保留来源。**来源说明地址从哪里来，不保证对某个 peer 可达，也不替代节点认证。**

## Structure

以下为 UDP listener 的候选格式示例，地址与时间用于说明编码，不代表实际测量。提交时须匹配真实资源和有效配置：

```json
[
  {
    "address": "198.51.100.42:18443",
    "origin": { "source": "configured", "configPath": "/nodes/<node-id>/listeners/mesh-udp/endpoints/0" },
    "observedAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-03T12:00:30Z"
  },
  {
    "address": "192.168.1.20:8421",
    "origin": { "source": "interface", "interface": "eth0" },
    "observedAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-03T12:00:30Z"
  },
  {
    "address": "198.51.100.42:40001",
    "origin": { "source": "stun", "observer": "stun.example.net:3478" },
    "observedAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-03T12:00:30Z"
  }
]
```

SocketState 提供 network、resource、generation；socketId 来自 map key，instanceId 来自上报 envelope。各 origin 分支分别包含 configPath、interface 或 observer。

## Sources

| origin.source | 含义 | 校验与失效 |
| --- | --- | --- |
| `configured` | 管理员在 Server 配置中明确提供的 endpoint | 必须匹配当前资源配置；修改／删除配置立即使旧声明失效 |
| `interface` | 节点从本机接口取得、该 socket 实际可使用的地址 | 接口、地址、路由或 socket 变化后重新核对；不把所有网卡地址无条件发布 |
| `stun` | 同一实际 UDP socket 的 STUN 映射观测 | 核对事务／来源／地址族，按原观测期限和 socket 代际失效 |

Node 报告配置项与当前资源的关联；Server 使用已保存配置核对，任意自报地址不能标成 configured 绕过校验。

## Config 与 State

配置声明本身保存在 Config。上例 configured 的时间表示“这条声明与当前运行资源的关联何时核对”，不是声明了公网可达。此运行副本过期不会删除源配置；静态候选仍可从有效 NodeConfiguration 获取，不能要求先有 State 才能首次尝试已配置入口。

configured 的地址可以是配置中的域名加端口。interface／stun 必须是实际 IP 加端口，IPv6 使用括号形式。域名由拨号端经受保护 resolver 解析，保留其配置来源和解析有效期，不把解析出的 IP 写回管理员配置。

所有来源都关联准确的资源和外层协议，UDP 候选不能升级为 TCP／VLESS 可达性。拨号 socket 的接口地址也不自动变成固定 listener，只能按相应协调方式使用。

## Interface candidates

- wildcard bind `0.0.0.0`／`::` 不是远端可拨地址，不能直接公告。
- 只枚举与该 socket 地址族、实际 bind、接口限制及系统保护通路相容的地址。
- 排除 Meshway 自身 TUN、loopback、未指定、多播等不适合作为远端候选的地址。
- LAN／其他 underlay 接口地址只是候选；两端地址落在同一个 RFC1918 前缀不证明它们在同一 LAN。
- IPv6 link-local 需要正确的本地链路作用域，不能把远端的接口 zone 原样当作本机接口使用。

接口名等诊断详情可留在自身／管理视图，peer 查询只给其建连所需信息，保留来源而不暴露所有本机接口。

## 使用候选

使用前检查 access／topology／routing、承载限制、资源代际和地址范围。来源不是信任等级，也不定义严格串行拨号顺序；configured 的存在不表示禁止其他来源。

实际连接必须认证为正确 Node。候选选择须受建连预算约束，不能因为来源变化无条件拆换健康通道。调度与 NAT 能力范围见 [Node](/v2/node)。

## 去重与验证

同地址可能同时由 configured／interface／STUN 产生。拨号按 peer、资源、协议、代际和规范地址去重，保留各来源证据；不重复发起相同连接，也不把一个来源的较长期限借给另一个已经过期的观测。

peer 验证记录属于实际发起方及对应 link，绑定使用的候选、时间和通道。B 自报自己有一个 STUN endpoint，不能直接把它标成“对所有节点 verified”；对 A 的成功也不能当作对 C 的可达证明。

## GeoIP／ASN

Server 从这些可见 IP 派生 [ipInfo](/v2/state-reference/ip-info)，包含独立的 GeoIP／ASN 结果、版本和查询时间。原始 Node 报告不接收可信地理／ASN 自声明。

公网 configured 地址可展示其入口网络信息，但不自动作为设备位置；节点 location／自动地区方案仅用合格本机观测。私网、Mesh 地址、缺数据和过期观测分别表达，不填 AS0 或虚构坐标。

## Peer 投影

`PeerEndpoint.candidate` 使用独立的 `PeerCandidate` 类型，只包含 address、source、observedAt、expiresAt；它不包含原始 interface／observer 详情。这里的 source 是裁剪后的来源枚举，不能与原始 `EndpointCandidate.origin` 混用。两种类型均没有 validation 字段，认证通道证据由 links 单独表达。
