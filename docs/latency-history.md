# 邻接延迟历史（第一版）

## 测量语义

此切片记录 **source → peer 的已认证直邻居 nonce round-trip**。测量来源是现有
`Runtime::probe_neighbor_observed`，不建立新连接，不发起全网拨号，不探测逻辑路径。
它不是 E2E、逐跳 trace、controller HTTP RTT 或 heartbeat RTT；不同边的 RTT 不能相加后
宣称端到端测量。Controller 接收的是有凭据来源约束的 Agent 自报告，不是独立远端证明。

- `success`：成功匹配 nonce 的真实 RTT；`rtt_us` 使用单调时钟，向上取整到微秒。
  `channel_associations` 恰好一个，来自 Runtime 同一次 proof，而非后续 registry 读取。
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

保留 7 天；每次合法 report/query 最多清理 1,024 条过期记录；查询始终过滤保留窗，
所以尚未清理的过期行也不可见。每 source 最多 50,000 行，每分钟最多 512 个新事件。
触及预算时明确拒绝批量，Agent 的 drop 表示遥测缺口。有限清理避免单次长事务；无业务
请求时不主动清理，后台定期维护可以复用同一个 bounded 清理接点。

## API 与前端 handoff

权威契约：`crates/protocol/src/latency_history.rs`。OpenAPI、Rust Agent client、TS、Zod
由既有生成命令生成；Web 使用生成的 `queryLatencyHistory` operation 与运行时 schema。

- Agent：`POST /api/v1/agent/latency/samples`，node bearer，1–64 个事件，128 KiB body limit。
- 管理读取：`POST /api/v1/networks/{network_id}/latency/history/query`。
  首版 admin bearer；P2b 唯一 read-network 权限接点为
  `Store::authorize_latency_history_read(token, network_id)`。
- 查询必选一个有方向的节点对、半开 `[from, until)` 时间窗（SQL 毫秒精度），最长 24 小时，
  窗口必须处于 7 天保留期内；`limit` 为 1–1000；`after`/`next` 为排他的十进制 row cursor。
  Cursor 是接收顺序，不是测量时间顺序；乱序送达仍能取全，图表按真实 `measured_at` 排序。
- 返回 `records[]` 包括 `network_id/source_node_id/sample/received_at/historical`，以及
  `next/retention_seconds/expected_interval_seconds`。`historical` 始终 true；即使当下已撤回、
  过期或切换承载，也只是历史证据，不能渲染成现在连通。没有原始事件时数组为空。

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
HTTP 使用调用者已有生成 `HttpClient`（含未来 underlay 注入），每批最多 64 条，10 秒请求
timeout，最多 3 次重试，1/2/4 秒 backoff；HTTP 与数据面采样不互相阻塞。无 CLI 本地配置。
无第二个 Agent/runtime。代际/watch 变化、withdrawal、lease 到期取消采样、队列与 in-flight HTTP。

`Manager::start_latency_history(client, epoch)` 是启动接点：**在新鲜 Applied runtime report
被 Controller 接受之后**由现有 Agent 主 loop 每代际启动一次，并持有 `JoinHandle<Summary>`。
旧任务完成或 epoch/snapshot 变化时重新启动；不要每个 heartbeat 启动一个任务。底层 Prober
publication fence 负责自主取消。此提交的真实 HTTP fixture 已调用启动接点；主 loop 对
`state/connections` 的接线留给拥有该文件的协调者/P2a，生产入口尚未启用自动采样。

DDL 在 `apps/controller/src/store/latency_history/schema.sql`，`upgrade` 是无版本的启动事务片段，
不会从 API 在运行服务建表。协调者应把它并入统一最终 schema version。现有 runtime begin
仅增加调用 `begin_epoch` 的最小接点，lease 成功与记录 epoch-start 在同一事务。

### 并行交付整合（只读核对 `56da5ad`）

- P2a 的 managed materials 已占用 schema v10；此切片不申请 v10，DDL 留给统一后续迁移。
- P2a 的 `probe.rs` publication / weak Runtime / deadline / Phase fence，以及
  `session::describe` / Runtime 原子 probe proof 接口未变。`history_source` 小 hook 可沿用。
  新的 NodeConnections materials 属于完整快照与 digest；不可拆出来重新拼接旧 intent。
- `reconcile_dataplane` 从 coherent payload 提取同一份 connections，随后保存 Manager snapshot；
  采样 context 使用该 snapshot 的 revision/digest，publication 撤回后整体取消。
- Agent 新增 `transport-runtime`，managed 不再隐式带 lab CLI；采样模块仍只挂在
  managed-connections，不新增 lab CLI 或本地配置入口，整合时由协调者重生成全部消费者。
- **启动 hook 的调用方必须在 R1 heartbeat 告知更高 revision 时立即 abort 已持有的 sampler
  handle**，与旧 dataplane/DNS 撤回一同执行；然后等待新 coherent bundle 与 Applied report。
  不能等旧 HTTP 请求结束或等下一采样周期。即使邻接 Runtime 暂时仍在，旧 revision/digest
  的 queued event 也不应再发。Controller 的 current revision/digest 校验是第二道 fence。

与 `feat/sidebar-07:apps/web/docs/console-design.md` §6.6 的交接映射：唯一 ID 为
`sample.measurement_id`；横轴 `sample.measured_at`，迟到信息 `received_at`；方向由
`source_node_id → sample.peer_node_id` 与 `sample.direction` 表达；carrier/epoch/revision 对应
`channel_associations/runtime_epoch/revision/digest/sampler_generation`。
此 endpoint 的方法由契约固定为 authenticated-neighbor nonce，生产 producer 为定期 runner；
首版不提供“检测一次”命令或完整路径，不应提前显示操作按钮。缺口原因未持久化，按未知
未采样/未上报表达，不能伪报历史离线原因。承载变化可由相邻成功 proof 的分段键变化标注，
单个 failed/timeout 不能推导真实 disconnect/reconnect 生命周期事件。

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
