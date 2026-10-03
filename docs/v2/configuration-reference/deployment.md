# 配置应用与进度

**V2 设计，尚未实现。** Console 一次点击“应用”，Controller 保存并发布目标配置，Node 拉取后独立校验、保存、应用并回报结果。

## 流程

```text
Console：baseRevision + 本地草稿／patch
  → 点击“应用”
  → Controller 校验、CAS 保存完整目标版本及一致执行快照
  → Node 拉取 full／delta，校验并持久保存
  → Node 直接进行本机资源安装／代际切换
  → 回报 applied 或 failed
  → SSE 更新 Console 的整体及逐节点进度
```

Console 的 patch 是编辑方式，服务端还原并校验完整 NetworkConfiguration。Node 的 full／delta 基于各自已保存 digest 生成；两者不是同一份补丁。

每用户每网络一份跨页面草稿。最终提交前展示有效修改数量、前后对照及删除／改址／身份／材料影响，规则见[编辑与应用对照](/v2/configuration-reference/editing)；预览不生成材料或预留资源，仍是普通 apply。

配置事务提交后返回 revision 和 publication 引用，各节点各自应用。Node 离线不阻塞其他节点；重新接入时同步当前目标，不依次执行所有历史版本。关闭 Console 或取消 HTTP 不撤销已受理配置。

`Network.currentRevision` 表示已受理的期望配置。每个节点的 saved／applied 引用可能不同，Console 如实显示；失败不会偷偷退回旧的 currentRevision。

## 节点阶段

| 阶段 | 含义 |
| --- | --- |
| receiving | 正在接收实际 payload |
| validating | 还原并检查完整 Schema、身份／材料、摘要及本机条件 |
| saved | 完整配置已私有持久保存 |
| applying | 正在安装资源、检查执行条件并切换运行代际 |
| applied | 本节点要求的配置资源已实际完成切换 |
| failed | 某个明确步骤失败，附带真实错误 |

**saved 只证明保存，applied 只证明本机配置应用，不等于端到端业务连通。** 业务路径验证使用实际连接证据或独立 observation。

同一 Node 按目标代际串行应用。执行前核对当前网络、实例、publication、digest 与有效许可；迟到的旧下载、回执或安装任务不能覆盖新目标。相同目标的重复投递／回报幂等，不重复安装。

## 执行依赖与安全边界

Controller 仍从同一源版本和可信依赖生成完整执行快照，Node 只使用自己收到的许可。

