# 配置、库存与 revision

本文描述当前实现。正在逐项对齐的新模型见[配置与状态模型（讨论稿）](/network-configuration)，其结构尚不是现行 API 合同。

每张网络有共享单调 `revision`。policy、IPAM、DNS、标签、承载与不可变节点身份按显式网络作用域读取/保存；策略源、连接、路径、公钥和材料由同 revision publication 投影。

## 地址与当前数据库

新库使用 `apps/controller/src/store/schema/schema.sql`，完整初始化与初始网络/default policy 同一个 IMMEDIATE 事务。普通启动只接受当前版本和完整结构，核对 admin.token、材料key、FK与全部持久化双栈地址；未知/损坏库拒绝，不自动清空或修复。旧开发数据重置须由部署者显式操作。

默认 IPv4 池 `100.96.0.0/16`，IPv6 随机 `fd00::/8` ULA `/64`；网络内地址唯一，撤销地址不复用。自定义初始化池须规范并有 IPv4 可用主机地址。重启不改址，显式参数与已存池不同时拒绝。IPv6 目前仅地址元数据，不代表 IPv6 TUN 已实现。

## 唯一 policy 与 CAS

GET `/api/v1/networks/{network_id}/policy` 返回当前 `NetworkPolicySnapshot`：network_id、revision、完整policy、必需但可null的source_jsonc、valid与真实diagnostics。无源模式、旧源或迁移提案。

POST `.../policy/save` 使用 expected_revision 与完整四层policy。源/JSONC一致性、当前权限、CAS、编译和发布预算都在保存边界检查；过期CAS为409，语义/预算错误为422。无变化保存不推进revision，只有注释改变也属于源变更。当前外部出口配置未接线，入口明确拒绝而非保存后忽略。

## 库存配置与 ACK

node bearer GET `/api/v1/agents/configuration` 返回 `ConfigurationSnapshot`，其中 `NodeConfiguration` schema 2 仅含schema_version、revision、network、node、peers。它不携带管理员源、编译器内部对象或业务授权。公开peer清单不是可达性或隧道健康证明。

摘要是递归按对象key排序的紧凑UTF-8 JSON SHA-256；数组保留语义顺序。POST `.../configuration/ack` 必须匹配该节点当前revision与digest；ACK只表示校验并持久保存，非业务生效。撤销、错凭据、错digest与旧revision拒绝。

业务执行使用独立的稳定 `NodeMeshExecution`，其epoch/instance、能力、同revision/digest、公钥/路径、预算及保存ACK保护见[执行合同](/mesh-execution)。DNS有自己的封闭快照/验证合同，库存或DNS可读不产生业务授权。
