# 网络状态视图

网络查询返回 `StateView`。Server 保存每个 Node 的已接受报告，再根据当前权限、时间和执行上下文生成视图。

## Structure

空网络的完整响应示例：

<!-- schema: StateView -->
```json
{
  "networkId": "00000000-0000-4000-8000-000000000001",
  "stateEpoch": "00000000-0000-4000-8000-000000000002",
  "sequence": 0,
  "configurationRevision": 1,
  "asOf": "2026-10-03T12:00:00Z",
  "nodes": {},
  "links": [],
  "adjacencies": [],
  "paths": []
}
```

## Fields

| 字段 | 含义 |
| --- | --- |
| networkId／stateEpoch／sequence | 网络、服务端状态世代与变化顺序 |
| configurationRevision | 当前裁剪／解释视图所用配置，不重标旧报告 |
| asOf | 本次读取时间 |
| nodes | NodeId → NodeStateView |
| links／paths | 当前可见报告的聚合结果 |
| adjacencies | 每对可见节点一条相邻连接结论，不随报告裁剪变化，见 [Adjacencies](#adjacencies) |

NodeStateView 包含 hostname、instanceId、reportSequence、observedAt、receivedAt、reportExpiresAt、lastAuthenticatedAt、freshness、report 和 derived。hostname 是当前配置里的规范主机名（显示名），对成员可见的依赖 peer 同样给出；它不是报告里的 OS hostname，也不扩大其他字段的裁剪范围。未报告时 report=null，不能用空对象伪造 NodeState。

report 以 `view: full | peer` 区分，内部 report 分别是完整 NodeState 或裁剪的 PeerState。完整 NodeState 也只保留读取者当前可见的对端：`links`、`adjacencies` 按对端、`paths`／`probes` 按路径上全部节点过滤，权限收缩后读取旧报告同样适用。freshness 按固定顺序判定：revoked（凭据永久吊销）；awaiting_report（从未报告）；expired（报告超过 TTL，**或节点没有活实例**——`meshway down`、进程退出、lease 到期都在这里，不论报告多新）；superseded（存在另一个活实例，与报告的实例不同——节点重启后旧报告尚未被替换）；config_mismatch（活实例报告的 applied 不在当前配置集合内）；否则 current。current 不等于每个 endpoint 已验证可达。

## 派生值 {#derived}

derived 包含 addressScope、location 和 locationAbsent：可见 endpoint IP 的类别（public／nonPublic／mesh），以及位置依据的本机公网观测（地址、来源、原期限、socket 与代际）或其缺失原因（noObservation／observationExpired／ambiguousObservation），见[地址类别与位置依据](/v2/state-reference/ip-info)。State 不做 GeoIP／ASN 查询，不用 Server 连接来源、Mesh IP、管理员标签或所选出口 IP 代替本机底层观测。

## Console 组合视图

| 片段 | 类型与来源 |
| --- | --- |
| 节点库存 | NodeInventoryView：元数据、配置摘要（含 Mesh DNS 名称 `dnsName` 与 `magicDNS`）、密钥状态、操作资格和 desiredExecution |
| 单节点设置 | NodeSettingsResponse：networkId／nodeId／revision 和非秘密 settings |
| 成员／网络 | MemberSummary／NetworkSummary，包含服务端 actions |
| State | NodeStateView：原报告、时间、新鲜度与派生结果 |
| 应用进度 | ConfigurationApplyProgress：固定目标、实际回执与 freshness |

SSE 与 GET 共用这些片段。实体权限更新不覆盖更新的运行报告；desiredExecution 不是实际 applied。当前没有 versionStatus 字段，不由 Console 比较版本字符串补造。

### Links

AggregatedLink 保留 a、b、linkId、transport、initiatorNodeId、phase、evidence、reports、inUse 和 since。节点对只是分组，不覆盖同一对节点的多个通道。

evidence 为 oneSided／matched／conflicting／expired，reports 引用原 Node／instance／reportSequence。证据不足时 inUse／since 可为 null；两端冲突不合成为健康，也不代表 access 双向。links 只聚合调用者能读完整报告的节点，所以成员视图中对端完整报告不可见时只有 oneSided；它是诊断证据，连接结论看 [adjacencies](#adjacencies)。

### Adjacencies {#adjacencies}

AggregatedAdjacency 为 `{a, b, status}`，每对节点一条（`a` 为较小的 Node ID）。Server 用两端各自的当前报告（freshness 为 current 且仍持有执行）判定，包括调用者读不到的完整报告；只给出结论，不披露报告内容。

| status | 条件（按序） |
| --- | --- |
| `connected` | 任一端报告一条 authenticated、未到期的通道，且另一端没有以不同承载、阶段或发起方报告同一 `linkId` |
| `unknown` | 有 authenticated 通道但两端报告冲突 |
| `unreachable` | 无通道，任一端的 `adjacencies` 为 `unreachable`（有需求但未建立） |
| `idle` | 无通道，任一端为 `idle`（获准但当前无需求，或按需边已因空闲关闭） |
| `unknown` | 两端都没有当前报告提到这一对，或报告已过期、实例已更替、配置不匹配 |

只列出调用者已能看到的关系：两端都在可见集合内，且至少一端是调用者可读完整报告的节点；单节点读取只列含该节点的对。`connected` 是已认证邻接的存在，不是业务端口可达或 access 双向的证明；closed／failed 记录不算现有连接。同一配置、同一组报告下 Owner、Admin 与成员看到的结论相同，GET 与 SSE（`state.delta` 的 `adjacencies`）相同。

### Paths

AggregatedPath 包含 pathId、reporter 和 PathState。每个报告者保留自身角色、完整路径、netmap、证据阶段与原时间。只聚合当前报告中未到期的路径，且路径上所有节点对调用者可见；旧流／新流可以同时走不同路径，不能压成一条“当前路径”。路径的来源与边界见 [PathState](/v2/state-reference/resources#paths)。

### RecentLinks

历史图层单独调用 history/query 的 recentLinks 模式。HistoryPage 的 from、to、asOf、nextCursor、coverage 与 result 表达窗口和来源；它不属于默认实时 SSE。

最近连线来自真实历史事件，不能展开 topology 的允许图来填充。无样本不能推断空闲，读取或重放不增加测量次数。

## 过期与恢复

freshness 依据原期限和当前资格计算；报告、endpoint 和测量各有期限。新联系或重复查询不能延长旧观测。Server 重启更换 epoch，重新同步当前状态；旧观测只保留其历史含义，身份吊销和必要回执独立持久化。
