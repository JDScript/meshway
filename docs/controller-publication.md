# Controller publication 前置切片

本切片接入现有 Controller 正常发布链路，保持 source v2 与既有节点 wire 合同。
它不是四层策略 v3 上线，也不代表自动组网、出口或真实 Linux TUN 闭环完成。

## 源 → 编译 → 节点 DTO

实现位于 `apps/controller/src/store/publication.rs` 及其子模块：

1. **Facts / RoutingFacts**：在调用持有的 Store 锁和 SQLite 事务内，按显式
   `NetworkId` 读取 revision、地址网络、活动节点、路由、Access、tag 目录 / 分配及
   不可变公钥绑定。无跨请求缓存，无锁外编译。连接源及 managed listener 的公共信息
   由 `Adjacency` 按需读取一次，复用既有连接校验器。
2. **compile**：一次 `prepare_routing` 的报告用于配置发布和 Access 联合编译。
   只对有规范公钥且获 Access 允许的有向端点对评估路由；全网精确路径结果保留在
   本次 `Publication` 中。缺少密钥 / 路由无效时不产生授权路径；Access 无效时保持
   `access_valid=false` 与空授权。路径长度、每节点路径数、邻接与路径身份并集的
   上限仍是硬失败，未截断、未绕过必经节点。
3. **project**：编译器内部报告 / 路径转换为 protocol 权威定义的
   `ConfigurationSnapshot`、`NodeDataplane` / `AgentDataplaneSnapshot`；不复制 DTO。
   配置与连接 revision 须一致，身份来自同事务事实。每节点公共连接投影只生成一次，
   同时服务于身份并集预算、连接预算与 dataplane 投影。
4. **budget**：配置 1 MiB（预留 32 KiB 诊断）、连接 1 MiB（预留 4096 字节，
   加既有私有材料预算）门槛集中在 publication；完整 dataplane GET 仍检查 2 MiB。
   DNS 继续使用既有校验 / 序列化预算，但复用 publication 的活动节点，不再重复读库存。

`configuration::snapshot` 也通过 `RoutingFacts` 与同一配置转换发布；单独配置读取
不强制读取无关 Access / 连接源。`agent_dataplane` 在原 Deferred 事务内使用
`Publication::read(...).dataplane(node_id)`。managed dataplane 继续在其原有材料边界
封装该结果；material secret、解密密钥及 material DTO 不存入 publication 公共事实。

## 写入口依赖闭包

各入口保持原有请求校验、鉴权时钟、事务类型、CAS、revision bump / commit 位置，
在写入后调用唯一的 `finish_change(db, scope, ChangeKind)`。
联合编译与路径 / 身份并集预算对每个 change kind 都执行；下表列出额外发布门槛。

| ChangeKind | 写入口 | 配置预算 | 连接含材料预算 | DNS 预算 |
| --- | --- | --- | --- | --- |
| RoutingOrSelectors | save_policy、set_country、save_tags、set_node_tags | 是 | 否 | 否 |
| Adjacency | save_mesh_access、save_connections、publish_connection_identity、publish_managed_capabilities、save_managed_listener | 是 | 是 | 否 |
| NetworkOrEnrollment | save_network、首次 enroll（含初始 listener 与归属） | 是 | 是 | 是 |
| NodeWithdrawal | revoke | 是 | 是 | 否 |
| MembershipWithdrawal | ownership::withdraw_user（含已撤回孤立 tag 修复） | 是 | 是 | 是 |

这些 kind 显式保留已有依赖闭包，不把全部写入口变成“所有源必须有效”的门槛。
例如 deny-all 下路由修复不强制解码无关的损坏连接源；有执行路径时仍按原逻辑读取邻接，
检查身份并集。撤回仍可使源引用失效，向剩余节点发布 fail-closed 配置 / 邻接 / 授权，
不要求先删除源中的被撤回节点。来源合法性检查仍由原入口决定。

幂等接入重试、无变化保存、心跳、ACK、运行观测不触发新增发布编译。
归属 claim 目前不参与 selector / 执行语义，其既有 revision bump 未新增发布门槛。
所有预算失败仍在 commit 前发生，事务整体回滚；地址、token 消耗、不可变绑定、
JSONC 注释与修订号均保持现有行为。没有 schema 或数据迁移，没有 HTTP / DTO 改动。

## 调用内减少的工作

此前一组写后门槛分别编译配置与授权的路由，并可重复读取 / 校验连接源、重复生成
参与路径节点的连接投影。现在一次发布上下文共享一份路由准备报告、活动库存、
公钥及每节点连接投影。dataplane GET 的配置与授权也共享同次路由准备与公钥读取。
管理保存的输入预验证及最终管理响应仍按既有逻辑执行，不宣称整个 HTTP 请求所有
管理诊断只有一次编译。每次新 GET 仍重新编译全网授权，未引入跨请求缓存。

## 后续 v3 接线落点

- `Facts`：读取未来正式存储的四层源与 compiler 所需网络事实。
- `compile`：调用 policy v3 纯编译器，持有有界内部结果；不让 protocol 依赖 policy。
- `project`：把内部索引 / 路径池 / 前缀表转换为另行审定的版本化节点 wire DTO。
- `budget` 与 `ChangeKind`：明确新源会影响哪些写入口，拒绝不可发布的完整依赖闭包。
- Agent 保留对 wire、精确路径交集及本机资源的防御校验；本切片未改 Agent。

正式 v3 源存储、HTTP、版本化执行合同、系统保护通路、自动邻接和出口 / profile
转换仍需后续接线；当前不序列化 policy v3 的内部工作对象作为 wire。

## 验证

独立 target，避免与其它 feature / 原生任务共用构建产物：

```sh
cargo fmt --check -p meshway-controller
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target cargo test -p meshway-controller --all-features --locked
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target cargo clippy -p meshway-controller --all-targets --all-features --locked -- -D warnings
```

publication 行为测试在一次事务中将同一编译结果投影给三个节点，并运行预算门槛：
实际路由准备调用计数为 1、三对端点的有向评估共 6 次、公共连接投影共 3 次，
预算及后续快照不增加计数；正常 Access 保存入口的 publication 编译计数也为 1。
结果与正常凭据 GET 逐字节一致。另测路由修复与邻接修改的门槛区别。
既有 Store / HTTP 测试覆盖网络隔离、CAS、JSONC、地址 / 绑定迁移保留、预算失败回滚、
撤回后的 fail-closed 快照及 managed 材料预算。
