# Server 接入组件

`meshway-control` 提供由宿主驱动的 Node 控制生命周期，HTTP 操作全部使用生成的 `meshway-control-client`。当前包含浏览器批准与接入密钥的持钥接入、认证与换钥、实例／lease、需求与续租、目标保存、安装回执、运行状态上报、SSE 变化通知、UDP 穿透协调与 Node 观察订阅。

daemon／CLI 通过本组件完成[控制生命周期](/v2/node/runtime#启动与接入)。自动调度和 Linux 网络安装由 daemon 协调；组件校验并传递宿主实际能力与安装结果，不自行证明 applied。

## 接口与顺序

| 方法 | 行为 |
| --- | --- |
| `Control::open` | 同步打开私有目录并独占锁；读取并验证已有身份／缓存，绑定 Server origin |
| `begin_enrollment` | 在发送请求前持久保存独立的 Ed25519／X25519 密钥及完整签名 intent；重试复用同一请求 |
| `poll_enrollment` | 用持钥证明查询；批准后的兑换码在兑换前私有持久化 |
| `redeem_enrollment` | 验证返回网络与公钥绑定，保存身份；丢失响应后可恢复同一兑换结果 |
| `cancel_enrollment` | 持钥取消待完成请求，或清理已确认的取消／撤回／过期请求；已兑换身份不会被当作临时请求删除 |
| `enroll_with_key`／`resume_key_enrollment` | 在同一私有记录中先保存接入密钥、独立 Node 密钥和 intent，再调用 `POST /agent-api/v1/enrollments`；重启复用原请求，成功后清除接入秘密 |
| `authenticate` | 先签名申请有界 challenge，再校验固定 Server／Node／key／generation 并消费证明，取得短期 Node 会话 |
| `refresh_authentication` | 在原会话截止前持钥取得新会话，保留同一有效实例；不会延长旧快照或 execution lease |
| `rotate_keys`／`rotation_pending` | 显式发起或恢复待确认换钥；新旧私钥、请求和 challenge 的持久化与结果恢复见下文 |
| `acquire_instance` | 使用宿主真实能力取得实例；同一次不确定请求保持原 instance ID |
| `renew_instance` | 更新当前实例的期限，验证返回实例与代际 |
| `sync_configuration` | 读取完整目标或用共享库应用 delta，检查 digest／身份／实例绑定，原子保存目标 |
| `refresh_configuration` | 显式续租并从同一 Server 事务取得精确匹配的 lease 与 full／delta；复用相同校验和持久化入口 |
| `watch_node`／`receive_event` | 创建单条、无自动重连的类型化 Node 事件流；把同认证上下文的通知映射为既有刷新或失效动作 |
| `execution_snapshot` | 从当前有效实例、lease 和匹配的已保存目标取得不可变执行快照；不证明 installed／applied |
| `dataplane_identity` | 验证同执行快照与已登记密钥后，取得受期限约束的 Noise 初始化及包平面签名句柄 |
| `create_demand` | 使用当前 Node 会话提交匹配当前 instance 的类型化需求；调用方保持重试 request ID，随后同步执行目标 |
| `renew_demand` | 以现有 demand ID 和绝对 expiresAt 请求续租；反馈不更新本机许可，须同步保存并取得新执行快照 |
| `cancel_demand` | 在 HTTP 发送前使旧快照及连续性失效并清空本机 execution deadline；不确定响应也不复活旧许可 |
| `query_dns` | 按当前 Node 身份[精确查询名称](/v2/configuration-reference/dns#node-精确名称查询)，无需 instance，不延长任何 lease |
| `acknowledge_saved` | 对已持久保存且仍属当前执行目标的配置发送 saved 回执；重试保持 attempt／sequence／时间 |
| `begin_application`／`finish_application` | 使用同一 ApplicationTicket 先确认 saved，再提交真实 applied／failed 结果 |
| `report_control_state` | 上报宿主提供的系统信息、当前能力和已保存配置；applied、通道、包流和测量保持空值 |
| `report_runtime_state` | 校验宿主真实 NodeState 的能力／saved／applied 绑定，复用同一 instance sequence 和重试 owner |
| `retry_state_report` | 重发同一不确定报告，保持 sequence 和观测时间，不制造新样本 |
| `query_udp_traversal`／`query_connectivity_peers`／`prepare_udp_traversal`／`accept_udp_traversal`／`cancel_udp_traversal` | UDP 穿透协调；契约见[Node 协调契约](/v2/node/connectivity#node-协调契约) |
| `create_node_observation`／`get_node_observation`／`renew_node_observation`／`cancel_node_observation`／`get_observation_tasks`／`report_observation_result` | Node 观察订阅、任务领取与结果上报；见[Node 订阅 API](/v2/node/diagnostics#node-订阅-api) |
| `stop` | 先停用本机控制上下文，再请求释放已知实例；请求失败不会恢复本机实例 |
| `shutdown` | 停止并等待已接纳的私有写入完成 |

接入时，宿主展示 Server 返回的 `EnrollmentRequestView.authorizationUrl` 作为浏览器批准入口；null 语义见 [Console origin](/v2/server#运行参数)，宿主如实提示而不自行拼接链接。Node 组件不持有浏览器用户会话，也不自行批准设备。

首次连接的调用顺序为 `open` → `begin_enrollment` → `poll_enrollment` → `redeem_enrollment` → `authenticate` → `acquire_instance` → `sync_configuration` → `acknowledge_saved`。之后由宿主按期限驱动续租、同步、报告与重试。

所有异步操作由同一个可变 driver 协调。组件没有后台重连循环；宿主负责重试退避、取消挂起操作及按有效期限停止运行。当前 HTTP 操作超时为 20 秒，不因一次 HTTP 尚未完成而延长实例或执行期限。

## 状态与恢复

`status()` 只返回身份、本地持久 intent ID、已保存目标引用和有效控制上下文。`enrollment_request_id` 不表示 Server 已收到请求。`cached_target()` 是私有配置缓存，不是执行许可。

- Ed25519 控制密钥和 X25519 数据面密钥分别生成。签名 transcript／intent digest 与 Server 共用协议实现。
- Server origin 与已登记的 `controllerId` 固定保存；不同 origin 或不同安装身份的 challenge 不能接管既有密钥。
- challenge 分配响应不确定时保留同一签名申请；申请已本地到期时使用新申请，session 响应不确定仍保留原 session intent。已本地到期证明遭拒允许重新证明，新鲜证明的凭据拒绝保持终止语义。
- 短期会话、instance、lease 和上报计数不写入恢复文件；进程重启后重新认证并申请新 instance。
- `stop` 停用会话的本机使用资格。仍有效的短期凭据可留在当前进程，显式 `authenticate` 后复用，以免反复启停耗尽 Server 的会话额度；它不能恢复已释放的实例。
- 期限同时受墙上时间和单调时钟约束，延迟响应与时钟回拨不会延长已取得的期限。
- 已接纳写入即使失去等待者也由阻塞 worker 完成，并持续持有目录锁。结果不确定时返回 `ReloadRequired`，重新打开后才能发起依赖该状态的操作。
- 损坏、缺字段、未知格式、错误绑定和异常文件权限都返回错误，不自动清理目录或生成替代身份。
- 锁文件中的初始化标记先于身份写入持久化；已经初始化后丢失 `control.json` 返回错误，不能作为新安装自动生成身份。

宿主可使用 `snapshot()` 在 HTTP 请求进行期间读取状态。快照复用控制组件的期限与失效标记；停止、身份上下文失效或销毁 driver 后，旧快照不再宣称具有有效实例。读取快照不产生报告或延长 lease。

## 事件通知与刷新

`watch_node()` 仅在已有有效会话和实例时创建 `NodeEvents`。它复用同一生成客户端和宿主保护的 HTTP socket pool，逐帧使用 `NodeEvent` 解码；没有独立解析器、后台重连或凭据 URL。daemon 持有唯一消费任务、容量为 1 的通知队列和 1–30 秒有界重连退避，停止时取消并 join。认证更新、实例失效或 down 后，已排队的旧上下文通知不再生效。

| 事件 | 本机动作 |
| --- | --- |
| `sync.reset`、`sync.ready`、`entities.delta`、`configuration.changed`、`resources.changed` | 唤醒既有控制周期，重新查询／原子刷新、校验和保存目标 |
| `observation.status`、`observation.removed` | 唤醒同一周期，复用观察任务 owner 的领取／更新路径 |
| 三种初始 snapshot、`state.delta`、配置进度与观察 sample | 不安装其投影视图，不把报告或测量当作执行许可 |
| `stream.closed: session_expired` | 清除旧会话与实例上下文，按现有到期恢复路径重新持钥认证 |
| `stream.closed: membership_revoked` | 停止旧上下文并阻断；不后台重试身份 |
| `stream.closed: node_lease_expired` | 按既有 lease 失效路径重新取得资格 |
| `stream.closed: server_shutdown`、EOF、网络／解码失败 | 只重连事件流；不撤销或延长独立取得的本机 lease |

此消费者仅使用通知唤醒普通 HTTP 同步，不维护 SSE 配置缓存，也不保存或发送 `Last-Event-ID`；每次重连取得完整批次，`sync.ready` 后再走正常同步。现有 5 秒定时刷新继续兜底，实例续租与报告期限独立。流当前沿用同一 HTTP 客户端的 20 秒总请求预算，到达预算后由宿主重连；普通请求的超时与保护不变。事件接收、保活和重连都不产生授权。

## 换钥与恢复

`rotate_keys()` 使用[换钥契约](/v2/api/authentication#换钥、重新登录与撤销)，同时更换控制和数据面公钥：

1. 生成独立新私钥，将旧身份、新私钥、固定 rotate intent 和 challenge 申请原子私有保存。
2. 使用旧钥申请并校验 challenge 的 Server、网络、Node、key、generation、摘要和期限；在发送 rotate 前持久保存完整 challenge。旧与新控制钥签同一 transcript。
3. 发送前停止旧本机执行上下文；Server 接受后校验新公钥与下一代际，把新私钥、公钥绑定、清空的旧目标缓存与移除 pending 一次原子写入。
4. 响应丢失或进程重启时重投同一请求，以新私钥恢复 Server 的同一换钥结果；不再次生成新钥。daemon 在重新认证前恢复已有 pending。

不确定失败必须保留两套私钥；不能假定 Server 未提交而删除新钥。原 challenge 已过期且 rotate 明确被拒时，只有新鲜旧钥认证仍成功才能清除 pending、回到旧身份；若旧钥证明失败，继续保留恢复材料。持久写入失败或等待者取消时，driver 进入 `ReloadRequired`，重开后读取原子写入前的 pending 或写入后的新身份，不使用内存中未经确认的绑定。旧执行不会因恢复或回滚而复活。

当前契约不发布持久 Node key 的到期时间或要求轮换的通知；短期 session 到期不是换钥触发条件。组件提供显式调用和 pending 恢复，daemon／CLI 尚无发起换钥或接入密钥的操作入口。

## 执行快照

`ExecutionSnapshot` 的类型定义于 `meshway-authority`；生产快照只由 `Control::execution_snapshot()` 签发，约束见[架构](/v2/architecture#node-内部共享的基础接口)。它绑定网络／节点、instance／generation、精确目标引用和本机失效代际，并保留原始 session、instance 与 execution 三个截止。缓存恢复、读取或 Clone 都不能延长期限。下列方法属于 authority 类型：

配置／需求改变后，Server 可能先撤回旧 execution lease。`sync_configuration` 只取得并保存新目标，不自动续租。持续运行使用 `refresh_configuration`，通过[原子刷新](/v2/execution#原子执行刷新)同时取得完整目标／delta 和该目标的 lease，避免分开的续租／查询夹入其他发布。已保存目标与当前可运行资格仍分别检查。

`authenticate` 在会话的 UTC 或单调剩余期限进入 60 秒窗口时主动更新认证。新会话必须在旧会话截止前成功验证，才能保留同一实例与连续性；过期、绑定不匹配或凭据拒绝仍停止旧上下文。认证更新不修改业务许可，也不延长已经取得的旧输出 deadline。

- `configuration()`／`ensure_current()`：检查当前有效性；目标内容包含私有配置，只交给受信运行组件。
- `now_ms()`／`expires_at()`：复用控制 owner 的 UTC 高水位和单调期限。
- `invalidated()`／`invalidated_at(permit_deadline_ms)`：供宿主取消阻塞 I/O；没有额外后台任务，permit 截止只能进一步缩短快照寿命。
- `same_execution()`：判断显式续租交接是否仍是相同 instance、目标和失效代际；调用组件仍须检查新旧快照有效性。
- `continuation_from(old)`：仅接纳同一 Control owner 发出的同目标续租或直接前驱 publication；校验 instance／generation、网络／节点、identity、地址与 selfKeys 不变，且旧 UTC／单调截止仍有效。返回旧配置只用于比较，不恢复其输出权限。

输出排队时保留快照，在最终 I/O 边界再次检查，并在阻塞期间选择失效通知。替换 publication 会取消旧输出 guard，但允许直接后继按不透明 lineage 比较仍未过期的旧授权。stop、取消需求、失效 barrier、跨 instance 或跳过已观察的 publication 都中断连续性；调用者不能只比较 digest 重新构造交接。旧排队输出永不取得新期限。数据面如何保留流／circuit 见[连续性](/v2/node/dataplane#包与流边界)。

## 安装回执与运行报告

`begin_application()` 为当前已保存、instance 目标一致的配置确认 saved 后返回 `ApplicationTicket`。Ticket 绑定 Control owner、instance、target 与 attempt；Clone 不产生新尝试。同一未完成尝试重用原 saved 请求、sequence 和时间；已完成后的重装使用下一 attempt。

`finish_application(ticket, result, observed_at)` 接收宿主真实 `Ok(())` 或 `ApplyFailure`，固定首次结果、sequence 和观测时间；不确定交付重试不能改结果。仍属同一 instance 的旧目标完成可进入 Server 历史，由 Server 排序避免覆盖当前目标；当前启用目标的 applied 还须具有有效执行快照。

`report_runtime_state` 与 `report_control_state` 共用报告 sequence／重试 owner。待确认报告存在时，新事实等待，重发原观测不制造样本。保存、安装、认证通道和业务包流分别表达；报告不会授予或延长执行资格。

## 私有存储与传输

每个控制身份使用独立的绝对私有目录。`control.json` 保存 origin、必要密钥、待完成接入及配置缓存；它包含秘密。目录为 owner-only，文件为 `0600`；读取有大小上限，写入使用临时文件、原子替换与同步。

文件保护原语由 `meshway-private-storage` 提供，Server 与 Node 共用目录、祖先路径、文件类型／权限、锁和原子发布实现；文件名、记录格式和业务恢复仍归各自领域所有。

当前私有记录格式为 2。仅对可识别且完整性／身份绑定有效的格式 1 执行升级：先持久备份原记录，再保留 origin、密钥、接入 intent 和身份绑定，清空派生 saved 缓存并原子写入格式 2。旧缓存不补默认字段变成新许可；重启重新同步。未知版本、当前格式损坏或备份失败均报错并保留原数据，不生成替代身份。

远端连接要求 HTTPS；明文 HTTP 仅允许数字 loopback 地址。HTTP builder 由宿主提供并保留其底层 socket／解析保护；生成客户端关闭重定向。原始密钥、兑换码、token、HTTP 响应体和底层错误不会进入组件的普通 Debug／Display 输出。

## 限制

IPC v3 的既有 `Status` 是拒绝未知字段的封闭对象，无法仅靠新增可省略字段让旧 CLI 接收事件流状态。因此当前 IPC／`meshway status` 未增加 `eventStream` 或最近事件时间。接入密钥和换钥的发起入口限制见上文。

组件测试使用 `mise exec -- cargo test -p meshway-control -p meshway-private-storage --all-targets --locked`，以真实 Server 与独立私有数据库运行；它不替代 TUN 或业务包流验收，见[验收范围](/v2/node/validation)。
