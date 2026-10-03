# 观测与历史

**历史记录发生过什么，不负责保持 link 存在。** 默认采用[按需连接与复用](/v2/state-reference/link-lifecycle)，只记录实际事件和测量。以下是下一版方案，尚待确认。

## 历史包括什么

| 事件 | 内容 |
| --- | --- |
| `dial_started` / `dial_failed` | 对应候选、建立阶段、失败原因及耗时；尚未成功时可只有 attemptId |
| `link_opened` | 实际认证完成的 link、承载和两端关联 |
| `link_closed` | 该 link 的真实关闭及原因，例如 idle、remote_closed、timeout、withdrawn |
| `probe_completed` | 一次真实 probe 的结果、方向、link／路径与耗时 |
| Controller 的 `report_expired` 等事件 | 只表示控制面自己的判断，不冒充节点真实 link_closed |

承载更换形成新旧 link 的可关联事件；不能把它等同于业务逻辑路径发生变化。关闭一条 link 也不等于该 peer 的所有通道都关闭。

## 上报流程

```text
连接 owner／统一探测器产生真实事件
        ├─ 更新本地最新 State
        └─ 有界历史事件队列
                 → 批量上报 Controller
                 → 去重、按权限与事件上下文校验
                 → 保存历史／供查询
```

NodeState snapshot 只提供最新状态，历史使用独立批次。相同 probe 可以被最新状态和历史引用，但周期性重复发送 snapshot 不能再次插入它。

统一探测来源不要求保存每次探测：可以按预算抽样／聚合。保留的每个原始点都必须对应真实事件；聚合则须明确桶范围和样本数量，不能每隔一段时间复制最后一个 RTT。

## Batch 示例

```json
{
  "instanceId": "<node-instance>",
  "events": [
    {
      "eventId": "<event-id>",
      "kind": "probe_completed",
      "peerNodeId": "<node-b>",
      "linkId": "<link-id>",
      "configurationRevision": 42,
      "nodeConfigurationDigest": "<node-config-digest>",
      "startedAt": "2026-10-03T12:00:00Z",
      "completedAt": "2026-10-03T12:00:00.012Z",
      "measurement": {
        "kind": "neighbor",
        "outcome": "success",
        "durationUs": 12000
      }
    }
  ]
}
```

source Node／network 由凭据决定；每个事件是明确的判别联合类型，不是任意 metadata。这里的 eventId 同时标识该测量，不另用一个 ID 把同一次 probe 算成两条事件。其他事件字段按其类型定义。

## 时间与可靠性

- 按 `(network, source Node, eventId)` 幂等；重试保持 ID／内容／事件时间不变。
- 记录发生时的实例和配置，不因收到时已换配置就篡改原事件的上下文。
- 迟到／旧配置的历史事件只按有界的历史接纳规则处理，不能据此更新当前 State 或恢复旧授权；确切窗口尚待定。
- 已撤销的凭据不能继续写入。Controller 可记录自己的撤销／报告过期事件，但不能凭此捏造节点实际关闭时刻。
- 队列满、上报失败、进程退出可以造成历史缺口；初版不承诺无损审计。采集／上报丢弃计数需可见，但不是网络丢包率。
- Controller 保存原始事件与 receivedAt；查询按事件时刻展示，分页／重复读取不产生新测量。

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

## 当前实现

用户实时 watch 的结果也可以通过本事件管道保存，但打开历史页面不隐式创建测量任务；实时指标和权限见 [Realtime Observation](/v2/state-reference/realtime)。

当前已有独立的 30 秒邻接历史和 5 秒健康探测；底层 timeout 可退休通道。上面“统一事件来源、历史不另发探测、按需 link 生命周期”的完整方案尚未实现，当前接口与预算见[邻接延迟历史](/latency-history)。
