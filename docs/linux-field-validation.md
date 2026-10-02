# Linux 跨地域 Docker 验收

**第一轮已于 2026-10-02 执行完成，结果为 `COMPLETED_WITH_FINDINGS`。**
真实两地 / 四地 TUN 互访、DNS、必经节点故障约束和代表端点的 lease 撤回通过；
原严格 runner 保留 `FAIL`，原因是控制面恢复后首尔 runtime 上报未在等待窗口内恢复新鲜度。
后续独立业务探测已证明四地实际包流恢复，不将这一结果代替失败的状态上报检查。

用户最新同意把第一轮提前到打洞之前：使用已验证主线，先核对主机与 Docker 部署条件，
在两地验证固定链路的正常 Controller / Agent / TUN，再扩展四地。
自动建连 / 主备、打洞完成后做第二轮，完整出口 / 域名分流仍属首版。
未审定 P3a / underlay 候选不进入第一轮部署，前端 / GeoIP / 模型大清理不阻塞本轮。
总体依赖见[里程碑](/milestones)。

## 登记范围与部署边界

| 登记别名 | 地区 | 当前状态 |
| --- | --- | --- |
| `jp-tyo` | 东京 | ARM64；两地 / 四地包流与同身份重启已测，存在 E2E 恢复窗口 |
| `us-nyc` | 纽约 | x86_64；直连与必经中转、停止 / 恢复已测 |
| `kr-icn` | 首尔 | ARM64；四地包流已测，发现 runtime 上报间歇过期 |
| `us-lax` | 洛杉矶 | x86_64；独立 Controller、监听 / 中转、控制面停止 / 恢复已测 |

别名是测试库存，不作为地图定位证据；地图位置须来自有效观测。
连接地址沿用用户登记记录，不把登录凭据放进仓库或实验报告。
东京现有管理地址落在 `100.96.0.0/16`，开始时须选择不冲突的测试地址空间。
2026-10-02 的只读预检还确认其管理路径经 WARP；本轮 overlay 使用独立的 `198.18.42.0/24`，
Docker bridge 使用逐机检查不冲突的 `172.31.249.0/28`。设备文件存在不等于容器 TUN 已成功工作，
实际 ioctl、包流和恢复仍须通过后面的运行验收。

固定镜像源码为 `4d106e01ef31c3bd3d1fa0ecf8528d9ddcc3c54f`，源归档 SHA-256 为
`48070b8b6edce69f568d0481ddc70caa0af7e6f1a806ee2ea3e32dd6fd71b0e1`。
两种架构的 Controller 实际 OpenAPI 导出均与该冻结合同一致；四地 image ID 可解析。
Docker 存储后端不同，构建机显示 OCI manifest digest、接收机显示 config digest，不能直接比较 raw ID 字符串；
本轮已核验每架构的 canonical config digest、全部归档 blobs 与 RootFS，确认复制的是同一镜像内容。
完整交付位于批准 TMP 的 `meshway-field-images-cccn4rq4/result.json` 与 `cleanup-manifest.json`。
镜像送达不代表 TLS、TUN 或真实跨地域互通已通过；容器部署 / 包流验收使用 `experiments/linux-field/run.ts`。

**构建器执行偏差补充**：后续核对原 `jp-tyo-builder-limits.json` / `us-nyc-builder-limits.json`
发现两个专用 buildx `docker-container` 构建器实际为 `Privileged=true`。Agent / Controller 五个运行容器始终非 privileged，
但构建器也不符合本轮限制，不能将“运行容器非 privileged”扩张为整条构建流程都符合。
两个构建器已按精确 ID 移除 container / metadata，`--keep-state` 保留原缓存与证据，清理错误 0；后续改用无专用特权构建器的流程，
非 root、cap-drop-ALL 的普通编译容器显式限制资源，不再启动这些构建器。原镜像内容校验 PASS、现场 FAIL 和原 inspect 均不改写。
整改明细见原镜像目录的 `builder-privilege-remediation.json`，原状态、移除命令和缓存前后核验分别记录。

### 第一轮已取得的运行证据

