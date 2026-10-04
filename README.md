# Meshway

Rust Server、React Console 与节点网络架构。以 Tailscale 为常规产品基线，扩展可插拔承载及按域名／IP 的有序路径路由。

## 文档与实现

[开发者文档](docs/v2/index.md) 是当前设计与使用入口；HTTP 的精确合同由 Rust 类型／实际路由生成，并随文档发布。

- **Server**：账户、网络隔离、配置版本／CAS、持钥节点接入、执行发布、State／SSE、历史、规则集和观察任务。
- **Console**：纯 SPA，消费生成 SDK；支持上述已交付管理与观测流程。
- **Node**：`meshwayd`／`meshway` 已接通持钥接入、认证、实例／lease、目标保存及状态上报；TUN／SOCKS5、插件与实际包流仍待实现。

旧节点／Apple 实现和实验已移出活跃源码树。历史参考保留在 `archive/pre-consolidation-main-007e9deb`、`archive/node-runtime-8da83a7f`、`archive/apple-007e9deb`；复用须逐项审查，不把历史实现当成新合同。

## 目录

| 目录 | 职责 |
| --- | --- |
| `apps/meshway-server` | Server 应用与私有领域／存储模块 |
| `apps/meshway-daemon`／`apps/meshway-cli` | Node daemon `meshwayd` 与本机 CLI `meshway` |
| `apps/meshway-web-console` | Web Console（`@meshway/web-console`） |
| `crates/meshway-*` | 共享契约／私有存储原语，以及 Node control 组件；职责见架构文档 |
| `packages/protocol` | 生成 OpenAPI |
| `packages/control-client` | 生成 TypeScript SDK 与运行时校验 |
| `docs` | 当前文档、字段参考与构建 |
| `scripts/protocol` | OpenAPI／客户端生成器及其回归 |
| `scripts/dev.py` | 持久开发服务的进程／端口归属与生命周期 |
| `docs/scripts` | 渲染页面链接、锚点和合同示例校验 |

Rust 包目录与 Cargo 包名相同。完整职责及 Node 目标结构见[架构](docs/v2/architecture.md)。

## 本地开发

工具链与 workspace 任务由 [mise](https://mise.jdx.dev/) 管理。`mise.toml` 固定 Rust、Node.js、pnpm 和 Python 的完整版本；`mise.lock` 保存 macOS arm64／Linux x64 下载信息，Rust 组件由 rustup 安装。持久开发服务另需系统提供 tmux／lsof。

首次进入仓库：

```sh
mise trust
mise install --locked rust node pnpm python
mise run setup
```

`mise run`／`mise exec` 在未激活 shell 时也使用固定工具链。编辑器和直接运行 `cargo` 的终端应配置 `mise activate` 或 mise shims；Rust 版本统一在 `mise.toml` 定义。Cargo／pnpm lockfile 固定依赖版本，升级 pnpm 时同时更新 `package.json` 的 `packageManager`。

前台启动 Server：

```sh
install -d -m 700 .meshway
mise exec -- cargo run -p meshway-server --bin meshway-server --locked -- \
  --data-dir "$PWD/.meshway/server" --listen 127.0.0.1:8420
```

另开终端运行 Console，显式指定 Server，避免连接其他实例：

```sh
MESHWAY_CONTROLLER_URL=http://127.0.0.1:8420 mise run dev:console
mise run dev:docs
```

文档位于 `http://localhost:5173/v2/`。持久开发服务由 `mise run services <action> [service]` 管理：action 为 `start`／`status`／`stop`／`restart`／`update`，service 为 `server`／`docs`／`all`。例如 `mise run services start docs`。它核验进程／目录／端口归属，不接管其他实例。源码合并不自动迁移数据库或切换已运行服务。

## 检查

Node 的构建、启动、批准接入和本机命令见 [Node 运行](docs/v2/node/runtime.md#启动与接入)。

```sh
mise tasks
mise run build
mise run check
```

`check` 与 CI 使用同一任务，串行完成 Rust fmt／Clippy／测试、生成 drift、Rust／TS SDK 消费、Console 检查／测试／构建和文档检查。可单独运行 `mise run check:console`、`mise run check:docs` 或 `mise run generate:protocol`；完整入口与说明见 `mise tasks`。

Rust 测试默认同时运行 4 个用例，可用 `RUST_TEST_THREADS=8 mise run test:rust` 调整。用例内部的 Tokio 并发单独控制；共享进程资源由对应测试的局部锁协调。

构建默认使用当前 checkout 的 `target/`、`CARGO_INCREMENTAL=0`；独立任务勿以不同 features 并发写入同一 target。大型构建前确认至少 50 GiB 可用空间。

Console UI 人工验证，不维护浏览器自动化套件。配置保存、应用回执、节点联系与真实包流分别验收；当前检查不声明新 Node 或真实 TUN/userspace 网络已交付。
