# 账户 + 冻结 P2a 的组合候选

仅 `integration/account-p2a` / `.worktree/account-p2a-integration`，不修改 main 或作者活跃 worktree。
基于 P2a `56da5ad`，叠加账户四个功能提交 `aa88d09`、`93710f2`、`34c7b2b`、`b73b753`。
账户稳定生成契约与完整 handoff 在原分支 `feat/account-integration` 的 `7ceb936`。

## 最小中心冲突

1. `lib.rs::contract_router` 同时保留 `.merge(managed_transports_api::routes())` 和 `.merge(accounts_api::routes())`。
2. `store.rs::enroll_bound` 同一注册根事务内同时保留 P2a `managed_transports::enroll_initial(...)` 和账户 `ownership::assign(...)`，然后共用原 revision / publication / commit。任何失败不消费令牌、不占用地址、不创建半个 node / material / ownership。

除此之外源代码 cherry-pick 自动合并。`18c4313` 的 Rust secret Debug adapter、P2a `transport-material.key`、根 Cargo manifests/lockfile、生成器与 agent/dataplane 实现没有账户改动。

## 必要权限接线

- `managed_transports_api.rs` 三个管理入口改用 `network_access`：`getManagedListeners=Read`，`saveManagedListener/createManagedEnrollmentToken=Manage`。
- `store/managed_transports.rs` 仅 `create_managed_token` 与 `save_managed_listener_inner` 的事务创建点改为可嵌套 business savepoint，根权限事务持有直到实际写入与最终 commit。材料生成、加密、角色投影和导入逻辑保留冻结实现。
- 三个 node-only managed API 仍使用原节点认证；用户 session、网络 Owner 和 bootstrap 不能读取 UUID / 私钥或发布节点编译能力。
- 账户独立 schema fixture 的“版本不变”断言在组合副本比较安装前后版本；原分支实际 v9 fixture 已完整通过。组合副本已继承 P2a v10，不在账户 hook 写版本。正式账户后续版本与启动 hook 安装位置仍由协调者决定。

## 已有针对性 HTTP 验收

`accounts::managed::managed_permissions_initial_enrollment_legacy_claim_and_membership_withdrawal_are_coherent` 使用独立 private tempfile 与真实 API：

- Member 可读 managed 公有 metadata，不能写 listener 或创建 managed token；其他网络 Owner 不能跨网读写。
- Owner 可以在根权限事务保存真实 WS/TLS listener，生成真实加密材料；用户 / bootstrap 均不能进入 node-only 材料或能力接口。
- 首次 managed enrollment listener/material 同事务；注入材料安装错误全部 rollback，重试分配原预期 IP；精确 enrollment retry 不重生成材料。
- legacy-unassigned 明确认领不改变 node ID、credential hash、公钥、双栈地址、tags、listener owner / settings / material version / 指纹 / nonce / 密文或 transport key。
- 直接 dialer 取得同获准材料，私钥为 null；membership 撤回时同事务撤回用户所属 App 与认领节点，并推进 revision。
- 注入撤回错误保持会员 / 节点状态；成功撤回后原 node-only endpoints 与精确 App retry 均拒绝，仍活动 dialer 的 managed-connections / managed-dataplane 材料投影为空，源逻辑路径保持并报告失效，不换路径。
- 公有 `compiled_capability_available=false`，保留归档材料；不修改 P2a UUID / key identity，不自动把 tag 变为身份。

生成、drift、SDK **14** tests、Web **145** tests、Web/SDK/e2e typecheck、5个 experiments TypeScript、fmt及Controller/Apple core全target/allfeature Clippy均通过。

组合副本最终执行 `CARGO_TARGET_DIR=target cargo test --workspace --all-features --locked -- --test-threads=2`：**494 passed / 0 failed / 7 ignored**（含1项doctest；显式外部代理/Docker/性能专项未执行）。其中Controller **73 单元 + 30 HTTP**，Apple core真实Controller用户归属接入与生成客户端secret Debug回归通过。原账户独立分支同命令477 passed / 0 failed / 7 ignored。

所有验收进程已退出；没有Docker、宿主路由/DNS/防火墙、真实控制面数据或live端口操作。两个worktree保持clean，分支与生成产物供协调者按同一权威源整合；schema v10仍归P2a，账户后续版本及正式启动迁移部署未声称完成。
