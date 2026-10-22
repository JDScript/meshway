# 工程流程：分层测试、触发与合入

本页定义代码从工作包到合入 main 的唯一流程、三层测试的触发规则、分支与证据的归属。入口与环境操作见[开发服务、测试与环境](/v2/development)，验收状态见[验收范围](/v2/node/validation)，性能口径见[分层性能测量](/v2/node/performance)。

## 三层测试

| 层 | 内容 | 入口 | 典型耗时 |
| --- | --- | --- | --- |
| T1 静态与单测 | fmt／严格 Clippy、workspace 测试、生成契约 drift、SDK 消费者、Console 类型／领域测试／构建、脚本测试、文档检查 | `mise run check` | planck 约 5 分钟；M3 约 15 分钟 |
| T2 隔离整组 | 真实 TUN、受保护包流、daemon／CLI／出口／漫游／故障注入／netmap 整组（25 行，见[默认矩阵](/v2/node/validation#matrix)） | `mise run validate:matrix`；远程 `mise run validate:remote` | 25 行在 planck 并行 8 约 18 分钟；短模式单行约 3 分钟 |
| T3 性能 | L0 circuit、原语、L1 transport、L2–L4 dataplane、常驻 lab L5 | `mise run perf:*`，lab 见[三节点环境](/v2/node/docker-lab) | L0 2 分钟；L3／L4 固定口径约 10 分钟 |

T2 的短模式（`--short`，持续流约 30 秒）只用于诊断与包内门槛，不构成验收；验收状态只由全长运行更新。T3 只在空闲的测量机上运行，宿主有其他构建或测量负载时的样本不作结论。

T1 的 Rust 行为测试由 `mise run test:rust` 使用 nextest 跨测试二进制调度，随后单独运行 doctest；失败重试为零，真实 Server fixture 按 test group 限制并发。配置、包内运行与 libtest 回退命令见[Rust 测试](/v2/development#rust-测试)。计时比较须使用同机、相同 feature 与明确的构建热身条件，不能把并发单例时长之和当作套件墙钟。

完整的测试清单、每个套件验证什么与本机开发门槛（`mise run check:fast`）见[测试清单与开发门槛](/v2/tests)。

## 触发规则

| 变更范围 | 包内门槛（合入前） | 整合（合入 main 时） | 合入后自动 |
| --- | --- | --- | --- |
| 任意 Rust／Console／docs | T1 相关 crate／包 | T1 全量 | T1 全量 |
| `crates/meshway-{connectivity,control,underlay,dataplane,dns,tun,transport*}`、`apps/meshway-daemon`、`apps/meshway-server/src/{execution,state}` | T2 短模式：`nat/off`、`nat+fallback/off` 加本次触及的承载／DNS 行 | T2 全矩阵（`validate:remote`，合入前） | 每晚 T2 全矩阵（`Validate` 定时 08:17 UTC）；push 不再触发——合入前已在 planck 跑过同一矩阵，再跑一次只与其他租户抢资源 |
| `crates/meshway-dataplane`、`crates/meshway-connectivity`、`crates/meshway-transport*`、`crates/meshway-tun`、`tools/meshway-*-perf`、`[profile.*]` | 对应层 `--quick` 对照 | 对应层固定口径（2 CPU，profile off，两次重复） | L0 ＋ L3／L4 固定口径 |
| `scripts/`、`dev/`、`mise.toml` | `test:scripts` ＋ 受影响入口一次真实运行 | 同左 | T1 |

验收矩阵的任何一行从通过变为失败即阻断合入；不能用单行 `native-udp/off` 代替相关行。

## 整组验收的使用方式

T2 的每一行都受生产时间常量约束（60 秒租约、续租、退避、70 秒持续流），runtime 单行约 5 分钟（`exit/off` 约 13 分钟），不以测试刻度改变生产代码。因此：

- **异步执行，不等待**：整组运行提交到测量机（`validate:remote`）或由 Actions 触发后立即返回，结果经 `--attach`、状态页或 PR 检查送达；任何人／agent 都不在会话里阻塞等待一行跑完。
- **失败靠证据定位，不靠重跑观察**：每次失败都保留 a／r／b／controller 日志、阶段标记与 pcap；先从证据定位到所属 crate，写进程内复现（虚拟时间或共享 Server fixture 的 Rust 测试，秒级），对着它修复，整组只做最后一次确认。
- **二分并行**：需要二分时在测量机上同时跑多个提交的同一行，一轮得到多个点。
- 验收状态只由全长行更新；`--short` 只用于诊断。
- 工作树下 `.meshway/` 的私有证据在 `git worktree remove` 之前移入仓库根的 `.meshway/`；回收工作树不得带走证据。

## 工作包流程

1. **开工**：`git worktree add .worktree/<topic> -b <type>/<topic> main`，`type` 取 `feat`／`fix`／`perf`／`docs`／`test`／`tooling`／`cleanup`／`build`。一个工作包一个分支一个 worktree；并行工作包先划定文件归属，manifest／lock／生成链由协调者改。
2. **实现**：按功能拆成清晰提交；不积压未提交改动；文档与代码同一提交或同一分支。
3. **包内门槛**：按上表运行；性能包附报告 ID 与宿主 load。
4. **整合**：rebase 到 main → T1 全量 → T2 相关行全长 → `git merge --ff-only` → 推送 → 合入后自动层运行并写入状态页。
5. **回收**：删除 worktree 与分支；证据写入 [changelog](/v2/changelog)（工作包、提交、T2／T3 报告 ID）。
6. **部署**：常驻 lab 只部署合入后自动层全绿的 main；首次初始化运行 `node:lab configure`。容器重建后的 bridge IP 由 Node State 自动更新，既有配置不因 IP 变化重做。

## v0.1.0 交付门槛 {#v0-1-gate}

v0.1.0 只含 Linux Node 与 Server：一个小团队能自行安装，在真实网络上连续运行 14 天无需人工值守，已知限制写明。到达门槛前**功能冻结**——不开新功能工作包；以下移到 v0.1.0 之后：自动方案地区推断（GeoIP）、多出口 DNS plan、IPv6 出口前缀变化与出口合成 PTB（I6-2，列为已知限制）、千节点规模、非 root／netstack、Apple。

| 里程碑 | 内容 | 退出条件 |
| --- | --- | --- |
| A 还债 | 已登记缺陷（X16、R4、X17 及 `fault/throttle` 行暴露的问题）；恢复状态机收敛为一个共享框架；[验收范围](/v2/node/validation)改为对外参考格式 | 无未登记的矩阵失败签名；`fault/throttle` 行连续 3 轮全绿 |
| B 交付件 | Server 存储迁移（schema 升级不 reset、不重新接入）；Server 部署形态（systemd unit、备份、远程升级）；含 compilerVersion 断网窗口的升级演练 | 在现场从上一版本按[安装](/v2/install)文档升级成功 |
| C 现场 soak | 常驻六台连续 14 天 | 零人工重启；每次 `blocked` 都有已证实的原因；空闲 CPU 每线程 <5%（中继／出口节点按 [P2](/v2/node/validation#remaining-boundaries) 结论定义口径） |
| D 审阅 | 威胁模型／安全审阅与最终代码审阅（只读，与 C 并行） | 无高危未修 |

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
| `mars-14900k` | `self-hosted,linux,x64,docker,cgroup2,mars` | Intel i9-14900K，32 线程，rootless Docker（cgroup v2；runner 服务设置 `DOCKER_CONTEXT=rootless`、`TMPDIR` 在用户私有目录），工作目录在 `/mnt/SSD-1` | T1。**不跑 T2**：rootless 下 NAT 路由器与 resolv-conf 行失败（`20261008T080838Z-75ed9dd4` 四行挂，同提交 planck 四行通过 `20261008T090110Z-*`），不是产品回归 |
| `mars-epyc7543` | `self-hosted,linux,x64,build,mars` | AMD EPYC 7543，128 线程，503 GB 内存，rootless Docker（cgroup v1，**容器资源限制无效**），工作目录在 `/mnt/SSD-2` | **编译机**：T1、x64／arm64 交叉构建产物；不跑 T2／T3 |
| `jp-tyo-arm64` | `self-hosted,linux,arm64,docker,jp-tyo` | Ampere Neoverse-N1，4 核，Docker | T3 arm64 原生 L0／原语参照 |

`check` 可在任意 x64 runner 运行；`validate` 钉在 `planck`（fixture 依赖 root Docker：NAT 路由器转发与系统 resolver 文件在 rootless 下不成立，且 `--cpus`／`--memory` 须真实生效）；`build` 在 `build` runner 交叉编译两种 Linux 目标并上传产物；`perf` 钉在 `planck`，`perf-arm64` 钉在 `jp-tyo`，保证时间序列同机可比。runner 的工作目录、mise／cargo 缓存均放在各机的大容量数据盘，不占根分区；`mars-epyc7543` 例外——它的数据盘与他人共享且常满（2026-10-11 剩 438 MB 导致 `build` 失败），工作目录改在 tmpfs `/dev/shm/meshway-ci/actions-work`（503 GB 内存机），checkout 与 Cargo target 全在内存里，job 之间作为热缓存保留，主机重启即丢，runner 重新创建；mise／cargo／rustup 缓存仍在数据盘。planck 的 runner 以 systemd drop-in 限 `MemoryMax=20G`／`CPUQuota=1600%`，`.env` 设 `CARGO_BUILD_JOBS=8`，因为它同时是其他项目的工作机（2026-10-10 一次满并发矩阵叠加 32 路冷构建把 runner 与宿主服务 OOM 杀掉）。mars 两台为共享机且无 sudo：用户级 systemd 在该用户最后一个登录会话结束后会停止，需要管理员执行 `loginctl enable-linger siqi` 才能常驻；`validate:remote` 的 ad-hoc 路径仍指向 planck。所有 runner 以普通用户运行同一组 mise 任务，不另维护脚本；工作流把 `MISE_GLOBAL_CONFIG_ROOT` 指向不存在的目录（`MISE_GLOBAL_CONFIG_FILE` 在 mise 2026.8 只追加、不替换），runner 用户的个人 mise 全局配置不参与工具解析；lab／fixture 的 Docker 调用遵守标准 `DOCKER_CONTEXT`（默认 `default`）；状态页收集器只用标准库 HTTP 与 `GITHUB_TOKEN`，runner 不需要 `gh`；job 结束后按 run 标签清理容器与 network，不留宿主改动。`validate` 的完整行证据（含 pcap 与私有材料）复制到 runner 用户的 `~/meshway-ci-evidence/validate/<run_id>-<矩阵 ID>/`（每次约 4 GiB，保留最近 8 次）——同一 runner 的下一次 checkout 会 `git clean -ffdx` 清掉工作目录；Actions artifact 只带 `matrix.json`、`summary.md`、各行 `report.json`／日志／`error.txt`，不含 pcap、pem 与 `private/`。
