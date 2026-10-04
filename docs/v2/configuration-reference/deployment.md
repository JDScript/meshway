# 配置同步、应用与恢复

Console 一次提交，Server 保存完整目标和一致执行快照，各 Node 独立校验、保存、安装并回报结果。Server 以真实回执展示进度；运行能力范围见 [Node](/v2/node)。

## Config 与 State

| | Config／执行目标 | State |
| --- | --- | --- |
| 内容 | 管理期望、可信身份／材料、业务许可与参数 | 实际资源、能力、通道、观测与安装结果 |
| 标识 | 源 revision、publication、节点 digest | instance、sequence、时间及执行引用 |
| 更新 | 源编辑或执行依赖变化 | 真实事件／报告 |
| 恢复 | 完整保存，历史恢复为新版本 | 不恢复旧观测为当前事实 |

两者有独立载荷和处理规则，不要求两条物理连接。endpoint／测量更新不伪造配置编辑；State 不携带额外业务授权，也不续活旧 lease。

## 普通 apply

```text
草稿／typed ops + expectedRevision
  → Server 校验、准备材料／资源，最终事务重检
  → CAS 保存完整源、绑定、current 指针与执行发布
  → 提交后通知；Node 获取 full／delta
  → 校验、私有保存（saved）→ renew 取得执行 lease → 本机安装／代际切换
  → applied／failed 与独立 State／SSE
```

编辑和影响对照见[整网草稿](/v2/configuration-reference/editing)。编辑 patch 与节点执行 delta 不是同一补丁。节点离线不阻塞其他节点，恢复后同步当前目标；关闭 Console 不取消已接纳的提交。

`currentRevision` 是期望，非全网 applied。失败默认修复／重试或显式恢复为新版本，不自动倒退整网配置；无需全体节点先 ready 的准备屏障或第二次激活命令。

## Full 与 delta

先形成完整 NodeConfiguration，再计算强类型增量。Node 核对自身网络／身份、base 和目标引用，在临时值中还原，验证完整类型、材料、权限及 digest 后持久保存；不直接把部分 patch 写入运行路由表。

基准不存在或不匹配时重新获取 full。数组顺序、删除和清空分别表达，remark 不作主键。完整参数包含当前执行所需材料，不能将缺少材料的空壳保存为可执行目标。参数示例见[NodeConfiguration](/v2/configuration-reference/node-configuration)。

## 执行发布与依赖 {#publication}

publication 固定源 revision、相关身份／公钥、材料、规则集／解析依据以及参与节点的目标 digest。源不变的需求或资源变化可以形成新发布，参数／凭据值编辑则形成新源版本；不能仅凭 revision 相同拼接输入，也不要求不同 Node 的 digest 相等。

需求先合并后联合编译，只替换实际改变的节点切片。无关节点保留仍有效的目标／lease／回执；有效重试不重复扰动它们。每条备用完整路径也满足授权和必经点，见[按需执行许可](/v2/execution)。

### 生命周期

- 一个网络同时可以存在多份有效 publication，每个节点只引用其中一份作为当前目标。新发布只改变受影响节点的引用；一份 publication 直到没有任何节点引用时才撤回。`supersedes` 记录来源，不表示旧发布整体立即失效。
- 节点取得新目标时，其执行 lease 被清空。`getNodeConfiguration` 只返回目标，不授予 lease；目标 enabled 时，节点须先 `renew` 取得有效执行期限，才能上报 `applied`。
- `renew` 与 `getNodeConfiguration` 在同一事务内处理本网络已到期的需求，并可能为受影响节点生成刷新发布。因此一次普通配置读取也可能带来新的 publication 与进度通知。

接收方独立检查相容身份、地址、完整路径与许可；依赖未就绪时对应业务等待／拒绝。节点本机按 owner 顺序准备保护和资源，不以部分安装成功宣称整份应用成功。

撤销、身份失效和 lease 到期独立于 HTTP／State 通道终止旧执行。改址同时更新 DNS、地址准入与路径，旧绑定按[地址退出条件](/v2/configuration-reference/addressing#改址与安全切换)保留；ACK 不表示旧流／NAT／缓存已退出。

## 阶段与回执

| 阶段 | 含义 |
| --- | --- |
| receiving／validating | 接收并还原、校验实际目标 |
| saved | 完整目标已私有持久保存 |
| applying | 安装资源、检查条件并切换本机代际 |
| applied | 本节点要求的资源完成切换 |
| failed | 明确步骤失败，带真实错误 |

**saved ≠ applied ≠ 业务连通。** 同一节点按目标代际协调应用，迟到下载、安装和回执不覆盖新目标。

持久回执使用 `attempt: Version`、`sequence: Sequence`：同实例／目标的新尝试递增 attempt，每次尝试内递增 sequence；传输重发复用原值和完整内容，修改同一位置报冲突。applied 需同尝试内更早的 saved，新的尝试不继承旧 applied。历史保留迟到证据，当前投影不按到达顺序覆盖。

instanceId 只用于同 boot 下仍有效实例的幂等 acquire。释放、过期或 Server 重启后使用新 instanceId；能力声明从认证后的 acquire 得到，普通 State 报告不能自行覆盖执行能力。

## 进度展示

按 revision 的 GET 与 `configuration.progress` SSE 共用 `ConfigurationApplyProgress`。目标固定于该次配置提交并按 NodeId 去重，后续 demand／refresh 不扩大原分母；无需 Console 拼合多个 publication 计数。

结果 phase 与 freshness 分开：历史 applied 可同时已过期或被替代，不是当前健康。返回 full／partial 范围，隐藏节点不进入成员可见计数；当前期望引用可由 Node 库存的 desiredExecution 对照。零目标返回 targetCount=0／空 publicationIds，离线节点不等于无目标。

配置通知不覆盖本地草稿／CAS 基准。首次同步与重连可以恢复当前进度，不能依赖浏览器完整接收所有中间事件。

### State 进度字段

`ConfigurationProgress` 包含 target、phase、observedAt 和可空 problem。它记录精确执行引用、实际阶段和原观测时间；不提供组件或下载字节百分比。持久 `ReceiptRequest` 则使用 saved／applied／failed 与明确 failure 分类，不能把联系丢失转换成安装失败。

## 恢复

| 场景 | 处理 |
| --- | --- |
| 校验／提交失败 | 保留草稿，不发布目标 |
| Node 下载／保存／安装失败 | 清理本次临时资源，保留有效拒绝边界，报告具体失败 |
| Node 离线或回报丢失 | 标注最后已知状态与新鲜度，按有界重试同步；未确认不直接等于失败 |
| Node 重启 | 缓存作为基准，重新认证、建立实例并取得有效执行资格 |
| Server 重启 | 从持久目标／回执恢复，重建 State epoch，重新同步当前事实 |

恢复历史配置仍形成新 revision，并检查当前身份、材料、精确资源版本和地址退出条件。缺失／吊销依赖明确失败要求替代；不换用最新数据或自动生成材料冒充历史恢复。

删除的 listener 同名重建是新资源，身份转换仍需设备证明，Node 永久删除不恢复。回滚不恢复旧会话／endpoint，也不清除地址退出限制；详见[历史合同](/v2/api/history)。
