# P3a 3834a9d 审查修复交接

原冻结候选为 `3834a9d`。三项修复已独立复核关闭并完成消费者验收，现已合入主线；
下文保留作者修复 / 复跑的原始归属，最终独立结论见本页末尾。现有运行服务尚未随这次合并更新。

## A1：历史 journal 与 incoming floor 分离

`mesh_execution::load` 只验证历史文件的私有权限/controller/credential/node/key/address 绑定、schema/capability/digest、库存/路径/数量等自身完整性；不将新 startup connections/inventory 的 revision 当作旧文件损坏。新 incoming 另外同时检查当前 connections floor、inventory floor、历史 Mesh journal revision/digest/网络及重叠公钥约束；materials journal 的现有单调校验继续在持久化前执行。未删除 journal，也未降低同 revision mutation 或真实 rollback 防御。

新增正常 Controller + 生成节点客户端 + `managed_tick` 回归，包含无管理员改动重启和停机后改变策略 revision 再重启；只有设备边界使用既有 MockDevice。历史/新 floor 单测另验证相同 revision 接受、旧 incoming 与 mutation 拒绝以及历史地址被篡改仍拒绝。原 Agent probe 被复制到新 `TMP/meshway-p3a-agent-fix-reprobe`，仅覆盖作者修复并把两项原反例断言改为修复后的预期；原报告/源码/target 不动。

## C1：saved intent 与 effective projection 分离

管理 `admin_snapshot` 保留完整保存意图及托管 listener 视图。单独的 `effective_snapshot` 用于节点、runtime/discovery/history 和旧 publication；Mesh publication 用本次待编译 policy 的 `effective_with_policy`，避免拿旧 block 过滤正在保存的新策略。projection 不回存 derived 候选。Store 和正常 Docker 管理 API 验证 block→GET/save→unblock 原 candidate ID 恢复，测试不补写候选。

## C2：64 个实际材料的事务/下载一致门

合同仍为 64。共享材料选择谓词精确匹配 enabled managed VLESS 的 owner/outgoing 投影，每个 listener 只算一次，legacy profile 引用不算托管材料，incoming-only dialer 不取得 owner 材料。事务预算在解密前检查实际计数，下载投影在解密前及返回前防御检查；数量超额报 Capacity，整个 source/listener/candidate/revision 更新回滚。

回归覆盖 v3 owner 第 65 listener、候选使 dialer 从 64→65、同材料的多个 endpoint 仍计一次、legacy 65 材料不能激活 v3及独立下载投影拒绝；正常 Docker 另验证生成管理 API 拒绝第 65 listener，运行中 Agent 仍消费 64 材料并恢复真实 TUN 业务。

## 作者执行证据（2026-10-02）

修复提交 `d5d6b03`（控制面 C1/C2）及 `9cc6464`（Agent A1）。本页与 fixture 更新是独立验收提交；仍需交回两路 reviewer 复核，不把作者 PASS 写成 reviewer 关闭或主线已合入。

