# 同步与分发

**设计草案。已确认方向：Config／State 分离并分别下发；Config 保存完整历史版本并支持回滚。**

## 两种更新

| | Config | State |
| --- | --- | --- |
| 内容 | 管理期望、编译后的许可和执行参数 | 实际能力、endpoint、通道、观测与使用选择 |
| 更新原因 | 配置提交；执行切片依赖改变 | 实际事件／观测 |
| 标识 | 源 revision + 一致执行发布引用 + 节点 digest | 当前实例、sequence、时间与执行配置引用 |
| 持久性 | 整网源完整版本；Agent 私有保存目标切片 | 最新值有期限，必要历史另存 |
| 回滚 | 历史配置作为新提交 | 不回滚到旧观测／旧通道 |

“分别”指独立载荷、排序和处理规则，不强制开两条网络连接。endpoint 更新不写成一版管理员配置；可信公钥／访问许可变化则必须走一致的身份和配置发布。

## Config update

一条完整链路是：

```text
管理员提交 NetworkConfiguration
  → 解析 defaults + 本节点选项 + 目标 listener 参数，准备缺省材料
  → 校验并编译公共事实；按获准需求生成候选路径与完整节点配置
  → CAS 事务保存完整配置、稳定引用和含节点切片的发布版本
  → full 或 node-specific delta
  → Agent 还原、校验并原子保存完整配置，发送保存 ACK
  → 资源准备与代际切换，独立上报实际 State
```

Config 是完整参数集合，State 是之后独立更新的运行事实。准备／编译失败不提交新版本；提交时还须核对所用输入未变。完整协议参数例子见 [NodeConfiguration](/v2/configuration-reference/node-configuration#example)。

```jsonc
{
  "networkId": "<network-id>",
  "nodeId": "<node-id>",
  "configurationRevision": 43,
  "executionPublication": "<consistent-publication-reference>",
  "targetDigest": "<target-node-config-digest>",
  "payload": {
    "type": "delta",
    "baseDigest": "<installed-node-config-digest>",
    "changes": []
  }
}
```

草案同时支持 `full` 载荷携带完整节点配置。changes 的操作类型待定，优先采用有明确增删／替换语义的强类型更新，不暴露任意对象 mutation。

先生成目标节点完整配置，再计算前后切片差异。Agent 检查网络、节点与基础 digest，在临时副本还原；随后校验完整 Schema、授权／材料一致性与摘要。初版所需材料随完整参数内联，不能保存缺少秘密材料的空壳后宣称可执行。

基准丢失／不匹配时重取 full。数组顺序、清空和删除必须有不同的明确表示，不把部分补丁直接施加到正在运行的路由表。

## State update

Node [上报自身 State](/v2/state-reference/reporting)，Controller 维护一份 [NetworkState](/v2/state-reference/network)。消费节点可[按需查询或订阅](/v2/state-reference/query)，不要求 Controller 预生成每个节点的永久 State 副本；查询和推送都在服务端按身份／配置做字段裁剪。

State 更新独立携带当前实例／序号和有效期。与执行相关的消息引用 `configurationRevision` 及确切 NodeConfiguration digest，不能将新 State 拼到不匹配的旧配置。读取当前报告不能刷新原始候选或测量期限。

Config 已撤回或 lease 到期时停止旧运行代际，不等 State 通道恢复。新的 State 不能续活已失效的旧授权；缺少必要配置时只允许有界缓存／重同步。

## 原子发布

完整配置版本、current 指针及必要的持久发布任务同事务提交，成功后替换当前不可变内存快照。若以后增加持久查询索引，也纳入同一提交边界。事务成功后才能发消息；中断后可重建／重发相同目标，不能把事务外提前发出的配置当已提交。

大量编译工作可在事务外进行，但提交时必须 CAS 检查配置与相关实体输入仍一致。单节点保存／激活的原子性不等于整网同时切换。

access／topology 规则没有 ID：本版增量可整段替换数组，错误／编辑定位使用配置 revision 与 JSON Pointer。remark 允许重复，不承担 patch 主键职责；不为增量重新要求用户分配规则 ID。

## 执行发布与恢复 {#publication}

执行发布将完整源配置 revision、相关 Node／User 身份与公钥依据、规则集内容以及参与节点的目标 digest 一致绑定，并提供持久的防回退顺序。协议材料已在完整配置内，目标 NodeConfiguration 直接嵌入发布的 nodes map。按需许可见[执行流程](/v2/execution)，具体引用／序号编码在 API 阶段统一。

源配置不变而按需路径、身份有效性／吊销或规则集／解析依赖改变时，可以产生新执行发布，不伪造一次管理员文本编辑。协议参数／凭据值的修改则进入新配置版本。A 使用 B 的 State 时，对照发布中 B 资源的可信绑定，不比较 A/B 的 node digest 相等，也不只比较源 revision 相等。

| 场景 | 执行结果 |
| --- | --- |
| 新配置已保存，资源未准备好 | saved 前进，报告 preparing／failed；不能标记 applied |
| 缺 peer 配置／材料或 State 对不上发布 | 有界等待或重同步，不启动未知权限的业务 |
| 权限缩减、Node 撤销或 lease 到期 | 先停止旧准入并保留拒绝边界；准备失败不能恢复已失效权限 |
| 节点 enabled=false | 撤回业务执行，保留获准的控制同步；回滚启用须重新取得有效执行许可 |
| 地址池／Node 地址变化 | 同版编译地址、DNS 与准入；受影响旧流退出，复用等待地址退出下界，saved 不等于切换完成 |
| 明确的新配置／使用选择取消接管 | 在安全切换后释放对应捕获，才恢复本地网络 |
| Agent 重启 | 缓存作恢复基准；取得当前实例和 fresh 执行许可后再激活 |
| Controller 丢失 RAM State | 更换 State epoch／reset；不丢失持久身份撤销和发布恢复下界 |

实例 acquire/resume 必须由节点凭据验证、幂等，并能 fence 被替代实例。执行授权 lease 与 State 联系时效分开；watch 重连或历史回放不能延长旧许可。不要求整网二阶段同时激活，切换期间可以明确不可用。

地址期望保存在配置，旧地址的执行退出依据独立持久保存；网络改段／节点改址的步骤见 [Addressing](/v2/configuration-reference/addressing#改址与安全切换)。

## Rollback

选择某个历史 NetworkConfiguration，重新校验当前身份、引用和资源依赖，保存为新 revision。历史默认不因界面分页而删除；实际保留策略另行确认。

listener 代际绑定按当前配置到目标的转换重新产生，不直接复制历史内部代际；已删除资源恢复时是新运行资源。历史凭据值作为明确恢复输入，仍须通过当前吊销／有效期校验，不能把普通同名新建暗中变成恢复旧值。

回滚不复活撤销身份、不恢复旧连接或 endpoint。所需历史规则集／材料依赖缺失时给出明确错误，不能用最新内容冒充历史版本。

此处不可复活指永久接入／密钥吊销。节点配置 enabled 的停用可以回滚；identity 是配置权威，但涉及 Tag→User／改绑 User 的回滚仍须当前设备登录证明，不能借历史版本绕过身份转换权限。

地址池／地址也可回滚为新版本，仍校验当前唯一性、有效依赖和旧绑定退出窗口；回滚不直接恢复旧 IP 会话或清掉 AddressRetirement。

已确认：历史凭据／证书吊销或规则集内容不可用时，**回滚失败并列出依赖**；由管理员提供替代后再作为新配置提交，不自动生成替代材料或换用最新数据继续回滚。历史邮箱仍绑定原 User ID，不能落到后来复用该邮箱的新 User。