- 各节点完成安装的时间可以不同；接收方独立校验相容的身份、地址、完整路径与执行许可。
- 对端尚未取得相容配置时，对应业务等待／拒绝；不能据此放宽权限或跳过必经节点。
- 本机资源检查和切换顺序仍由资源 owner 处理，失败不能留下可执行的半份配置。
- 改址／地址复用仍满足[旧绑定退出条件](/v2/configuration-reference/addressing#改址与安全切换)；此时显示具体等待原因。
- 撤销和执行 lease 到期独立终止旧运行，SSE 保活、重复心跳或应用重试不能延长旧许可。

ExecutionPublication 是一致快照和进度关联，不要求所有参与节点先给出准备票据。成功节点无需等待其他节点确认自己已完成本机应用。

## 失败、恢复与回滚

| 场景 | 行为 |
| --- | --- |
| Controller 校验／提交失败 | 保留 Console 草稿，不发布新目标 |
| Node 下载／校验／保存失败 | 报告失败，清理本次临时资源，不执行不完整配置 |
| Node 安装失败 | 报告失败步骤和仍有效的配置；保留拒绝边界，不伪称整份 applied |
| Node 离线／控制请求失败 | 显示联系状态，按有界退避重连／同步；不阻塞其他节点 |
| Node 重启 | 以缓存为基准，fresh 认证／实例确认后同步当前有效目标 |
| applied 回报丢失 | 重新上报／查询；未确认不等于未应用，也不等于失败 |
| Controller 重启 | 从持久目标／回执恢复，旧回执只作最后已知证据，等待当前报告 |

默认失败后修复／重试，或显式回滚为新 revision／publication。临时资源可以自动清理；旧运行仅在资源和权限／lease 仍有效时维持，不能用本机回退复活已失效许可。

回滚仍检查当前身份、吊销、材料和地址退出条件，并走同一普通应用流程；不倒退版本号或恢复旧连接。

## Console 展示

每个节点展示目标、当前阶段、实际 applied 引用、最后回报时间／新鲜度、等待或错误原因。离线与未报告是联系状态，不伪造成 Node 自报 failed。

整体显示“2/5 已应用”“1 节点失败”“2 节点未确认”等真实计数；没有目标节点时显示“无需节点部署”。人数按本次 publication 的固定目标集合计算，重复读取／重放不会加一，后续 demand／refresh 发布不混入原分母。

一次提交可涉及多个 publication。`GET /configuration/revisions/{revision}/progress` 由服务端汇总该次配置应用目标，按 NodeId 去重并保留每个节点的精确目标／实际 applied 引用和替代关系；Console 不自行合并计数。重复／重试不增加分母，后续 demand／refresh 不凭相同源 revision 混入；无法证明旧目标已满足时标 superseded，不能仅凭 revision 相同算成功。

configuration.progress SSE 携带相同 revision 的服务端 summary，与该 GET 复用类型和去重计算；节点变化使用相同片段类型。客户端直接替换汇总，不需要收到每条事件后再 GET 或自行相加。

汇总带 `scope: full | partial`；已被后续目标替代的发布显示 superseded 和可见替代引用，不永久挂为未确认。联系状态由服务端按真实时间计算，不让 Console 自定在线阈值。

配置变化通知保持草稿和 CAS 基准；`configuration.changed` 表示期望已提交，`configuration.progress` 才展示各节点处理结果。首次同步和重连查询能恢复当前进度，不依赖回放全部中间事件。

## 字段提案

### NodeConfigurationProgress

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `target` | ConfigurationRef | revision／publication／digest |
| `phase` | ConfigurationApplyPhase | 上表节点阶段 |
| `step` | ApplyComponent? | 当前步骤；无执行步骤为 null |
| `components` | Map&lt;ApplyComponent, ComponentProgress&gt; | 各组件真实阶段／错误 |
| `download` | DownloadProgress? | 实际 payload 字节计量，未计量为 null |
| `observedAt` | Timestamp | 原观测时间，不因读取刷新 |
| `problem` | Problem? | 类型化、脱敏的失败 |

ApplyComponent 为 validate、storage、listeners、underlay、tun、dns、dataplane。ComponentProgress 字段为 `phase: pending | running | done | failed | notApplicable`、`problem: Problem?`；notApplicable 只表示本配置不需要该组件。

DownloadProgress 字段为 `receivedBytes: u64`、`totalBytes: u64?`，使用同一字节口径；总量未知不估百分比。组件 done 或下载完成都不单独等于整份配置 applied。

### 回执与 API

saved／applied／failed 关键回执绑定目标、当前实例／代际和原结果时间，服务端核对后保存。连续细节进入 NodeState／SSE，两者来自同一应用状态机；异常目标、旧实例或过期许可的自报不推进当前进度。

新 Controller 回执合同使用 `attempt: Version` 与 `sequence: Sequence`：在同一 instance／target 内，新的本机应用尝试递增 attempt（从 1 开始）；每次尝试内的结果递增 sequence（从 0 开始）。传输重发复用同一 attempt／sequence 和完整内容，修改同一位置的内容报冲突。applied 需要同一尝试内更早 sequence 的 saved；同 phase 的后续真实事件可以使用新 sequence。历史保留迟到证据，当前投影按 attempt／sequence 而非到达时间排序；新的尝试不会沿用旧尝试的 applied。

执行需求重试恢复当前仍有效的决策，不重新发布或延长其他节点 lease。新增／取消需求先与全部有效需求联合编译，再只替换实际变化的节点切片；无关节点保留原 publication、lease 和回执。撤销仍使依赖该身份／密钥的目标失效，不恢复已撤回 publication。

实例 acquire 的同一 instanceId 仅用于当前 boot 下仍有效实例的幂等重试，能力声明必须相同。释放、过期或 Controller 重启后重新 acquire 使用新 instanceId；历史 instanceId 不再成为新实例。acquire 使用共享 Capability 值（transport 的 roles／streams、feature 的 version）；它是认证后的执行能力声明，NodeState.capabilities 是观测来源，不能自行覆盖已编译能力或授权。

Console 通过 publication 查询和 `configuration.progress` SSE 获取状态。Agent 拉取目标配置并正常应用，分别报告保存和应用结果；无需第二个激活命令。Owner／Admin 看本网络进度，普通成员看相关范围，统计明确标注范围。
