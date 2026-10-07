# 工程流程：分层测试、触发与合入

本页定义代码从工作包到合入 main 的唯一流程、三层测试的触发规则、分支与证据的归属。入口与环境操作见[开发服务、测试与环境](/v2/development)，验收状态见[验收范围](/v2/node/validation)，性能口径见[分层性能测量](/v2/node/performance)。

## 三层测试

| 层 | 内容 | 入口 | 典型耗时 |
| --- | --- | --- | --- |
| T1 静态与单测 | fmt／严格 Clippy、workspace 测试、生成契约 drift、SDK 消费者、Console 类型／领域测试／构建、脚本测试、文档检查 | `mise run check` | planck 约 5 分钟；M3 约 15 分钟 |
| T2 隔离整组 | 真实 TUN、受保护包流、daemon／CLI 整组（7 个承载／DNS／NAT 行） | `mise run validate:matrix`；远程 `mise run validate:remote` | 全矩阵并行约 12 分钟；短模式单行约 3 分钟 |
| T3 性能 | L0 circuit、原语、L1 transport、L2–L4 dataplane、常驻 lab L5 | `mise run perf:*`，lab 见[三节点环境](/v2/node/docker-lab) | L0 2 分钟；L3／L4 固定口径约 10 分钟 |

T2 的短模式（`--short`，持续流约 30 秒）只用于诊断与包内门槛，不构成验收；验收状态只由全长运行更新。T3 只在空闲的测量机上运行，宿主有其他构建或测量负载时的样本不作结论。

## 触发规则

| 变更范围 | 包内门槛（合入前） | 整合（合入 main 时） | 合入后自动 |
| --- | --- | --- | --- |
| 任意 Rust／Console／docs | T1 相关 crate／包 | T1 全量 | T1 全量 |
| `crates/meshway-{connectivity,control,underlay,dataplane,dns,tun,transport*}`、`apps/meshway-daemon`、`apps/meshway-server/src/{execution,state}` | T2 短模式：`nat/off`、`nat+fallback/off` 加本次触及的承载／DNS 行 | T2 相关行全长 | T2 全矩阵 |
| `crates/meshway-dataplane`、`crates/meshway-connectivity`、`crates/meshway-transport*`、`crates/meshway-tun`、`tools/meshway-*-perf`、`[profile.*]` | 对应层 `--quick` 对照 | 对应层固定口径（2 CPU，profile off，两次重复） | L0 ＋ L3／L4 固定口径 |
| `scripts/`、`dev/`、`mise.toml` | `test:scripts` ＋ 受影响入口一次真实运行 | 同左 | T1 |

验收矩阵的任何一行从通过变为失败即阻断合入；不能用单行 `native-udp/off` 代替相关行。

## 工作包流程

1. **开工**：`git worktree add .worktree/<topic> -b <type>/<topic> main`，`type` 取 `feat`／`fix`／`perf`／`docs`／`test`／`tooling`／`cleanup`／`build`。一个工作包一个分支一个 worktree；并行工作包先划定文件归属，manifest／lock／生成链由协调者改。
2. **实现**：按功能拆成清晰提交；不积压未提交改动；文档与代码同一提交或同一分支。
3. **包内门槛**：按上表运行；性能包附报告 ID 与宿主 load。
4. **整合**：rebase 到 main → T1 全量 → T2 相关行全长 → `git merge --ff-only` → 推送 → 合入后自动层运行并写入状态页。
5. **回收**：删除 worktree 与分支；证据写入 [changelog](/v2/changelog)（工作包、提交、T2／T3 报告 ID）。
6. **部署**：常驻 lab 只部署合入后自动层全绿的 main；`node:lab deploy` 之后必须 `node:lab configure`（容器重建会改变 bridge IP）。

## 分支与证据

- 已完全合入 main 的分支（tip 为 main 祖先）在回收时删除；保留价值在提交、报告 ID 与 changelog，不在分支名。
- 未合入且不再推进的分支打包为 `.meshway/branch-archives/<date>-*.bundle` 后删除 ref，清单与恢复命令见该目录 README；`archive/*` 冻结参考分支按根 AGENTS 保留。
- 原始报告是私有证据：本机 `.meshway/{validation,perf}/`，测量机 `/mnt/data/Meshway/ci-results/`；状态页只保存摘要索引。

## 状态页与性能追踪

合入后自动层把每次运行的摘要（commit、触发、各层耗时、矩阵每行结果与报告 ID、L0／L3／L4 关键指标、二进制体积）追加到孤立分支 `ci-results` 的索引，文档站 [/status](/v2/status) 渲染最近运行与指标时间序列；相对上一次 main 结果变化超过 10% 的指标高亮。原始报告留在测量机，索引不含秘密与原始样本。

## 测量机

x86_64 测量机（planck，32 核，Docker，无 sudo）承担 T1 全量、T2 全矩阵与 T3 固定口径；本机只做包内门槛与诊断。远程运行通过 `validate:remote` 推送分支并回收摘要；GitHub Actions 的 self-hosted runner 以同一用户身份运行同一组 mise 任务，不另维护一套脚本。
