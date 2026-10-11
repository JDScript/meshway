# Node → Server

`POST /agent-api/v1/state` 上报完整的单节点快照，使用 `StateReport`。网络／节点身份由认证上下文确定。

## Report structure

完整请求结构示例。instanceId 须来自当前有效实例；system、capabilities 和资源清单须来自实际 Node：

<!-- schema: StateReport -->
```json
{
  "instanceId": "00000000-0000-4000-8000-000000000001",
  "sequence": 18,
  "observedAt": "2026-10-03T12:00:00Z",
  "state": {
    "system": { "hostname": "linux-host", "os": "linux", "arch": "arm64", "agentVersion": "0.1.0" },
    "capabilities": {},
    "configuration": { "saved": null, "applied": null, "progress": null },
    "listeners": {},
    "sockets": {},
    "links": {},
    "adjacencies": {},
    "paths": {},
    "selection": { "appliedProfile": null },
    "probes": [],
    "problems": []
  }
}
```

envelope 不接受任意 source Node／network；Server 从节点凭据确定作用域。instance 必须是当前有效运行实例，不能仅换一个随机字符串就接管旧运行代际。

## 接收步骤

1. 检查节点凭据、网络、撤销状态和当前实例。
2. 校验类型、数量／字节预算和 sequence。
3. 对资源和执行报告检查其关联的已下发配置；socket 的用途／generation 与资源引用必须一致。可引用的配置是：当前 lease 目标；本实例近 5 分钟内有回执的目标；以及本网络近 15 分钟内（最多 8 次）发布里该节点的目标——节点可如实报告刚保存、随后被新 revision 取代的配置。后两类只是历史：活动 listener／socket／link／path 仍必须关联当前执行目标。

已知属于本节点的历史配置，其资源事实本身合法但执行 scope 已变化时，报告返回 HTTP 409 `state_scope_changed`，不写入新的 State 样本、序号或接收时间。Node 丢弃该待交付报告，原子刷新执行目标，再以更高序号采集并提交新事实；不改写旧样本或重用其序号。该报告错误不授予权限或延长 lease，也不等于凭据失效。未知配置、非法资源和畸形事实仍拒绝为无效请求；未验证的 409 不享受此精确重同步处理。
4. 检查观测时间、允许的时钟偏差和子项期限，不按接收时间重新生成测量时间。
5. 原子替换该节点已接受的报告，记录 receivedAt，并通知相关读取／订阅者。

旧 sequence 拒绝；同 sequence 同内容重试幂等，不刷新原报告的新鲜度；同 sequence 改内容拒绝。整个 snapshot 校验失败时保留前一份报告直到它自然失效，不部分接纳看似合法的资源。

snapshot 是完整清单：同实例中缺少以前的 socket／listener／link，表示它不再出现在本次清单里。不能因为合并 map 时漏做删除而让旧资源永久存活。

endpoint 可以来自配置、接口或 STUN，使用[不同来源字段](/v2/state-reference/endpoints)。配置来源需与 Server 已存值核对，接口／STUN 保留实际资源和原观测时刻；配置声明不能当作已验证可达。

## 上报频率

报告频率不能改变观测口径：资源变化可形成新快照，周期性联系只证明当前实例仍可联系。

新的联系报告只更新联系事实。没有新的 STUN 观测或 probe 时，沿用其原始 observedAt／expiresAt；不能靠 heartbeat 延长旧候选、旧延迟或旧认证证据。

历史事件不靠周期性 snapshot 复制产生。连接／探测 owner 的真实事件进入独立有界队列并批量上报，见[观测与历史](/v2/state-reference/observations)。

## 过期与故障

- 联系超时表示 Server 当前没有新鲜报告，不证明所有旧通道在同一瞬间断开。
- endpoint、link、probe 按各自期限失效；整体 State 活跃不能覆盖子项到期。
- 配置不匹配、实例更替、节点撤销会使对应执行状态不可用于建连。
- 遥测上报失败不直接改写管理员配置；数据面是否停止仍由自身授权／lease 规则决定。

## 返回值

`StateAccepted` 包含 sequence、receivedAt、duplicate。同序号同内容的重试保留原接收时间；它不是执行 lease 或配置应用回执。该入口不接受字段级 delta。
