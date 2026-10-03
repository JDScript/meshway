# NetworkState

Controller 在网络作用域内维护的最新状态集合。**逻辑上是全网视图，物理上可以是内存 map；不需要每次拼装或保存一份巨大 JSON。**

## Structure

```json
{
  "networkId": "<network-id>",
  "stateEpoch": "<controller-state-epoch>",
  "sequence": 205,
  "configurationRevision": 42,
  "nodes": {
    "<node-a>": {
      "instanceId": "<node-a-instance>",
      "reportSequence": 18,
      "receivedAt": "2026-10-03T12:00:00Z",
      "reportExpiresAt": "2026-10-03T12:01:00Z",
      "freshness": "current",
      "report": {},
      "derived": {}
    }
  }
}
```

report／derived 在此省略内部字段；report 的完整内容见 [NodeState](/v2/state-reference/node)。Node 实体已经存在但没有报告时，查询可构造 `awaiting_report`、report=null，不虚构空能力或在线证据。

## Fields

| 字段 | 含义 |
| --- | --- |
| `stateEpoch` | Controller 当前 State 集合的世代；重新初始化后改变 |
| `sequence` | 本世代全网状态变化序号，独立于各节点 reportSequence 和配置 revision |
| `configurationRevision` | 当前解释／裁剪状态时采用的配置版本，不把历史报告重标为此版本 |
| `nodes` | Node ID → 已接受报告、Controller 接收元数据与派生值 |
| `freshness` | current／awaiting_report／expired／superseded／config_mismatch／revoked 等明确状态 |

实际序号和 cursor 编码尚待合同定版。对外查询使用不透明、绑定视图的 cursor，不把全网内部序号当作任意可复用分页令牌。

## 派生值 {#derived}

Controller 可以派生，但必须保留输入来源：

- 联系／报告是否新鲜；
- reported 配置是否对应当前有效执行；
- 从 socket 公网 IP 观测得到的 GeoIP 近似位置、数据集版本和有效期；
- 双侧 link 报告是否匹配；
- 当前可见节点和允许范围内的建连辅助信息。

GeoIP 不是 GPS。不能用 Controller 连接来源 IP、Mesh IP、管理员 country 或业务选择的出口 IP 替代要求的底层 socket 观测。

最近一次认证联系与报告／子观测的新鲜度分别维护。一个重试请求可以证明节点仍能联系 Controller，但不能更新旧 report 的 receivedAt，或刷新旧 STUN／probe 的期限。

## 存储方式

```text
NetworkId
  → NodeId
      → 当前接受的 NodeState + 接收元数据
```

每次更新对应节点，不复制整个网络。可以按 peer／资源建立内存索引，但它们都可重建；没有独立可编辑的 Link Entity。

links／paths 的全网聚合按查询需要生成或缓存，不提前展开所有 Node 两两组合。订阅仅维护 cursor／过滤范围等必要状态，不要求为每个节点维护一份永久网络副本。

## 失效与重启

查询时必须依据当前时钟重新计算 freshness；订阅还应通知到期／移除，即使期间没有新上报。旧 endpoint 不再放进“可用于建连”的结果；管理诊断若展示最后一次记录，明确标为历史／过期。

State 主要存 RAM。重启后更换 stateEpoch，等待新的完整报告；可恢复的旧观测只作历史，不能按启动时间延长 TTL。旧 cursor 要求重新获取 snapshot。

跨重启的用户选择、身份撤销、实例隔离下界和必要回执独立持久化；它们不是这份最新观测 map。完整分类见[存储与访问](/v2/storage)。

配置变化会触发可见范围及兼容性重新计算，即使节点没有新报告。Node／User 身份、凭据、撤销与配置历史仍按各自持久存储处理，不随 State 丢失而恢复权限。
