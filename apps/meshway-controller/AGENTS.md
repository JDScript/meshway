# Fresh Controller implementation

本应用按 `docs/v2/` 和新共享库继续实现。已有账户和网络元数据闭环是共同基线；不参考旧 `apps/controller` 的 DTO／存储实现来重新塑造需求。Console 由用户协调，本线程不向 Console 会话发送任务。

agent 协调只使用原生 subagent 任务与结果回执；禁止通过 session.prompt、synthetic 或其他会话写入 API 向主会话／其他 agent／Console 注入消息，避免把 agent 内容显示成用户发言。中途需要协调时正常返回带来源的 checkpoint／阻断，由协调者接收再续接；不改对话历史或会话 parent。已误标 user 的协调回执不是用户新增指令。

## End-to-end work packages

- **Management/configuration** owns `accounts/`、`networks/`、`configuration/`，相关 `store` 子模块、配置输入／规范化／视图／编辑逻辑及 `http/management*`、`http/configuration*`。完成账户恢复、成员和加入链接、配置编辑／预览／CAS／完整版本／回滚／管理活动。已有 `http/auth.rs`、`http/networks.rs` 可随该组扩展。
- **Node/execution** owns `enrollment/`、`execution/`，对应 `store`／`http/agent*`／`http/enrollment*`，共享 Node 认证／执行值类型和 `crates/configuration/src/compile/`／`execution/`。完成所有 Node 接入生命周期、接入密钥与设备授权、完整编译／发布／需求合成／实例／lease／ACK／撤回。
- **State/observations/resources** owns `state/`、`observations/`、`resources/`，相关 `store`／`http/state*`／`http/events*`／`http/observations*`／`http/resources*`，共享报告／观测值与事件 envelope。完成真实报告、可见性裁剪、query／watch／SSE、历史、测量任务和规则集／GeoIP／ASN。

每组交完整可运行候选，包含字段、存储、服务、HTTP 和非浏览器行为测试；不只交类型骨架、模拟服务或未接线的库。修补继续原工作包。声明、saved／applied 与真实包流分别验证，不拿列表或 ACK 宣称隧道成功。

## Shared seams

- Config 源、规范化值、bindings 和 revision 由管理／配置组统一提交。它提供事务内提交入口，供成员移除、Node 接入／删除／身份转换复用；不可用串行的独立事务模拟原子性。
- Node 身份、公钥、实例与执行授权由 Node／执行组验证。State 组只接纳已认证、同实例、相容 publication 的报告；原始报告不自授权，也不声明可信 GeoIP。
- 执行编译按完整可信输入形成快照；并存需求合成完整节点目标，不能 last-demand-wins。跨配置不可假设仅 revision 相等就相容。
- 状态／事件组拥有统一事件序列、快照截点和权限收缩处理。数据库成功提交交给该入口后，再使变更可见；HTTP 取消不遗失已提交结果，事件传输不延长身份或 lease。
- 原始 JSONC、完整规范化值都加密持久化；普通视图脱敏，配置 source 是显式秘密读取。当前 master key／私有目录／执行器取消和关闭边界须保留。
- 当前 `store/authorization.rs` 的会话检查在调用者 Transaction 内复用，网络权限仍需同事务重检。当前 creator 字段不授予永久权限。

## Integration ownership

根 Cargo／pnpm manifests 和 lockfile、生成工具及最终合并由协调者维护。协议新文件按上述领域放入 `crates/protocol/src/`，只定义一份 DTO；JSON／SSE 都从实际路由生成。各组可在自己的 worktree 做必要 `App`、`http/mod.rs`、`lib.rs` 装配以完成测试，须在交付中列明共享文件改动，由协调者统一整合；不得直接编辑他人工作树。

Schema 只有当前初始化形态，不加历史升级／兼容轨。管理组维护当前基础 SQL，Node／执行组使用 `store/schema/execution.sql`，状态／资源组使用 `store/schema/state.sql`；总入口合并为同一 SQLite schema 和事务体系。跨组表／列／调用签名需要一致时集中提交一份接口说明和实际签名，协调者一次收敛，不重新拆成小任务。

只在本工作树指定 target 构建，`CARGO_INCREMENTAL=0`，先检查可用空间。已完成目录在提交整合、验证通过、无 WIP／进程／挂载占用后清理；必要源码／提交／证据保留。
