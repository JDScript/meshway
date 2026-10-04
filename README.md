# Meshway

Rust Server、React Console 与节点网络架构。以 Tailscale 为常规产品基线，扩展可插拔承载及按域名／IP 的有序路径路由。

## 文档与实现

[开发者文档](docs/v2/index.md) 是当前设计与使用入口；HTTP 的精确合同由 Rust 类型／实际路由生成，并随文档发布。

- **Server**：账户、网络隔离、配置版本／CAS、持钥节点接入、执行发布、State／SSE、历史、规则集和观察任务。
- **Console**：纯 SPA，消费生成 SDK；支持上述已交付管理与观测流程。
- **Node**：新 `meshwayd`／`meshway` 与启动时插件架构处于设计阶段，实际网络运行尚未接入。

旧节点／Apple 实现和实验已移出活跃源码树。历史参考保留在 `archive/pre-consolidation-main-007e9deb`、`archive/node-runtime-8da83a7f`、`archive/apple-007e9deb`；复用须逐项审查，不把历史实现当成新合同。

## 目录

| 目录 | 职责 |
| --- | --- |
| `apps/meshway-server` | Server 应用与私有领域／存储模块 |
| `apps/console` | Console |
| `crates/meshway-*` | model、configuration、transport-spec、protocol、control-client、serde |
| `packages/protocol` | 生成 OpenAPI |
| `packages/control-client` | 生成 TypeScript SDK 与运行时校验 |
| `docs` | 当前文档、字段参考与构建 |
| `scripts` | 生成、开发服务与检查 |

Rust 包目录与 Cargo 包名相同。完整职责及 Node 目标结构见[架构](docs/v2/architecture.md)。

## 本地开发

需要 Rust 1.98、Node.js 24、pnpm 11.24、Python 3；持久开发服务另需 tmux／lsof。

```sh
pnpm install --frozen-lockfile
install -d -m 700 .meshway
cargo run -p meshway-server --bin meshway-server --locked -- \
  --data-dir "$PWD/.meshway/server" --listen 127.0.0.1:8420
```

另开终端运行 Console，显式指定 Server，避免连接其他实例：

```sh
MESHWAY_CONTROLLER_URL=http://127.0.0.1:8420 pnpm dev:console
pnpm dev:docs
```

文档位于 `http://localhost:5173/v2/`。也可用 `python3 scripts/dev.py start|status|stop|restart|update server|docs|all` 管理本 checkout 的持久服务；它核验进程／目录／端口归属，不接管其他实例。源码合并不自动迁移数据库或切换已运行服务。

## 检查

```sh
cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --all-targets --all-features --locked -- --test-threads=1
pnpm protocol:check
pnpm test:codegen
cargo build -p meshway-server --bin meshway-server --locked
pnpm test:protocol
pnpm --filter @meshway/console check
pnpm test:console
pnpm build:console
pnpm test:scripts
pnpm check:docs
```

Console UI 人工验证，不维护浏览器自动化套件。配置保存、应用回执、节点联系与真实包流分别验收；当前检查不声明新 Node 或真实 TUN/userspace 网络已交付。
