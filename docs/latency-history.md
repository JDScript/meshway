# 邻接延迟历史（第一版）

## 测量语义

此切片记录 **source → peer 的已认证直邻居 nonce round-trip**。测量来源是现有
`Runtime::probe_neighbor_observed`，不建立新连接，不发起全网拨号，不探测逻辑路径。
它不是 E2E、逐跳 trace、controller HTTP RTT 或 heartbeat RTT；不同边的 RTT 不能相加后
宣称端到端测量。Controller 接收的是有凭据来源约束的 Agent 自报告，不是独立远端证明。

- `success`：成功匹配 nonce 的真实 RTT；`rtt_us` 使用单调时钟，向上取整到微秒。
  `channel_associations` 恰好一个，来自 Runtime 同一次 proof，而非后续 registry 读取。
  耗时口径为一次 authenticated-neighbor probe 调用的最终 observation；组合已交付稳定性修复
  `58ce66c/fa5ebcd` 后，`round_trip` 从首次请求开始，**包含排队、丢失及内部重试等待**，
  不是去掉重试等待后的纯网络 RTT。Runtime 的 absolute deadline 内内部最多 3 次同 request ID
  请求、使用 fresh hop nonces，但 runner 仍每 API call 只记录一个最终样本，不拆成 3 个事件。
  cancellation/shutdown/replacement 不续重试，timeout 仍退休该 exact channel。
- `timeout`：真实已发出的邻接 probe 到期，`rtt_us = null`、通道关联为空。
- `failed`：已建立且配置允许的邻接上的 probe 尝试失败；`failure_reasons` 恰好一个，
  区分 `channel_unavailable`、`authentication_failed` 与 `local_send_failed`。
  没有 RTT/通道 proof，也不是网络丢包率。
- 未建立/离线、不获准、probe capacity busy、队列 drop、上报丢弃或代际取消：**没有测量记录**。
  这些是缺口，不能补 0、补 timeout，不能由缺 report 推导丢包。

Runtime 的失败接口不返回原子关联的通道元数据。首版明确保留未知，而不是用探测前的
carrier 填充超时记录。`sampler_generation` 表示本地采样运行代际，不是物理 channel ID；
listener/candidate/transport/initiator 表达所测通道的配置关联，不能断言同标签下的 socket
一直未重建。更精细的物理 channel 分段需要后续公开非秘密的 Runtime 通道生命周期标识。

## 身份、时间与存储

上报体没有 source/network 字段；Controller 由 node bearer 推导，两者均不可由 Agent 覆盖。
事件 identity 是 `(network_id, source_node_id, measurement_id)`，包含 peer、方向、epoch、
revision/digest、sampler generation、真实开始/完成时刻与结果。

批量原子提交；同 ID 同内容幂等，首次 `received_at` 不变；同 ID 改内容拒绝并回滚整个 batch。
只有当前、Applied、未过期 runtime、自身及 peer 未撤回、当前获准 adjacency 可接收新事件。
旧 epoch、旧 revision/digest、被撤回身份或失权连接不能续写。收到新 heartbeat、runtime report
或浏览器查询不会刷新、重测或重解释已有事件。

接受新事件的时钟边界：开始时间距接收最多 120 秒、未来最多 5 秒；完成不早于开始、
不晚于接收加 5 秒、尝试时长不超过 60 秒；开始不能早于 epoch 取得时间减 5 秒。
RTT 与 wall-clock 尝试时长做合理性核对。时钟异常事件拒绝，不使用接收时间代替测量时间。
相同 epoch 的 begin 重试不会刷新 epoch-start。

7 天是最大时间保留窗，不保证任何邻接规模都有完整 7 天。每 source/network 最多 50,000
条记录；接纳新事件后，事务内按 `measured_ms,row_id` 淘汰该 source 最旧记录，使近期数据
持续更新。正常批量最多淘汰 64 条；超出有界恢复范围的已损坏占用报错。验证、去重、新事件、
必要淘汰、覆盖水位与 ledger 在同一事务，任何失败全部回滚。TTL 清理也仅该 source/network，
每次最多 1,024 条；查询始终过滤保留窗，未清理的过期行不可见。

