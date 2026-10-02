# 固定 56da5ad 审查修复证据

原审查：`$OPENCODE_TMP/meshway-managed-vless-review-astra.md`。
保留 `56da5ad` 和原反例目录不变；此分支只新增提交，不rebase。
下述为作者实际修复验证，仍待Astra固定新SHA独立复核。

## MV1 — 共享dirty carrier退休边界

- Runtime在split之下包裹完整carrier。业务record/batch尚未完全flush时，读、写、
  flush、shutdown所有同步poll共享同一个packet fence。读侧的WS/TLS/control
  隐式发送也必须先enter；retire同步等待在途poll退出，返回后任何侧都不能再poll。
- write future只订阅cancel，不重复持同一fence mutex；共享carrier本身负责同步
  I/O边界。部分record退休会关闭整个carrier，dirty状态不可清为可复用；没有
  后台writer、没有取消TLS/Noise，也没有改packet/nonce/retry缓存。
- 保留原完整TLS→WS→VLESS→Noise、完整body已缓冲、EOF延迟原场景到另拷
  `$OPENCODE_TMP/meshway-managed-vless-fix-repro`，依赖只改为作者工作树。
  独立target实际执行：**晚写/旧交付 0/20**，退出0。
- 仓库行为回归 `apps/agent/tests/managed_ws_retirement.rs`：current-thread和
  two-worker各20轮，**2 passed**，40轮均无晚写/旧payload；同Runtime重新认证
  真正新carrier后交付43，保留E2E/nonce缓存。remote旧channel被延迟EOF保留时
  使用既有已认证优先级替换，没有为测试放宽准入。
- 同步读侧优先/并发retire边界2项测试和完整dataplane（包括R3/R5）回归通过；
  全部构建使用 `CARGO_BUILD_JOBS=2`、独立 `target/mv-fix`。
- 日志：`managed-vless-mv1-original-scenario.log`、`managed-vless-mv1-tests.log`、
  `managed-vless-mv1-dataplane.log`（均位于批准TMP）。

## MV2 — generated异常脱敏

- `secret_debug::adapt_client`作为checked生成源adapter，统一opaque ApiError
  Debug/Display；成功解码失败和异常成功status均丢弃body/raw_body，解码错误只存
  固定分类，不保留serde错误中的secret value。status/retryable和公开category
  保留；status-first非2xx仍在body I/O之前返回。未修改transport注入构造模板。
- 生成重建后实际loopback HTTP：malformed 200、unknown-field 200、携带secret
  enum值的200、unexpected201，异常/ApiError/nested Option的Display和Debug
  均无UUID/private-key sentinel，body/raw_body为空。既有DTO直接/嵌套sentinel
  同时通过：`secret_debug` **2 passed**。
- 日志 `managed-vless-mv2-tests.log` / `managed-vless-mv2-generate.log` 位于批准TMP。

## MV3

- Managed stream capability进入controller联合连接编译，owner必须同时具有通用
  accept和对应stream，每个dialer同时受通用dial和对应stream检查。管理source
  保留；不兼容source使用既有whole-plan invalid/withdraw投影，秘密材料同步为空。
  能力撤回/恢复沿既有网络revision发布；不降为TCP，不绕路径。
- 新行为回归覆盖owner tcp-only/empty、每个不同dialer（tcp-only与WS可用）、
  owner通用accept撤回、stream撤回/恢复及幂等能力报告。逐阶段核对summary、
  source诊断、valid、有效listener/candidate/material、最小角色披露、同revision
  digest及原材料不变；controller定向 **7 passed**。
- 原API反例拷贝到 `$OPENCODE_TMP/meshway-managed-control-fix-audit`，原场景
  仅改依赖树和修复期望；真实HTTP异常无sentinel，真实API tcp-only owner得到
  `available=false, valid=false, listeners=0, materials=0`，退出0。
- 日志 `managed-vless-mv3-tests.log` / `managed-vless-mv2-mv3-original-scenarios.log`。

公共HTTP DTO/endpoint保持固定合同；最终全消费者矩阵尚未运行。
