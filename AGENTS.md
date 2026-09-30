# Meshway

Rust 控制面与 agent、Next.js + shadcn/ui 控制台、VitePress 文档站。

## 项目边界

- `apps/controller`：控制面服务。`apps/agent`：节点进程。
- `crates/`：共享 Rust 协议基础，保持 crate 数量克制；Agent 生成客户端位于 `apps/agent/src/generated`。
- `apps/web`：Next.js 控制台。`packages/`：生成的前端协议与客户端。
- `docs`：VitePress 产品设计、架构和开发文档。
- `references`：独立的上游参考 checkout；研究版本见其 README，默认不修改。
- 旧 Meshing 项目已废弃，不将其代码、记忆或合同作为本项目依据。

## 协议与类型安全

- Rust 契约及实际路由声明是 HTTP API 的权威来源，生成 OpenAPI 和客户端。
- Controller ↔ agent、controller ↔ web 均使用生成接口，不手工复制 DTO 或客户端 endpoint。
- Web 对外部响应使用生成的运行时 schema 校验；TypeScript 类型声明不能代替校验。
- 禁止通过 `any`、未验证的类型断言、宽泛 JSON 值掩盖协议不一致。
- 修改协议必须同时重生成产物并通过 drift check；不要直接编辑生成文件。
- WASM 可用于后续共享策略逻辑，不是 HTTP 通信的必需依赖。

## 实现与验证

- 前后端按契约推进真实纵向功能；空状态和错误状态必须真实，不伪造节点、流量、健康状态。
- 将提案、用户已确认需求和已实现功能明确区分。控制面注册和心跳不代表隧道已建立。
- 管理员、注册令牌、节点凭据的权限分开；Web 的每个管理 mutation 都必须在服务端检查登录身份。
- 注册令牌与节点身份在同一事务内绑定；幂等恢复不能换身份或恢复已撤销节点。本地凭据原子、私有地保存，不输出到普通日志或 Web 客户端。
- 端到端加密优先；逐跳可信路由是显式的架构选择，不允许因连接失败静默降低加密语义。
- Multi-transport 切换与逻辑路径变更是不同操作，不得为了可达性偷偷绕过必经节点。
- 建连方向与业务方向独立：下一跳可以主动 dial 当前节点，已认证的双向通道供两个方向复用；通道双向不代表访问授权双向。
- 节点能力、监听器、可达 endpoint、主动连接策略与中继 / 出口职责分别建模。代理 UUID / password 不替代 Meshway 节点身份。
- 用户已确认先推进 Linux 传输实验；协议按编译期插件边界扩展，批量与 buffer 所有权属于公共接口。实验密钥、授权和网络必须与现有控制面身份及运行数据隔离。
- 用户已确认首版 multi-transport 采用优先级主备；按流负载分担和同流多路径聚合不属于首版要求。
- 用户已确认首版域名匹配仅支持 suffix，按 DNS 标签边界匹配；路由规则有序 first-match，默认动作可选 direct/reject，初始空策略为 reject。
- 配置 ACK 只表示 Agent 已校验并持久保存，不等同于数据面生效；逻辑路径预览不等同于真实 trace 或访问授权。
- Rust 使用 `cargo fmt`、Clippy 和必要的行为测试；JS 使用严格 TypeScript 与各包检查。
- 并行任务明确文件所有权；根 workspace 配置、lockfile 和整体验证由协调者维护。
- 独立完成的工作形成清晰提交。提交前检查生成产物、文档和实际运行命令一致。