512/minute 是**新事件 insert 预算**，不是请求频率限制；重复请求仍需资格与当前配置校验。
独立的有界 admission ledger 保存 canonical body SHA-256、首次接收时刻和到期时间，直到
原测量的 120 秒重试窗与其首次接收的 60 秒速率窗均结束（加 1 ms 边界余量）。滚动淘汰
不会删除该约束，同 ID 重试不重建记录、不刷新 receipt、不占新样本预算；改变内容拒绝。
retained row 的 identity 仍一直约束内容；row 已被淘汰且 ledger 超出上述窗口后，旧 ID
不再有永久 tombstone，重新标时的新事件可按新 admission 验证。TTL 外不承诺永久 ID 记忆。

## API 与消费

权威契约：`crates/protocol/src/latency_history.rs`。OpenAPI、Rust Agent client、TS、Zod
由既有生成命令生成；Web 使用生成的 `queryLatencyHistory` operation 与运行时 schema。

- Agent：`POST /api/v1/agent/latency/samples`，node bearer，1–64 个事件，128 KiB body limit。
- 管理读取：`POST /api/v1/networks/{network_id}/latency/history/query`。
  `managementBearer` 接受 bootstrap 或当前网络成员的用户会话；统一入口
  `Store::authorized_latency_history` 在同一 `network_operation(Read)` 事务内检查权限和查询。
- 查询必选一个有方向的节点对、半开 `[from, until)` 时间窗（SQL 毫秒精度），最长 24 小时，
  窗口必须处于 7 天保留期内；`limit` 为 1–1000；`after`/`next` 为排他的十进制 row cursor。
  Cursor 是接收顺序，不是测量时间顺序；乱序送达仍能取全，图表按真实 `measured_at` 排序。
- 返回 `records[]` 包括 `network_id/source_node_id/sample/received_at/historical`，以及
  `next/retention_seconds/expected_interval_seconds`。`historical` 始终 true；即使当下已撤回、
  过期或切换承载，也只是历史证据，不能渲染成现在连通。没有原始事件时数组为空。
- 覆盖元数据：`retention_from` 是当前 TTL 下界；`capacity_trimmed_before` 是持久的
  source-wide 保守容量裁剪水位，覆盖所有 peer，不是“最早返回点”；请求 `from` 早于水位时
  `capacity_trimmed=true`。Web 应显示“此前部分历史因容量被淘汰”，与“没有样本”区分。
  它不证明任意区间连续覆盖，也不推导网络离线/丢包。分页不是固定快照，每页返回当前水位；
  跨页裁剪会反映在元数据中。AUTOINCREMENT cursor 永不因删除重用。

**曲线分段键**：网络、source、peer、direction、runtime epoch、revision/digest、sampler generation、
transport、initiator、listener、candidate。任何键变化须断开；不能跨方向/承载/代际连线。
Failed/timeout 单独标记有方向的节点对，关联为空时不要硬归入一条 carrier 曲线。
标注“已认证邻接 RTT”，不可标“访问延迟/E2E”。30 秒是采样调度期望值，超 64 个现有邻居时
轮转分批，不保证每条边固定 30 秒。忙/离线/上报失败均可形成缺口。

首版查询是 **有预算的原始事件分页，不聚合、不补点**。未来前端如自行降采样，须在同一
分段内只对 success 做 min/max/median 等运算，并保留成功样本数量、timeout/failed 数量与缺测
区别。平均值不可包含空值/零占位；不要把分页截断或无数据当成零流量、零延迟或网络 loss。

## 有界 runner 与中心接点

新增 `apps/agent/src/managed_connections/latency_history.rs`：固定内部默认 30 秒间隔、
4 个并发 probe、2 秒 timeout、每轮最多 64 个既有获准邻居（轮转）、256 个事件非阻塞队列。
HTTP 使用已接受 runtime report 的同一生成 `HttpClient`（含未来 underlay 注入），每批最多
64 条，10 秒请求 timeout，**总共最多 3 次 attempt（初次 + 2 次重试）**，1/2/4 秒 backoff；
没有同步日志/pipe I/O 阻塞健康上报。无 CLI 本地配置。
无第二个 Agent/runtime。代际/watch 变化、withdrawal、lease 到期取消采样、队列与 in-flight HTTP。

