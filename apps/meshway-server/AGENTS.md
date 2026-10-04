# Meshway Server

Server 按 `docs/v2/` 和当前共享契约实现；不以归档 Controller 的 DTO／SQL 重新定义产品。Console 独立于 Server，业务视图使用生成 API。

## 领域

- `accounts/`、`networks/`、`configuration/`：账户／成员、完整配置、材料、编辑、CAS、历史。
- `enrollment/`、`execution/`：持钥接入、实例／lease、需求合成、发布、回执、撤回。
- `state/`、`observations/`、`resources/`：真实报告、权限裁剪、SSE、历史、任务、规则集及可信输入。
- `http/`：实际路由、抽取与类型化响应。`store/`：私有事务／SQL。共享公开合同位于 `crates/meshway-protocol`。

## 提交边界

- 配置的 source、规范值、bindings 和 revision 由唯一事务入口提交；接入／删除／成员生命周期复用它，不串接独立事务伪装原子。
- 身份、公钥、实例和报告授权在同一有效上下文检查；多个需求合成完整目标，不 last-demand-wins，不仅比较源 revision。
- 已接纳 SQLite job 的提交后结果进入统一 journal；HTTP 取消不丢结果。缓存与订阅不代替当前权限检查。
- 模块回归保留网络隔离、秘密、所有权、撤回、重试、lease 和进程退出语义。禁止靠假 Node／假执行载荷证明闭环。
- schema 只有当前初始化形态与完整性检查；不加历史兼容轨，不静默清库。

## 工程

遵循根 AGENTS 的模块／测试、工具、原生协调和缓存规则。根 workspace、lockfile、生成工具和最终消费者适配由协调者维护；领域任务交完整可验证候选，列明共享装配变化。
