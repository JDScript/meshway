# Resources 与 Links

下一版 NodeState 的资源结构。**listener 是配置资源，socket 是运行资源，STUN 是 socket 上的一次观测。** 三者不能合成一个“节点公网 IP”。

## ListenerState

```json
{
  "mesh-udp": {
    "phase": "listening",
    "socketIds": ["7"],
    "loadedCertificateSha256": null,
    "problem": null
  }
}
```

| 字段 | 含义 |
| --- | --- |
| map key | 对应本节点配置中的 listener key |
| `phase` | `preparing`、`listening`、`stopped`、`failed` |
| `socketIds` | 指向本次报告的 sockets；不得指向不存在的 socket |
| `loadedCertificateSha256` | 实际加载证书的公开指纹；非 TLS 时 null |
| `problem` | 明确失败原因或 null |

`listening` 只说明本机监听成功，不说明公网可达。

资源默认关联本报告的 `configuration.applied`。如果要报告另一份 saved 配置的准备过程，资源须显式携带自己的配置引用并标记 preparing；这种记录供诊断，不能作为 peer 可用入口。切片／材料版本不匹配的旧资源不会因 listener key 相同而复用。

## SocketState

```json
{
  "7": {
    "resource": { "kind": "listener", "key": "mesh-udp" },
    "generation": 3,
    "network": "udp",
    "boundAddress": "0.0.0.0:8421",
    "endpoints": [
      {
        "address": "198.51.100.42:40001",
        "source": "stun",
        "observer": "stun.example.net:3478",
        "validation": "observed",
        "observedAt": "2026-10-03T12:00:00Z",
        "expiresAt": "2026-10-03T12:00:30Z"
      }
    ]
  }
}
```

| 字段 | 含义 |
| --- | --- |
| map key | 当前 Node 实例内的 socket 标识 |
| `resource` | listener，或编译连接计划中的出站／协调资源 |
| `generation` | 资源代际，不能只按相同端口判断同一个 socket |
| `network` | 实际外层 `udp`／`tcp`，不由内层业务协议推测 |
| `boundAddress` | 内核实际 bind 地址与端口 |
| `endpoints` | 此 socket 的候选及观测，分别记录来源、验证程度与期限 |

出站 socket 使用例如 `{ "kind": "dial", "key": "<connection-resource-key>" }`，不冒充 listener。标识绑定到实例＋资源＋generation＋socketId；任一更替都不能借用旧认证／观测。

STUN 的 `observed` 表示得到映射，不等于对端已验证可达。认证探测得到的验证需要关联具体 peer 和通道，不能把某个 peer 的成功推广给所有节点。TCP／VLESS 的可达性不能使用另一个 UDP socket 的映射证明。

## LinkState {#links}

```json
{
  "<node-b>": [
    {
      "linkId": "<public-channel-id>",
      "transport": "native_udp",
      "initiatorNodeId": "<node-a>",
      "socketId": "7",
      "phase": "authenticated",
      "observedAt": "2026-10-03T12:00:01Z",
      "expiresAt": "2026-10-03T12:00:31Z"
    }
  ]
}
```

linkId 为非秘密的运行通道标识，需要有实际握手绑定才能关联两端；不是私钥、会话密钥，也不能用两个节点各自的 socketId 相同来配对。

每条 link 必须以报告节点为一端。A 不能通过自己的报告替 B 声明一条 B—C link。

active 需要区分“认证通道仍存在”和“正在承载业务”。建议另报业务依赖／诊断依赖及流量计数；保持连接可以是 idle、速率为 0，诊断 probe 也不能冒充业务流量。具体实时指标见 [Realtime Observation](/v2/state-reference/realtime)。

Controller 聚合时区分单侧报告、双侧匹配、冲突、过期。双侧匹配要求同一通道、两端身份／承载与当前有效配置相容；收到 A 的报告不自动生成 B 的报告。

这些仍是有身份来源的 Agent 自报告，不是 Controller 独立重做握手的证明。业务准入由数据面实际身份和接收端配置执行。

link 默认建议按需求建立并复用，无活跃依赖时再空闲回收；不为保持历史曲线而常驻。生命周期与保活区别见[按需连接](/v2/state-reference/link-lifecycle)。

## PathState {#paths}

包含运行时实际选定的有序节点路径、所针对的配置、使用／确认结果及观测时间。只有计划、只入发送队列、已到达目标，是不同证据，不能统一标为“路径已验证”。

报告者必须参与该路径，并说明其来源角色／可观察阶段。中转 C 看到密文经过，不等于它能证明目标 B 已把明文交给应用；不能在聚合时抹掉这个区别。

不从若干当前 links 拼出一条路径后冒称它正在承载业务，也不将各边 RTT 相加伪造端到端测量。全网查询只聚合有来源的报告。
