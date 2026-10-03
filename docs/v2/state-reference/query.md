# 查询与订阅

Node 可以主动从 Controller 查询所需 peer 的最新 State；需要持续变化时再订阅。**主动查询不需要服务端预先生成逐节点副本，但仍必须在服务端做权限过滤。**

## 可见范围

可见范围不只是“access 直接允许访问谁”，而是当前配置允许的业务及其执行依赖：

| 对象 | 为什么可能需要读取 |
| --- | --- |
| 自己 | 配置接收情况、派生地区及诊断 |
| 获准业务的对端 | 建连、接收与必要的反向拨号；不因此产生反向新流授权 |
| routing 必经／可选中转节点 | 取得执行获准路径所需的入口／候选 |
| 允许使用的出口 | 在方案范围内选择可用出口 |
| 其他 topology 邻接 | 仅在获准执行／协调确实需要时；不能仅凭默认 allow 就读全网 State |

例如 access 允许 A→B，routing 要求经 C，则 A 可以获得必要的 C listener／候选信息，即使没有访问 C 上业务服务的权限。**知道如何连接 C，不等于获得访问 C 服务的 access。**

原始报告中的全部内容不应原样转发：节点通常只需要相关资源的可达候选、能力、必要执行状态；不需要 peer 的全部接口、其他业务的 paths、详细 problems 或完整延迟历史。管理员／普通成员／Node 凭据采用各自的字段视图。

peer 查询只能在服务器计算的可见范围内进一步缩小。请求中指定一个 Node ID 不是授权；对不可见／不存在的请求对象统一返回不可用语义，不能借此枚举隐藏节点。

## Query

以下为读取操作示意，实际路径与字段尚未发布：

```json
{
  "nodes": ["<node-b>", "<node-c>"],
  "include": ["listeners", "endpoints", "capabilities"],
  "configurationRevision": 42,
  "nodeConfigurationDigest": "<requester-config-digest>"
}
```

- nodes 是希望查询的子集；省略时建议返回本次执行所需的相关节点，数量受预算限制。
- include 使用固定枚举，不开放任意 JSONPath；endpoints 是经字段裁剪的 socket 候选投影，保留实际资源关联。
- Config 引用用于发现调用方基准是否过旧，不赋予权限；服务端始终按当前身份／配置校验。

返回：

```json
{
  "type": "snapshot",
  "networkId": "<network-id>",
  "configurationRevision": 42,
  "stateEpoch": "<state-epoch>",
  "cursor": "<opaque-view-cursor>",
  "asOf": "2026-10-03T12:00:02Z",
  "nodes": {
    "<node-b>": {
      "freshness": "current",
      "instanceId": "<node-b-instance>",
      "reportSequence": 9,
      "receivedAt": "2026-10-03T12:00:00Z",
      "endpoints": [
        {
          "resource": { "kind": "listener", "key": "mesh-udp" },
          "generation": 3,
          "socketId": "7",
          "network": "udp",
          "address": "198.51.100.42:40001",
          "source": "stun",
          "validation": "observed",
          "observedAt": "2026-10-03T12:00:00Z",
          "expiresAt": "2026-10-03T12:00:30Z"
        }
      ]
    }
  }
}
```

响应省略未展开的其他投影字段。current 表示报告上下文／期限可用，不意味着每个 endpoint 都完成 peer 认证；每项还要检查自己的到期与验证状态。

## Watch / delta

可从 snapshot 返回的 cursor 订阅后续变化：

```json
{
  "type": "delta",
  "stateEpoch": "<state-epoch>",
  "baseCursor": "<previous-view-cursor>",
  "cursor": "<next-view-cursor>",
  "updated": {},
  "removed": ["<node-c>"]
}
```

建议初版每个 updated 项替换该 peer 的完整可见状态，removed 明确删除已缓存项；不提前设计每个深层字段的 patch。updated 和 removed 不得包含同一个 Node。

主动轮询也可以携带 cursor，只取变化；两种传输复用同一裁剪／过期逻辑。服务端可用有界事件缓冲，不承诺无限补发；cursor 过旧或 stateEpoch 改变时返回 reset，调用方清理旧视图并重取 snapshot。

watch State 只订阅已经存在的事实，不请求新探测。用户要实时测量某条路径时，使用独立的 [Realtime Observation](/v2/state-reference/realtime) 任务，明确源节点、路径、指标和租约。

权限、配置或查询范围改变时同样不能沿用旧视图：重新授权后发送删除或 reset；撤销整个读取资格时终止订阅。不能因最初握手通过，就永久推送后来无权看的数据。

## 一致性与规模

cursor 绑定网络、调用身份、查询范围和所依据的配置版本。State 可以在 Config 未变化时推进；A 与 B 的报告分别保留其源实例／配置引用，不伪造一个全网同时发生的观测。

节点可以先按需 query；频繁读取相同 peer 的场景再使用 watch，避免每轮下载整个 NetworkState。大管理视图可采用有界分页，但动态页不是冻结全网快照，必须返回读取时刻和相应一致性语义。

State 不携带协议 password、UUID、TLS 私钥或新的访问许可。这些属于受保护的 Config／材料发布。查询得到的候选必须在已有配置许可内使用，接收端仍独立执行[访问检查](/v2/configuration-reference/access#enforcement)。
