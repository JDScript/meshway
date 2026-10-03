# 配置应用与进度

**V2 发布流程设计，尚未实现。** Console 一次点击“应用”，Controller 按执行依赖协调 prepare／activate，Node 回报实际应用结果。

## 已确定与待定

| 项目 | 当前决定 |
| --- | --- |
| Console | 基于上一份配置编辑草稿／diff，一次应用自动协调 |
| 准备屏障 | 按执行依赖组，必要参与节点全部就绪后再激活该组 |
| 激活后失败 | 如实报告，修复／重试或显式回滚为新发布 |
| 整次发布等待超时 | **待定**：有期限失败，还是持续等待人工处理 |
| prepare 资源租约 | 必须有界；与整次发布等待多久分开 |

## 一条完整流程

```text
Console：baseRevision + 本地草稿／patch
  → 点击“应用”
  → Controller 校验、CAS 保存完整目标配置 revision
  → 编译并生成执行依赖组
  → prepare：Node 拉取、校验、持久保存、准备可提前准备的资源
  → prepared：Node 回报确切实例／目标／准备句柄及有效期
  → activate：组内就绪且其他执行条件满足，Controller 持久记录激活决定
  → Node 切换资源／运行代际
  → applied 或 failed 回报
  → SSE 更新 Console 的整体及逐节点进度
```

Console 的 patch 是编辑方式。服务端从明确 baseRevision 还原并验证完整 NetworkConfiguration，仍保存完整版本；对 Node 发送的 full／delta 是另一个层次，基于各节点已保存 digest 计算。

`Network.currentRevision` 表示已受理的**期望配置**，不代表整网已应用。应用失败后，它不会偷偷退回旧值；草稿后续基于这个新 revision 编辑。各节点正在执行什么由 applied 的完整配置引用说明。

配置保存／编译的持久结果提交后可返回 revision 和本次 publication 引用，不占用一个 HTTP 请求一直等节点。发布由 Controller 继续执行，关闭 Console 不取消；重新打开可查询并恢复进度。

显式“取消准备”只结束尚未决定激活的 publication，不撤销已经受理的期望配置或其他组已完成的结果。需要恢复之前的期望时执行显式配置回滚；不能把关闭 HTTP／SSE 当成取消。

## 准备、激活与确认

| 阶段 | 可以证明什么 |
| --- | --- |
| receiving | 正在读取真实 payload；尚未完成校验 |
| saved | 完整配置已校验并私有持久保存 |
| preparing | 校验证书／协议／本机资源，准备可隔离的资源和切换计划 |
| prepared | 本实例对这个确切目标已完成可提前做的准备，持有有限期准备句柄 |
| applying | 已收到有效激活指令，正在切换 |
| applied | 本节点要求的配置资源已实际完成切换 |
| failed | 某个明确步骤失败，附带真实错误与仍有效的配置引用 |

**saved ACK 不是 prepared；prepared 不是 applied；applied 也不是端到端业务连通证明。** 需要验证实际路径时使用独立 observation，不把配置成功计数当成隧道健康。

prepare 不能把新业务规则直接装成有效授权。能提前做的包括解析、凭据校验、完整持久化、资源归属检查和隔离的资源准备；必须在激活时才能完成的端口替换／路由切换等检查如实列出，不能假称已实际安装。

这是一条分阶段协调流程，不承诺跨机器原子事务。组内所有节点 prepared 后，仍可能有节点在收到 activate 前断线，或在安装时失败。applied 是结果回报，不要求管理员再点一次“确认”，也不要求成功节点无限等待所有回报才承认本机已切换。

## 执行依赖组

每份 ExecutionPublication 对应一组需要协调的完整节点配置；其 nodes map 是本次准备目标。参与集合和目标 digest 在发出 prepare 前固定。