正常 `meshway-agent run --apply-connections` 已接线：**新鲜 Applied runtime report 被
Controller 接受后**调用 `Manager::latency_report_accepted`。Owner 根据 epoch、publication、
revision/digest 只持有一个任务；同代际 finished/error 恢复以 30/60/120/240/300 秒有界 backoff
并再次等待 fresh Applied 接受，不在每 heartbeat 生成任务，也不因遥测错误关闭健康 Runtime
或 E2E cache。withdraw/shutdown、可信更高 revision、失效/变更 snapshot 与 ACK conflict
走相同 abort + join 边界；Drop 也会 abort，不 detach。

显式 stop 返回表示 probe/report futures 与队列已 drop，不再入队或发起旧事件请求。取消无法
召回已由 Controller 提交的历史；in-flight HTTP 可能已被远端接收，因此仍有独立服务端 fence。
Summary 的部分 counters 不涵盖所有被取消的 queued/in-flight 条目，不可当成丢包统计。
采样不拨通新邻接；但它并非完全被动，现有 Runtime probe timeout 会 unregister **该 exact
channel**。本任务保留既有认证/超时语义，没有调整数据面或性能对照。

DDL位于`apps/controller/src/store/schema/schema.sql`，历史、账户/ownership与当前policy一同初始化。普通启动严格验证完整当前结构，不采用未知片段或补壳；runtime epoch acquisition时间仍是实际采样边界。
正常 controller 启动若已有 latency 表，执行无副作用完整 `validate_schema`（columns/PK/
AUTOINCREMENT/UNIQUE/FK/index 形状、完整 trigger 集合/归属/精确定义及 FK integrity），缺失
约束的同名表拒绝启动。当前四张 history 表没有合法 trigger，额外 main/temp trigger 均拒绝；
不影响 base/account 表上的合法 trigger。验证不删除或修复表、索引或 trigger。
关键 epoch/admission/sample、victim 删除及 coverage 写入核对 affected-row count；忽略写入
不会虚报 inserted 或提交半份 ledger/淘汰水位。合法同 epoch 重试和已有 victim identity 是
明确核对的零变更分支，不在每个请求扫描整库 schema。

### 可视化边界

唯一ID为`sample.measurement_id`；横轴`sample.measured_at`，迟到信息`received_at`；方向由
`source_node_id → sample.peer_node_id` 与 `sample.direction` 表达；carrier/epoch/revision 对应
`channel_associations/runtime_epoch/revision/digest/sampler_generation`。
此 endpoint 的方法由契约固定为 authenticated-neighbor nonce，生产 producer 为定期 runner；
首版不提供“检测一次”命令或完整路径，不应提前显示操作按钮。缺口原因未持久化，按未知
未采样/未上报表达，不能伪报历史离线原因。承载变化可由相邻成功 proof 的分段键变化标注，
单个 failed/timeout 不能推导真实 disconnect/reconnect 生命周期事件。

新sampler在原2秒预算内有界等待共享pending槽位，RTT含排队/重试等待；持续占用仍缺测，不延长周期/检查窗口。远端恢复复验只验持续runtime报告，**未查询持久history API**。客户端遵守[固定窗口分页](/spa-api-audit#历史分页)，不能沿用cursor跨扩大窗口。

## 验证入口

使用独立 target，避免并行 feature 构建替换 Agent CLI：

```sh
CARGO_TARGET_DIR=target/latency-history cargo test -p meshway-agent -p meshway-controller --all-features latency_history
CARGO_TARGET_DIR=target/latency-history cargo test --workspace --all-features
CARGO_TARGET_DIR=target/latency-history cargo clippy --workspace --all-features --all-targets -- -D warnings
pnpm protocol:generate
pnpm protocol:check
pnpm typecheck
```

`real_agent_sampler_generated_http_persistence_restart_and_withdrawal` 建立两个真实 loopback
authenticated Runtime/Agent session，调用 runner、通过生成 API 上报、Controller 持久化、
停止并重启后查询；在 runner 前即使 diagnostic probe/runtime report 已发生，查询仍为空。
UDP relay 测试先完成真实认证，再只在测试 socket 丢弃后续报文，验证真实 timeout 与无邻接
缺口的区别。其余 store 行为测试覆盖不可变事件、批量回滚、时钟、旧 epoch、网络隔离、
失权/撤回/过期、分页与 bounded 清理。测试仅用临时数据目录和独立 loopback socket。
