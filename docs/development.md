# 开发环境与验收

本页维护检查入口和可复现证据；产品目标见[概念](/concepts)，排期见[里程碑](/milestones)。历史验收只证明对应切片，不代表当前 checkout 已重新执行全部检查。

## 环境与服务

仓库固定 Rust 1.98.0（`rust-toolchain.toml`）、Node.js 24 与 pnpm 11.24.0（根 `package.json`）。凭据存储当前要求 Unix 私有目录 / 文件权限。

```sh
pnpm install --frozen-lockfile
pnpm dev
pnpm dev:status
pnpm dev:update
```

统一入口是 `scripts/dev.py`，管理当前 checkout 的 controller、Web、文档及已初始化 demo；日志和数据在 `.meshway`。服务归属、保留可写容器与更新命令详见[统一开发入口](/dev-services)，首次 Agent 注册见[本地联调](/getting-started)。

## 开发检查

从仓库根目录执行，按改动范围选择：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
pnpm protocol:check
pnpm test:protocol
pnpm test:web
pnpm typecheck
pnpm lint
pnpm build
pnpm test:e2e
```

修改 Rust 契约后先 `pnpm protocol:generate`，再做 drift check；生成不能代替比较。仅文档改动运行 `pnpm --filter @meshway/docs build`（等价 `pnpm build:docs`），保留 VitePress 默认死链检查。完整类型检查包含根 E2E TypeScript。

首次浏览器测试需 `pnpm exec playwright install chromium`。`pnpm test:e2e` 构建 Rust（Agent transport-lab feature）与生产 Web，再用独立临时数据和端口执行 Playwright。`MESHWAY_E2E_TMPDIR` 可指定临时目录父路径；测试 feature 不自动开启 TUN。Apple 专用检查由[Apple 文档](/apple-app)维护。

## 注册与持久化验收矩阵

已完成的注册基线验证控制面凭据闭环，后续托管数据面另行验收：

| 行为 | 源证据 |
| --- | --- |
| 单事务消费、并发仅一身份、原绑定幂等恢复、过期 / 撤销不复活、权限分离 | `apps/controller/tests/api.rs`、`apps/agent/tests/roundtrip.rs` |
| 私有 0700 / 0600、独占锁、错误 / 损坏状态不覆盖、响应丢失复用身份 | `apps/agent/tests/roundtrip.rs`、controller store 测试 |
| Web 登录 / 登出、匿名 action 重放拒绝、真实注册与撤销、故障显示 | `e2e/control-plane.spec.ts` |
| 必填可空、未知字段、Unicode 长度、类型化 HTTP 错误与真实往返 | `packages/control-client/test/client.test.ts`、生成 Rust client 测试 |

App 浏览器批准 / PKCE 兑换、原绑定恢复与 v8 迁移见[设备登录](/device-login)及 `e2e/device-login.spec.ts`；现有管理员批准不代表本地用户账户体系完成。

## 配置与路由切片

旧版路由 / 端点授权已贯通配置、生成客户端和管理编辑；first-match allow/reject、初始 routing reject 是**待迁移现状**，目标为[四层模型](/concepts#四层策略)。控制意图验收不代替 TUN 或真实访问。

### 新切片验收清单

| 行为 | 源证据 |
| --- | --- |
| v1 地址升级保留身份、稳定地址、容量失败回滚、revision / CAS / ACK | `apps/controller/src/store/tests.rs`、`apps/controller/tests/api.rs` |
| schema / digest / 绑定校验、旧版本拒绝、缓存与回执恢复 | `apps/agent/tests/roundtrip.rs` |
| 文件发布前失败与 `CommittedNotDurable`、重试 / 重启仍须 fsync 后 ACK | `crates/protocol/src/local/write.rs`、`apps/agent/src/state/durability_tests.rs`、Agent roundtrip 测试 |
| suffix 标签边界、CIDR、非法 / 映射地址、池内归属、循环与必经节点约束 | `crates/policy/src/tests.rs`、`e2e/routing.spec.ts` |
| 图形 / JSONC 共用草稿、注释保存、409 保留原基准、匿名 mutation 拒绝 | `e2e/access.spec.ts`、`e2e/routing.spec.ts`、`e2e/connections.spec.ts` |

测试文件中的具体用例是证据入口，不维护会随新增测试过期的总数。当前接口与容量以[协议](/protocol)和[配置分发](/configuration)为准。

## 数据面与实验

| 切片 | 验收入口与结论 |
| --- | --- |
| 托管邻接 | `experiments/managed-connections/run.py`：真实注册、公钥认证、反向 TLS-VLESS、撤回与 lease，见[运行层](/connection-runtime) |
| 托管 IPv4 | `experiments/managed-dataplane/run.py`：隔离 no-bypass、真实 ICMP / HTTP、授权 / 必经节点 / lease / 撤销，见[数据面](/managed-dataplane) |
| 节点 DNS / resolver | `experiments/managed-dns/run.py`，加 `--system-resolver` 验 glibc 与恢复，见[DNS](/dns) |
| Netcheck / carrier 观测 | `experiments/netcheck`、`experiments/managed-discovery`：kernel NAT 与真实 UDP FD / 来源 / 期限，见[Netcheck](/netcheck)、[endpoint 观测](/endpoint-discovery) |
| Transport / 性能 | `experiments/transport`、`experiments/tunnel-throughput`：方法与非秘密 artifact 由目录 README 维护，见[实验](/transport-lab) |

各 runner 的非秘密结果文件记录镜像、资源、负载、运行 ID 与清理结果；私有配置、凭据与密钥不进入 Git。旧运行 artifact 是历史证据，重现用当前 runner 和其 README，不依赖本机临时目录仍存在。

网络实验限定独立 Docker namespace，共享 performance lock 串行运行；不改宿主路由、DNS、防火墙或 Docker daemon，不使用 host network / privileged。独立 transport 实验身份与生产注册身份分开。远端联测留待 TUN / 打洞完善，不用本机城市标签冒充跨区域结果。