- 按新执行实际需要的源、必要中转、目标／出口及资源切换依赖计算，不能只按“管理员改了哪一行”选节点。
- 新路径必经的节点离线，该组明确等待；不会静默从名单中删除，也不能绕过它。
- 纯库存引用、暂不启用的候选和无关终端，不因存在于整网 Config 就加入当前激活屏障；将来要启用其执行分支时再准备所需依赖。
- 共享节点的整份配置或互相影响的资源变更，需要合并协调／串行处理，不能让两组同时覆盖同一节点的目标。
- 真正独立的组可以分别推进。当前整个网络可能同时显示一组 applied、另一组 waiting。
- 旧许可持有者的退出是撤回／lease／地址复用条件，不要求已经吊销的节点再给新配置投 prepared 票。

例如一条新路径 A→C→B 需要三者相容的配置，则三者都准备后激活这一组；无关 D 不阻塞。C 不可用时不能把该组自动改成 A→B。

重新编译改变目标／依赖时创建新 publication 并报告替代关系，不能在原 ID 下换内容或把原 prepared 回报用于新目标。客户端计数针对确切 publication 的固定目标集合；后续按需执行发布不混入原配置应用的分母。

## 激活决定与恢复

Controller 在决定激活前重新检查：

1. 当前期望配置、身份／权限和相关依赖仍允许这组执行。
2. 所有必要节点的 prepared 回报来自当前实例，匹配 publication、target digest 和 preparationId，且准备租约仍有效。
3. 所有必要的退出／地址复用条件已满足；下载齐全不代替这些条件。

**先持久记录 activate 决定，再发送指令。** activate／abort 二选一，已决定 activate 不能改写成“从未提交”。发送可以通过 Agent 下行事件提示并由 Node 拉取当前命令；节点只需出站访问 Controller。

激活指令至少绑定 network／Node、publication、target digest、当前 instance／generation、preparationId 与有效执行许可。Node 独立检查匹配与期限，重复收到时幂等回报，不重复安装；过期或已被替代的指令拒绝。

| 故障 | 行为 |
| --- | --- |
| 保存／prepare 失败，尚未激活 | 自动清理候选准备资源，报告失败；不发 activate |
| 准备租约到期 | 该 prepared 资格失效，需要重新准备；整组是否继续等待由待定等待策略决定 |
| Node 在 prepare 后重启 | 旧准备句柄失效，即使配置仍在磁盘也须重新准备 |
| Controller 在记录决定前重启 | 不猜测已激活；恢复后重新确认实例／准备资格与当前依赖 |
| Controller 已记录 activate，发送或响应丢失 | Node 查询／重试获取同一持久决定；当前实例／许可仍要核对 |
| 已应用但 applied 回报丢失 | 重新回报／查询；Console 显示未确认，不伪称失败或未应用 |
| 新配置／撤销取代当前目标 | 旧准备和旧激活资格失效，按新目标收敛，保留此前真实结果 |

Node 恢复已提交目标时仍需 fresh 认证、当前实例和新的本机准备资格，不能重放旧实例 activate。可以复用已校验内容；能否复用运行资源由实际 owner 和配置一致性检查决定。

## 失败与回滚

**默认采用失败回报＋修复／显式回滚。** 自动跨节点回滚可以成为后续发布策略，目前未选作默认。

- 准备阶段自动清理本次候选资源，这是局部清理，不生成一份回滚配置。
- 激活时的局部错误可以回退尚未提交的本机操作；只有旧资源／旧权限仍有效时才能继续旧运行，不能声称任意安装失败都可恢复。
- 已开始切换的失败可能留下其他节点成功、部分资源已改变的状态。失败节点阻止不完整执行，保留拒绝边界，Controller 如实报告部分结果。
- 需要恢复旧配置时，按当前 CAS、身份、吊销、材料和地址退出条件创建新 revision／publication，再走相同流程。不能倒退 revision 或让 Node 自行恢复失效许可。

旧运行仅在自身许可和 lease 仍有效时维持。已受理的缩权、停用、吊销和地址退出不因等待 prepared 被无限延后；prepare 本身不授予新业务权限。若正在准备的变更包含这些约束，失败也不保证旧业务继续可用。

## Console 展示

建议每次配置应用展示：目标 revision、分组状态和各节点的当前阶段，而不是一个估算百分比。