2026-10-02，独立 run `r1-5038bd8a8e3f` 已完成东京 ARM64 ↔ 洛杉矶 x86_64 的正常注册、
平台托管 VLESS WebSocket/TLS、真实 TUN 双向 ICMP / HTTP，以及实际 DNS UDP/TCP 的 PTR / A 查询。
overlay 为 `198.18.42.0/24`，HTTP canary 只绑定各自 Mesh IP，实际 route get 均为 `meshway0`。

- 首批东京→洛杉矶 3/3：321、106、107 ms；反向 3/3：约 106–107 ms，两方向 HTTP 内容完全匹配。
  首包建连成本保留，不将其删去后声称全部样本都是稳态延迟；本轮没有吞吐性能结论。
- 首次启动修正了 fixture 的私有 bind 目录所有权：仅新 bootstrap 文件经一次性 CHOWN 容器移交，
  运行 Agent 仍只有 NET_ADMIN，没有增加 DAC_OVERRIDE；洛杉矶既有测试节点身份未重建。
- **重启恢复仍需单独测量**：首次仅重启东京后，原身份、TUN 和邻接认证已恢复，但业务请求一度失败；
  后续没有修改身份或配置即恢复。独立真实 Runtime 复现现已确认同一机制：存活端仍使用重启端已丢失的 E2E receiver 会话。
  后续 fixture 按 120 秒有界恢复采样，逐次保留失败 / 成功和实际恢复耗时，不能以邻接 current 提前判业务成功。

首批证据和原失败报告保存在批准 TMP 的 `meshway-field-r1-5038bd8a8e3f/`；
`--resume` 会把原 `result.json` 留为独立 `result.previous-*.json`，不覆盖失败证据。
整轮结果与已知缺口如下，不将部分通过扩张为全部恢复验收成功。

### 四地结果与收尾

| 检查 | 实际结果 |
| --- | --- |
| 东京↔洛杉矶 | ICMP / HTTP / DNS 通过，TUN MTU 1000；稳定 ICMP 约 106–109 ms，保留首包建连成本 |
| 东京→洛杉矶→纽约→首尔及反向 | 两方向 ICMP 3/3、HTTP 内容匹配；本批稳态样本约 368–374 ms，首个冷请求 ICMP 可达约 1108 ms |
| 纽约↔洛杉矶 | 两方向 ICMP 3/3、HTTP 内容匹配；本批稳态样本约 60–66 ms |
| 必经纽约停止 | 东京↔首尔失败；首尔→洛杉矶备用边仍可实际通信，没有绕过必经纽约 |
| 必经纽约恢复 | 有序路径业务恢复，本次从启动到成功采样约 9.111 秒 |
| 东京容器重启 | 原 identity 文件字节保持；本次记录 4 轮探测，其中 3 轮失败，55.606 秒时双向业务采样成功 |
| Controller 停止 / lease 到期 | 东京实际业务失败、TUN proof withdrawn，Mesh 路由未落回底层；本批此项直接核对的是东京端点 |
| Controller 恢复 | 后续四个方向 TUN ICMP / HTTP 均成功；首尔 runtime 报告仍间歇 stale，严格状态恢复 gate 未过 |

这些是本次有限采样及从操作开始到成功探测结束的耗时，不是首次恢复精确时间、总体 SLA 或吞吐性能结论。
逻辑有序路径经过真实包流和停必经点的故障验证，本轮没有实现逐包 trace。
底层查询还发现首尔→纽约的未标记转发 FIB 指向现有 `CloudflareWARP`；其他已查目标指向物理接口。
这份证据是路由查询而非抓包 trace，因此本轮不宣称每段均为纯公网直连，更不代表打洞成功。

**第二项恢复缺口：runtime 上报间歇停滞。** 首尔 Agent 在继续同步配置、DNS active 和真实业务恢复时，
反复报 `connection runtime observation changed; resynchronizing`；Controller 上记录一度停在 sequence 25，
后来推进到 66，但截至后续查询仍再次超过 60 秒而 stale。该现象已交独立本地复现 / 修复，
不把偶尔提交成功写成持续新鲜，也不为了 runner 变绿而移除 guard 或把旧报告更新时间伪装成新样本。

