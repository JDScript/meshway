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
3. 对资源和执行报告检查其关联的已下发配置；socket 的用途／generation 与资源引用必须一致。
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