| 列 | 内容 |
| --- | --- |
| Node | 当前用户有权看到的节点 |
| 目标 | revision／publication／digest |
| 最后回报阶段 | receiving／saved／preparing／prepared／applying／applied／failed |
| 当前步骤 | 校验、落盘、listener、underlay、TUN、DNS、数据面等实际执行项 |
| 当前有效配置 | applied 引用，可能仍是旧目标或 null |
| 时间／新鲜度 | 原回报时间、接收时间、实例；过期时标明最后已知状态 |
| 等待／错误 | 必要节点未就绪、准备过期、等待旧地址退出或实际失败代码 |

整体显示“3/5 节点已准备”“2/5 已应用”“1 节点未确认”等真实计数；offline／未报告是 Controller 联系状态，不伪造成 Node 自报失败。没有目标节点时显示“无需节点部署”，不制造 0/0 百分比。

字节进度只有实际计量且总量已知时才显示百分比；按步骤固定填 20%／80% 不是实际进度。修改来源／SSE 断线不覆盖用户未保存草稿。

## 字段提案

### NodeConfigurationProgress

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `target` | ConfigurationRef | 本次完整目标；包含 revision／publication／digest |
| `phase` | ConfigurationApplyPhase | 上表 Node 自报阶段 |
| `step` | ApplyComponent? | 当前步骤；无执行步骤为 null |
| `components` | Map&lt;ApplyComponent, ComponentProgress&gt; | 各组件真实阶段与错误 |
| `download` | DownloadProgress? | 实际 payload 字节数，未计量为 null |
| `observedAt` | Timestamp | 节点本次观测时间，不因重复读取刷新 |
| `problem` | Problem? | 类型化、脱敏的失败 |

`ApplyComponent` 对应 validate、storage、listeners、underlay、tun、dns、dataplane。`ComponentProgress` 字段为 `phase: pending | running | prepared | applied | failed | notApplicable`、`problem: Problem?`；notApplicable 表示本配置不需要该组件，不用于掩盖缺失能力。

`DownloadProgress` 字段为 `receivedBytes: u64`、`totalBytes: u64?`，统计同一种 payload 字节口径；total 未知不估计比例。状态绑定 NodeState 上报实例，不从 body 接受另一个 Node 身份。

### 准备与应用回执

关键回执经 Agent API 可靠提交，服务端检查目标／实例后保存；连续细节进入 NodeState 和 SSE。二者消费同一应用状态机，不各自产生一套阶段。

| 回执 | 必要内容 | 语义 |
| --- | --- | --- |
| saved | ConfigurationRef、instanceId、保存时间 | 完整内容真实持久保存 |
| prepared | 上述目标／实例、preparationId、preparedUntil、激活时仍需完成的检查 | 有限期准备资格 |
| applied | 目标／实例、实际完成时间 | 本节点切换成功的回报 |
| failed | 目标／实例、失败阶段、Problem | 真实失败，不推断其他节点结果 |

prepared 不是业务执行 lease；SSE 保活也不续任一种资格。回执重试保持同一身份／目标和事件内容，旧实例、旧目标或过期 prepared 不推进当前组状态。

applied 回报须对应服务端已决定并对该实例授予的激活，未获激活的异常自报不能计为成功。回执中的失败阶段与当前进度 phase=failed 分开保存，重启后仍能解释失败发生在准备还是应用期间。

## API 与 SSE

- Console 配置提交返回目标 revision 和本次 publication 引用；`GET .../publications/{publicationId}` 查询组进度，访问权限按当前网络检查。
- SSE 增加 `configuration.progress`，携带 publication、当前分组状态及有权查看的逐节点进度。首次 sync 和重连查询包含当前状态，重放不重复累计计数。
- `configuration.changed` 仍表示期望配置版本已提交；不解释为全网完成。
- Agent 拉取 prepare／activate／abort 指令，按 exact target 处理；保存、准备、应用及失败分别回执，具体生成 DTO 随 API 定版。

Owner／Admin 可看本网络完整进度；普通成员只看相关范围，裁剪后的计数明确标注范围，不能把可见子集全成功宣传成全网完成。持久记录与内存字段见[数据库](/v2/storage/database#executionpublication)和[State](/v2/state-reference/node)。
