# Controller 一致发布与预算

publication在当前网络、同次Store锁/SQLite事务内共享事实、编译结果和节点投影。旧模式与IPv4 Mesh v3都保留这一边界；控制面发布成功不等于Agent已应用或业务可达。[Mesh v3合同](/p3a-mesh-execution)说明源模式/执行协商。

## 源→事实→编译→节点DTO

1. 按显式`NetworkId`读取源revision、地址/活动库存、tag、listener/候选、不可变公钥与所需epoch能力。一次调用内共享，不拼独立HTTP读取，不使用可变连接级selected-network。
2. 纯编译器校验源与授权/必经路径，返回有界内部结果。配置候选不是实测可达；合法但路径不可执行与非法源分开诊断。
3. 从内部结果投影protocol定义的封闭版本化节点DTO，配置/连接/路径/公钥同revision；私有材料另在最小披露边界附加，不放入公共Facts。
4. 对完整节点投影检查实际路径、identity union、安装前缀、连接/材料与序列化字节预算。失败整体回滚，不截断、不跳必经点、不返回可执行部分计划。

`apps/controller/src/store/publication.rs`及相关源存储/执行模块是实际实现入口；`crates/policy`不做网络I/O，`crates/protocol`不反向依赖policy。compiler internal不作为不可信wire直接Deserialize。

## 保存与依赖闭包

写入口保留根事务内权限/新时间检查、网络内CAS与revision bump，在commit前完成受影响依赖的发布校验。策略/tag、连接/能力/listener、公钥、网络/接入、节点/成员撤回各有明确依赖；不将所有写入变成“所有无关源都必须可用”。

例如deny-all下修复路由不强制解码无关损坏连接；有执行路径时须核对邻接/身份预算。撤回可以使saved source引用失效，保留源/诊断、节点effective fail-closed；不能把过滤后的derived候选回存源，导致解除block后丢失原意图。

幂等接入、无变化保存、心跳/ACK/运行观测不凭空制造业务revision。JSONC只改注释也按源CAS保存；注释必须与typed policy等价。任意预算或材料选择失败回滚身份/IPAM/token/owner/listener/revision的整个组合事务。

## 发布与运行是不同证据

| 证据 | 含义 |
| --- | --- |
| saved source / revision | 当前网络管理意图已提交 |
| 编译/节点快照 | 该revision有完整可安装或明确拒绝的投影 |
| saved ACK | Agent验证并私有持久化，非applied |
| runtime report | 当前epoch的应用自报告及其新鲜度 |
| 实际TUN业务 | 本次授权/路径上的真实包流 |

Agent防御校验身份/网络/版本/digest/预算和本机资源；缓存只提供rollback floor，不能离线激活或续租。更高revision、撤回和lease独立停止旧运行，不等待HTTP。完整出口/域名与自动组网仍未完成，不用纯编译/发布成功补齐。

## 验证入口

Controller Store/HTTP行为回归覆盖网络隔离、CAS/JSONC、同事务三节点投影、依赖门槛、材料容量、预算失败回滚和撤回；正常Linux包流由`experiments/p3a-mesh/run.py`验收。命令见[开发检查](/development)，保持独立Cargo target，新增契约必须生成/drift及全部消费者检查。
