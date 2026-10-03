# Node → Controller

节点上报自身 State 的目标流程。**先支持完整的单节点快照**，没有必要每次上传整张网络；字段级 delta 可在有实际带宽需求时加入。

## Report structure

```json
{
  "instanceId": "<current-node-instance>",
  "sequence": 18,
  "kind": "snapshot",
  "observedAt": "2026-10-03T12:00:00Z",
  "state": {
    "system": { "hostname": "linux-host", "os": "linux", "arch": "arm64", "agentVersion": "<version>" },
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

envelope 不接受任意 source Node／network；Controller 从节点凭据确定作用域。instance 必须是已确认的当前运行实例，不能仅换一个随机字符串就接管旧运行代际。

## 接收步骤

1. 检查节点凭据、网络、撤销状态和当前实例。
2. 校验类型、数量／字节预算和 sequence。
3. 对资源和执行报告检查其关联的已下发配置；socket 的用途／generation 与资源引用必须一致。
4. 检查观测时间、允许的时钟偏差和子项期限，不按接收时间重新生成测量时间。
5. 原子替换该节点已接受的报告，记录 receivedAt，并通知相关读取／订阅者。

旧 sequence 拒绝；同 sequence 同内容重试幂等，不刷新原报告的新鲜度；同 sequence 改内容拒绝。整个 snapshot 校验失败时保留前一份报告直到它自然失效，不部分接纳看似合法的资源。

snapshot 是完整清单：同实例中缺少以前的 socket／listener／link，表示它不再出现在本次清单里。不能因为合并 map 时漏做删除而让旧资源永久存活。

endpoint 可以来自配置、接口或 STUN，使用[不同来源字段](/v2/state-reference/endpoints)。配置来源需与 Controller 已存值核对，接口／STUN 保留实际资源和原观测时刻；配置声明不能当作已验证可达。

## 上报频率

建议资源变化事件触发上报，并做有界合并；周期性报告用于确认当前实例仍可联系，频率由平台策略统一定义。具体周期与预算待定，不增加设备手工 CLI 配置。

新的联系报告只更新联系事实。没有新的 STUN 观测或 probe 时，沿用其原始 observedAt／expiresAt；不能靠 heartbeat 延长旧候选、旧延迟或旧认证证据。

历史事件不靠周期性 snapshot 复制产生。连接／探测 owner 的真实事件进入独立有界队列并批量上报，见[观测与历史](/v2/state-reference/observations)。

## 过期与故障

- 联系超时表示 Controller 当前没有新鲜报告，不证明所有旧通道在同一瞬间断开。
- endpoint、link、probe 按各自期限失效；整体 State 活跃不能覆盖子项到期。
- 配置不匹配、实例更替、节点撤销会使对应执行状态不可用于建连。
- 遥测上报失败不直接改写管理员配置；数据面是否停止仍由自身授权／lease 规则决定。

## 后续 delta

若支持节点上报 delta，它必须有同实例的 baseSequence，使用明确的更新／删除操作。缺少基础时请求 snapshot；不能把未知基础上的局部更新当完整状态。先保证 snapshot、删除和过期语义，再决定具体 delta 编码。
