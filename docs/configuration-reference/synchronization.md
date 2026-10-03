# 同步与分发

**设计草案。已确认方向：Config／State 分离并分别下发；Config 保存完整历史版本并支持回滚。**

## 两种更新

| | Config | State |
| --- | --- | --- |
| 内容 | 管理期望、编译后的许可和执行参数 | 实际能力、endpoint、通道、观测与使用选择 |
| 更新原因 | 配置提交；执行切片依赖改变 | 实际事件／观测 |
| 标识 | 源配置 revision + 节点配置 digest | 当前实例、sequence、时间与配置引用 |
| 持久性 | 整网源完整版本；Agent 私有保存目标切片 | 最新值有期限，必要历史另存 |
| 回滚 | 历史配置作为新提交 | 不回滚到旧观测／旧通道 |

“分别”指独立载荷、排序和处理规则，不强制开两条网络连接。endpoint 更新不写成一版管理员配置；可信公钥／访问许可变化则必须走一致的身份和配置发布。

## Config update

```jsonc
{
  "networkId": "<network-id>",
  "nodeId": "<node-id>",
  "configurationRevision": 43,
  "targetDigest": "<target-node-config-digest>",
  "payload": {
    "type": "delta",
    "baseDigest": "<installed-node-config-digest>",
    "changes": []
  }
}
```

草案同时支持 `full` 载荷携带完整节点配置。changes 的操作类型待定，优先采用有明确增删／替换语义的强类型更新，不暴露任意对象 mutation。

1. Controller 编译新目标，先做 node-specific slice，再计算前后切片差异。
2. Agent 检查网络、节点、基础 digest，在临时副本还原。
3. 对完整目标做 Schema、跨字段、授权／材料一致性和摘要校验。
4. 原子保存后发保存 ACK；通过资源准备与代际切换后另报执行状态。

基准丢失／不匹配时重取 full。数组顺序、清空和删除必须有不同的明确表示，不把部分补丁直接施加到正在运行的路由表。

## State update

State 更新独立携带当前实例／序号和有效期。与执行相关的消息引用 `configurationRevision` 及确切 NodeConfiguration digest，不能将新 State 拼到不匹配的旧配置。

Config 已撤回或 lease 到期时停止旧运行代际，不等 State 通道恢复。新的 State 不能续活已失效的旧授权；缺少必要配置时只允许有界缓存／重同步。

## 原子发布

完整配置版本、current 指针及必要的持久发布任务同事务提交，成功后替换当前不可变内存快照。若以后增加持久查询索引，也纳入同一提交边界。事务成功后才能发消息；中断后可重建／重发相同目标，不能把事务外提前发出的配置当已提交。

大量编译工作可在事务外进行，但提交时必须 CAS 检查配置与相关实体输入仍一致。单节点保存／激活的原子性不等于整网同时切换。

grants／connections 没有 ID：本版增量可整段替换数组，错误／编辑定位使用配置 revision 与 JSON Pointer。remark 允许重复，不承担 patch 主键职责；不为增量重新要求用户分配规则 ID。

## Rollback

选择某个历史 NetworkConfiguration，重新校验当前身份、引用和资源依赖，保存为新 revision。历史默认不因界面分页而删除；实际保留策略另行确认。

回滚不复活撤销身份、不恢复旧连接或 endpoint。所需历史规则集／材料依赖缺失时给出明确错误，不能用最新内容冒充历史版本。
