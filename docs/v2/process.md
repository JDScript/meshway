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

`Check`／`Validate`／`Perf`／`Build` 完成后触发 `Status` workflow，在串行 `status-index` 组运行 `scripts/ci/collect.py --publish --push`。脚本从 GitHub Actions API 读取 run、触发、commit、job runner／时间／结论和 artifact 元数据，仅下载性能／矩阵报告及 Node 产物提取摘要，按 run ID 增量写入孤立分支 `ci-results` 的 `index.json`。下载过期不抹去已采集证据，缺测不填零。

本地查看：

```sh
mise run status:sync
mise run dev:docs
```

打开[CI 与性能状态](/v2/status)。`status:sync` 获取远端 results 索引并原子写入被 Git 忽略的 `docs/public/status/index.json`；远端未创建时可使用本地 results 分支，`mise run status:sync -- --local` 强制离线。两者都不存在时页面显示明确空状态。浏览器仅请求站内静态索引，不需要 token。

页面包含最近运行表、按 runner／测量参数分组的 main 性能曲线、最近 main 验收行×运行网格和按 target／编译机分组的二进制大小／SHA-256。相对上一有效 main 结果变化超过 10% 的指标高亮；quick、dirty、不完整样本不作为变化基线，短矩阵明确标记为诊断。未过权限或已过保留期的 artifact 链接可能不可下载；索引更新时间代表采集，不代表重新测量。

手工本地采集使用 `mise exec -- python3 scripts/ci/collect.py --publish`，只提交本地 `ci-results`；显式 `--push` 才发布远端。命令参数、版本化 schema 和 matrix 输入适配接口见源码 `scripts/ci/README.md`。原始报告留在测量机与 GitHub artifact，索引只保存字段白名单摘要。

## 测量机与 runner

| runner | 标签 | 机器 | 角色 |
| --- | --- | --- | --- |
| `planck` | `self-hosted,linux,x64,docker,cgroup2,planck` | AMD Ryzen 9 9950X，32 线程，Docker，系统 systemd | **T3 x86 固定口径**（唯一性能基准机）；T2 全矩阵；T1 |
| `mars-14900k` | `self-hosted,linux,x64,docker,cgroup2,mars` | Intel i9-14900K，32 线程，rootless Docker（cgroup v2，容器资源限制有效），工作目录在 `/mnt/SSD-1` | T2、T1 |
| `mars-epyc7543` | `self-hosted,linux,x64,build,mars` | AMD EPYC 7543，128 线程，503 GB 内存，rootless Docker（cgroup v1，**容器资源限制无效**），工作目录在 `/mnt/SSD-2` | **编译机**：T1、x64／arm64 交叉构建产物；不跑 T2／T3 |
| `jp-tyo-arm64` | `self-hosted,linux,arm64,docker,jp-tyo` | Ampere Neoverse-N1，4 核，Docker | T3 arm64 原生 L0／原语参照 |

`check` 可在任意 x64 runner 运行；`validate` 只在 `docker,cgroup2` runner 运行（fixture 依赖 `--cpus`／`--memory` 真实生效）；`build` 在 `build` runner 交叉编译两种 Linux 目标并上传产物；`perf` 钉在 `planck`，`perf-arm64` 钉在 `jp-tyo`，保证时间序列同机可比。runner 的工作目录、mise／cargo 缓存均放在各机的大容量数据盘，不占根分区。mars 两台为共享机且无 sudo：用户级 systemd 在该用户最后一个登录会话结束后会停止，需要管理员执行 `loginctl enable-linger siqi` 才能常驻；`validate:remote` 的 ad-hoc 路径仍指向 planck。所有 runner 以普通用户运行同一组 mise 任务，不另维护脚本；job 结束后按 run 标签清理容器与 network，不留宿主改动。