运行报告的有限重试修复已完成 RR1/RR2 独立复核；随后组合 `f399931` 的全 Rust 检查出现新的待定位失败：
既有正常 CLI 自动 history 测试在首条持久样本等待 45 秒后超时。原测试在前一版 `40d2bd8` 全套中通过，
本次测试与 sampler 源码未改；现有证据尚不能区分报告接受、采样器启动 / 退避、peer 就绪或调度因素。
组合检查保留 **169 passed / 1 failed（仅停止前部分）**，不批准合入 / 部署运行报告候选，未把该超时当作环境噪声重跑到绿。
原日志及诊断交接：批准 TMP 的 `meshway-field-recovery-integration-f399931.md` 与对应 `-logs/`。

### 恢复修复已合入，等待同身份复验

后续诊断用原样 CLI 及真实 UDP 延迟对照确认了一种可稳定复现的触发机制：报告更早被接受后，
sampler 的首个 tick 尚无 peer，30 秒 tick 又与邻接探测争用同一 pending 槽；原逻辑直接跳过，
下一次机会推迟至 60 秒。修复仅在原 2 秒测量预算内有界等待槽位，成功耗时包含真实等待和自己的 nonce 探测，
持续占用仍为缺测；未缩短 5 秒 / 30 秒周期或扩大原 45 秒检查。原 169/1 episode 缺少的事件未被补造。

运行报告 RR1/RR2 和该 Capacity 机制均已独立复核关闭。新组合 `3b346b8` 实际完整 Rust
**613 passed / 7 ignored**，包含原 CLI、100ms 每向延迟压力、报告 guard、E2E、underlay 与 Apple core；
workspace Clippy / fmt 通过，未变消费者按源码及日志哈希核验沿用。
三项修复已以 `1144875`、`461f7d6`、`4c9a5be` 合入主线。

原 `f399931` 失败与其镜像的未批准状态保留。新的固定源码 `4c9a5be` 正通过非 privileged 流程构建，
将由 `retest.ts` 复用原 Controller DB、身份、公钥、CA、连接 / 路径 / Access 配置验证；
原 v11 源快照已从只读数据副本经原 Controller 和生成 GET 接口补齐，不能以候选启动后的响应充当旧基准。
此处是代码合入与验收通过，尚不代表新镜像的远端复验通过。

交接报告：批准 TMP 的 `meshway-runtime-report-review-fixed-da8b9b7.md`、
`meshway-report-history-review-fff98d1.md`、`meshway-combined-recovery-integration-3b346b8.md`。

收尾已完成：

- 删除精确 owner label 匹配的 **5 个测试容器、4 个 bridge**，清理错误为 0，测试端口不再监听。
- 两个专用 buildx daemon 先停止，后完成上述特权构建器精确移除；镜像、构建 cache、归档、独立 Controller / Agent 私有数据和身份均保留。
- 四台 SSH 正常，既有容器 ID / 镜像 / 端口 / 运行及原 healthy 状态保留；IPv4 / IPv6 RPDB、物理 / WARP IPv4 路由、resolver 文件和 forwarding 值与预检一致。
- IPv6 差异如实保留：纽约 WARP 的 link-local 地址变化，首尔 RA 默认路由 lifetime 从 8995 刷新为 8999，gateway / interface / metric 未变。
  未恢复旧配置，不声称全部 IPv6 观测逐字节一致。未手工编辑宿主防火墙或 Docker daemon 配置。

完整收尾证据在上述 run 目录：`round-summary.json`、`runtime-cleanup.json`、`host-postflight-comparison.json`、
`host-observed-ipv6-differences.json`、`underlay-fib-lookups.json`、`control-restored-business-probes.json`。
原 `result.json` 的 FAIL 与所有前次失败报告原样保留；summary 记录公开证据 SHA-256。
优先修复 E2E 重同步及运行上报这两个实际缺口后做针对性远端复验；自动建连 / 主备 / 打洞仍按第二轮验收。

### 已独立确认的 E2E 重启恢复缺口

固定 `4d106e0` 与主线 `1b6d061` 的相关 Runtime 文件相同。独立的一跳和 A–中转–B 测试均确认：
A 重启后，新的外层认证、邻接探测、fresh E2E echo 与 A→B 请求成功；B 旧 sender ID / 到期时间不变，
B→A 回包因 A 已无对应 receiver session 被正确拒绝。外层重连但端点未重启的控制组则可继续使用旧 E2E 会话。

