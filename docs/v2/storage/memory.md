# Server 内存容器

本页列 Server 的内存容器和投影职责。内部字段使用源码命名；公开 State 字段只在 [State Reference](/v2/state-reference) 定义，持久恢复依据见[数据库](/v2/storage/database)。所有队列／缓存有预算。

## SnapshotCache

键为 `(networkId, revision)`，当前指针来自数据库。

| 字段 | 类型／作用 |
| --- | --- |
| network_id／revision | 绑定完整版本 |
| document | 不可变 ConfigurationDocument，含原文与完整规范值 |
| bindings | 稳定用户、listener 与数据集依据 |
| encoded_size | 缓存项的编码大小预算 |
| ciphertext_digest | 确切 SQL 密文指纹；事务回滚后 revision 复用不能命中旧项 |

缓存最多 32 项、按编码大小合计 64 MiB。它不回答当前权限或 currentRevision；命中前仍读取确切 SQL 版本并核对密文。地址分配另检查 AddressRetirement，缓存缺少地址不证明旧绑定已退出。

## StateHub 与接受报告

| 容器 | 字段 | 含义 |
| --- | --- | --- |
| StateHub | epoch／sequences／exhausted | 当前状态世代、各网络顺序及耗尽边界 |
| StateHub | reports | (NetworkId, NodeId) → StoredReport |
| StateHub | events／bytes | 统一事件日志与报告总预算 |
| StoredReport | report | 完整 StateReport，含来源实例／序号／NodeState |
| StoredReport | instance_generation | 接受时的实例代际 |
| StoredReport | received_at／expires_at | 原接受时间和有效期 |
| StoredReport | authenticated_at | 真实认证联系，与报告新鲜度分开 |
| StoredReport | bytes／next_expiration | 大小及下个子项失效时间 |

freshness 结合当前时间、实例、配置和撤销计算。单份报告最多 1 MiB、每网络最多 10000 份、进程报告预算 128 MiB。报告期限 60 秒，时钟偏差校验 30 秒；子项仍保留各自期限。查询或重复报告不延长原报告期限；永久退休提交可释放大报告，持久认证仍拒绝旧身份。

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
| 方案状态 | 真实选择和当前身份／allowed／locked 约束 |

具体投影见[网络视图](/v2/state-reference/network)与[IP 信息](/v2/state-reference/ip-info)。查询裁剪不产生另一份可编辑 State，也不补造缺失结果。

## StreamSubscription 与 Journal

| 对象 | 字段 | 含义 |
| --- | --- | --- |
| Subscription | identity／owner | 已认证读取主体，及用于每主体流／断线缓存上限的身份指纹 |
| Subscription | authorization | 当前裁剪依据的摘要 |
| Subscription | cursor／deadline | 发送位置和下一次投影的最早自算截止 |
| Subscription | pending／replay | 当前可交付帧和恢复缓冲（`ReplayBuffer`） |
| Subscription | disconnected_at | 断线时间，用于有界断线缓存 |
| EventLog | epoch／sequence／entries | 有界变化记录与可重放下界 |

identity 不由客户端声明权限，订阅输入不能扩权。同步截点、权限缩小和 cursor/reset 由 [SSE 合同](/v2/api/events)统一定义；客户端的已应用 cursor 与服务端发送位置不同。完整秘密配置不广播。

事件日志保留最多 512 项；单帧最多 2 MiB、批次最多 8 MiB。每次发帧重新校验当前权限，包括初始快照和重放缓冲中的帧。

## Observation 运行态

公开任务／样本复用[实时观察类型](/v2/state-reference/realtime)，Server 额外持有：

| 字段 | 含义 |
| --- | --- |
| accepted_at／network | 接纳时间与作用域 |
| segment_high_water／window_high_water | 已接纳的路径 segment 下界与样本窗口结束时间高水位 |
| task／paths | 当前任务视图和获准候选完整路径 |
| subscribers | 独立 ObservationSubscriptionId → principal、authorization_digest、generation、expires_at／renew_after、terminal |
| seen | EventId → 内容摘要、版本与窗口是否部分完成 |

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
