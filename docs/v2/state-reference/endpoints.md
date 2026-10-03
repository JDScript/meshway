# Endpoint Candidates

将配置声明、接口地址和 STUN 结果统一为 endpoint 候选，用 `source` 保留来源。**来源说明地址从哪里来，不保证对某个 peer 可达，也不替代节点认证。** 以下为下一版提案。

## Structure

下面是一个已应用配置的 UDP listener 所关联的候选节选：

```json
[
  {
    "address": "198.51.100.42:18443",
    "source": "configured",
    "configPath": "/nodes/<node-id>/listeners/mesh-udp/endpoints/0",
    "validation": "declared",
    "observedAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-03T12:00:30Z"
  },
  {
    "address": "192.168.1.20:8421",
    "source": "interface",
    "interface": "eth0",
    "validation": "observed",
    "observedAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-03T12:00:30Z"
  },
  {
    "address": "198.51.100.42:40001",
    "source": "stun",
    "observer": "stun.example.net:3478",
    "validation": "observed",
    "observedAt": "2026-10-03T12:00:00Z",
    "expiresAt": "2026-10-03T12:00:30Z"
  }
]
```

外层 SocketState 保留实际 network、resource、instance／generation／socketId。各来源使用自己的确定字段：configured 的 configPath、interface 的接口标识、stun 的 observer；不要求所有来源都伪造一个 observer。

## Sources

| source | 含义 | 校验与失效 |
| --- | --- | --- |
| `configured` | 管理员在 Controller 配置中明确提供的 endpoint | 必须匹配当前资源配置；修改／删除配置立即使旧声明失效 |
| `interface` | 节点从本机接口取得、该 socket 实际可使用的地址 | 接口、地址、路由或 socket 变化后重新核对；不把所有网卡地址无条件发布 |
| `stun` | 同一实际 UDP socket 的 STUN 映射观测 | 核对事务／来源／地址族，按原观测期限和 socket 代际失效 |

“self claim”在此采用更明确的 configured 名称，指平台配置中的声明，不新增节点手工维护地址的入口。Node 可以报告配置项与当前资源的关联；Controller 已有原配置，不必靠 Node 重复上传来重建这份权威值。任意自报地址不能标成 configured 绕过核对。

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

## 尝试顺序

建议在**同一个获准 peer／承载的可用候选范围内**，初始来源偏好为：

```text
configured → interface → stun
```

这不是严格串行等待，也不是信任等级：

1. 先应用 access／topology／routing、承载限制、资源代际和地址范围检查。
2. 同配置／代际下已有健康认证通道时，优先复用，不仅因出现更高来源排序就拆换。
3. 对未建立通道按来源偏好发起有界并发／短间隔竞争，避免错误声明或不可达 LAN 地址阻塞其余候选直到长超时。
4. 最终使用须认证为正确 Node；结合显式承载优先级、真实结果和失败退避选择，source 不能无条件压过它们。

例如两端确实在同一 LAN 时，interface 可能比配置的公网转发入口更快；跨网的相同私网前缀则可能完全不可达。已认证且健康的 STUN 直连也不应为了一个未验证声明反复重建。

如果管理员要“只能使用某个入口／协议”，应是显式约束，不把 configured 的存在暗中解释成禁止其他来源。具体竞速时序与最终优选规则仍待确认。

## 去重与验证

同地址可能同时由 configured／interface／STUN 产生。拨号按 peer、资源、协议、代际和规范地址去重，保留各来源证据；不重复发起相同连接，也不把一个来源的较长期限借给另一个已经过期的观测。

peer 验证记录属于实际发起方及对应 link，绑定使用的候选、时间和通道。B 自报自己有一个 STUN endpoint，不能直接把它标成“对所有节点 verified”；对 A 的成功也不能当作对 C 的可达证明。

## GeoIP／ASN

Controller 从这些可见 IP 派生 [ipInfo](/v2/state-reference/ip-info)，包含独立的 GeoIP／ASN 结果、版本和查询时间。原始 Node 报告不接收可信地理／ASN 自声明。

公网 configured 地址可展示其入口网络信息，但不自动作为设备位置；节点 location／自动地区方案仅用合格本机观测。私网、Mesh 地址、缺数据和过期观测分别表达，不填 AS0 或虚构坐标。
