# Server 内存容器

本页列 Server 持有的容器、索引和投影职责。字段为逻辑结构参考，实际内部布局以源码为准；公开 State 字段只在 [State Reference](/v2/state-reference) 定义，持久恢复依据见[数据库](/v2/storage/database)。所有队列／缓存有预算。

## ConfigurationCache

键为 `(networkId, revision)`，当前指针来自数据库。

| 字段 | 类型／作用 |
| --- | --- |
| networkId／revision | 绑定完整版本 |
| configuration | 不可变的解密／校验后 NetworkConfiguration |
| bindings | 稳定用户、listener 与数据集依据 |
| digest | 本版本确定编码的摘要 |
| addressIndex | 可重建的版本内地址占用索引 |
| compiled | 按依赖 digest／compilerVersion 区分的有界内部编译缓存 |

派生索引／编译缓存按需建立，不提供独立写入口。地址不存在于 RAM 索引不代表旧绑定已退出；分配还检查 AddressRetirement。内部编译图不作为 Node 自授权输入。

## NetworkState 与接受报告

| 容器 | 字段 | 含义 |
| --- | --- | --- |
| NetworkState | networkId／stateEpoch／sequence | 作用域与当前变化顺序 |
| NetworkState | configurationRevision | 解释该视图的配置版本 |
| NetworkState | nodes | NodeId → 已接受报告，不为每个读者复制整网 |
| StoredNodeState | instanceId／reportSequence | 来源实例／序号，未报告可为空 |
| StoredNodeState | receivedAt／reportExpiresAt | 原接受时间和有效期 |
| StoredNodeState | lastAuthenticatedAt | 真实认证联系，与报告新鲜度分开 |
| StoredNodeState | report | NodeState 或明确未报告 |

freshness 结合当前时间、实例、配置和撤销计算。查询或重复报告不延长原期限；失去某个读者的可见性不删除全局报告。永久退休提交可释放大报告，持久认证仍拒绝旧身份。

### 报告的共享类型 {#nodestate}

| 类型 | 唯一字段参考 |
| --- | --- |
| NodeState／ConfigurationState | [NodeState](/v2/state-reference/node) |
| Capability | [能力](/v2/state-reference/capabilities) |
| ListenerState／SocketState／LinkState／PathState | [资源与连接](/v2/state-reference/resources) |
| EndpointCandidate | [候选](/v2/state-reference/endpoints) |
| ProbeObservation／ObservationSample | [观测](/v2/state-reference/observations) · [实时观察](/v2/state-reference/realtime) |

报告里的安装引用按精确配置上下文接纳；正在安装的资源不自动作为当前可用 peer 入口。UDP 观测不证明 TCP 可达，单侧报告不制造另一端的事实。

## 派生投影

由配置、已接受报告、使用选择和固定数据集计算，不是另一份可写 State。

| 内容 | 输入与约束 |
| --- | --- |
| GeoIP／ASN／location | 合格 endpoint、观测期限和数据集版本；不刷新原时间、不补坐标 |
| links／paths 聚合 | 保留真实报告者、通道／路径标识和证据阶段 |
| 应用进度 | 固定提交目标、实际 receipt、当前实例与 lease |
| Node／成员 actions | 当前用户、网络权限和配置身份 |
| 方案／版本状态 | 真实选择、身份约束、能力与兼容规则；尚无输入时明确未知 |

具体投影见[网络视图](/v2/state-reference/network)与[IP 信息](/v2/state-reference/ip-info)。缓存 key／失效规则必须包含所有权威输入；尚未实现的派生能力不靠字段占位表示已完成。

## StreamSubscription 与 Journal

| 对象 | 字段 | 含义 |
| --- | --- | --- |
| Subscription | id／principal／networkId | 本进程订阅及已认证作用域 |
| Subscription | filter／authorizationDigest | 服务端验证的选择与当前裁剪依据 |
| Subscription | cursor／expiresAt | 发送位置和主体／订阅期限上界 |
| Journal | epoch／events／oldestCursor | 有界变化记录与可重放下界 |
| Delivery | pending／replay | 当前订阅可交付帧和恢复缓冲 |

principal 不由客户端声明权限，filter 不能扩权。同步截点、权限缩小和 cursor/reset 由 [SSE 合同](/v2/api/events)统一定义；客户端的已应用 cursor 与服务端发送位置不同。完整秘密配置不广播。

## Observation 运行态

公开任务／样本复用[实时观察类型](/v2/state-reference/realtime)，Server 额外持有：

| 字段 | 含义 |
| --- | --- |
| acceptedAt／network | 接纳时间与作用域 |
| task／paths | 当前任务视图和获准候选完整路径 |
| subscribers | 独立 ObservationSubscriptionId → 主体、授权依据、generation、expiresAt／renewAfter、终态 |
| seen | measurementId → 内容摘要、版本与窗口完成状态 |

同会话多标签页独立续租和取消；共享任务不共享权限。到期／终态停止诊断依赖，短期保留结束视图后移除，不能从重放队列复活旧样本。任务／每会话／全局均有预算。

数据库与内存变化保持事务边界：仅对实际写集保存撤销信息，失败恢复原任务；无变化读取／扫描不复制全部测量索引。维护任务由 App 持有、停止和等待退出，已接纳 SQL 工作独立完成。

## 其他工作状态

| 内容 | 内存责任 |
| --- | --- |
| 编译需求 | 固定已认证源、目标、配置／依赖和期限；持久资格不因缓存而延长 |
| 可信 DNS 关联 | 网络、源 Node、名称／CNAME、地址集合、执行上下文和 TTL |
| 资源下载／转换 | 当前定义／revision／artifact 基准、预算、取消和实际结果 |
| 限流／KDF／队列 | 明确作用域、容量与补充／释放依据 |

持久认证 challenge／短期凭据仍属于[认证存储](/v2/storage/database)，不因字段看似临时就复制进内存权威。Node 本地 socket、协议栈、包会话、TUN 和 DNS 缓存由节点组件拥有，Server 仅接纳其报告／协调结果。