- **原反例新副本**：`TMP/meshway-p3a-agent-fix-reprobe`，独立 `TMP/meshway-p3a-agent-fix-target`、jobs=2。原 `review_` 三项通过，历史 R7 随保存库存 R8 可读但旧 incoming 被拒绝；真实 Controller/生成 client/managed_tick 从停机 R5 经管理更新后恢复至 R8；原三节点 UDP/fence 探针也通过。日志 `meshway-p3a-agent-author-fix-reprobe-final.log`。只调整新副本的两项错误行为断言为修复预期，原 report/probe/target 均未修改，manifest/lock 与仓库逐字节一致。
- **当前源码全部 Rust 消费者**：`CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/p3a-fixes cargo test --workspace --all-features --locked -- --test-threads=2`，**578 passed / 7 ignored**。日志 `p3a-fixes-rust-all.log`。包含 4 个控制面数量/intent 回归、历史/floor 单测以及正常 managed_tick 的无配置变更和离线 revision 变更重启。
- **检查**：fmt、workspace all-targets/all-features Clippy、drift、当前 fixture strict TS 均通过。日志 `p3a-fixes-clippy.log`、`p3a-fixes-drift.log`；本轮未改 HTTP/DTO 形状或放宽 64 合同，生成产物无变化。
- **正常 Linux TUN**：`python3 experiments/transport/performance_lock.py docker build -f experiments/p3a-mesh/Dockerfile -t meshway-managedtest:p3a-review-fixes .`；`python3 experiments/p3a-mesh/run.py meshway-managedtest:p3a-review-fixes`。完整共享 performance lock，独立 netns，**PASS / 21 evidence / cleanupErrors=[]**。除原 Mesh 包流/顺序/拒绝/stall/lease/撤回外，验证 C1 管理 GET-save 原候选恢复、A1 停机时管理员撤 grant 推进 revision 后正常重启、再恢复 grant，以及 C2 第 65 个平台生成 VLESS listener 被拒绝且 revision/源不变、实际 Agent 正常保存消费 64 材料并继续 ICMP/HTTP。报告 `TMP/meshway-p3a-muqqbrry-c78f1ffc340a-Teidvn/result.json`；日志 `p3a-fixes-docker-normal.log`。
- 镜像 ID 已由本次 report 自身记录：`sha256:84276b4b6f780671178e99140d72560eda8580bfe68b929fedb997d8d781d346`。64 个 VLESS listener 是平台真实生成、下发、绑定的资源；实际业务承载仍为 fixture 的 UDP，不声称本次经过这些 VLESS listener 传输业务。

`TMP` = `/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode`。初次测试 helper 使用了未启用的 reqwest JSON feature，已改为既有 serde bytes API；一次材料测试先提交了重复 normalized endpoint，已改为同一 listener 两个合法 advertised endpoint，未放宽生产验证。初次失败日志保留。

原文件所有权保持；未改 underlay/tun*、根 manifest/lock、main/live/远端，不扩状态型 Mesh 回包、GeoIP、UI，也未再委派。

## 独立关闭与主线整合

2026-10-02，两个原 reviewer 分别在冻结 `a36e24e` 的新副本重放原反例，旧证据保持原样：

- **A1 关闭**：8 项定向检查通过。真实 Controller / 生成客户端 / `managed_tick` 验证历史 R5 → bootstrap R6，
  journal 原字节保留且旧代未激活 → 新 epoch 安装 R8。真正 rollback、same-revision mutation 和地址 / key / network 篡改仍拒绝。
- **C1 / C2 关闭**：7 项独立探针与 7 项冻结树定向测试通过。saved intent 不丢失，编译新 policy 不使用旧 block；
  owner / dialer / legacy 激活 / enable / unblock 的超额投影均原子拒绝，重复 endpoint 只计同一份材料。
- **隔离消费者通过**：本次实跑生成器 7、SDK 16、旧 Web 184；严格 TS / lint / Web build、Apple 脚本、七组实验 TS、docs、fmt。
  核验并沿用作者精确代码版本的 Rust 578 / 7 ignored、Clippy 与 drift，没有将沿用结果伪称本次重跑。
- OpenAPI 原有 161 schemas / 58 paths / 67 operations 定义完全相同；新增 24 schemas 和 4 operations，
  主线现在为 185 schemas / 62 paths / 71 operations。

主线提交对应：`6050413`（Controller）、`0dac17d`（Agent）、`d88fa84`（原验收记录）、
`08502cb`（C1/C2）、`fdadd59`（A1）、`07a8039`（修复验收）。合并后的后端与冻结 `a36e24e` 一致，
独立 console 的源码、workspace 和 lockfile 保持原样；本线程未 push。

最终报告均在批准 TMP：

- `meshway-p3a-agent-review-fixed-a36e24e.md`
- `meshway-p3a-control-review-fixed-a36e24e.md`
- `meshway-p3a-integration-consumers-a36e24e.md` 及同名 `-logs/`

本次合入的是 IPv4 Mesh v3 正常发布 / 执行切片。正式 underlay 准备 / 主循环接线仍未合入，
单向 grant 状态型回包、P3b / P3c / P3d 仍未完成；远端第一轮继续固定使用已选定的 `4d106e0`。
