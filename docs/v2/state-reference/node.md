# NodeState

节点对**自身运行事实**的报告内容。网络／Node 身份由上报凭据确定，不接受节点在 body 中指定另一个报告者。以下字段为下一版提案。

## Structure

```json
{
  "system": {
    "hostname": "linux-host",
    "os": "linux",
    "arch": "arm64",
    "agentVersion": "<agent-version>"
  },
  "capabilities": {
    "transport:native_udp": {}
  },
  "configuration": {
    "saved": { "revision": 42, "digest": "<node-config-digest>" },
    "applied": { "revision": 42, "digest": "<node-config-digest>" }
  },
  "listeners": {},
  "sockets": {},
  "links": {},
  "paths": {},
  "selection": {
    "requestedProfile": "mesh",
    "appliedProfile": "mesh"
  },
  "probes": [],
  "problems": []
}
```

## Fields

| 字段 | 类型 | 来源与含义 |
| --- | --- | --- |
| `system` | object | 节点报告的 OS、架构、版本和 OS hostname；不覆盖 Config hostname |
| `capabilities` | [命名空间能力集合](/v2/state-reference/capabilities) | transport:xxx 等当前实际构建能力；不同于管理许可或执行成功 |
| `configuration.saved` | 配置引用或 null | 已校验、持久保存的完整节点配置 |
| `configuration.applied` | 配置引用或 null | 当前运行代际使用的配置；不等于业务连通 |
| `listeners` | `Record<ListenerKey, ListenerState>` | 运行中的本机监听资源 |
| `sockets` | `Record<SocketId, SocketState>` | 实际外层 socket、所属资源及 endpoint 观测 |
| `links` | `Record<PeerNodeId, LinkState[]>` | 对各 peer 的认证通道；可有主备／更替期记录 |
| `paths` | 按运行路径标识索引 | 本节点观察到的选路／使用结果，非全网权威路径表 |
| `selection` | object | 请求使用／已执行的路由方案，必须处于 Config 允许范围 |
| `probes` | `ProbeObservation[]` | 少量最新真实测量；重复快照引用相同事件，不制造新历史点 |
| `problems` | 类型化问题列表 | 当前有来源的失败原因，不使用任意服务端异常文本 |

尚未保存或执行配置时使用 null，不填写伪造的 revision=0。不同配置引用允许短暂并存，例如 saved 已更新但 applied 仍在切换；必须如实报告。

## 字段来源

节点能报告实际 capability 和资源状态，不能自行报告新的 User／Tag 归属、access、出口资格或可信公钥绑定。后者属于 Controller 的实体／Config。

GeoIP 位置由 Controller 根据合格的底层公网 IP 观测计算，放在 [NetworkState 的派生部分](/v2/state-reference/network#derived)，不直接采用节点自报 country。用户方案选择是受配置约束的使用状态，不等于修改管理员方案。

## 标识与时间

上报 envelope 中的 instance／sequence 用于排序。每个 socket、link 和测量仍有自身代际、观测时间与到期时间；新的整体上报不能给旧 STUN 映射或旧 probe 重新计时。

listener/sockets/links 的标识是资源定位键，不是新的持久业务 Entity。字段细节见[资源与连接](/v2/state-reference/resources)。
