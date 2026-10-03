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
      "lastAuthenticatedAt": "2026-10-03T12:00:00Z",
      "freshness": "current",
      "report": {},
      "derived": {}
    }
  }
}
```

report／derived 在此省略内部字段；report 的完整内容见 [NodeState](/v2/state-reference/node)。Node 实体已经存在但没有报告时，查询可构造 `awaiting_report`、report=null，不虚构空能力或在线证据。

上例是查询视图。RAM 保存已接受 report 和来源／期限，derived／freshness 按当前输入生成；内部若缓存派生结果，配置、报告、选择或数据集变化须使缓存失效。

## Fields

| 字段 | 含义 |
| --- | --- |
| `stateEpoch` | Controller 当前 State 集合的世代；重新初始化后改变 |
| `sequence` | 本世代全网状态变化序号，独立于各节点 reportSequence 和配置 revision |
| `configurationRevision` | 当前解释／裁剪状态时采用的配置版本，不把历史报告重标为此版本 |
| `nodes` | Node ID → 已接受报告、Controller 接收元数据与派生值 |
| `freshness` | current／awaiting_report／expired／superseded／config_mismatch／revoked 等明确状态 |
| `lastAuthenticatedAt` | 节点最近真实认证联系；未取得时 null，不由浏览器推断 |

实际序号和 cursor 编码尚待合同定版。对外查询使用不透明、绑定视图的 cursor，不把全网内部序号当作任意可复用分页令牌。

## 派生值 {#derived}

Controller 可以派生，但必须保留输入来源：

- 联系／报告是否新鲜；
- reported 配置是否对应当前有效执行；
- 从可见 endpoint IP 得到的 GeoIP／ASN 数据；节点位置只取合格本机观测，并保留版本／有效期；
- 双侧 link 报告是否匹配；
- 当前可见节点和允许范围内的建连辅助信息。

GeoIP 不是 GPS。不能用 Controller 连接来源 IP、Mesh IP、管理员 country 或业务选择的出口 IP 替代要求的底层 socket 观测。

详细字段见 [GeoIP／ASN](/v2/state-reference/ip-info)。GeoIP 与 ASN 可独立缺失，位置、region／city、坐标／精度和 locationAbsent 不以零值替代。

## Console 组合视图

SSE／GET 可以组合以下明确类型，字段来源仍分别标识：

| 视图内容 | 来源 |
| --- | --- |
| Node 实体元数据 | id、registeredAt、createdBy、credentialsRevokedAt |
| 当前配置视图 | hostname、identity、addresses、enabled 及对应 revision |
| 当前运行报告 | 本页的实例、时间、report 与 freshness |
| actions | 当前用户、membership 和目标配置的服务端权限计算 |
| versionStatus | 当前真实构建报告与 Controller 发布／协议兼容规则 |
| links／paths | 当前有来源报告的服务端聚合 |
| recentLinks | 明确历史窗口内的真实事件摘要，可选图层 |

versionStatus 为 current、outdated（仍兼容但建议更新）、unsupported、unknown；缺报告／过期或无足够兼容依据时 unknown，不在 Console 比较版本字符串。联系状态由服务端阈值／原时间计算，不将最近 heartbeat 宣称为隧道健康。

GET 与 SSE 复用相同片段类型：NodeEntityView／MemberView 的元数据和 actions 通过 entities.snapshot／entities.delta 更新，settings 使用 ConfigurationView，运行报告使用 NodeStateView。actions 受配置影响时同样发送实体增量，客户端不为权限变化另外拼 GET 游标。

有实体但无配置、凭据已吊销、从未报告分别展示；当前列表删除 Node 后不再给出可操作项。历史／诊断保留的最小墓碑明确已删除、不可恢复，不伪造配置或在线状态。

### Links

| 字段 | 含义 |
| --- | --- |
| `a` / `b` | 节点对，用于布局／分组，不是唯一通道 key |
| `linkId` | 实际通道标识；同节点对多个通道分别保留 |
| `transport` / `initiatorNodeId` | 真实承载和发起方 |
| `phase` | 有来源的当前通道阶段 |
| `evidence` | oneSided／matched／conflicting／expired |
| `reports` | 各报告者的实例／序号／时间引用 |
| `inUse` | 有真实业务依赖时 true；证据不足为 null，不能默认 false |
| `since` | 实际建立时间；不知道时 null，不用 receivedAt 冒充 |

两端报告冲突时保留冲突，不强行合成一条健康边。聚合不抹掉主备、承载切换或建连方向，也不表示 access 双向。

### Paths

按源／目的分组，内部仍保留 pathId、完整有序 nodes、publication 和每个报告者的角色／证据／时间。并存旧流与新流可能走不同路径，不能压成一个“当前路径”而覆盖历史会话。

### RecentLinks

历史图层使用有界窗口并可切换，只从真实 link／probe 事件生成。每项保留 linkId、节点对、transport、lastEventAt、关闭原因和来源；可带最近成功 RTT 及其测量时间／方向，没有则 null。

首版历史层默认关闭；开启后以用户选定的固定 from／to 调用 history/query，使用明确的 recentLinks 结果模式。结果返回 `window: {from,to}`、coverage、cursor 和历史摘要，不加进默认实时 SSE。刷新窗口重新查询并校验范围，不能将滚动窗口复用为旧分页 cursor。

当前连接用当前报告显示，历史淡线明确标为历史。HistoryCoverage 只能解释数据裁剪／缺口，空闲需要 link_closed 等真实原因；不能因为没有样本推断整段时间一直空闲。截断时返回窗口和覆盖限制，不展开 topology 的 N² 允许图，也不新增探测填充画面。

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
