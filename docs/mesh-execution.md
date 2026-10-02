# IPv4 Mesh 执行合同

Controller 只保存当前四层 `NetworkPolicy`。纯编译器与正常 Agent/TUN 的 IPv4 Mesh 执行边界已接通；新单模型合同的消费者整合与正常 Linux 包流仍须验收，编译或 fixture 通过不替代真实组网。

## 源与发布

新网络原子保存默认连接允许模板、成员 grants 与空有序 Mesh 路由。不存在源模式选择、迁移提案或激活步骤。数据库仅安装当前 schema；旧开发数据由部署者显式重置，普通启动对未知、损坏或缺失结构明确拒绝。

同事务 publication 读取当前网络 revision、活动库存/地址/tag、listener/候选、epoch 能力及不可变公钥，再完整编译、预算和投影。配置候选不是实测可达。连接 block 优先，grants 并集，Mesh 路由从上到下 first-match；当前节点执行只支持一条 IPv4 Mesh 精确主路径。参与者缺能力/密钥时撤回整个本节点执行，不能跳过必经点。

公共出口规则、profile/assignment/自动地区配置和外部目的地 grants 当前保存入口拒绝。完整出口、域名分流、状态型 Mesh 回包、IPv6 overlay、自动邻接/主备、打洞尚未完成。

## HTTP 与类型

| 操作 | 权限与语义 |
| --- | --- |
| `getNetworkPolicy`：GET `/api/v1/networks/{network_id}/policy` | 当前网络 Read；源、JSONC、revision、valid、诊断 |
| `saveNetworkPolicy`：POST `.../policy/save` | 当前网络 Manage；根事务身份复核、revision CAS、依赖发布预算后 commit |
| `negotiateMeshExecution`：POST `/api/v1/agents/mesh-execution` | node bearer 推导网络/身份；当前 epoch/instance 能力发布与同事务下载，不供 SPA 轮询 |
| `acknowledgeMeshExecution`：POST `.../mesh-execution/ack` | 本节点当前 epoch/revision/digest 保存回执；非 applied 或业务 lease 续期 |

`NetworkPolicySnapshot` 含必需完整 policy，无 source mode/迁移字段。`source_jsonc` 响应必须出现，可 null。执行响应为 `{ revision, snapshot }`，snapshot 始终是具体 `MeshExecutionSnapshot`；`valid:false` 与空 paths 是真实失败关闭投影。

`NodeMeshExecution` 只含稳定执行合同，不下发完整源或 compiler internal。库存 `/agents/configuration` schema 2 仅含 revision、network、self、peers，仍有独立 node-scoped digest/ACK；DNS、兑换与心跳保持各自语义。旧 policy/access/Flow source DTO 与旧 dataplane 下载端点均已删除。

begin 新 runtime epoch 清除旧能力/回执/报告，迟到请求不得写新 epoch；不从 agent_version 猜能力。能力变化推进同网 revision。精确模型与运行时 schema 由[生成链](/protocol)产出。

## 执行与寿命

Agent 独立验证 schema/capability、身份/网络/地址、公钥、revision/digest、路径与预算，并检查本机资源。私有 journal 仅提供恢复/rollback floor，不允许无 fresh HTTP 离线激活或续租。更高可信 revision、撤回、ACK 冲突和 lease 独立停止旧代际，不等 HTTP 完成。TUN 停止后 Mesh 地址池保留 sink，不能落回底层默认路由。

预算每节点 128 精确路径、每路径最多 8 节点、64 union identities、64 托管材料、完整 2 MiB。事务门槛与下载防御一致，超限整体拒绝，不截断或部分发布。管理 GET 保留 saved intent，effective candidate 过滤不写回源。

真实验证入口为 `experiments/p3a-mesh/run.py`；它使用正常 Controller 下发、Agent 生命周期与 TUN，而非编译器-only 模拟。普通容器仅获所需 NET_ADMIN/TUN，隔离 bridge、私有状态与精确清理；不使用 privileged/host network。历史原始测量与失败证据由 Git 和私有产物保留，不把旧合同运行记录当作新合同已通过。
