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

统一入口是 `scripts/dev.py`，管理当前 checkout 的 controller、文档及已初始化 demo；日志和数据在 `.meshway`。服务归属、保留可写容器与更新命令详见[统一开发入口](/dev-services)，首次 Agent 注册见[本地联调](/getting-started)。

## 开发检查

后端与共享契约检查从仓库根目录执行，按改动范围选择：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
pnpm protocol:check
pnpm test:protocol
pnpm test:codegen
pnpm check:experiments
pnpm --filter @meshway/control-client typecheck
pnpm typecheck:apple-scripts
pnpm build:docs
```

修改 Rust 契约/HTTP路由/客户端后先 `pnpm protocol:generate`，再做 drift，并检查 workspace all-features（含 Agent/Apple core）、SDK、Apple scripts 和 `pnpm check:experiments`。后者通过 `scripts/check-experiments.mjs` 自动发现全部 `experiments/*/tsconfig.json`，执行严格 `tsc --noEmit` 与存在的 `*.test.ts`，不维护手写项目清单。`apps/console` 由独立工作流维护 UI 与构建，后端同步生成契约和接口变更。仅文档改动运行 `pnpm build:docs` 与 `git diff --check`，保留 VitePress 死链检查。

按用户要求，浏览器自动化套件、依赖与 CI 入口已移除，不默认重建浏览器测试；console UI 由其工作流手工验证。Controller/Agent 的协议、鉴权和数据面行为测试继续保留。Apple 原生检查使用不依赖浏览器框架的进程与 HTTP helper，入口见[Apple 文档](/apple-app)。

## 注册与持久化验收矩阵

已完成的注册基线验证控制面凭据闭环，后续托管数据面另行验收：

| 行为 | 源证据 |
| --- | --- |
| 单事务消费、并发仅一身份、原绑定幂等恢复、过期 / 撤销不复活、权限分离 | `apps/controller/tests/api/main.rs`、`apps/agent/tests/roundtrip.rs` |
| 私有 0700 / 0600、独占锁、错误 / 损坏状态不覆盖、响应丢失复用身份 | `apps/agent/tests/roundtrip.rs`、controller store 测试 |
| 用户会话、网络权限与设备授权 | `apps/controller/tests/api/accounts/`、`device_login.rs`；页面交互手工验证 |
| 必填可空、未知字段、Unicode 长度、类型化 HTTP 错误与真实往返 | `packages/control-client/test/client.test.ts`、生成 Rust client 测试 |

App 浏览器批准 / PKCE 兑换、原绑定恢复与迁移见[设备登录](/device-login)，Controller/Apple core 保留真实接口行为测试；浏览器登录与批准界面手工验证。

## 配置与路由切片

IPv4 Mesh使用单一当前policy、grants并集与有序first-match。CAS/epoch/同revision发布和正常TUN验收见[执行合同](/mesh-execution)，完整出口尚待接线。

### 新切片验收清单

| 行为 | 源证据 |
| --- | --- |
| 当前 schema 原子初始化、损坏状态拒绝、稳定地址、容量失败回滚、revision / CAS / ACK | `apps/controller/src/store/schema/tests.rs`、`store/tests.rs`、`apps/controller/tests/api/policy.rs` |
| 执行快照跨网隔离、凭据类别、epoch 与撤销、保存回执 | `apps/controller/tests/api/mesh_execution.rs`、`apps/agent/src/state/connections/dataplane/tests/` |
| schema / digest / 绑定校验、旧版本拒绝、缓存与回执恢复 | `apps/agent/tests/roundtrip.rs` |
| 文件发布前失败与 `CommittedNotDurable`、重试 / 重启仍须 fsync 后 ACK | `crates/protocol/src/local/write.rs`、`apps/agent/src/state/durability_tests.rs`、Agent roundtrip 测试 |
| suffix 标签边界、CIDR、非法 / 映射地址、池内归属、循环与必经节点约束 | `crates/policy/src/tests.rs`、Controller API 行为测试 |

测试文件中的具体用例是证据入口，不维护会随新增测试过期的总数。当前接口与容量以[协议](/protocol)和[配置分发](/configuration)为准。

## 数据面与实验

| 切片 | 验收入口与结论 |
| --- | --- |
| 托管邻接 | `experiments/managed-connections/run.py`：真实注册、公钥认证、反向 TLS-VLESS、撤回与 lease，见[运行层](/connection-runtime) |
| 托管 IPv4 | `experiments/p3a-mesh/run.py`：隔离 no-bypass、真实 ICMP / HTTP、授权 / 必经节点 / lease / 撤销，见[数据面](/managed-dataplane) |
| IPv4 Mesh | `experiments/p3a-mesh/run.py`：正常Controller/Agent/TUN、grants、有序路径、重启、撤回、HTTP stall/lease |
| 系统保护 | `apps/agent/tests/underlay_fixture/main_loop.py`：正常 Controller/Agent/TUN 生命周期；`run.py` / `review.py` 复用同源镜像验证 socket 与 journal 恢复 |
| 跨地域 | `experiments/linux-field`：固定同身份恢复已复验，自动组网/打洞另验，见[远端边界](/linux-field-validation) |
| 节点 DNS / resolver | `experiments/managed-dns/run.py`，加 `--system-resolver` 验 glibc 与恢复，见[DNS](/dns) |
| Netcheck / carrier 观测 | `experiments/netcheck`、`experiments/managed-discovery`：kernel NAT 与真实 UDP FD / 来源 / 期限，见[Netcheck](/netcheck)、[endpoint 观测](/endpoint-discovery) |
| Transport / 性能 | `experiments/transport`、`experiments/tunnel-throughput`：方法与非秘密 artifact 由目录 README 维护，见[实验](/transport-lab) |

各 runner 的非秘密结果文件记录镜像、资源、负载、运行 ID 与清理结果；私有配置、凭据与密钥不进入 Git。旧运行 artifact 是历史证据，重现用当前 runner 和其 README，不依赖本机临时目录仍存在。

网络实验限定独立Docker namespace，通过同一 `scripts/performance_lock.py`（含继承FD）串行；不改宿主路由/DNS/防火墙/daemon，不用host network/privileged，构建器也遵守。独立实验身份与生产身份分开。不同feature/CLI与SDK原生构建不得并行共用Cargo target，以免替换被测二进制。远端固定链路不代表自动打洞，不用城市标签冒充实测位置。
