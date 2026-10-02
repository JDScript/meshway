# IPv4 Mesh v3 执行合同

**源策略→同事务publication→生成客户端→正常Agent/TUN的IPv4 Mesh切片已合入并完成真实Linux验收。** 当前已实现grants叠加、有序first-match、必经节点、重启/撤回/HTTP stall/lease；完整出口、自动邻接/主备、打洞与Mesh状态型回包未交付。远端固定版本恢复结果见[跨地域验收](/linux-field-validation)。

## 源、迁移与发布

- 源唯一为policy的`NetworkPolicy`，管理envelope引用它；compiler internal不是可直接信任的节点wire DTO。
- v12保留旧v2表、身份/地址/JSONC，只添加显式source mode与v3源/执行状态；DDL/schema gate/version在同事务，升级不自动激活或放宽reject。
- 旧Access/Routing迁移必须证明等价。非空旧执行授权的direct/exact-hop与自动候选图通常不能证明等价，报告诊断、保留原源；管理员显式替换是新决策，不能标成无损迁移。
- publication在同事务读取源、活动库存/地址/tag、连接候选、epoch能力与不可变公钥，整体编译/预算后投影本节点精确路径/材料。候选是配置事实，不是实测可达。
- 连接block优先、grants并集、Mesh有序first-match；本切片唯一主路径，无主备。任何参与者缺能力/密钥则撤回完整本节点执行，不删除必经点再选路。
- 公共出口规则、profile/assignment/自动地区配置与外部目的地grants当前入口拒绝，不能保存后静默丢弃尚未接线的出网语义。

## 当前HTTP操作

这些操作已进入主线生成OpenAPI，不是旧冻结审查快照中的候选。HTTP接口独立于Next Server Actions；确切DTO/字段以[生成链](/protocol)为准。

| 操作 | 权限/作用域与效果 |
| --- | --- |
| `getNetworkPolicy`：GET `/api/v1/networks/{network_id}/policy-v3` | 当前网络Read；源/JSONC、source mode、CAS与诊断，不是运行状态 |
| `saveNetworkPolicy`：POST `.../policy-v3/save` | 同网Manage/root鉴权事务；显式替换/激活、网络revision CAS，整个依赖发布通过后commit |
| `negotiateMeshExecution`：POST `/api/v1/agents/mesh-execution` | node bearer推导身份/网络；进程nonce/当前epoch能力发布mutation，再同事务下载配置/私有材料，不供SPA轮询 |
| `acknowledgeMeshExecution`：POST `.../mesh-execution/ack` | 当前node/epoch的revision/digest保存回执，不证明applied或续数据面lease |

begin新runtime epoch清旧能力/receipt/报告，迟到请求不能写新epoch；不从agent_version猜能力。v3模式能力变化可推进同一网络revision。旧节点兼容读取只给明确诊断的fail-closed库存，不把v3授权包装成可执行v2；原DNS/exchange/heartbeat/ACK body兼容保留。

保存请求`source_jsonc`省略/null都不保存JSONC；响应nullable字段仍必须出现。节点`snapshot`在legacy mode为null，v3 mode缺失/null均拒绝；空paths/materials是真实空投影，不是unknown。strict成功DTO新增字段也须消费者升级/协商。

## Agent执行与寿命

正常`run --apply-dataplane`仍显式opt-in，先建立runtime lease再协商。Agent独立验证schema/capability、身份/网络/地址、公钥、revision/digest、路径与预算；不用v2 routing intersection重新解释v3。

历史`.mesh-execution.json`私有journal只验证自身完整性并提供回滚下限，不能拿新库存revision将旧历史误判为损坏；新incoming再检查当前floor与same-revision mutation。没有fresh HTTP不离线激活/续租。撤旧代→私有持久化/准备sink/MTU/admission→ACK→执行；物理bind/TUN成功另报运行状态。

硬预算每节点128精确路径、每路径最多8节点、64 union identities、完整2MiB快照；托管材料owner/dialer投影最多64，事务与下载一致校验，不截断或部分发布。管理GET保留saved intent，effective projection不写回源。

lease watchdog、更高可信revision、源撤回与ACK冲突独立fence旧packet/采样/运行代际，不等HTTP结束。carrier双向仅复用通道，业务授权仍有方向。正式[underlay owner](/underlay-routing)正常主循环尚未启用，既有pin/sink需按真实ownership迁移。

## 仍需完成

单向grant可交付请求，但尚无Mesh状态型回包admission；往返业务现须双向grant，不得把反向主动访问一并隐式授权。Linux出口原型的flow tracker不等于正常Mesh回包完成。

默认STUN观察点/托管发现、GeoIP与自动方案实测未接通；TCP/VLESS-only不能从UDP映射补TCP可达性。完整域名/IP出网、IPv6 overlay、自动邻接/主备与打洞见[首版剩余工作](/milestones)。

## 重现

`experiments/p3a-mesh/README.md`维护镜像/runner参数；`run.py`使用普通Controller、生成管理API、正常Agent及真实TUN，无本地profiles，独立bridge/私有状态、仅容器NET_ADMIN/TUN，非privileged/非hostnetwork。构建命令通过共享锁：

```sh
python3 scripts/performance_lock.py docker build -f experiments/p3a-mesh/Dockerfile -t meshway-managedtest:p3a-execution .
python3 experiments/p3a-mesh/run.py meshway-managedtest:p3a-execution
```

批准TMP保留`p3a-fixes-docker-normal.log`、原P3a fixture result与`meshway-p3a-agent-review-fixed-a36e24e.md`/`meshway-p3a-control-review-fixed-a36e24e.md`。重启journal、saved/effective分离和材料容量回归已独立关闭；原失败不改写。消费者检查见[开发与验收](/development)，不以编译器或mock原型替代真实包流。