- 三项独立测试通过，真实 TCP / Noise、未缩短或暂停时钟。约 58.13 秒仍拒绝；旧预算约 59.05–59.07 秒到期后新会话交付成功。
- 59 秒是旧 sender cache 的预算，不是业务恢复上界：还要等待下一次发包、握手、排队与应用重传。
- B–中转通道在多跳反例中从未变化，因此仅清“邻接重连缓存”不能完整修复；普通 rekey 时盲目清反向 cache 还可能造成双向反复握手。
- 后续修复需经端到端认证的接收状态代际或有界 resync，并明确 wire / 能力协商、旧版本行为和 anti-replay；本轮没有降低加密、缩短 TTL 或修改固定远端 binary 来掩盖问题。

该 Runtime 缺口随后通过有界 authenticated `resync-v1` 修复，已独立审查、全消费者验收并合入 `9c26105`，
详见[E2E 恢复合同](/e2e-session-recovery)。快速路径要求参与节点支持扩展；旧参与节点、控制丢包 / 预算竞争仍保留原 TTL 边界。
这是代码层关闭，原现场 FAIL 不改为 PASS；新的同身份 Controller / Agent / TUN 复验尚未执行。

独立报告：批准 TMP 的 `meshway-field-e2e-restart-recovery-review.md`；原远端报告没有 sender ID 采样，
只证明其现象与已独立复现的机制一致，不将后一次成功探测时间伪称为远端首次恢复时刻。

- 使用独立 Docker 网络、测试目录和容器，保持原 SSH 与现有服务。
- TUN 设备及必要网络权限限定在容器 namespace 内，不用 host network / privileged，
  不更改宿主路由、DNS、防火墙或 Docker daemon。若环境不满足，记录阻塞，不扩大权限绕过。
- 使用已审合的固定镜像 / binary，记录架构与摘要；升级保留节点身份和私有持久化。
- 容器接入后，listener、协议参数、凭据、路径与 DNS 由 Web 管理；部署入口只传接入和持久化信息。
  当前本地 demo 的 profiles / 启动参数是迁移对象，不复制为正式产品流程。
- 控制通信使用可达的 HTTPS 入口，先验证底层保护，再启用业务捕获。

## 分阶段验收

### 1. 单台部署

核对系统架构、Docker / TUN 条件、管理网段和端口；完成接入、心跳、配置持久化、
实际执行报告、容器重启及同身份恢复。保存 ACK 与数据面运行分别记录。
镜像启动失败、配置不支持、端口占用均应可从控制台查明原因。

### 2. 两地互访

第一轮通过生成 API 配置固定监听 / 候选与授权，运行真实 Mesh IP / DNS 的 ICMP 和 HTTP，
双向验证。记录实际 dial 方向、承载、配置来源、直连 / 指定路径、重连和撤回。
第二轮再验证按需自动建连、主备与打洞；不以第一轮固定链路结果代替这些能力。

### 3. 四地有序路径与恢复

第一轮扩展直连、指定中转 / 必经节点及反向 dial；关闭必经节点时不得绕过，
覆盖容器重启、连接中断、控制面失联 / lease 到期和授权撤回。
endpoint 自动重绑定、打洞与主备切换进入第二轮。
持续记录实际邻接延迟历史、端到端请求结果、吞吐 / CPU / MTU / 并发及失败窗口；
不把邻接 RTT 相加当作端到端测量，不裁掉断流窗口后报告性能。

### 4. 出口与域名分流

P3c 正式下发后，在同一环境验证出口开关、公共规则 / 方案 / 兜底、IP / 域名规则、
DNS 与出口一致、局域网单独授权、回包 / NAT、方案切换及撤回。
这一步完成之前，跨地域 Mesh 通过不代表首版出网闭环完成。

## 证据口径

每次保留源码提交、镜像 / binary 摘要、拓扑、配置 revision、观测时间、测试命令、
结果与清理状态；敏感接入材料独立私有保存。公网端口映射可达与真实打洞成功分别记录。
云主机的公网可达性不证明受限 NAT / UDP blocked 能力，本地隔离 NAT 场景继续保留。
远端性能发现决定后续定向优化，不恢复已经结束的无界性能矩阵。
