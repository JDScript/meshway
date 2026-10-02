# AR1 / AR2 / SG1 合并前修复交付

落点：`.worktree/account-p2a-integration` / `integration/account-p2a`；在冻结 `e4d51b5` 后新增提交，不 rebase，不改原 `7ceb936` 账户树、Astra 固定审查树或 P2a WS/material/codegen实现。

| 项目 | 修复提交 | 定向证据 |
| --- | --- | --- |
| AR1 | `70efa24` | `accounts::expiry` 修前2失败/1对照通过；修后3通过。保持session expiry=当前秒+2、确认pending300ms、expiry+1释放外部writer及原5秒busy timeout；invite/approve均401，无invitation/binding/enrollment写入。认证principal和事务检查使用取得根写锁后的新now。 |
| AR2 | `18be869` | remove/leave均清当前用户撤销membership对应的撤销node assignments；旧孤儿可经同API授权重试清理，无变更重试不推进revision。当前角色/历史最严格角色管理上限仍校验；不是恢复会员、节点或tag identity。tag cleanup及后续revision失败均回滚membership/node/revision/tag关联；活动assignment、Access引用仍422。 |
| SG1 | `2cc31cc` | 已安装gate对照权威DDL manifest及实际解析的列/PK/复合FK/索引结构；验证active membership partial unique、不可变binding/ownership guards及所有node同网ownership覆盖。缺trigger、同名弱guard、缺/非unique/错误partial索引、弱化单列FK、缺表、缺ownership、FK关闭均拒绝。 |

## 准确 migration hook

- `store/account_schema.rs::install(&Transaction)` 仅在调用方迁移事务安装无版本账户/ownership片段；安装后调用 `validate`。任一Err必须由调用方回滚整体根事务。
- `store/account_schema::validate(&Connection)` 可接收协调者的迁移事务Deref；只读取实际库，期望manifest由内存里的权威schema生成，不在实际库上补DDL。只识别已知完整定义，未知/partial/不同guard不凭表名或空FK check接受。
- 分支 `Store::install_account_integration_schema` 先取得IMMEDIATE事务；任何账户相关schema对象存在则validate，完全不存在才install，成功后commit。后续生产迁移版本和移除该branch启动hook由协调者统一；**P2a v10不变，不宣称账户生产user_version已安装**。
- `docs/local-accounts.md` 已从 `7ceb936` 同步完整API handoff，并明确以上修复、claim保留tags与撤回清tags的区别；Web SSO/CLI request/poll仅保留原handoff，没有新增功能。

## 本轮验证（按修复范围）

全部使用独立worktree `CARGO_TARGET_DIR=target`、`CARGO_BUILD_JOBS=2`、feature构建串行、私有TMP fixtures，无Docker/live服务/真实admin凭据。

- 原Astra `$TMP/account-review-astra` 五项“断言bug存在”的源、manifest、lock和日志未改。另复制到 `$TMP/account-review-sol-fixed-20261002`，仅依赖改指修复树及将预期改为正确行为：**5 passed / 0 failed**，实际invite/approve/network=401，catalog删除=200，partial guards startup拒绝。
- 新定向HTTP expiry **3**、withdrawal **3**、startup **1**通过；完整Controller HTTP **37 passed / 0 failed**（包含既有65 operation的隔离矩阵与原账户/managed场景）。
- 账户领域 **25 passed / 0 failed**；schema迁移/gate **5 passed / 0 failed**，包含原无损SQL值/版本不变与DDL失败rollback。
- Apple core真实Controller用户登录→批准→ownership→原exchange/heartbeat/撤销接入 **1 passed / 0 failed**。
- Controller全target/allfeatures Clippy `-D warnings`、fmt check通过；SDK **14** tests、Web/SDK/e2e TypeScript检查通过。协议/路由/生成模板未变，本轮没有重跑完整494套件；统一合入消费者验收与Astra复核由协调者继续。

原Astra报告：`$TMP/meshway-account-integration-review-astra.md`。修复副本保留同一场景和TTL/锁等待边界，不把提高TTL或减少等待当作修复。错误/输出不包含fixture密码、token、PHC或材料原值。
