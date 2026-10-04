# Server 接入组件

`meshway-control` 提供由宿主驱动的 Node 控制生命周期，HTTP 操作全部使用生成的 `meshway-control-client`。当前包含浏览器批准的持钥接入、认证、实例／lease、目标保存、saved 回执和控制状态上报。

daemon／CLI 通过本组件完成[控制生命周期](/v2/node/runtime#启动与接入)。自动调度由 daemon 唯一协调；SSE 消费、接入密钥方式、换钥和实际网络安装尚未接线。组件不声明 transport 能力或 applied 结果。

## 接口与顺序

| 方法 | 行为 |
| --- | --- |
| `Control::open` | 同步打开私有目录并独占锁；读取并验证已有身份／缓存，绑定 Server origin |
| `begin_enrollment` | 在发送请求前持久保存独立的 Ed25519／X25519 密钥及完整签名 intent；重试复用同一请求 |
| `poll_enrollment` | 用持钥证明查询；批准后的兑换码在兑换前私有持久化 |
| `redeem_enrollment` | 验证返回网络与公钥绑定，保存身份；丢失响应后可恢复同一兑换结果 |
| `cancel_enrollment` | 持钥取消待完成请求，或清理已确认的取消／撤回／过期请求；已兑换身份不会被当作临时请求删除 |
| `authenticate` | 校验固定 Server／Node／key／generation 的 challenge 后签名，取得短期 Node 会话 |
| `acquire_instance` | 使用宿主真实能力取得实例；同一次不确定请求保持原 instance ID |
| `renew_instance` | 更新当前实例的期限，验证返回实例与代际 |
| `sync_configuration` | 读取完整目标或用共享库应用 delta，检查 digest／身份／实例绑定，原子保存目标 |
| `execution_snapshot` | 从当前有效实例、lease 和匹配的已保存目标取得不可变执行快照；不证明 installed／applied |
| `create_demand` | 使用当前 Node 会话提交匹配当前 instance 的类型化需求；调用方保持重试 request ID，随后同步执行目标 |
| `cancel_demand` | 取消当前实例的需求；成功确认后立即使本机旧执行快照失效，重新同步后取得新快照 |
| `query_dns` | 按当前 Node 身份[精确查询名称](/v2/configuration-reference/dns#node-精确名称查询)，无需 instance，不延长任何 lease |
| `acknowledge_saved` | 对已持久保存且仍属当前执行目标的配置发送 saved 回执；重试保持 attempt／sequence／时间 |
| `report_control_state` | 上报宿主提供的系统信息、当前能力和已保存配置；applied、通道、包流和测量保持空值 |
| `retry_state_report` | 重发同一不确定报告，保持 sequence 和观测时间，不制造新样本 |
| `stop` | 先停用本机控制上下文，再请求释放已知实例；请求失败不会恢复本机实例 |
| `shutdown` | 停止并等待已接纳的私有写入完成 |

接入时，宿主使用 `EnrollmentRequestView.id` 展示浏览器批准入口。Node 组件不持有浏览器用户会话，也不自行批准设备。

首次连接的调用顺序为 `open` → `begin_enrollment` → `poll_enrollment` → `redeem_enrollment` → `authenticate` → `acquire_instance` → `sync_configuration` → `acknowledge_saved`。之后由宿主按期限驱动续租、同步、报告与重试。

所有异步操作由同一个可变 driver 协调。组件没有后台重连循环；宿主负责重试退避、取消挂起操作及按有效期限停止运行。当前 HTTP 操作超时为 20 秒，不因一次 HTTP 尚未完成而延长实例或执行期限。

## 状态与恢复

`status()` 只返回身份、本地持久 intent ID、已保存目标引用和有效控制上下文。`enrollment_request_id` 不表示 Server 已收到请求。`cached_target()` 是私有配置缓存，不是执行许可。

- Ed25519 控制密钥和 X25519 数据面密钥分别生成。签名 transcript／intent digest 与 Server 共用协议实现。
- Server origin 与已登记的 `controllerId` 固定保存；不同 origin 或不同安装身份的 challenge 不能接管既有密钥。
- 短期会话、instance、lease 和上报计数不写入恢复文件；进程重启后重新认证并申请新 instance。
- `stop` 停用会话的本机使用资格。仍有效的短期凭据可留在当前进程，显式 `authenticate` 后复用，以免反复启停耗尽 Server 的会话额度；它不能恢复已释放的实例。
- 期限同时受墙上时间和单调时钟约束，延迟响应与时钟回拨不会延长已取得的期限。
- 已接纳写入即使失去等待者也由阻塞 worker 完成，并持续持有目录锁。结果不确定时返回 `ReloadRequired`，重新打开后才能发起依赖该状态的操作。
- 损坏、缺字段、未知格式、错误绑定和异常文件权限都返回错误，不自动清理目录或生成替代身份。
- 锁文件中的初始化标记先于身份写入持久化；已经初始化后丢失 `control.json` 返回错误，不能作为新安装自动生成身份。

宿主可使用 `snapshot()` 在 HTTP 请求进行期间读取状态。快照复用控制组件的期限与失效标记；停止、身份上下文失效或销毁 driver 后，旧快照不再宣称具有有效实例。读取快照不产生报告或延长 lease。

## 执行快照

`ExecutionSnapshot` 只能由当前 `Control::execution_snapshot()` 取得。它绑定网络／节点、instance／generation、精确目标引用和本机失效代际，并保留原始 session、instance 与 execution 三个截止。缓存恢复、读取或 Clone 都不能延长期限。

配置／需求改变后，Server 可能先撤回旧 execution lease。`sync_configuration` 只取得并保存新目标，不自动续租；宿主显式 `renew_instance` 获得匹配新目标的执行期限后，才能取得可用执行快照。已保存目标与当前可运行资格分别检查。

- `configuration()`／`ensure_current()`：检查当前有效性；目标内容包含私有配置，只交给受信运行组件。
- `now_ms()`／`expires_at()`：复用控制 owner 的 UTC 高水位和单调期限。
- `invalidated()`／`invalidated_at(permit_deadline_ms)`：供宿主取消阻塞 I/O；没有额外后台任务，permit 截止只能进一步缩短快照寿命。
- `same_execution()`：判断显式续租交接是否仍是相同 instance、目标和失效代际；调用组件仍须检查新旧快照有效性。

输出排队时保留快照，在最终 I/O 边界再次检查，并在阻塞期间选择失效通知。换代、down、driver 销毁和不确定持久化会使旧快照失效；挂起的 HTTP 不阻止通知。已经失效的快照不会因重新上线或新 lease 恢复。各组件的安装、包流和 applied 仍由各自实际行为验收。

## 私有存储与传输

每个控制身份使用独立的绝对私有目录。`control.json` 保存 origin、必要密钥、待完成接入及配置缓存；它包含秘密。目录为 owner-only，文件为 `0600`；读取有大小上限，写入使用临时文件、原子替换与同步。

文件保护原语由 `meshway-private-storage` 提供，Server 与 Node 共用目录、祖先路径、文件类型／权限、锁和原子发布实现；文件名、记录格式和业务恢复仍归各自领域所有。

远端连接要求 HTTPS；明文 HTTP 仅允许数字 loopback 地址。HTTP builder 由宿主提供并保留其底层 socket／解析保护；生成客户端关闭重定向。原始密钥、兑换码、token、HTTP 响应体和底层错误不会进入组件的普通 Debug／Display 输出。

## 验证

```sh
mise exec -- cargo test -p meshway-control -p meshway-private-storage --all-targets --locked
```

集成测试启动真实 Server 和独立私有数据库，覆盖批准／兑换、取消、重启恢复、保存／回执、实例释放／重取及永久删除。交付故障在真实事务完成后的 HTTP 响应边界注入，检查兑换与报告重试；它不替代 TUN 或业务包流验收。
