# Mesh 路径诊断与 ping

`meshway ping` 由 daemon 执行获准的端到端 Mesh 探测，展示实际路径、源端通道和往返耗时。它与 [Realtime Observation](/v2/state-reference/realtime) 共用 Server 任务、执行许可和测量来源。

## 命令

```sh
meshway ping peer-name
meshway ping 100.64.0.2 --count 3
meshway ping fd12:3456::2 -c 3 --json
```

| 参数／行为 | 合同 |
| --- | --- |
| target | 当前网络的 hostname、本网络 FQDN 或精确分配给某节点的 Mesh IP，由 Server 在授权事务内解析 |
| `-c`／`--count` | 1–1000，默认 10 |
| `--json` | 每行一个类型化 `PingEvent`，保持真实事件顺序 |
| probe | 默认 `icmp:*` 服务授权、auto 路径选择；单个探测期限 5 秒，发送间隔至少 1 秒，每任务一个 in-flight |
| Ctrl-C | 停止本机等待并取消本订阅，退出码 130；不修改持久 up/down 意图 |
| 退出码 | 正常完成且存在成功回复为 0；全部失败／超时或操作错误为 1 |

当前入口依托 Linux TUN runtime 中已安装的 Manager／SecureNode。未上线或真实资格已失效时返回明确错误；`control-only` 与非支持的运行模式返回 unsupported。正常 publication 同步中的请求可进入有界 Preparing 队列，由实际 Control owner 检查当前实例，安装 ready 后才创建订阅和探测；排队不授予权限。请求固定本机 operator revision，down／再次 up 不会复活旧请求。

## 授权与路径

默认探测需要源到目标的 `icmp:*` 权限，按同一 Access、routing、topology 和当前方案计算完整路径，保留必经节点。TCP／UDP 服务权限不能代替 ICMP 权限。输入某个 IPv6 Mesh 地址只负责选择其所属 Node，不强制使用 IPv6 外层承载。

诊断使用独立 `DiagnosticPermit` 和与业务共用的候选 wire（版本见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)）上的诊断 request／reply，复用同一 SecureNode 的 Noise、逐跳证明和原路径回复。探测不经过两端 OS ICMP 栈，不建立业务 FlowTable 记录；成功说明获准 Node 路径完成了认证往返，不代表某个应用端口接受连接。

源、每个中转和终点均需声明并实现 `feature:path-observation` version 1。获准路径缺少必需能力时任务为 unsupported；没有 Access 或违反完整路径时为权限拒绝。具备能力的完整候选仍可使用，不能借缺能力跳过必经点。

## 输出事实

- **preparing**：等待 Server 接纳、发布、安装、建连、下一次发送周期或结果上报，显示本机等待耗时。真正 setup 的独立预算为 60 秒，跨合法交接／暂停保持原截止；正常周期与结果背压不充当新的建连。尚未发送的等待不生成 measurement，也不消耗 ping 次数或计为 probe loss；setup 超限以失败 phase 结束。
- **direct**：已选完整路径只有源和目标；显示本次 request 的实际 remote `IP:port`、transport 和 acquisition。
- **via**：按实际完整路径列出有序中转 Node ID；当前接口不额外披露中转 hostname。
- **STUN**：仅在成功协调的已认证通道保存了对应 offer 的 STUN 来源时标注。地址使用实际通道的 remote，不以候选列表中的地址冒充已建立通道。
- **timeout／local-failed**：保留失败类型和空 RTT；不补成 0。多跳路径没有一个代表整条路径的 transport，实际源端通道位于 `probe.channels`。

`ProbeChannels.request` 保存发送关联的 channel/socket ID、peer、transport、本地／远端数值地址和获取来源；`reply` 是必填可空字段，表示实际收到回复时的源端通道。通道切换时保留发送与接收各自的事实，不用完成时的 snapshot 覆盖它们。

## 计时与执行

