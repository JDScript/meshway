# 查询与订阅

State 查询返回当前身份可见的事实，不授予新权限。Node 读取自身及执行所需 peer，用户按网络角色和设备身份读取；规则见[权限](/v2/permissions)。

## Query

| 操作 | 用途 |
| --- | --- |
| getNetworkState | 用户读取当前网络可见 StateView |
| getNodeState | 用户读取指定 Node 的 StateView 子集 |
| queryPeerState | Node 查询自身及获准执行依赖，服务端从凭据确定网络 |

queryPeerState 接受 StateQuery：

<!-- schema: StateQuery -->
```json
{
  "nodes": ["00000000-0000-4000-8000-000000000002"]
}
```

nodes 省略／null 时使用服务端可见集合，空数组表示空子集。指定 ID 只能缩小范围，不获得其他 Node 的存在性或内容。当前请求没有 include、任意 JSONPath 或客户端指定的授权 revision。

返回 [StateView](/v2/state-reference/network)。GET／query 不返回可用于开启 SSE 的 cursor，流的恢复点来自完整 SSE 同步。

## 可见范围

| 对象 | 可见依据 |
| --- | --- |
| 自身 | 当前 Node／用户设备身份 |
| 业务对端 | access 与实际执行依赖 |
| 中转／出口 | 获准完整路径或方案所需的执行依赖 |
| 其他邻接 | 确有获准协调需要；topology 默认允许不等于读取全网 |

例如 A→B 必经 C，A 可取得必要的 C 入口信息，不因此获得访问 C 上业务服务的许可。未知依赖由[业务需求](/v2/execution)解析，不由查询中的任意中转 ID 决定。

netmap 链（实现中，Node 尚未切换）：可见范围是读者所属节点当前 netmap 投影的节点并集，与是否存在业务需求或观察无关，只随 netmap serial 变化；Node 读者只获得其可拨的 peer listener 端点细节。

完整报告和 peer 报告使用不同类型。peer 视图不转发全部接口、无关路径、秘密或详细问题；端点仍保留原 socket／来源和期限。

## Watch

watchNetwork／watchNode 首次发送完整可见批次，再发送 delta；数据结构、cursor、reset 与权限重检见 [SSE](/v2/api/events)。updated 替换完整可见项，removed 删除旧项，不能仅对 map 做追加合并。

订阅只消费已有事实，不触发探测。需要测量时创建[观察任务](/v2/state-reference/realtime)。配置或身份变化会重裁剪，即使没有新上报；过期候选不再作为可用入口返回。

查询时间不改变报告时间。不同 Node 的报告保留各自实例和执行引用，不把一个查询截点描述成全网同时采样，也不将可见性当作业务授权。
