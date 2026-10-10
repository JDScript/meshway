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
| `refresh_authentication` | 在原会话截止前持钥取得新会话，保留同一有效实例；不改变已发布的 netmap 或其租约 |
| `rotate_keys`／`rotate_keys_with_request`／`rotation_pending` | 显式发起或恢复待确认换钥；宿主请求 ID 与最后一次成功结果一并私有保存，重放不再次轮换 |
| `acquire_instance` | 使用宿主真实能力取得实例；同一次不确定请求保持原 instance ID |
| `renew_instance` | 更新当前实例的期限，验证返回实例与代际 |
| `sync_configuration` | 读取完整目标或用共享库应用 delta，检查 digest／身份／实例绑定，原子保存目标 |
| `refresh_configuration` | 显式续租并从同一 Server 事务取得精确匹配的 lease 与 full／delta；复用相同校验和持久化入口 |
| `watch_node`／`receive_event` | 创建单条、无自动重连的类型化 Node 事件流；把同认证上下文的通知映射为既有刷新或失效动作 |
| `dataplane_keys` | 本身份已登记的包平面密钥（`DataPlaneKeys`）；不是授权，组件只对公钥与内容 selfKeys 一致的期望内容使用 |
| `begin_sync`／`SyncCall::send`／`finish_sync`／`sync` | 一次 netmap 长轮询：持有 driver 准备、释放 driver 发送、再持有 driver 校验、保存并发布；见 [Netmap 发布](#netmap-发布) |
| `sync_loop` | 以 `tokio::sync::Mutex<Control>` 驱动长轮询直至停止，按结果发布 `Option<meshway_engine::Netmap>` |
| `netmap`／`netmap_reference`／`netmap_lease` | 已发布的已验证内容与签名租约（租约在 Control 时钟上已结束则为空）、持有的内容引用（下一请求的基准）与当前租约 |
| `set_sync_transport` | 安装 sync 的 HTTP 交换实现 |
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

所有异步操作由同一个可变 driver 协调。组件没有后台重连循环；宿主负责重试退避、取消挂起操作及按有效期限停止运行。普通 HTTP 操作总超时为 20 秒，SSE 使用下述建立／初始同步和读空闲预算；等待 HTTP 不延长实例或执行期限。

## 状态与恢复

`status()` 只返回身份、本地持久 intent ID、已保存目标引用和有效控制上下文。`enrollment_request_id` 不表示 Server 已收到请求。`cached_target()` 是私有配置缓存，不是执行许可。

- Ed25519 控制密钥和 X25519 数据面密钥分别生成。签名 transcript／intent digest 与 Server 共用协议实现。
- Server origin 与已登记的 `controllerId` 固定保存；不同 origin 或不同安装身份的 challenge 不能接管既有密钥。
- challenge 分配响应不确定时保留同一签名申请；申请已本地到期时使用新申请，session 响应不确定仍保留原 session intent。已本地到期证明遭拒允许重新证明，新鲜证明的凭据拒绝保持终止语义。
- 短期会话、instance、lease 和上报计数不写入恢复文件；进程重启后重新认证并申请新 instance。签名的 netmap 及其租约例外，见 [Netmap 发布](#netmap-发布)。
- `stop` 停用会话的本机使用资格。仍有效的短期凭据可留在当前进程，显式 `authenticate` 后复用，以免反复启停耗尽 Server 的会话额度；它不能恢复已释放的实例。
- 期限同时受墙上时间和单调时钟约束，延迟响应与时钟回拨不会延长已取得的期限。
- 已接纳写入即使失去等待者也由阻塞 worker 完成，并持续持有目录锁。结果不确定时返回 `ReloadRequired`，重新打开后才能发起依赖该状态的操作。
- 损坏、缺字段、未知格式、错误绑定和异常文件权限都返回错误，不自动清理目录或生成替代身份。
- 锁文件中的初始化标记先于身份写入持久化；已经初始化后丢失 `control.json` 返回错误，不能作为新安装自动生成身份。

宿主可使用 `snapshot()` 在 HTTP 请求进行期间读取状态。会话／实例上下文与执行输出使用独立失效标记：publication 交接仅使旧执行截止不可用，保留仍有效的 authenticated／instance 事实。停止、身份上下文失效或销毁 driver 后，旧快照不再宣称具有有效实例。读取快照不产生报告或延长 lease；实际字段投影见[运行状态](/v2/node/runtime#安装范围与状态)。

## 事件通知与刷新

`watch_node()` 仅在已有有效会话和实例时创建 `NodeEvents`。它复用同一生成客户端和宿主保护的 HTTP socket pool，逐帧使用 `NodeEvent` 解码；没有独立解析器、后台重连或凭据 URL。daemon 持有唯一消费任务、容量为 1 的通知队列和 1–30 秒有界重连退避，停止时取消并 join。重复刷新提示合并为一个待处理提示，终止通知不丢弃；不把旧事件积压转换为逐条刷新。认证更新、实例失效或 down 后，已排队的旧上下文通知不再生效。

| 事件 | 本机动作 |
| --- | --- |
| `config.snapshot`、`entities.snapshot`／`entities.delta` | 本节点目标、可见密钥绑定或成员集合改变时唤醒既有控制周期；peer 的 publication／观察投影变化不冒充本节点目标变化 |
| `state.snapshot`／`state.delta` | 本节点实例改变或删除时重新同步；普通报告与样本不触发完整刷新 |
| `configuration.changed`、`resources.changed` | 新配置 revision 或资源变化通知触发既有刷新 |
| `sync.reset` | 断档、背压、epoch 改变触发刷新；`view_changed` 按随后完整投影的本节点目标判断，避免 peer／观察范围变化造成刷新反馈 |
| `sync.ready`、其他初始 snapshot、配置进度与观察事件 | 不安装其投影视图，不把报告或测量当作执行许可；观察领取／更新仍由既有观察调度驱动 |
| `stream.closed: session_expired` | 清除旧会话与实例上下文，按现有到期恢复路径重新持钥认证 |
| `stream.closed: membership_revoked` | 撤回旧上下文，复用 HTTP 运行凭据拒绝的退避再认证路径；事件本身不把 daemon 永久置为 blocked |
| `stream.closed: node_lease_expired` | 按既有 lease 失效路径重新取得资格 |
| `stream.closed: server_shutdown`、EOF、网络／解码失败 | 只重连事件流；不撤销或延长独立取得的本机 lease |

此消费者仅使用通知唤醒普通 HTTP 同步，不维护 SSE 配置缓存，也不保存或发送 `Last-Event-ID`；每次重连取得完整批次，相同目标、revision、可见密钥集合和本节点实例不重复刷新。初始批次的变化提示合并至 `sync.ready`；不完整批次不触发刷新。实际目标变化的即时唤醒、重复提示合并及定时兜底见[控制调度与恢复](/v2/node/runtime#控制调度与恢复)，权限关闭事件独立处理。

SSE 复用受保护客户端，但不设置流的总请求截止：建立连接／取得响应头最多 20 秒，随后取得完整初始批次最多 20 秒，持续读取的空闲超时为 30 秒（Server 每 10 秒保活）。保活字节重置读空闲等待，不延长初始批次预算。EOF／超时由独立的宿主维护定时器推进重连，无需完整控制周期；连接成功本身不是 `sync.reset`。事件接收、保活和重连都不产生授权。

## 换钥与恢复

`rotate_keys()` 使用[换钥契约](/v2/api/authentication#换钥、重新登录与撤销)，同时更换控制和数据面公钥：

1. 生成独立新私钥，将旧身份、新私钥、固定 rotate intent 和 challenge 申请原子私有保存。
2. 使用旧钥申请并校验 challenge 的 Server、网络、Node、key、generation、摘要和期限；在发送 rotate 前持久保存完整 challenge。旧与新控制钥签同一 transcript。
3. 发送前停止旧本机执行上下文；Server 接受后校验新公钥与下一代际，把新私钥、公钥绑定、清空的旧目标缓存与移除 pending 一次原子写入。
4. 响应丢失或进程重启时重投同一请求，以新私钥恢复 Server 的同一换钥结果；不再次生成新钥。daemon 在重新认证前恢复已有 pending。

不确定失败必须保留两套私钥；不能假定 Server 未提交而删除新钥。原 challenge 已过期且 rotate 明确被拒时，只有新鲜旧钥认证仍成功才能清除 pending、回到旧身份；若旧钥证明失败，继续保留恢复材料。持久写入失败或等待者取消时，driver 进入 `ReloadRequired`，重开后读取原子写入前的 pending 或写入后的新身份，不使用内存中未经确认的绑定。旧执行不会因恢复或回滚而复活。

当前契约不发布持久 Node key 的到期时间或要求轮换的通知；短期 session 到期不是换钥触发条件。操作员使用 `meshway rotate-key` 发起换钥，接入密钥使用 `meshway up --enrollment-key @file` 或 stdin，见[CLI 与 IPC](/v2/node/runtime#cli-与-ipc)。

daemon 先持久接纳带 CAS 的换钥操作，再由唯一 Control owner 执行；Server 结果与新绑定持久化、daemon 操作回执落盘后才同步回答 CLI。CLI 断开不撤回已接纳操作，进程重启保留原请求 ID 恢复。接入密钥只在待完成意图中私有保存，接入完成后清除原值；幂等回执保留摘要。

## Netmap 发布

Control 只验证、保存并发布签名 netmap；它不签发任何执行快照、guard 或期限句柄。租约到期由 [Engine](/v2/node/runtime#engine) 的唯一计时器负责，Control 不按消费者重复检查。

`finish_sync` 依次执行：请求被替换检查 → `SyncResponse::check(request)` → 以钉住的 Server 钥验证 `signature`（keyId／generation 与钉住钥不符为 `ServerKeyMismatch`，其余失败为 `SignatureInvalid`；均拒绝该回复、不改变状态、不撤回已发布租约，`sync_loop` 按网络错误退避）→ full 验证 digest、delta 以 `apply_netmap_delta` 重建并复核目标 digest、unchanged 匹配持有内容 → 网络／节点／keys 绑定与租约指向 → 拒绝 serial 回退与同内容更早 `issuedAt` 的租约 → 保存 → 发布。结果为：

| `NetmapChange` | 含义 |
| --- | --- |
| `New { previous }` | 首次安装或更大 serial；内容先保存再发布 |
| `Renewed` | 同内容的新租约；Engine 只延长自己的计时器，旧租约已结束时重新安装同一内容 |

在 `finish_sync` 之前已有其他安装、撤回或停止的回复返回 `Pending` 且不改变任何状态。`peerFacts` 在 netmap 部分安装后并入 Control 的最后已知事实（按当前投影过滤）并原样交给 `PeerFactsSink`，不进入内容；消费者拒收时，或已发布租约结束／撤回后（最后已知事实随 netmap 结束），下一请求不带游标、取得完整事实；Server 对无游标请求不挂起，因此租约结束后的恢复不等待 hold。地址或出口就绪变化（不含仅新鲜度续期）及 netmap 变化时写 `peer-facts.json`。重启载入后，`take_facts_replay()` 一次性给出保存的事实（base 为 null 的完整替换）；宿主在不联系 Server 而发布已持有 netmap 的同一位置把它交给 sink（daemon 为每次 up 会话开始处）；首个请求仍不带游标，Server 可达时以完整事实替换。单次请求总上限 90 秒（Server 挂起上限 60 秒），连接级失败 1 秒起倍增至 30 秒退避；立即返回且无变化的回复至少间隔 1 秒。

| 事件 | 已发布租约 |
| --- | --- |
| Server 不可达、超时、5xx、409、session 过期／401、instance 过期或重新取得 | 保持，直至 Engine 计时到期 |
| 类型化身份拒绝（`membership_revoked` 等）、`stop`、开始换钥、身份记录写入、私有写入结果不确定 | 立即撤回（发布 `None`），Engine 应用空期望 |

State、回执、协调与观察结果只为 Control 持有且租约未结束的精确内容发言（`held_netmap`）；instance 只是 State／回执绑定，不进入期望内容。

## 安装回执与运行报告

`begin_application()` 为当前已保存、instance 目标一致的配置确认 saved 后返回 `ApplicationTicket`。Ticket 绑定 Control owner、instance、target 与 attempt；Clone 不产生新尝试。同一未完成尝试重用原 saved 请求、sequence 和时间；已完成后的重装使用下一 attempt。

`finish_application(ticket, result, observed_at)` 接收宿主真实 `Ok(())` 或 `ApplyFailure`，固定首次结果、sequence 和观测时间；不确定交付重试不能改结果。仍属同一 instance 的旧目标完成可进入 Server 历史，由 Server 排序避免覆盖当前目标；当前启用目标的 applied 还须对应 Control 持有、租约未结束的精确内容。

已完成的当前 ticket 在下一次目标刷新前先结清。Server 明确返回[回执域冲突](/v2/api/problems#node-配置回执冲突)时，`ReceiptRejected` 保留类型化 Problem 与 Server request ID；被拒 ticket 终止，不能改绑、重写内容或再次提交。daemon 清除对应待确认完成，刷新后以新 attempt 重新确认 saved 和安装；网络／交付不确定仍重发原 ticket。回执被拒不等于实际组件已经停止，也不延长任何租约。

`report_runtime_state` 与 `report_control_state` 共用报告 sequence／重试 owner。待确认报告存在时，新事实等待，重发原观测不制造样本。保存、安装、认证通道和业务包流分别表达；报告不会授予或延长执行资格。

## 私有存储与传输

每个控制身份使用独立的绝对私有目录。目录为 owner-only，文件为 `0600`；读取有大小上限，写入使用临时文件、原子替换与同步。

| 文件 | 内容 |
| --- | --- |
| `control.json` | 身份记录：origin、密钥、待完成接入、身份绑定与换钥状态；包含秘密 |
| `netmap.json` | `{ format: "meshway-netmap-cache/2", netmap, lease, signature, seal }` |
| `peer-facts.json` | `{ format: "meshway-peer-facts-cache/1", networkId, nodeId, netmap: NetmapRef, cursor, peers: NodeId → PeerFacts }`；最后已知 peer 事实，无签名 |
| `execution.json` | 旧 publication 目标缓存 `{ format: "meshway-execution-cache/1", saved }`，随旧执行链删除 |

缓存文件缺失、格式未知、损坏或绑定不符时忽略，不阻断身份也不生成授权。

`peer-facts.json` 不是授权：它只提供地址候选，每条承载、通道和包的准入仍由已验签 netmap 与认证握手决定，伪造或过时的地址最多连不上。因此它不带 Server 签名或本节点 seal，只要求私有目录与 `0600`。仅当 `netmap.json` 作为执行授权载入时才读取；网络／节点须与身份一致，内容按载入 netmap 的投影过滤（不在投影内的 peer 丢弃，listener 细节只保留本节点可拨的）。`netmap` 与载入内容不一致时不重放。写入失败或中断只留下旧或新的完整文件，不影响 sync。换钥清空内存中的事实；文件与 `netmap.json` 同生命周期。

`signature` 是 Server 对租约的签名（`NetmapSignature`，权威），`seal` 是本节点控制钥对同一租约的签名 `{ signer: "node", keyId, keyGeneration, signature }`，只记录本身份接受了该内容。两者都签 `meshway_protocol::netmap_lease_transcript`：依次为 `meshway/netmap-lease/v1\0`、网络与节点 UUID、revision、serial、内容 digest、`issuedAt`／`expiresAt` 毫秒、signer 类型字节（0 = node，1 = server）和签名钥 key ID／generation，定长大端。digest 覆盖内容，full 与 delta 在重建后核对 digest 等于签名中的引用即被覆盖。

Server 签名钥在接入时随 `EnrollmentResponse.serverSigningKey` 返回，与身份绑定一起保存在 `control.json` 中钉住；为 null 时节点不接受任何 netmap。重新打开时，格式 2 文件须同时满足格式、digest、网络／节点／keys 绑定、租约指向、钉住钥的 Server 签名与本节点 seal 才载入；租约未到期即恢复为执行授权（不联系 Server），已到期只作 delta 基准。格式 1 文件（只有本节点 seal）至多作为 delta 基准，从不恢复授权；首个已验签的 sync 回复把它重写为格式 2。重启依赖系统 UTC，进程间不保留时钟高水位。同内容续租在保存的截止被延后至少 1 小时时才重写文件。

文件保护原语由 `meshway-private-storage` 提供，Server 与 Node 共用目录、祖先路径、文件类型／权限、锁和原子发布实现；文件名、记录格式和业务恢复仍归各自领域所有。

当前身份记录格式为 5，不再包含任何派生缓存，编译器变化不改变身份版本。格式 1–4 的 `saved` 只做完整性与身份绑定检查（4 按当前 demand-7 目标验证）：先持久备份原记录 `control-v<n>-<sha256>.json`，再保留 origin、密钥、接入 intent、身份绑定和换钥状态，原子写入格式 5。旧缓存不补默认字段变成新许可；重启重新同步。未知版本、当前格式损坏或备份失败均报错并保留原数据，不生成替代身份。

远端连接要求 HTTPS；明文 HTTP 仅允许数字 loopback 地址。HTTP builder 由宿主提供并保留其底层 socket／解析保护；生成客户端关闭重定向。原始密钥、兑换码、token、HTTP 响应体和底层错误不会进入组件的普通 Debug／Display 输出。

## 限制

IPC v5 与 CLI／daemon 同版本发布，Ping 通道新增 `localEndpoint` 获取来源；旧版本请求返回 `ErrorCode::Version`。事件连接状态与最近有效事件接收时间只是观测，不代表执行许可，见[公开运行状态](/v2/node/runtime#安装范围与状态)。

Server 已提供预发布的 [netmap 同步](/v2/execution#netmap-同步)端点；本组件尚未调用它，现行刷新仍按上文接口进行。

组件测试使用 `mise exec -- cargo test -p meshway-control -p meshway-private-storage --all-targets --locked`，以真实 Server 与独立私有数据库运行；它不替代 TUN 或业务包流验收，见[验收范围](/v2/node/validation)。