准备阶段日志不包含凭据。INFO 的 `Diagnostic preparation stage changed` 使用稳定 `stage`：`subscription_queued`（本机接纳）、`subscription_accepted`（订阅响应携带任务 phase／publication 引用，不等于已安装许可）、`installed`（任务许可与本机已安装执行匹配并通过绑定校验）、`first_hop_channel`（等待首跳认证承载）、`path_handshake`（完整路径握手）、`path_established`（握手完成）。事件携带适用的 subscription／observation／publication／instance 引用；首跳与握手阶段附带本机 setup 耗时。DEBUG 提供 `subscription_request`、`installation_pending` 与握手重发事件。`Diagnostic preparation budget expired` 记录最后等待阶段／订阅确认状态，不作为 RTT。

当前 IPC v4 的 Preparing 只携带 `elapsedMs`，CLI 显示等待耗时；具体阶段由上述日志关联，不从耗时推断 STUN、安装或某段路径失败。

RTT 使用源端单调时钟：从完整 probe 被正式 transport sender 接纳，到完整回复被 receiver 取得并在之后通过 SecureNode 认证／关联验证。它包含发送排队、承载与终点处理；控制面请求、建连和结果上报不进入该差值。

本机发送须达到 `LocalComplete` 才形成有效发送回执。未接纳、失败或完成不确定单独计为本机失败。只有实际发送后未在期限内获得有效回复，才进入探测未回应统计；它不是底层 wire 丢包率。

source 每 observation 跨 CID 至少间隔 1 秒；relay／terminal 使用每秒一个 token、burst 2 的有界接收预算，容纳正常延迟抖动造成的压缩到达。关联记录、任务、发送和结果队列均有界。窗口为不重叠的最终测量窗口，重复读取或上报重试不生成新样本；traffic 当前仍为 unsupported。

## 本机与远端生命周期

IPC v4 的一次 Ping 请求可返回多个 `ResultBody::Ping`，复用原 Response envelope；sequence 从 0 连续增长。Preparing 可先于唯一 Accepted，Sample 绑定 Accepted 的 ObservationId，Completed／Failed 结束流。无终态的 EOF 为错误；初始请求及每帧等待有界，取消不自动发起第二次测量。

Node 订阅绑定真实 control key、instance／generation、网络和 Server boot。CLI 与用户订阅可共享同一 source task；取消一个订阅只移除它的需求，其他合法订阅继续。Node 普通续租、tasks 读取、State 和结果上报都不续活观察。

Control 以 RPC 开始时的时钟高水位和单调时钟固定已确认的订阅截止；相同响应重放、失败续租或时钟回拨均不延长它。结果上报受阻时仍提供有界等待进度；到期由独立本机计时明确结束 CLI，不等待 HTTP 返回，也不以该本机订阅到期暂停其他有效共享订阅。

实例替换、权限撤回、租约／原 probe deadline 到达、scope 失效均阻止旧排队输出和样本交付。合法直接交接可保留 cipher，旧 request／reply／sample 不换绑到延长后的权限。daemon 重启不恢复旧 CLI ping 意图。

## Node 订阅 API

这些操作使用 Node 凭据，由唯一 Control driver 消费生成客户端；不接受客户端声明 sourceNodeId 或 networkId。

| operation | 路由与输入 |
| --- | --- |
| `createNodeObservation` | `POST /agent-api/v1/observations`，`CreateNodeObservation` |
| `getNodeObservation` | `GET /agent-api/v1/observations/{subscriptionId}`，必需 instanceId／generation query |
| `renewNodeObservation` | `POST /agent-api/v1/observations/{subscriptionId}/renew`，当前 instance 与绝对 expiresAt |
| `cancelNodeObservation` | `POST /agent-api/v1/observations/{subscriptionId}/cancel`，当前 instance；返回源 executor 是否仍有共享 task |

任务领取和结果使用既有 `getNodeTasks`／`reportTaskResult`。授权、30 秒订阅上限、10 秒续租节奏、SSE 与结果窗口的共同定义见 [Realtime Observation](/v2/state-reference/realtime)。

## 限制

`traffic` 测量当前为 unsupported；中转 hostname 不额外披露；daemon 重启不恢复旧 CLI ping 意图。验收状态见[验收范围](/v2/node/validation)。
