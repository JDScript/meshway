# Realtime Observation

主动路径观察通过 HTTP 调度，使用短租约诊断许可，结果经网络 SSE 交付。Server 接纳报告不等于独立完成包级测量；采样来源与运行能力见 [Node](/v2/node)。

它与普通 State 读取不同：query/watch State 订阅既有事实；Realtime Observation 可以明确请求节点产生新的测量。

HTTP 创建／续租／取消任务，网络 SSE 按任务和订阅权限发送结果。连接 SSE 本身不创建探测任务，见[事件流](/v2/api/events#path-observation)。

本机 [`meshway ping`](/v2/node/diagnostics) 使用 Node 实例订阅，复用同一任务与执行器。Node 只能为自身当前实例创建、读取、续租和取消自己的订阅；用户会话订阅与 Node 实例订阅各自鉴权，可共享实际任务但不能互相取消。

## Start

完整创建请求结构示例。Node ID 须来自当前网络，源、目标服务与完整路径须获当前用户和配置授权：

<!-- schema: CreateObservation -->
```json
{
  "subscriptionId": "00000000-0000-4000-8000-000000000001",
  "sourceNodeId": "00000000-0000-4000-8000-000000000002",
  "targetNodeId": "00000000-0000-4000-8000-000000000003",
  "service": { "type": "tcp", "port": 443 },
  "selection": {
    "mode": "explicit",
    "path": ["00000000-0000-4000-8000-000000000002", "00000000-0000-4000-8000-000000000004", "00000000-0000-4000-8000-000000000003"]
  },
  "metrics": ["rtt", "traffic", "probeLoss"],
  "intervalMs": 1000
}
```

`ObservationSubscriptionId` 标识本页需求，`ObservationId` 标识可复用的节点测量任务。创建请求重试沿用 subscriptionId；另开页面创建新 ID。ObservationTask 返回 id、subscriptionId、实际 intervalMs、任务 expiresAt、订阅 subscriberExpiresAt 和 renewAfter。用户操作使用用户会话，Node 领取和上报使用自己的认证凭据。

| 字段 | 含义 |
| --- | --- |
| `subscriptionId` | 本次页面需求的独立 UUID，也用于有界创建幂等；不是 SessionId 或凭据 |
| `sourceNodeId` | 发起测量的节点，用户须有权让它执行诊断 |
| `targetNodeId` | 当前网络中的业务目标节点 |
| `service` | 必填精确授权范围：`{type:tcp,port}`、`{type:udp,port}` 或 `{type:icmpEcho}`；不把 Node 可见性当作所有端口可访问 |
| `selection.mode` | auto／explicit；不使用含义模糊的 current |
| `selection.path` | explicit 必填的完整有序路径；auto 输入省略，实际选择在结果的 `path` 中返回 |
| `metrics` | 请求 RTT、traffic、probeLoss；`unsupportedMetrics` 明确列出无法提供的项 |
| `intervalMs` | 期望周期，示例为 1 秒；实际周期受节点／网络预算约束 |

auto 可用于空闲网络：Server 根据目标计算获准候选，执行端须结合 State 选择／建立实际路径，未完成时为 preparing、path=null。首次选定后按该路径采样，发生重选时开启新分段，不能每个样本换路还画成同一序列。

Server 用相同的 access／topology／routing 与稳定 SubjectBindings 编译精确服务范围，然后只发布 `NodeConfiguration.diagnostics` 中的独立许可。该许可包含观察 ID、源实例、完整候选、服务范围、到期时间与认证原路回复语义；不会插入普通 `permits`、业务 `decisions` 或伪造 Node demand。source／中转／终点通过正常 Node configuration 拉取一致材料。源任务准备并不表示这些节点已安装，Node 结果仍须匹配真实实例、当前 publication 和执行 lease。

netmap 链（实现中，Node 尚未切换）：Server 只在 source 当前 netmap 上用同一本地编译器判定——决定为 Permitted、服务在决定内且目标为该 Mesh 节点才接受，路径全部声明 `feature:path-observation` 为可观察，否则任务为 unsupported；Denied 与 unsupported 分开返回。创建、续租和取消都不写诊断需求、不编译、不产生 publication；续租只改订阅自身。Node `meshway ping` 的目标名在 source netmap 上解析。

explicit 固定指定路径，不因故障绕过必经点。跟随某条已有业务流是另一个模式，当前不隐式加入；同源／目的可能有多条并存路径。

## 数据流

```text
Console 创建 watch
   → Server 检查用户、源节点与路径权限，授予短租约
   → Source Node 校验当前配置，准备这条获准路径
   → Node 周期性回报实际测量／计数窗口
   → Server 向有权订阅者转发结果
   → Console 展示；可选写入历史
```

Server 负责鉴权、调度和转发，不用浏览器到 Server 的耗时代替 Mesh RTT。请求不能让 source 执行其配置不允许的路径，也不能产生新的业务 access。

普通成员仅可请求自己的 User 身份设备发起获准诊断，并只读取相关结果；Owner／Admin 可操作本网络。Tag owner 不因授 Tag 权限获得对其他 Tag 节点的诊断权。详见[操作权限](/v2/permissions)。

相同网络／源／路径／指标的观察可合并成一个受控采样任务，多个用户订阅复用结果。底层实际探测器与历史采集共用事件来源，不因多开几张图就重复发包。

同一登录会话的两个标签页也持有不同 subscriptionId。GET／renew／cancel 以订阅 ID 操作，只影响该需求；真正的会话吊销才撤销该会话的全部订阅。各订阅独立保存主体、网络、权限摘要和租约，不按 SessionId 覆盖当前网络。

节点结果采用 `result.type=phase|sample` 联合。phase 可报告 preparing／failed／completed／unsupported，无需编造 path 或空样本；sample 必须包含实际路径分段和非空测量窗口。Server 的 expired／cancelled 来自真实订阅生命周期。终态在当前权限下短期可读，随后以 `observation.removed` 或明确 reset 清理；取消／到期后的旧样本不能经排队或重放重新送达该订阅。

任务复用还须绑定相容的授权／执行发布、测量模式和周期；每个订阅者独立验证权限。只可观察某服务的用户，不能因复用了管理员任务就读取路径上其他人的流量；结果必须有明确的业务／统计范围。

同 revision 新增或续租观察会改变有关节点的执行发布，现有合法订阅可换绑到新的完整目标并回到 preparing；旧分段与旧 publication 的排队样本不继续投递。订阅权限摘要与 publication 分开，新建观察不会自行吊销同会话其他标签页。配置 revision、成员角色、身份或会话失效会停止相应订阅。

## 连接状态

Node 上报实际认证 link 是否仍存在，以及是否有业务／诊断依赖。原始 LinkState 使用 phase、businessDependencies、observationIds，聚合视图另有可空 inUse，见[资源与连接](/v2/state-reference/resources#links)。

- authenticated 但 idle：连接有效，业务流量可以为 0。
- businessDependencies：业务依赖数量，不等于此刻一定有连续字节传输。
- observationIds：诊断任务依赖，不能把探测流量标成业务活跃。
- Server 报告过期：未知，不据此断言物理 socket 已关闭。

显式 realtime watch 可以作为**临时诊断需求**准备／保持获准路径；没有必要要求平时全网常驻连接。普通历史查询本身不触发建连。

## RTT

由源节点向路径终点发带关联标识的认证探测，回复按约定路径返回，使用单调时钟测量往返。不是相邻边 RTT 求和，也不是 HTTP 上报时间。

ObservationTask 提供任务、源、实际路径、pathSegment 和实例／配置引用，latest 中的 ObservationSample 提供 measurementId、version、windowStart／windowEnd、partial 和测量内容。超时不填 0；排队时间和内部重试是否包含必须明确。缺少纯网络计时边界时展示“探测往返耗时”，不虚称线速 RTT。

尚在发现／建连时显示 preparing，并单独记录建立耗时。只有实际进入探测发送的请求才参与 RTT／未回应率统计，不能将“还没有建立路径”记成一次已经发送的 probe 丢失。

探测未回应率必须有明确的请求和重试口径。TCP／可靠承载自己的重传仍然可能发生，所以探测结果不是底层 wire 丢包率。

## Traffic

TrafficWindow 定义**源端口径**。当前 traffic 为 unsupported，下面说明字段含义，不是已有测量结果：

| 指标 | 含义 |
| --- | --- |
| `txBytes` | 源端从业务侧接入并提交给该路径的 Mesh IP 字节 |
| `rxBytes` | 源端从该会话反向路径验证并交付本机的 Mesh IP 字节 |
| `counterGeneration` | 计数代际，重置后不能沿用旧差分基线 |
| 速率展示 | 用同一实例／路径分段中的真实字节数和窗口时长计算，不是额外 wire 字段 |

tx 提交不等于目标已收到；若还需要目标交付字节，必须由目标实际计数独立回报。中转节点不重复计入端到端流量。

握手、保活和诊断探测不计入业务字节。外层加密后的 wire bytes、transport 重传或各段负载可作为另一个指标，不能混入同一个“业务流量”数字。计数器重置／路径变更须重建基线；没有窗口数据时是未知，不补零速率。

Server 将 traffic 列入 `unsupportedMetrics`，样本的 `traffic` 为 null；未计数值不得上报成零。RTT／probeLoss 路径的源、中转与终点须在 acquire 声明 `feature:path-observation` version 1。缺能力且无其他完全支持的获准完整候选时，任务为 unsupported，并且不发布不可执行诊断许可；权限拒绝另行表达。能力声明只表明构建支持，不是测量成功证据。

## Probe loss

**探测未回应率**按已有最终结果的实际发送探测计算：

```text
timeout / (repliedWithinDeadline + timeout)
```

- pending 尚未到 deadline，不进分母。
- 本地未发送成功的失败单独计数，不当作网络丢包。
- 重复回复按 measurement ID 去重；迟到回复单独记录，不当成另一个成功样本。
- 分母为 0 时返回 null；样本少时显示数量和窗口，不能把一两个样本推广成稳定质量。
- 窗口按探测开始时间归属，未完成时标记 partial；如果后续修订窗口，用同一窗口 ID 替换而不是累加。

deadline 之后收到的回复仍记为迟到，本口径保留“未在期限内回应”的结果。它包含真正丢失、拥塞、排队和端点处理过慢等影响，不能单独证明是哪一段丢了包。

**业务包丢包率**是另一项指标：需要对应业务数据的可靠序号、确认和乱序窗口。不能通过 RTT 超时次数、两端任意字节差或 TCP 重传猜算；没有这种测量能力时明确不提供。

## 租约与取消

- Server 为每个订阅授予最多 30 秒，且不超过用户会话与源实例期限；10 秒后可按当前权限续租。重试创建不续租。周期为 1000–30000 ms。
- 超出并发／频率预算返回 observation_limit；renewAfter 给出续租节奏，页面后台停止续租。
- 用户关闭／切换网络／退出登录、权限撤回或配置失效时停止订阅，并取消／停止续租节点任务。
- 断开控制通道或丢失取消消息时，Node 仍按本地租约停止；结果不反过来续活任务。
- 最后一个订阅者离开后释放诊断依赖，是否保留 link 回到普通业务／空闲回收策略。
- 诊断订阅有独立持久资格与当前 schema 生命周期；账号／成员撤销事务同步取消该资格。App 的观察维护器不依赖 HTTP／SSE 轮询，定期撤回到期或失权的诊断发布。Node 普通 renew 不能续活观察，已安装许可还须按自己的短期限硬停止。
- 探测超时只产生观察结果，不能由观察器直接拆掉业务 link；健康处理由连接 owner 的统一策略决定。

## State、Realtime、History

State 给出最新事实；Realtime 是当前观察任务的结果流；History 保存选定的真实事件／窗口。三者使用相同来源标识，不互相制造样本。是否保存 watch 全部原始点、抽样或聚合，由保留预算决定。

Server 操作为 `createObservation`／`listObservations`／`getObservation`／`renewObservation`／`cancelObservation`、`getNodeTasks`／`reportTaskResult`。用户操作以 subscriptionId 区分需求，Node 结果使用共享 taskId。状态、样本和移除分别经 `observation.status`／`observation.sample`／`observation.removed` 发送；普通 State／History 读取不创建观察。
