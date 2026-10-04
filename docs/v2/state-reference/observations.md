# 观测与历史

**历史记录发生过什么，不负责保持 link 存在。** 只记录实际事件和测量；状态与连接生命周期见[连接观测](/v2/state-reference/link-lifecycle)。

## 历史包括什么

| 事件 | 内容 |
| --- | --- |
| `dial_started` / `dial_failed` | 对应候选、建立阶段、失败原因及耗时；尚未成功时可只有 attemptId |
| `link_opened` | 实际认证完成的 link、承载和两端关联 |
| `link_closed` | 该 link 的真实关闭及原因，例如 idle、remote_closed、timeout、withdrawn |
| `probe_completed` | 一次真实 probe 的结果、方向、link／路径与耗时 |
| `collection_gap` | 来源报告的实际采集缺口及丢弃数量，不是链路丢包率 |

承载更换形成新旧 link 的可关联事件；不能把它等同于业务逻辑路径发生变化。关闭一条 link 也不等于该 peer 的所有通道都关闭。

## 上报流程

```text
连接 owner／统一探测器产生真实事件
        ├─ 更新本地最新 State
        └─ 有界历史事件队列
                 → 批量上报 Server
                 → 去重、按权限与事件上下文校验
                 → 保存历史／供查询
```

NodeState snapshot 只提供最新状态，历史使用独立批次。相同 probe 可以被最新状态和历史引用，但周期性重复发送 snapshot 不能再次插入它。

统一探测来源不要求保存每次探测：可以按预算抽样／聚合。保留的每个原始点都必须对应真实事件；聚合则须明确桶范围和样本数量，不能每隔一段时间复制最后一个 RTT。

## 上报结构

`POST /agent-api/v1/history` 接受 `HistoryReport { instanceId, events }`。每项 ObservationEvent 包含 eventId、configuration、occurredAt 和 payload；configuration 是确切 ConfigurationRef，payload 按 kind 判别。

| payload.kind | 主要字段 |
| --- | --- |
| `dial_started` | peerNodeId、attemptId、transport |
| `dial_failed` | peerNodeId、attemptId、transport、reason、可空 durationUs |
| `link_opened` | peerNodeId、linkId、transport |
| `link_closed` | peerNodeId、linkId、transport、reason |
| `probe_completed` | probe：共享 ProbeObservation |
| `collection_gap` | dropped、from、to |

ProbeObservation 包含 measurementId、kind、linkId、transport、path、publicationId、startedAt、completedAt、outcome、durationUs。可空项与必填项以[生成契约](/v2/contracts/openapi.json)为准。source Node／network 由凭据决定，不接受任意 metadata 或客户端声明的报告者。

## 时间与可靠性

- 按 `(network, source Node, eventId)` 幂等；重试保持 ID／内容／事件时间不变。
- 记录发生时的实例和配置，不因收到时已换配置就篡改原事件的上下文。
- 迟到／旧配置的历史事件只按有界的历史接纳规则处理，不能据此更新当前 State 或恢复旧授权。
- 已撤销的凭据不能继续写入。Server 判断报告过期不等于节点实际关闭时刻，不能伪造 link_closed。
- 队列满、上报失败、进程退出可以造成历史缺口；初版不承诺无损审计。采集／上报丢弃计数需可见，但不是网络丢包率。
- Server 保存原始事件与 receivedAt；查询按事件时刻展示，分页／重复读取不产生新测量。

历史读取以**当前**用户权限过滤；普通成员只看其获准业务／执行依赖相关范围，不能用事件发生时曾有的权限重新授权当前请求。权限矩阵见[操作权限](/v2/permissions)。

## Query

Console 使用 `POST /api/v1/networks/{networkId}/history/query`，输入固定 from／to、mode、limit，以及可选 kinds、sourceNodeId、peerNodeId、linkId、bucketSeconds、cursor。没有 pathId 筛选字段。raw／buckets／recentLinks 返回不同判别类型；桶保留真实样本与结果计数，不能把平均值当成新测量。

历史图层可选 recentLinks 摘要模式，使用同一真实事件源和当前权限；返回明确 window、coverage 和每条实际 link 的摘要。它不是新的实时探测或当前连通状态。

返回事件／桶、nextCursor 和覆盖范围；cursor 绑定窗口、模式、筛选、身份与读取上界。迟到数据由下一轮查询接纳，分页不跨窗口复用。HistoryCoverage 说明裁剪和缺口，不能单靠缺样本判断空闲；recentLinks 从这些真实事件派生。

coverage 区分 `pruned`（retention／capacity）与 `collectionGap`。后者保留来源 Node、instance、event ID、原始 from／to 与 dropped；仅在原始区间与查询窗口相交时展示，不按裁剪后的时间比例虚构丢弃数量。coverage 独立于 raw／buckets／recentLinks 和 kinds 筛选；来源级 gap 没有 peer／link 归因，仅完整获准读取该来源的用户可见，不能当成某条连接的丢包率。无样本的桶不补零。

历史接纳按事件携带的完整 ConfigurationRef 选择已保留的真实资源／路径上下文，再核对当前凭据、实例及接纳时间窗。该上下文不会成为当前 State 的执行准入。neighbor／connect 使用显式获准邻接及承载集合；path 必须匹配获准完整路径，不能从其中切出任意业务子路径。

## 延迟口径

| 测量 | 含义 |
| --- | --- |
| neighbor | 已认证邻接通道上的 probe |
| path | 指定完整路径的端点 probe |
| connect | 承载发现／建连／握手耗时 |
| controller | 控制面请求耗时 |

durationUs 是对应操作的完成耗时。要称为“纯网络 RTT”，必须有排除排队／内部重试的实际测量边界；不能只改显示名称。邻接 RTT 也不能相加后宣称实测端到端延迟。

耗时使用单调时钟，UTC 时间用于历史定位。失败／超时即使记录了尝试耗时，也不能画成成功 RTT；没有真实计时则为 null，不能补零。

## 空闲时怎样显示

```text
12:00 link 建立，随后有 probe 样本
12:02 无业务依赖，空闲回收 link
12:02–12:10 没有 RTT 点
12:10 新业务触发建连，记录 connect 耗时和新 link 的 probe
```

如果保留了回收事件，可以显示“空闲、未测量”；如果缺少事件，只能显示“无样本／原因未知”。缺测或超时不填零，不用固定采样周期反推丢包率。

## 与实时观察的关系

实时观察结果可通过本事件管道保存，但打开历史页面不隐式创建测量任务；实时指标和权限见 [Realtime Observation](/v2/state-reference/realtime)。

Server 接纳和查询事件不证明节点已完成对应测量；采样必须有真实运行来源，范围见 [Node](/v2/node)。
