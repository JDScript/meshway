# NodeState

节点对**自身运行事实**的报告内容。网络／Node 身份由上报凭据确定，不接受节点在 body 中指定另一个报告者。

## Structure

以下为尚未保存或应用配置的完整结构示例，不代表运行测量。实际能力必须与该实例 acquire 时的声明相符：

<!-- schema: NodeState -->
```json
{
  "system": {
    "hostname": "linux-host",
    "os": "linux",
    "arch": "arm64",
    "agentVersion": "0.1.0"
  },
  "capabilities": {
    "transport:native_udp": { "kind": "transport", "roles": ["dial", "listen"], "streams": ["udp"] }
  },
  "configuration": {
    "saved": null,
    "applied": null,
    "progress": null
  },
  "listeners": {},
  "sockets": {},
  "links": {},
  "adjacencies": {},
  "paths": {},
  "selection": {
    "appliedProfile": null
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
| `configuration.progress` | ConfigurationProgress 或 null | 当前目标及 receiving／validating／saved／applying／applied／failed 的真实进度 |
| `listeners` | `Record<ListenerKey, ListenerState>` | 运行中的本机监听资源 |
| `sockets` | `Record<SocketId, SocketState>` | 实际外层 socket、所属资源及 endpoint 观测 |
| `links` | `Record<PeerNodeId, LinkState[]>` | 对各 peer 的认证通道；可有主备／更替期记录 |
| `adjacencies` | `Record<PeerNodeId, "idle" \| "unreachable">` | 已获准但当前无通道的邻接：`idle` 为按需边因空闲关闭或尚无需求，`unreachable` 为有需求（业务、中继或常驻边）但未建立通道；有通道的 peer 只出现在 `links`；状态变化不单独触发上报，随常规上报（20 秒）更新 |
| `paths` | 按运行路径标识索引 | 本节点观察到的选路／使用结果，非全网权威路径表 |
| `selection.appliedProfile` | ProfileKey 或 null | 当前实际方案，必须处于 Config 允许范围 |
| `probes` | `ProbeObservation[]` | 少量最新真实测量；重复快照引用相同事件，不制造新历史点 |
| `problems` | 类型化问题列表 | 当前有来源的失败原因，不使用任意服务端异常文本 |

尚未保存或执行配置时使用 null，不填写伪造的 revision=0。不同配置引用允许短暂并存，例如 saved 已更新但 applied 仍在切换；必须如实报告。

Node 须校验并保存后按有效许可安装，不等待其他节点完成。progress 为 null 表示当前无进度报告，不表示目标已经完成。阶段与回执见[配置应用与进度](/v2/configuration-reference/deployment)。

执行发布引用固定身份／材料／相关节点切片依赖，和源 revision、当前 Node digest 一起核对；不能仅凭 revision 相同将报告用于另一次执行发布。详见[发布与恢复](/v2/configuration-reference/deployment#publication)。

## 字段来源

节点能报告实际 capability 和资源状态。User／Tag 身份、enabled、access 和出口资格由 Config 决定，公钥绑定与永久吊销由认证存储决定；Node State 不能自行改变它们。

GeoIP 位置由 Server 根据合格的底层公网 IP 观测计算，放在 [StateView 的派生部分](/v2/state-reference/network#derived)，不直接采用节点自报 country。用户方案选择是受配置约束的使用状态，不等于修改管理员方案。

GeoIP／ASN 作为 Server 的 ipInfo 派生信息返回；节点位置的近似坐标、精度、来源及缺失原因见[IP 信息](/v2/state-reference/ip-info)。Node 的 State 上报不会自行成为可信地理或 ASN 数据源。

用户方案选择跨重启保存；恢复时仍在 allowed 且未被锁定才继续使用，否则采用当前管理员默认。它不进入管理员配置回滚历史，切换不能带走旧出口流／NAT 资格。

请求选择由 Server 的持久 RoutingSelection 与当前配置确定，读取时投影；Node 只上报实际 appliedProfile。提交选择使用受控操作，不通过普通 State 报告覆盖选择。

## 标识与时间

上报 envelope 中的 instance／sequence 用于排序。每个 socket、link 和测量仍有自身代际、观测时间与到期时间；新的整体上报不能给旧 STUN 映射或旧 probe 重新计时。

listener/sockets/links 的标识是资源定位键，不是新的持久业务 Entity。字段细节见[资源与连接](/v2/state-reference/resources)。
