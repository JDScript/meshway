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

## MV2 / MV3

待修复验证。公共HTTP DTO/endpoint保持固定合同；最终全消费者矩阵尚未运行。
