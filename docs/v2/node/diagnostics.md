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
| `-v`／`--verbose` | 人读输出另显示目标 Node ID、preparing 等待与每个通道的获取来源 |
| probe | 默认 `icmp:*` 服务授权、auto 路径选择（与业务首包同一规则：首跳有通道的第一条可观测路径，见[中继兜底](/v2/node/connectivity#relay-fallback)）；单个探测期限 5 秒，发送间隔至少 1 秒，每任务一个 in-flight |
| Ctrl-C | 停止本机等待并取消本订阅，退出码 130；不修改持久 up/down 意图 |
| 退出码 | 正常完成且存在成功回复为 0；全部失败／超时或操作错误为 1 |

当前入口依托 Linux TUN runtime 中已安装的 Manager／SecureNode。未上线或真实资格已失效时返回明确错误；`control-only` 与非支持的运行模式返回 unsupported。正常 publication 同步中的请求可进入有界 Preparing 队列，由实际 Control owner 检查当前实例，安装 ready 后才创建订阅和探测；排队不授予权限。请求固定本机 operator revision，down／再次 up 不会复活旧请求。

## 授权与路径

默认探测需要源到目标的 `icmp:*` 权限，按同一 Access、routing、topology 和当前方案计算完整路径，保留必经节点。TCP／UDP 服务权限不能代替 ICMP 权限。输入某个 IPv6 Mesh 地址只负责选择其所属 Node，不强制使用 IPv6 外层承载。

诊断使用独立 `DiagnosticPermit` 和与业务共用的候选 wire（版本见[受保护 circuit](/v2/node/dataplane#候选受保护-circuit)）上的诊断 request／reply，复用同一 SecureNode 的 Noise、逐跳证明和原路径回复。探测不经过两端 OS ICMP 栈，不建立业务 FlowTable 记录；成功说明获准 Node 路径完成了认证往返，不代表某个应用端口接受连接。

许可在参与节点上保留同一完整候选集合；可选中转只须参与其中至少一条完整路径，不要求出现在所有候选中。每次实际握手与包准入仍检查所选完整路径及本节点在该路径中的位置，安装许可不授予其他候选的转发权限。

源、每个中转和终点均需声明并实现 `feature:path-observation` version 1。获准路径缺少必需能力时任务为 unsupported；没有 Access 或违反完整路径时为权限拒绝。具备能力的完整候选仍可使用，不能借缺能力跳过必经点。

## 人读输出

每次探测一行，最后一行汇总；preparing 只在 `-v` 下显示：

```text
pong from meshway-nyc (100.64.0.6) via 192.168.97.2:35000 native_udp in 0.10 ms
pong from meshway-nyc (100.64.0.6) via relay meshway-lax (192.168.97.3:52242 native_udp) in 0.22 ms
no reply from meshway-nyc (100.64.0.6) within 5 s
meshway-nyc: 3 sent, 3 replied, 0% loss · rtt min/avg/max 0.22/1.43/2.07 ms
```

direct 显示本次 request 通道的实际远端与承载；中继按路径顺序显示中转节点名，括号内是首跳通道（不是整条路径的承载）。中转名称取自本节点已安装 netmap 的 peers（`status`），取不到时显示 Node ID。RTT 为测量值截断到 0.01 ms；没有测量时写明，不补零。loss 为 `timeout／sent`，本机发送失败单独计数。

## 输出事实

- **preparing**：等待 Server 接纳、发布、安装、建连、下一次发送周期或结果上报，显示本机等待耗时。真正 setup 的独立预算为 60 秒，跨合法交接／暂停保持原截止；正常周期与结果背压不充当新的建连。尚未发送的等待不生成 measurement，也不消耗 ping 次数或计为 probe loss；setup 超限以失败 phase 结束。
- **direct**：已选完整路径只有源和目标；显示本次 request 的实际 remote `IP:port`、transport 和 acquisition。
- **via**：按实际完整路径列出有序中转 Node ID；当前接口不额外披露中转 hostname。
- **STUN**：仅在成功协调的已认证通道保存了对应 offer 的 STUN 来源时标注。地址使用实际通道的 remote，不以候选列表中的地址冒充已建立通道。
- **timeout／local-failed**：保留失败类型和空 RTT；不补成 0。多跳路径没有一个代表整条路径的 transport，实际源端通道位于 `probe.channels`。

`ProbeChannels.request` 保存发送关联的 channel/socket ID、peer、transport、本地／远端数值地址和获取来源；`reply` 是必填可空字段，表示实际收到回复时的源端通道。通道切换时保留发送与接收各自的事实，不用完成时的 snapshot 覆盖它们。

## 计时与执行

准备阶段日志不包含凭据。INFO 的 `Diagnostic preparation stage changed` 使用稳定 `stage`：`subscription_queued`（本机接纳）、`subscription_accepted`（订阅响应携带任务 phase／publication 引用，不等于已安装许可）、`installed`（任务许可与本机已安装执行匹配并通过绑定校验）、`first_hop_channel`（等待首跳认证承载）、`path_handshake`（完整路径握手）、`path_established`（握手完成）。事件携带适用的 subscription／observation／publication／instance 引用；首跳与握手阶段附带本机 setup 耗时。DEBUG 提供 `subscription_request`、`installation_pending` 与握手重发事件。`Diagnostic preparation budget expired` 记录最后等待阶段／订阅确认状态，不作为 RTT。

IPC 的 Preparing 只携带 `elapsedMs`，`ping -v` 显示等待耗时；具体阶段由上述日志关联，不从耗时推断 STUN、安装或某段路径失败。

首轮准备由安装、认证通道和握手接收事件推进，不等待额外的 1 秒首轮节拍或 100 ms 握手完成轮询。握手重发仍由 SecureNode 的原重发期限决定；后续 probe 保持至少 1 秒的真实发送间隔，合法交接不重置已有 probe 的节流截止，迟到的就绪事件不越过原 setup 截止。`first_hop_channel` 到 `path_handshake` 是邻接获取耗时，`path_handshake` 到 `path_established` 才是完整路径握手耗时。

新排队的 CLI 订阅优先于已接纳订阅的维护和后台控制请求取得一次有界执行机会。`subscription_accepted` 日志中的 `queue_ms`／`request_ms` 分别记录本机排队和请求往返；Server 的 `Node observation subscription admitted` 记录数据库队列及事务处理耗时。订阅响应中的任务元数据仅可在对应 execution grant 到达后使用一次；只有它覆盖当前全部源端诊断许可时才能省去额外 tasks 查询，缺少用户订阅等任一任务时仍读取完整任务列表。该优化不把订阅响应当成许可，也不把旧元数据或输出换绑到后继 publication。

RTT 使用源端单调时钟：从完整 probe 被正式 transport sender 接纳，到完整回复被 receiver 取得并在之后通过 SecureNode 认证／关联验证。它包含发送排队、承载与终点处理；控制面请求、建连和结果上报不进入该差值。

本机发送须达到 `LocalComplete` 才形成有效发送回执。未接纳、失败或完成不确定单独计为本机失败。只有实际发送后未在期限内获得有效回复，才进入探测未回应统计；它不是底层 wire 丢包率。

source 每 observation 跨 CID 至少间隔 1 秒；relay／terminal 使用每秒一个 token、burst 2 的有界接收预算，容纳正常延迟抖动造成的压缩到达。关联记录、任务、发送和结果队列均有界。窗口为不重叠的最终测量窗口，重复读取或上报重试不生成新样本；traffic 当前仍为 unsupported。

## 本机与远端生命周期

一次 IPC Ping 请求可返回多个 `ResultBody::Ping`，复用原 Response envelope；sequence 从 0 连续增长。Preparing 可先于唯一 Accepted，Sample 绑定 Accepted 的 ObservationId，Completed／Failed 结束流。无终态的 EOF 为错误；初始请求及每帧等待有界，取消不自动发起第二次测量。

Node 订阅绑定真实 control key、instance／generation、网络和 Server boot。CLI 与用户订阅可共享同一 source task；取消一个订阅只移除它的需求，其他合法订阅继续。Node 普通续租、tasks 读取、State 和结果上报都不续活观察。

创建订阅把诊断需求加入联合编译；只有实际执行切片变化的节点取得新的 publication，相同编译结果复用既有 publication。读取、正常采样和结果上报本身不改变诊断需求，也不产生新的 publication。当前 `DiagnosticPermit` 的截止时间属于不可变执行切片：订阅续租若延长共享任务的有效截止，会发布合法后继；同一续租重放或未改变有效截止的共享订阅更新复用既有目标。取消最后一个订阅、到期、权限／路径或参与节点资格变化仍须撤回或重新编译，旧样本不会换绑到后继 publication。

Server 的观察维护由已提交的变化通知及真实订阅／保留截止驱动；通知在有界窗口内合并，截止不被推迟。空闲时不以固定频率重复全量重授权；每次实际 API 操作仍在事务中检查身份和权限，发布改变授权绑定时重新检查任务视图。

Control 以 RPC 开始时的时钟高水位和单调时钟固定已确认的订阅截止；相同响应重放、失败续租或时钟回拨均不延长它。结果上报受阻时仍提供有界等待进度；到期由独立本机计时明确结束 CLI，不等待 HTTP 返回，也不以该本机订阅到期暂停其他有效共享订阅。

实例替换、权限撤回、租约／原 probe deadline 到达、scope 失效均阻止旧排队输出和样本交付。合法直接交接可保留 cipher，旧 request／reply 输出及源端 sample 不换绑到延长后的权限；中继已提交关联状态对新入站 reply 的接续规则见[发送流水线与连续性](/v2/node/dataplane#发送流水线与连续性)。daemon 重启不恢复旧 CLI ping 意图。

## Node 订阅 API

这些操作使用 Node 凭据，由唯一 Control driver 消费生成客户端；不接受客户端声明 sourceNodeId 或 networkId。

| operation | 路由与输入 |
| --- | --- |
| `createNodeObservation` | `POST /agent-api/v1/observations`，`CreateNodeObservation` |
| `getNodeObservation` | `GET /agent-api/v1/observations/{subscriptionId}`，必需 instanceId／generation query |
| `renewNodeObservation` | `POST /agent-api/v1/observations/{subscriptionId}/renew`，当前 instance 与绝对 expiresAt |
| `cancelNodeObservation` | `POST /agent-api/v1/observations/{subscriptionId}/cancel`，当前 instance；返回源 executor 是否仍有共享 task |

任务领取和结果使用既有 `getNodeTasks`／`reportTaskResult`。授权、30 秒订阅上限、10 秒续租节奏、SSE 与结果窗口的共同定义见 [Realtime Observation](/v2/state-reference/realtime)。

## netcheck {#netcheck}

`meshway netcheck` 让 daemon 执行一次有界诊断，复用现有观测，不在 CLI 中实现 STUN：

| 部分 | 来源与含义 |
| --- | --- |
| UDP socket 映射 | 已安装 Manager 的每个 Native UDP traversal socket 立即按其[持续观测](/v2/node/connectivity#continuous-stun)执行一轮同 socket STUN（同一查询预算与元数据，不新建 socket），最多等待 5 秒；报告每个 observer 最近一次经验证的映射与 `fresh`（本次诊断内获得） |
| 端口 | `preserved`：本次映射端口都等于绑定端口；`changed`：有不同端口；`unknown`：本次无映射 |
| 映射 | 只比较不同 IP 的 observer：`independent`：两个以上不同 IP 的 observer 本次看到相同映射；`dependent`：不同（端点相关映射 NAT，对端从 STUN 未见过的端口到达，见[来源学习](/v2/node/connectivity#source-learning)）；`unknown`：不同 IP 的 observer 不足两个（只配置一个 STUN 服务器时总是 unknown） |
| 过滤 | 不报告：判断过滤行为需要观测点从其他地址主动发包（RFC 5780 CHANGE-REQUEST），公共 STUN 服务器不提供 |
| 外层 PMTU | 每个 observer：`sizeProbe` 为本次向它发送的一个 DF Binding 请求（以可忽略属性填充到路由 MTU，最多 1,500 B）及是否得到回答；回答证明该大小可达，未回答不是测量（observer 可能丢弃大请求）。`pathMtu` 为探测后到该 observer 的路由 MTU（ICMP 学到的更小值，否则接口 MTU）。通道自己的路径 MTU 见 `meshway status` 的 MTU 列 |
| STUN RTT | 各 observer 本次最快的往返；只计单次发送即得回答的事务，重传后的往返不确定，记为 null |
| IPv4／IPv6／UDP | 本次该地址族有映射；UDP 另在存在已认证 Native UDP 选定通道时为真 |
| Server | Node 控制客户端（同一受保护 socket 与 origin）对只读 `getAuthOptions` 先预热一次，再计时第二次往返，5 秒上限；不使用凭据 |

没有已安装数据面（down、control-only、准备中）时 `unavailable=notRunning`，只报告 Server 部分。诊断不改变 State、候选、授权或 up/down 意图；结果只返回 IPC。`--json` 输出 `NetcheckReport`；人读输出：

```text
Network check
  Server     https://meshway-dev-api.jdscript.dev/ · reachable in 80.15 ms
  UDP        yes
  IPv4       yes
  IPv6       no
  STUN       74.125.250.129:19302 in 38.97 ms · 1500 B DF answered · route MTU 1500

SOCKET                    BOUND          MAPPED                OBSERVER              PORT     MAPPING
listener lab-udp          0.0.0.0:35000  104.28.152.187:36928  74.125.250.129:19302  changed  unknown
dial meshway-sha/lab-udp  [::]:40651     —                     —                     unknown  unknown
```

## 限制

`traffic` 测量当前为 unsupported；中转 hostname 不额外披露；daemon 重启不恢复旧 CLI ping 意图。验收状态见[验收范围](/v2/node/validation)。
