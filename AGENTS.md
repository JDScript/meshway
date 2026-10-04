# Meshway

## 当前边界

- `apps/meshway-server`：Rust Server；`apps/meshway-web-console`：pure SPA（`@meshway/web-console`）。
- `crates/meshway-*`：当前共享模型、配置／编译、协议、生成客户端与严格序列化。
- `packages/`：生成 OpenAPI／TypeScript SDK；`docs/v2/`：当前唯一开发者规范与使用入口。
- Rust 包根目录和 Cargo 包名统一 `meshway-xxx`。现行 HTTP 根仍是 `/api/v1`／`/agent-api/v1`，目录整理不自行改 wire。
- 新 Node 尚未实现。设计为 `meshwayd`＋`meshway` 本机 CLI，up/down/ping/status 与网络选择，业务配置在 Server 管理；支持无 TUN userspace 与系统 TUN，启动时加载随发行包整体安装的插件。具体 ABI／入口按文档定版后实施。
- 旧 Node、Apple、运行库与实验退出活跃树，保存在冻结 Git 分支及必要证据中。新实现只按明确白名单复用，不恢复长期 legacy 目录或旧 HTTP 兼容轨。Meshing 项目不是本项目依据。
- 冻结参考：`archive/pre-consolidation-main-007e9deb`、`archive/node-runtime-8da83a7f`、`archive/apple-007e9deb`。未完成配置补全与旧 review WIP 保留在其原工作树，不当成已交付功能合入。

## 信息来源与文档

- 产品、领域边界、字段和真实未完成项维护在 `docs/v2/`；每个主题只有一个定义位置，其他页面引用它。
- 文档采用对外参考格式：结构、字段、行为、示例、限制。移除逐轮讨论、修复日志、过时 handoff 和重复 dump；追溯使用 Git 与私有原始证据。
- HTTP 类型和实际路由生成 OpenAPI／SDK，文档原样发布同一契约。不能把未定设计写成可用 API，不能以文档描述替代真实行为验收。
- 新产品语义不确定时使用 question 集中询问。构建缓存、进度或测试数量不代表业务功能完成。

## 类型、权限与数据完整性

- Server↔Node／Console 使用生成客户端；外部响应使用生成运行时 schema。禁止 `any`、未验证断言或宽 JSON 掩盖协议不一致。
- 修改协议／路由后重生成并通过 drift，不手改生成文件。可省略、null、未知、过期和不适用分别表达。
- 逐请求／最终事务检查真实主体与网络权限；类型化 NetworkId 不代替授权。Node 凭据不能作为用户管理凭据。
- 配置、身份／材料绑定和执行发布保持一致；CAS、永久吊销、成员生命周期、lease、取消与回执排序不可削弱。普通启动不因数据库错误清库。
- 配置原文／规范值和必要秘密私有保存，公开视图脱敏；密码安全哈希，私有文件原子持久化，凭据不进普通日志。
- State、saved、applied、认证通道和真实包流分别验收。缺测不填零，重放／读取不制造历史样本；状态不会自授权限或延长 lease。
- 新 Node 保留端到端保护与原完整路径回复，承载切换不绕必经点；打洞、反向拨号、中继与真正直连分别证明。TCP/VLESS 打洞不因优先 UDP 而被排除。

## 源码组织

- App 做装配和用例协调，库按真实职责和消费者划分，不按表／临时阶段机械拆包，不让库反向依赖 App。
- 单文件模块 `foo.rs`；有子模块用 `foo/mod.rs`。私有单测为所属模块的 cfg(test) 子模块；黑盒测试遵守 Cargo `tests/*.rs` 或 `tests/<suite>/main.rs`，共享 helper 放子目录。
- 不为测试扩大生产可见性、复制实现或用临时 path/autotests 配置绕过边界。生成／vendor 由其权威来源维护。
- Rust 业务代码 forbid unsafe；确需 native/FFI 的桥接隔离审计，不静默放开应用 lint 或 TLS 默认 features。
- 取消后的已接纳事务仍由数据库 owner 完成；异步生产者有明确停止／join，缓冲和任务有界。

## 验证与环境

- 工具链版本和 workspace 任务统一在 `mise.toml`，下载锁定在 `mise.lock`；使用 `mise run`／`mise exec`，本地与 CI 共用 `mise run check`。包内脚本由所属包维护；根目录不另维护一套 pnpm 任务别名。
- Rust：fmt、严格 Clippy、必要行为测试；协议改动还检查 workspace/all-features、drift、Rust／TS SDK 消费者。当前 workspace 是 Server＋共享契约，不声称 Node／Apple 已接通。
- Console：严格 TypeScript、格式／lint、领域测试和构建；UI 人工验证，不维护浏览器自动化套件。
- 文档：`mise run check:docs` 检查构建、链接／锚点、公开 OpenAPI 一致性及标注完整示例；外部链接按需额外核对。
- 运行服务、身份数据与源码合并分开处理。先核对实际进程／接口；不把重新编译当作部署，不覆盖另一工作流账户／配置。
- 不改未知归属的宿主路由／DNS／防火墙／Docker daemon，不用 host network 或 privileged（含构建器）。测试资源隔离并可精确清理。
- 大型构建前检查磁盘，低于 50 GiB 先处理已结束且无进程／挂载引用的缓存。`CARGO_INCREMENTAL=0`，相同任务复用指定 target；不同 feature 的可执行测试不并发共用 target。不全量 prune 未知 volumes。

## 协作

- 并行工作明确文件所有权，manifest／lock／生成链及最终整合由协调者维护。跨组件行为一起验证，不把类型、存储、HTTP 分别当业务交付。
- agent 沟通只用原生 subagent 回执并标明来源；禁止 session.prompt／synthetic／会话写入 API 将协调消息伪装成 user，不改已有历史。
- Console 交接由用户转达，本线程不主动联系 Console 会话。子任务默认使用用户指定的 `openai/gpt-6-astra-ultrafast`。
- 工作树统一放仓库 `.worktree/`。完成、整合、验证且无 WIP／进程／挂载引用后回收，保留分支和必要证据。遇到未知变化先调查，不重置或删除。
- 独立完成的工作形成清楚提交；主线合并前核对源码、生成物、文档和实际运行命令一致。
