# Resources 与 Links

listener 是配置资源，socket 是运行资源，endpoint 是带来源的候选。以下类型属于 [NodeState](/v2/state-reference/node)，精确字段见[生成契约](/v2/contracts/openapi.json)。

## ListenerState

`listeners` 以配置中的 listener key 为索引。

| 字段 | 含义 |
| --- | --- |
| `phase` | preparing／listening／stopped／failed |
| `socketIds` | 指向同一报告中的实际 sockets |
| `loadedCertificateSha256` | 实际加载证书的公开指纹；非 TLS 或没有结果时为 null |
| `problem` | 类型化失败原因或 null |
| `netmap` | 显式 NetmapRef；省略／null 继承报告的 configuration.applied |

listening 只说明本机监听成功，不证明公网可达。Linux Node 对正在重试绑定的 listener 报告 `phase: failed`、空 `socketIds` 与 `problem.code: resource_unavailable`（`resource` 为该 listener），绑定后改为 listening，见 [listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)。安装中资源须显式引用其目标，不能冒充当前 peer 可用入口。listener key 相同不代表材料、切片或运行代际相同。

## SocketState

`sockets` 的 map key 是实例内 socket 标识。以下是完整 SocketState 的格式示例，地址与时间不代表实际测量；实际提交须有相容 applied 配置及真实资源：

<!-- schema: SocketState -->
```json
{
  "resource": { "kind": "listener", "key": "mesh-udp" },
  "netmap": null,
  "generation": 3,
  "network": "udp",
  "boundAddress": "0.0.0.0:8421",
  "endpoints": [
    {
      "address": "198.51.100.42:40001",
      "origin": { "source": "stun", "observer": "stun.example.net:3478" },
      "observedAt": "2026-10-03T12:00:00Z",
      "expiresAt": "2026-10-03T12:00:30Z"
    }
  ]
}
```

| 字段 | 含义 |
| --- | --- |
| `resource` | `{kind: listener | dial | coordination, key}`，关联准确资源 |
| `netmap` | 显式目标；省略／null 继承 applied，安装阶段必须显式提供 |
| `generation` | 本资源代际，不能只按相同端口认作同一 socket |
| `network` | 实际外层 udp／tcp，不由内层业务协议推测 |
| `boundAddress` | 内核实际 bind 地址与端口 |
| `endpoints` | EndpointCandidate 数组，保留 origin 和原期限 |

标识绑定实例、资源、generation 和 socketId；任何更替都不能借用旧认证或观测。出站 socket 不冒充 listener。

STUN 只表示映射观测。对某 peer 的认证成功不证明其他 peer 可达，UDP 映射不能证明 TCP／VLESS 可达。来源与 peer 裁剪见 [Endpoint Candidates](/v2/state-reference/endpoints)。

## LinkState {#links}

`links` 以 PeerNodeId 为 key，每个 peer 可有多个通道。

| 字段 | 含义 |
| --- | --- |
| `linkId` | 实际握手绑定的非秘密通道标识 |
| `transport` | 实际使用的承载 |
| `initiatorNodeId` | 主动建连方，不代表业务发起方向 |
| `socketId` | 本次报告中承载该 link 的 socket |
| `phase` | preparing／authenticated／closed／failed |
| `businessDependencies` | 当前业务依赖数量；不是字节计数或即时速率 |
| `observationIds` | 使用该 link 的诊断任务，不计作业务依赖 |
| `observedAt`／`expiresAt` | 原观测时间与期限 |
| `since` | 已知的状态起始时间，未知为 null |

报告节点必须是通道一端。A 不能报告 B—C 通道；两端 socketId 相同也不能用于配对。authenticated 可以空闲，businessDependencies 大于零不意味着此刻持续传输字节。

Server 聚合为 [AggregatedLink](/v2/state-reference/network#links)，区分 oneSided／matched／conflicting／expired。双侧匹配要求同一通道、身份、承载与相容执行上下文；不自动生成另一端报告，也不把自报告当成 Server 独立完成握手的证明。

## PathState {#paths}

`paths` 以运行路径标识为 key（Node 用 circuit 标识的十六进制），每份报告最多 64 条（`STATE_MAX_PATHS`）。Node 报告它参与的每条已建立业务 circuit（Mesh 与出口业务；诊断和出口 DNS circuit 不报告），按 circuit 标识顺序取前 64 条；circuit 集合变化即触发新报告。

| 字段 | 含义 |
| --- | --- |
| `nodes` | 实际有序完整 Node 路径，origin 在前、终点在后 |
| `netmap` | 本路径所在的 applied netmap |
| `reporterRole` | origin（首节点）／relay（中间节点）／target（Mesh 终点）／exit（出口终点） |
| `evidence` | selected／sent／peerConfirmed／delivered |
| `observedAt`／`expiresAt` | 原观测时间与期限 |

报告者必须参与路径，只报告自己能观察的阶段：端点在端到端握手完成后报告 `peerConfirmed`，中继在双向转发握手后报告 `sent`。中转看到密文不代表目标已交付明文；selected、sent 与 delivered 不能合为“路径已验证”。Server 校验：2–16 个不重复节点、报告者位置与角色一致、其他节点都在本节点 netmap 投影内、`netmap` 为报告的 applied，否则拒绝整份报告。

有业务流量的承载 link `businessDependencies` 大于零（AggregatedLink `inUse: true`），同时其两端与途经节点各报告一条对应 path；`inUse` 只说明该通道承载着依赖，不给方向，方向取 path 的节点顺序。

Server 保留原报告者并聚合为 AggregatedPath。不能从当前 links 拼出路径后称它正在承载业务，也不能相加邻接 RTT 伪造端到端测量。主动测量见[实时观察](/v2/state-reference/realtime)。
