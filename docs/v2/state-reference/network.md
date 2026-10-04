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

NodeStateView 包含 hostname、instanceId、reportSequence、observedAt、receivedAt、reportExpiresAt、lastAuthenticatedAt、freshness、report 和 derived。hostname 是当前配置里的规范主机名（显示名），对成员可见的依赖 peer 同样给出；它不是报告里的 OS hostname，也不扩大其他字段的裁剪范围。未报告时 report=null，不能用空对象伪造 NodeState。

report 以 `view: full | peer` 区分，内部 report 分别是完整 NodeState 或裁剪的 PeerState。freshness 按固定顺序判定：revoked（凭据永久吊销）；awaiting_report（从未报告）；expired（报告超过 TTL，**或节点没有活实例**——`meshway down`、进程退出、lease 到期都在这里，不论报告多新）；superseded（存在另一个活实例，与报告的实例不同——节点重启后旧报告尚未被替换）；config_mismatch（活实例报告的 applied 不在当前配置集合内）；否则 current。current 不等于每个 endpoint 已验证可达。

## 派生值 {#derived}

derived 包含 ipInfo、location 和 locationAbsent。地址、数据集和原观测期限决定结果，见 [GeoIP／ASN](/v2/state-reference/ip-info)。没有坐标不填零，不用 Server 连接来源、Mesh IP、管理员标签或所选出口 IP 代替本机底层观测。

## Console 组合视图

| 片段 | 类型与来源 |
| --- | --- |
| 节点库存 | NodeInventoryView：元数据、配置摘要、密钥状态、操作资格和 desiredExecution |
| 单节点设置 | NodeSettingsResponse：networkId／nodeId／revision 和非秘密 settings |
| 成员／网络 | MemberSummary／NetworkSummary，包含服务端 actions |
| State | NodeStateView：原报告、时间、新鲜度与派生结果 |
| 应用进度 | ConfigurationApplyProgress：固定目标、实际回执与 freshness |

SSE 与 GET 共用这些片段。实体权限更新不覆盖更新的运行报告；desiredExecution 不是实际 applied。当前没有 versionStatus 字段，不由 Console 比较版本字符串补造。

### Links

AggregatedLink 保留 a、b、linkId、transport、initiatorNodeId、phase、evidence、reports、inUse 和 since。节点对只是分组，不覆盖同一对节点的多个通道。

evidence 为 oneSided／matched／conflicting／expired，reports 引用原 Node／instance／reportSequence。证据不足时 inUse／since 可为 null；两端冲突不合成为健康，也不代表 access 双向。

### Paths

AggregatedPath 包含 pathId、reporter 和 PathState。每个报告者保留自身角色、完整路径、publication、证据阶段与原时间。旧流／新流可以同时走不同路径，不能压成一条“当前路径”。

### RecentLinks

历史图层单独调用 history/query 的 recentLinks 模式。HistoryPage 的 from、to、asOf、nextCursor、coverage 与 result 表达窗口和来源；它不属于默认实时 SSE。

最近连线来自真实历史事件，不能展开 topology 的允许图来填充。无样本不能推断空闲，读取或重放不增加测量次数。

## 过期与恢复

freshness 依据原期限和当前资格计算；报告、endpoint 和测量各有期限。新联系或重复查询不能延长旧观测。Server 重启更换 epoch，重新同步当前状态；旧观测只保留其历史含义，身份吊销和必要回执独立持久化。
