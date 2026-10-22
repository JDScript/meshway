# 恢复状态机收敛设计

> **设计稿（RC-D），尚未实现。** 本页盘点 Node 侧所有可能 fail closed 的 owner 当前的失败分类、重证、恢复与 Status 投影，给出把它们收敛为一个共享框架的方案与迁移顺序，对应 [v0.1.0 交付门槛](/v2/process#v0-1-gate)里程碑 A 的“恢复状态机收敛为一个共享框架”。现行行为的唯一定义仍是[宿主内核归属的结束与重证](/v2/node/runtime#ownership-end)、[内核变更失败](/v2/node/runtime#kernel-change)、[DNS 健康检查与恢复](/v2/node/dataplane#dns-health)、[出口审计与清理重试](/v2/node/exit-design#exit-audit)与 [listener 绑定与重试](/v2/node/connectivity#listener-bind-retry)；验收状态只以[验收范围](/v2/node/validation)为准。本页不加入文档导航。盘点基于 main `3077913a`。

## 不变量 {#invariant}

本页的框架只服务两条不变量（定义见仓库 `AGENTS.md`），其他一切是它们的推论：

1. **分类**：每条 fail-closed 路径区分两类并写明 owner。读到**已证实**的不一致是持久阻断（Status `blocked`＋原因）；读取失败、超时或变更结果未知是暂态阻断，由命名 owner 按有界退避重证，原因消失后以同一 netmap 自动恢复（Status `retrying`＋最早原因）。不存在无 owner、无重证周期的阻断；一次失败的读取或变更不证明归属已变。
2. **边沿**：事件循环每轮至少推进一个状态，不得有电平分支。`watch` 克隆在等待前标记已见、处理时 `borrow_and_update`，已关闭的分支禁用，同一事实只处理一次直到它变化；定时恢复不被立即就绪的分支饿死。

以及两条边界，框架不得越过：

- **授权只有一份**：Engine 的 `Desired` 仍是唯一授权状态。恢复状态只描述“某 owner 的资源是否可证明为本 owner 所有”，不携带、不延长任何权限或租约。
- **领域判定归 owner**：什么算已证实的不一致（外来 nft 元素、链路设置被改写、socket 权限不符）只由该 owner 判定；框架只接收判定结果。

## 现状盘点 {#inventory}

### owner 一览 {#owners}

共 16 个 owner。“判定点”指持久／暂态在哪里分开；“恢复路径”指谁触发、以什么为准重装。Manager 的 traversal、STUN、候选节奏与数据面路径暂停只调度连接尝试，不撤回授权、不 fail closed，不在本表（它们已共用 `meshway_model::Backoff`）。

| # | owner（实现位置） | 判定点：持久／暂态 | 重证／重试 | 恢复路径 | Status 投影 | 日志 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 宿主内核归属（`underlay/linux/mod.rs` `Host::recover`、`protection.rs` `Ending::end`） | `OwnershipEnd::{Unavailable, Conflict}`，由结束者给出；`Conflict` 覆盖 `Unavailable`，反之不会；重证失败按 `Error` 变体判（`reproof_failed`：Netlink／Io／Drain 为暂态） | `Reproof`：结束后 1 s，翻倍至 8 s 间隔，无总期限；`Conflict` 不重证 | worker 在 `recovery_at` 调 `recover`→`Kernel::open_term`（同进程启动路径）→ worker 停止 DNS owner、以同一 offer 重装 | `lastError: underlay`＋`kernelOwnership: …` 详情（worker 写入报告快照）；phase 由 worker 的 `persistent` 快照决定 | `Kernel ownership re-proof unavailable; retrying`（同一失败一次）、`… proved again; the Host owns a new term` |
| 2 | 内核安装与自有对象重装（`kernel/mod.rs` `end_after`／`ended`、`repair.rs`） | `ended(operation, error)`：Drain→`drain_unavailable`，Netlink／Io→`install_unavailable`／`repair_unavailable`，其余→`install`／`repair` 冲突；`install_cancelled`→冲突 | 重装读失败：1 s 间隔、至多 5 次（按次数）后结束为 `repair_unavailable`；1 min 内第 4 次重装→`repair_limit` 冲突 | 结束后归 #1 | 经 #1 | `Owned capture objects unreadable; the repair is retried`、`… keep being removed; ownership ends` |
| 3 | monitor（`kernel/monitor/mod.rs`） | `ended(error)`：Io／Netlink（含截断数据报）→`monitor_unavailable`；其余（含 `Worker`）→`monitor` 冲突；读到外来对象→冲突 | 不重试，结束后归 #1 | 经 #1 | 经 #1 | `Underlay kernel protection conflict`（`reason`、`table`、`interface`、`prefix`） |
| 4 | netlink 请求连接（`kernel/term.rs`） | 本期 Kernel 存续时连接结束→`connection_unavailable` | 不重试，归 #1 | 经 #1 | 经 #1 | `Kernel netlink connection ended; kernel ownership ends` |
| 5 | egress capture owner（`kernel/egress/owner.rs`、`verification.rs`） | 回读 `Verification::{Holds, Unavailable, Changed}`；变更失败后 `Reread::{Done, Start, Unavailable, Changed}`；`Changed`→`verify` 冲突 | 审计 `Cadence` 1 s；未知 `Unverified` 30 s 时长→`verify_unavailable`；变更 `retried` 1→60 s、30 s 时长→`change_unavailable`；MSS clamp `ClampRetry` 1→60 s 无期限 | 期满结束 #1（暂态），由 #1 恢复 | `changePending`（Host 现值 watch）；clamp 重试无投影 | `Egress capture verification unavailable; …`、`… change failed; read back, retrying`（第 1、2、4… 次）、`… owner stopping …` |
| 6 | 出口角色 owner（`kernel/exit/owner/{mod,install}.rs`） | 同 #5 的 `Verification`／`Reread`；`Changed` 与 30 s 未知都只停止角色，不结束 #1 | 审计 1 s、未知 30 s；变更 `retried` 30 s；清理 1→60 s，**无总期限**，至 Host 停止 | 角色停止→#7 | `changePending`、`exit.cleanupPending`（现值 watch） | `Exit role audit unavailable; …`、`Exit owner stopping …`、`Exit role cleanup failed; resources kept, retrying`、`… completed after retrying` |
| 7 | 出口驱动与能力探测（`node/host/linux/exit/driver.rs`、worker `ExitFailed`／`exit_recovered`） | 驱动 `Health { failed, ended }` 不分类 | 能力探测按 State TTL／3 周期；`ready` 且角色未装→`exit_recovered` | worker `Engine::reapply` 以同一内容重装角色 | `exit.capability`；角色失败不改 phase | `Exit role failed; withdrawing it alone`、`Exit prerequisites hold; installing the exit role` |
| 8 | DNS 锚点 guard（`kernel/anchor.rs`） | 读取中断只关读取 socket（D3）；锚点结束只因 owner 删除或 #1 结束；结束一律报 `SystemDnsError::Conflict`／`anchor_ended` | 不重试，经 #9 的 recover 观察 | #1 新一期后由 #9 重新准备 | 经 #9 | 经 #9 |
| 9 | DNS owner actor（`node/runtime/network/dns/actor`、`meshway-dns` resolved／resolv.conf 后端） | 后端给出 `SystemDnsFailure { error, check, … }`；actor **不区分**持久／暂态 | observe 每 250 ms、期限 2 s；故障后 recover 固定 1 s，无退避、无期限；hold／install 失败只写详情，不置故障 | recover 成功→`recovered` 边沿→worker `DnsRecovered`→以同一 offer 重装（hold 清除故障） | `lastError: dns`＋检查详情；`Report::current()` 读 fault 现值；phase 由 `persistent()` 中 `Problem::Dns` 恒为暂态 | `DNS owner health check failed`（同一失败一次）、`DNS owner recovered its system installation` |
| 10 | 控制中继 listener（`underlay/linux/proxy/mod.rs`） | 不结束 #1；`EACCES` 等“需人工”的原因只改间隔 | U1 口径 1→60 s，`EACCES` 固定 60 s，无期限 | 自身下一次尝试 | `controlRelay`（现值 watch，`retrying`／`atCapacity`） | `Protected control relay cannot listen; retrying, kernel ownership kept` 等（同一错误一次） |
| 11 | Manager listener 重绑 U1（`connectivity/manager/rebind.rs`） | `recoverable`：`EACCES` 为否，仍重试 | 1→60 s，非 recoverable 固定 60 s，无期限；path 变化立即重试并清零 | 绑定成功→worker `ListenersChanged`→`reapply` | `lastError: transport`＋详情、`sockets[].unavailable` | listener 绑定失败／恢复各一条 |
| 12 | Mesh 池重叠（`kernel/network.rs` overlap、worker `PoolOverlap`） | 只有暂态（地址离开即恢复） | 无重试，等待 overlap watch 变化 | 地址离开→`overlap_cleared`→按 offer 重装 | `lastError: underlay`＋`meshPoolOverlap: …`，`persistent=false` | `Mesh pool overlap; …`、`… ended; installing the netmap again` |
| 13 | 网络 worker（`node/runtime/network/worker/{mod,lifecycle,timing,withdrawal}.rs`） | `ApplyFailure::{Validation, Dataplane}` 不重试；其余暂态；`persistent(engine, problem, retrying, overlap)` 在 7 处调用点合成 | `Retry`：5 s、15 s，**第 3 次失败后停止**；`lifecycle::next` 按优先级汇合 #1 重证、#9 两条边沿、`host.ended()` | 撤回：`Engine::reconfig(None)`；重装：同一 offer 的 `reconfig`／`reapply` | `Report { plane, problem, problem_detail, persistent, … }` 快照 | 撤回／重装各一条（`Withdrawn::reinstalled`，trigger 字段） |
| 14 | Engine（`meshway-engine`） | 不分类：组件失败返回 `Error`，可部分安装为 `Degraded` | 无，由 #13 驱动 | `reconfig`／`reapply` 幂等 | 经 #13 | — |
| 15 | Control（`node/runtime/control/{mod,failure,cycle}.rs`、`meshway-control` `sync.rs`／`artifacts.rs`） | `meshway_control::Recovery::{Retry, Withdrawn, Storage, Halt}`；Host 打开失败只有 `Problem::Underlay` 重试 | sync 1→15 s 无期限；控制周期 1→15 s；artifact 5→300 s；Host 打开 1→15 s | 下一次 sync／周期 | `phase`（`retrying`／`blocked`）、`lastError`、`retryAt`、`sync`、`syncError` | `Node control operation failed`（operation、kind、status、decision） |
| 16 | Status 跟踪（`node/runtime/status.rs` `View::derive`／`track`） | 不判定：`blocked` = Control `Blocked`，或包平面 `failed` 且 `!Report::recovering()` | 输入变化时重新求值（X18 后只被未见变化唤醒） | — | `phase`、`dataPlane`、`lastError`／`lastErrorDetail`／`networkError`、`changePending`、`controlRelay`、`exit.cleanupPending` | — |

### 与不变量的偏差 {#deviations}

共 13 处。类型：**A** 一次未知被判为持久；**B** 暂态到期被判为持久或无 owner；**C** 已证实被当作暂态；**D** Status 投影绕过 owner 现值。

| # | 类型 | 位置 | 现状 | 后果 |
| --- | --- | --- | --- | --- |
| G1 | B | worker `timing.rs` `Retry`、`withdrawal.rs` `persistent` | 暂态安装失败只重试 2 次（5 s、15 s），之后 `retry.at = None`，归属未结束且问题非 `dns` 时 `persistent = true` | TUN 读失败、承载 `CarrierClosed`、未结束归属的安装失败约 20 s 后为 `blocked`，原因未证实，也无 owner 再试；恢复只靠下一次 offer |
| G2 | B | DNS actor `Command::Hold`／`Install` | hold／install 失败只写 `detail`，不置 `fault`，因此没有 `DnsRecovered` 边沿；`persistent()` 对 `Problem::Dns` 恒为暂态 | worker 两次重试用完后 Status 为 `retrying`，但没有任何 owner 在重试 |
| G3 | C、D | DNS actor 与 `persistent()` | actor 不区分持久／暂态：`link_settings`、`record_pending`、`foreign_domain`、namespace 等已证实不一致同样每秒重证，Status 恒为 `retrying` | 与 [DNS 健康检查](/v2/node/dataplane#dns-health)“不可自动恢复（保持阻断）”一栏相反 |
| G4 | A | `kernel/anchor.rs` `Guard::check` | 锚点随 #1 的暂态结束而结束时也报 `SystemDnsError::Conflict`／`anchor_ended` | DNS 层看到的是“冲突”，真正的判定在 #1；G3 修正后会把暂态结束误判为持久 |
| G5 | A | egress、exit 两处 `Installing::drop` | 安装 future 被取消→`OwnershipEnd::Conflict { check: "install_cancelled" }` | 一次取消（进程停顿、上层超时）使宿主保持阻断至重启；取消不证明外来写入者 |
| G6 | A | `kernel/mod.rs::ended`、`monitor::ended`、`Host::reproof_failed`、`verification::uncertain`、egress `stopped` | 按 `Error` 变体分类写了 5 份，集合各不相同（Drain 只在两处；`uncertain` 把 `Conflict` 当未知；monitor 把 `Worker` 当冲突）；`Error::Conflict` 同时表示外来对象、出口清理待重试（`ExitRole::retire`）、中继路径被占、HostOwner 已持有 | 内部错误与“正在重试”可被读成已证实冲突；分类不可审计 |
| G7 | C | 出口 #6／#7 | 审计 `Changed`（已证实）与 `verify_unavailable` 走同一路径：停止角色→清理→能力探测 `ready` 后自动 `reapply` | 已证实的外来改动被下一次安装越过；出口没有 `blocked` 的表达 |
| G8 | D | #1→#13→#16 | 旧一期 monitor 在首次重证关闭它之前读到外来对象，把 `Unavailable` 改为 `Conflict`：`recovery_at()` 变 `None`，`HostFailed` 分支因 `installed = None` 禁用，worker 无事件；`report.persistent`、`problem_detail` 停在暂态快照 | Status 显示 `retrying`／`proving ownership again`，实际已持久阻断 |
| G9 | A、C | Control `Handle::open`、`network::startup_problem` | 启动时读到外来对象（`Conflict`）按 `Problem::Underlay` 每 1–15 s 重试（`retrying`；[控制调度与恢复](/v2/node/runtime#控制调度与恢复)写为 1–30 s），运行中同一事实为 `blocked` 至重启；启动 resolver 打不开→`configuration`→`blocked`，运行中同一事实为 `relay_unavailable` 暂态；resolv.conf 预约读失败→`dns`→`blocked` | 同一事实在两处分类相反 |
| G10 | C | #10、#11 | `EACCES`、权限不符的外来 socket 由文档定为需人工，却每 60 s 重试并显示 `retrying` | 已证实的阻断被显示为暂态 |
| G11 | A | Control `Recovery::Storage` | 本机记录写入结果不确定→`blocked`＋撤回 | 结果未知按不变量应重读判定；现为文档明定的例外 |
| G12 | D | #16 `Report::recovering` | phase 由 worker 快照 `persistent` 与 `changePending`／`cleanupPending` 两个现值“或”出 | 同一事实有快照与现值两份；G8 即其后果 |
| G13 | B | #5 `ClampRetry` | MSS clamp 替换失败无期限重试，无 Status 投影 | 降级（不 fail closed），但重试不可见 |

## 重复与耦合 {#duplication}

### 每个 owner 各写一份的逻辑

| 逻辑 | 份数 | 现有实现 |
| --- | --- | --- |
| 持久／暂态分类 | 7 | `ended`、`monitor::ended`、`reproof_failed`、`uncertain`、egress `stopped`、DNS（无）、Control `Recovery` |
| 退避间隔 | 已统一 | `meshway_model::Backoff`（F6b）；计数与重置各 owner 自管 |
| 期限 | 6 种口径 | 30 s 时长（egress／exit 审计与变更）、5 次（repair 读）、3 次（worker）、无期限（#1、#9、#10、#11、出口清理）、Control 无期限 |
| pending 发布 | 4 | `KernelChangePending`、`exit cleanups`、`ControlRelayState::Retrying`、`ListenerUnavailable`；字段相同（reason、attempts、retryAt），类型 3 个 |
| “同一失败只记一次” | 5 | `Reproof.failure`、proxy `report`、DNS `details` 比较、`Retry` 第 2ⁿ 次、`Unverified` 首次 |
| 边沿化 | 4 | DNS `Health::failed` 的版本确认（S4）、worker 布尔标志 `exit_recovered`／`listeners_changed`／`overlap_cleared` 加 `std::future::ready`、Status 跟踪的 `mark_unchanged`（X18）、`host.ended()` 以 `installed` 门控 |
| 恢复后以同一 netmap 重装 | 6 入口 | `Work::{Offer, Retry, DnsRecovered, HostRecovered, ExitRecovered, ListenersChanged}` 共用一个分支，各自前置处理不同 |
| persistent 合成 | 7 调用点 | worker 各分支调用 `persistent(…)` 写入快照 |

### 跨 owner 的时序依赖

过去两周的同族缺陷都出在下列耦合点，而不是单个 owner 的判定：

| 耦合 | 缺陷 | 机制 |
| --- | --- | --- |
| DNS 锚点生命周期依附 #1 的一期 | D3、X13 | 锚点 guard 用一期的取消令牌；读取中断曾取消锚点；一期结束后 DNS recover 永远读到 `anchor_ended` |
| #1 的一期是全局令牌 | X13 | `Shared::conflict` 一次取消、从不更新，暂态结束被所有 owner 读成永久冲突 |
| DNS 恢复与 #1 重证的顺序 | X15 | DNS 已重证（`recovered`）而 #1 结束：重装在 DNS hold 之前就失败，DNS 两条电平交替就绪，`biased` select 饿死到期的重证 |
| worker 对 DNS 两个 watch 的确认 | S4 | 故障在 worker 看到前已重证，worker 不撤回，查询许可却等下一次 hold，包平面保留内容而 DNS 不服务 |
| 撤回清理与报告的先后 | H4 | `reconfig(None)` 等待 egress pause 重试与出口退役约 31 s 后才发布原因；DNS 先发布故障后发布详情 |
| Status 跟踪对 owner 的 live watch | X18 | 只借用不确认的 DNS guard 克隆让 `changed()` 恒就绪，跟踪任务占满一个线程 |
| 出口 owner 停止与 worker 的同步审计 | X15 | 停止的出口 owner 不回答排队的探测审计，worker 等它的清理重试 73 s |

共同根因：每个 owner 把自己的恢复状态以**不同形状**（取消令牌、两个 bool watch、`Option<RetryPending>`、`OwnershipEnd`＋`recovery_at`）暴露，worker 与 Status 各自把它们**重新拼成**“是否失败、是否在恢复、是否持久”，拼法散在多个分支里，每加一个 owner 或一种顺序就多一处电平或快照。

## 方案 {#options}

### 方案 A：每个 owner 一个 `Recovery` 原语 {#option-a}

`meshway_model` 增加无 I/O、无 tokio 的状态机（时间由调用方传入，与 `Backoff` 同层）。owner 持有一个，把每次判定喂给它，再把它的 `Health` 发布到自己唯一的 `watch`：

```rust
/// One owner's judgement of one read-back or change.
pub enum Observation {
    Holds,
    /// The read failed, timed out, or a change's outcome is unknown.
    Unknown { check: &'static str, reason: String },
    /// Read back and different from what this owner owns.
    Proven { check: &'static str, reason: String },
}

pub enum Horizon {
    /// Unknown for this long escalates to the parent owner as Unknown
    /// (egress → kernel ownership); never to Blocked.
    Escalate(Duration),
    /// Retried until it holds (owners without a parent).
    Unbounded,
}

pub struct Policy { pub backoff: Backoff, pub horizon: Horizon, pub reread_blocked: Option<Duration> }

/// What an owner publishes; carries no authority.
pub enum Health {
    Healthy,
    Retrying(Retrying),
    Blocked(Blocked),
}
pub struct Retrying { pub check: &'static str, pub reason: String, pub since: SystemTime, pub attempts: u32, pub retry_at: SystemTime }
pub struct Blocked  { pub check: &'static str, pub reason: String, pub since: SystemTime }

pub enum Step {
    /// Nothing to do until the next scheduled read.
    Hold,
    /// Read and judge again at this instant.
    RetryAt(Instant),
    /// The horizon passed: hand `check` to the parent owner as Unknown.
    Escalate(&'static str),
    /// Proven: stay failed closed; re-read only at `reread_blocked`.
    Block,
}

impl Recovery {
    pub fn new(policy: Policy) -> Self;
    pub fn observe(&mut self, observation: Observation, now: Instant) -> Step;
    pub fn health(&self) -> Health;
    /// True once per transition Healthy→Retrying, Retrying→Blocked, *→Healthy:
    /// the owner logs exactly there.
    pub fn transitioned(&self) -> bool;
}
```

owner 只提供两个回调：**重读并判定**（返回 `Observation`）与**重装**（它自己的领域操作，或不提供，由 worker 以同一 netmap 重装）。原语内含：退避、期限、`since`／`attempts`、同一失败只记一次、`Health` 形状。`RetryPending`（reason、attempts、retryAt）成为 `Retrying` 的投影，现有 `changePending`、`cleanupPending`、`controlRelay` 字段不变。

- 优点：分类、期限、pending、日志去重各只有一份；owner 判定仍归 owner；可逐 owner 迁移，每步行为可等价。
- 不足：不解决 worker 的汇合与 Status 的拼合（X15、S4、X18、G8、G12 那一类）。

### 方案 B：worker 层统一汇合 {#option-b}

各 owner 只发 `Health` 事件，worker 用一个状态机决定撤回、重装与 phase；owner 内部的分类与重试保持现状。

- 优点：修掉跨 owner 时序类缺陷；Status 只读一个汇合值。
- 不足：7 份分类与 6 种期限仍在各 owner 内，汇合层要理解每种原始形状（令牌、bool、`Option`），等于把重复搬到一处；G1–G7、G9–G11 不受影响。

### 方案 C：A＋B（推荐） {#option-c}

每个 owner 用方案 A 的原语产出统一的 `Health`；worker 用一个**边沿汇合器**消费所有 `Health`；Status 直接读同一组 `Health` 现值。

**边沿汇合器**（`meshway-node` 的 `runtime/network/worker`，需要 tokio，不进 `meshway-model`）：

```rust
/// Every fail-closed owner's Health, consumed as edges.
struct Healths { owners: [(OwnerKey, watch::Receiver<Health>); N] }

impl Healths {
    /// The first owner (in priority order) whose Health changed since it was
    /// last taken, or a due retry_at of an owner the worker drives (kernel
    /// re-proof). Marks it seen. Closed receivers are disabled.
    async fn next(&mut self) -> (OwnerKey, Health);
    /// Current values of all owners, read without waiting.
    fn snapshot(&self) -> Snapshot;
}

/// Pure: what the plane does for the current snapshot. Idempotent: the same
/// snapshot twice gives the same action, which applied twice changes nothing.
fn decide(snapshot: &Snapshot, installed: bool) -> Action;
enum Action { Nothing, Withdraw { cause: Cause }, ReproveHost, Reinstall { trigger: OwnerKey } }
```

- worker 只在边沿上调用 `decide`，并以**当前快照**（不是边沿本身）决定动作：X15 的“DNS 已恢复而归属已结束”只得到 `ReproveHost`，S4 的“未处理过的故障已被重证”在快照里是 `Retrying→Healthy` 两次变化中的第一次边沿，仍撤回再重装。`decide` 是纯函数，跨 owner 的顺序都写成它的测试用例。
- 重装只有一个执行点（`Reinstall`），撤回只有一个执行点（`Withdraw`），日志在执行点（H4 规则不变）。
- worker 自己的 apply 重试也是一个 `Recovery`（`Unbounded`），修正 G1。
- **Status**：`phase` 与 `lastError`／`lastErrorDetail` 由 `Snapshot` 求出（见下），`Report.persistent` 与 `Report::recovering` 删除，修正 G8、G12。`Report` 仍是 worker 的安装事实（plane、netmap、completion），不再携带恢复判定。
- 授权：`Health` 不进入 Engine；`Desired` 仍只由 `Engine::reconfig` 算出，`decide` 只决定何时调用它。

**Status 投影规则**（替代 `persistent`／`recovering`；IPC 13 形状不变）：

| 条件（按序） | `phase` | `lastError`／`lastErrorDetail` |
| --- | --- | --- |
| Control `Blocked` | `blocked` | Control 的问题；包平面另见 `networkError` |
| 某 owner `Blocked` | `blocked` | 该 owner；多个时取 #1 宿主内核归属，否则 `since` 最早者 |
| 某 owner `Retrying` 且包平面已撤回 | `retrying` | 同上规则取最早原因 |
| 其余 | 现有规则（running／synchronizing／controlReady…） | — |

`changePending`、`exit.cleanupPending`、`controlRelay`、`sockets[].unavailable` 成为对应 owner `Retrying` 的投影，读的是同一 `watch` 现值。

### 方案对比

| | A | B | C |
| --- | --- | --- | --- |
| 分类／期限／pending 只有一份 | 是 | 否 | 是 |
| 跨 owner 时序（X15、S4、G8） | 否 | 是 | 是 |
| Status 只读 owner 现值（G12） | 否 | 是 | 是 |
| 可逐 owner 迁移、每步可等价验证 | 是 | 否（一次换掉 worker） | 是（先 A 后汇合） |
| wire／IPC 变化 | 无 | 无 | 无 |
| 改动量 | 约 1,800 行 | 约 900 行 | 约 2,700 行（见[迁移计划](#migration)） |

### 推荐

采用方案 C。先在 `meshway_model` 落地无 I/O 的 `Recovery`／`Health`，逐个 owner 迁移，每步只把现有分类与期限搬进同一原语并补齐偏差（分类、期限、pending、日志各一份）；然后用一个边沿汇合器和纯函数 `decide` 替换 worker 的 `lifecycle::next`、布尔标志、`Retry` 与 7 处 `persistent`，让 Status 直接由同一组 `Health` 现值求 `phase` 与最早原因。授权仍只在 Engine，领域判定仍在各 owner，IPC 13 与 wire 不变。

## 迁移计划 {#migration}

每步一个提交、单独过 `mise run check`；涉及内核的步骤另跑隔离 NET_ADMIN 测试（本地一次性容器）。“行为”一栏标明是否改变外部可见行为；改变的步骤以对应偏差的红测先行（旧实现失败、新实现通过）。

| 步 | 内容 | 行为 | 回归／验收测试 | 估计 |
| --- | --- | --- | --- | --- |
| M0 | `meshway_model::recovery`：`Observation`、`Policy`、`Recovery`、`Health`、`Step`；`RetryPending` 移入 model（underlay 重导出） | 不变 | 新单测：期限内未知保持、`Escalate` 只在期满、`Proven`→`Block`、去重日志只在转换 | +300 |
| M1 | `verification.rs`：`Unverified`、`Retry` 建在 `Recovery` 上（`Escalate(30 s)`）；egress、exit 审计与变更改用；`ClampRetry` 发布 `Retrying`（G13，日志） | 不变（G13 仅日志） | `unreadable_verifications_keep_ownership_until_the_limit_and_a_change_ends_it`、`a_failed_change_is_classified_by_its_reread_and_bounded`、`retried_changes_complete_from_the_target_or_the_start_and_publish_their_retry`、`requests_are_served_between_slow_but_successful_audits`；X11／X12 隔离测试 | ±250 |
| M2 | #1 宿主内核归属：`Reproof` 建在 `Recovery`（`Unbounded`，1→8 s）上，发布 `Host::health()`；保留 `ownership_ended`／`recovery_at` 作为其投影 | 不变 | X13 `an_unavailable_ownership_end_is_proved_again_and_a_foreign_object_keeps_it_ended`、`a_missed_drain_ends_ownership_as_unavailable` | ±200 |
| M3 | #10 控制中继、#11 U1：改用 `Recovery`（`Unbounded`，1→60 s）；`controlRelay`、`sockets[].unavailable` 为其投影 | 不变 | proxy 单测（7）、`control_attachment_listens_once_a_held_socket_is_released`、rebind 单测 | ±200 |
| M4 | 分类收敛：一份 `classify(operation, &Error) -> Observation` 替换 5 处（G6）；`install_cancelled`→`Unknown`（G5，待决定 Q5）；`Error::Worker` 不再是冲突；repair 读失败改为 30 s 时长（替代 5 次） | 改变 | 红测：取消安装后可重证；monitor 内部错误为暂态；`a_repair_whose_read_fails_stays_owed_and_completes`；X13 reproof | ±250 |
| M5 | #6／#7 出口：清理重试与审计发布同一 `Health`；`Proven` 时角色保持撤回、能力报 `ownershipConflict`，按 Q4 处理（G7） | 改变 | `exit_recovery::netmap_exit_failed_cleanup_is_retried_and_the_role_installs_again`、`exit_audit_read_failures_are_unknown_and_a_failed_cleanup_completes_on_retry`；新红测：审计 `Changed` 后不自动重装 | ±300 |
| M6 | #8／#9 DNS：后端 `SystemDnsFailure` 按检查项映射 `Unknown`／`Proven`（以 [DNS 健康检查](/v2/node/dataplane#dns-health)两栏为准）；recover 保持 1 s；hold／install 失败也进入 `Health`（G2）；锚点因 #1 结束时报 `Unknown`，由 #1 恢复（G4）；`Health::failed`／`recovered` 两个 watch 并为一个 | 改变 | DNS actor 单测、D3 锚点隔离测试（2）、`a_settings_conflict_names_the_differing_fields`；新红测：hold 失败后有恢复边沿；`link_settings` 外来为 `Blocked` | ±350 |
| M7 | worker 汇合：`Healths::next`＋`decide` 替换 `lifecycle::next`、`exit_recovered`／`listeners_changed`／`overlap_cleared`、`Retry`（改为 `Unbounded` 5→60 s，G1）与 `persistent` | 改变 | X15 lifecycle 三个用例移植为 `decide` 用例（`a_due_reproof_is_selected_before_dns_facts_that_stay_ready`、`a_fault_re_proved_before_the_plane_looked_is_one_failure_and_one_recovery`、`a_recovered_dns_fault_and_an_ended_term_reach_the_reproof_without_spinning`）；`runtime::network` 单测（23）；新红测：暂态安装失败不成为 `blocked`、G8 的升级产生边沿 | ±500 |
| M8 | Status：`derive` 按[投影规则](#option-c)从 `Healths::snapshot` 求 `phase`／最早原因；删除 `Report.persistent`／`recovering`；跟踪任务用同一汇合器等待（G12） | 改变（只有 G1／G3／G8 的 phase） | `a_transient_failure_retries_and_only_a_persistent_one_blocks`、X18 `the_tracker_is_not_woken_by_a_dns_guard_send_it_has_seen`、`runtime::status` 单测 | ±200 |
| M9 | Control 启动分类与运行一致（G9）；按 Q6 处理存储未知（G11） | 改变 | `runtime::control` 单测；新红测：启动读到外来对象与运行中同为 `blocked`、resolver 未打开为 `retrying` | ±150 |

顺序理由：M0–M3 是行为不变的搬移，先把原语在最简单的 owner 上用实；M4–M6 逐个修正 owner 内的偏差，各自有隔离测试；M7–M8 一次替换汇合与投影，这是风险最高的一步，放在所有 owner 已发布统一 `Health` 之后；M9 涉及 Control，与包平面无耦合，最后做。M4–M9 每步后跑一次本地 lab 限额复现（X13／X15 方法）。

**估计**：约 2,700 行改动（其中删除约 900 行），10 个提交；按每步一次 check 加隔离测试，M0–M6 各 0.5–1 天，M7–M8 共 2–3 天，M9 0.5 天，合计约 8–10 个工作日，另加整体验收。

## 验收 {#acceptance}

- **逐步**：上表各步的既有测试全部保持通过，偏差修正的红测先行。X13 reproof、X15 lifecycle、X18 tracker、F6a repair 与 X11／X12 的隔离测试是本框架的回归基线，迁移后它们断言的行为不变，只是驱动对象换为 `Recovery`／`decide`。
- **整体**：`fault/throttle` 行（F7）连续 3 轮全绿：全程无 `blocked`、撤回期间 `retrying`＋最早原因、≤ 10 秒以同一 netmap 恢复、撤回次数等于重装次数、空闲 CPU 最忙线程 < 5 %；随后 25 行默认矩阵全过。
- **不变**：IPC 13 Status 形状（`meshway status --json` 字段集合）、wire v6、HTTP 契约、`compilerVersion` 均不变；`check:docs` 与 drift 无生成物变化。

## 待用户决定 {#decisions}

| # | 问题 | 选项 | 推荐 |
| --- | --- | --- | --- |
| Q1 | 30 s 期限是否统一 | (a) 所有 owner 30 s；(b) 有上级 owner 的（egress、出口审计／变更、repair 读）用 30 s 后**上交上级为暂态**，无上级的（#1、DNS、中继、U1、Control、出口清理）不设总期限、只封顶间隔 | (b)：期限的意义是“交给更完整的重证”，不是“放弃”；DNS recover 保持 1 s 固定间隔以守住 resolved 重启 5 s 内恢复 |
| Q2 | 持久阻断是否允许自动重读 | (a) 不重读，直到重启；(b) 保持 `blocked`，以慢间隔（60 s）只读重证，证据消失（外来对象移走、设置恢复）即自动恢复，从不覆盖外来对象 | (b)：与启动路径（G9）、DNS 现行可恢复行为一致，满足“每次 `blocked` 都有已证实的原因”，并去掉“需重启”的人工步骤；读到仍不一致则继续 `blocked` |
| Q3 | Status phase 映射是否改 | (a) 维持 `persistent`；(b) 按[投影规则](#option-c)：`blocked` 当且仅当某 owner `Blocked`（已证实），worker 重试不再“用完” | (b)；确定性校验失败（`Validation`）记为 `Blocked`（证明是确定拒绝），新 offer 到来即解除 |
| Q4 | 出口角色读到已证实外来改动（G7） | (a) 维持自动重装；(b) 角色保持撤回、能力 `unavailable(ownershipConflict)`，按 Q2 只读重证，证据消失后重装 | (b)；出口不影响本节点 phase，只影响其他源的出口可用性 |
| Q5 | `install_cancelled` 的分类（G5） | (a) 维持冲突；(b) 暂态，由 #1 重证（重证按启动路径读 journal 与内核） | (b) |
| Q6 | 本机存储写入结果不确定（G11） | (a) 维持 `blocked`＋撤回（文档现行）；(b) 按 X12 方式重读记录：一致即完成、损坏为 `Blocked`、读失败为 `Retrying` | v0.1.0 取 (a) 并在 `AGENTS.md` 写为明示例外；(b) 放到 v0.1.0 之后 |
| Q7 | 多个原因时 `lastError` 取哪一个 | (a) 严格最早 `since`；(b) 宿主内核归属优先（其余 owner 因它失败），否则最早 | (b)，即现行 H4 规则 |
| Q8 | 是否新增可省略 IPC 字段列出全部 owner 的 `Health` | (a) 新增 `recovery[]`；(b) 不新增，现有字段为其投影 | (b)：功能冻结，现有 `changePending`／`cleanupPending`／`controlRelay`／`lastErrorDetail` 已覆盖 |
| Q9 | 控制中继与 U1 的 `EACCES`／外来 socket（G10） | (a) 维持 `retrying`；(b) 记为 `Blocked`（Q2 的慢重读），phase `blocked`，`controlRelay.state` 仍为 `retrying`、`reason` 标明外来（IPC 不变） | (b) |
