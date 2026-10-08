# Changelog

按合入 main 的顺序记录工作包、提交与证据 ID。规则见[工程流程](/v2/process)；性能数字的口径见[分层性能测量](/v2/node/performance)，验收状态见[验收范围](/v2/node/validation)。

## 2026-10-07

| 工作包 | 合入提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| H 流水线 | `2fcc2b10` | owner／worker API、PacketBuffer 贯通 Session、L4 `manager` 阶段 | runtime `20261007T010018Z-e430176f` |
| J tun-rs codec | `0a03e15f` | `meshway-tun` 改用 tun-rs 的 `gso_split`／`GROTable` | tun `20261007T021421Z-42104e65` |
| K UDP GSO 边界 | `19938b7b` | 地址族 payload 上限、仅 GSO 上下文的 EIO 降级、接收区间引用 | packet `20261007T022232Z-c09c4a42` |
| I 流式释放 | `85443d60` | 持久 crypto 队列、有序前缀释放、控制优先、harness `--modes` | runtime `20261007T034408Z-e0dce6be` |
| L 密码归因 | `d0e60ba9` | `perf:crypto` 工具与 L0 归因 | crypto `assessment.md` |
| M 密码后端 | `7eac33a6` | ring AEAD、硬件 SHA2 预 key HMAC | runtime `20261007T044453Z-eff72ec3`；L3 `20261007t045428z-fb7362ea` |
| MTU 默认 | `05f0b933` | 内层 MTU 1366 由固定帧开销导出 | — |
| R 切片校验去重 | `83caeb36` | `validate_obsolete_slice` 共享实现 | — |
| S crate 卫生 | `a4dffc3c` | 未用依赖、workspace 继承、测试独占 wrapper | — |
| O owner 路径 | `5a30247d` | UDP 7 MiB 缓冲、Circuit 稳定分配、共享 Context | L3 `20261007t064600z-c4127ba8` |
| Q 控制面 | `ebd42554` | SSE 刷新、换钥、接入密钥、IPC v4、Server SSE 热点 | runtime `20261007T090243Z-fb5f1714`／`20261007T091327Z-063ce0a0` |
| X 回执分类 | `c3cbed6c` | 回执 409 按原因分类，非 fencing 恢复，状态投影 | runtime `20261007T100459Z-ea847031` |
| Y Manager 路径 | `98855128` | 未准入事件队列、批次 scope 检查 | L3/L4 `20261007t112743z-6f3b2103` |
| Z 内存 | `6725d11b` | musl daemon 采用 mimalloc、TUN 存储复用 | `z-validation/receipt.md` |
| AA 控制面风暴 | `f39ba2b6` | SSE 读空闲超时、候选查询节流 | runtime `20261007T154104Z-ac45dd04`／`20261007T155007Z-80295767` |
| PT 性能收尾 | `6e3119b2` | Manager 批前缀消费、`--quick`、performance.md 重组 | L3/L4 `20261007t204126z-cbe7469c` |
| T 二进制 | `3887256b` | ring-only TLS、fat LTO、features 收紧 | `binary-size/20261007T192625Z`；runtime `20261007T204330Z-e843809f` |
| W1 承载模型 | `a8ed42d6` | `node/carriers.md` | — |
| V1 矩阵重验 | `9b30c19b` | 满 MTU／多 MSS 用例；v5 矩阵 5 行通过、4 行未通过 | `validation/v1/20261007-main-3887256b/` |

| CI1 矩阵工具 | `c18f4377` | `validate:matrix`／`validate:remote`、`--short`；planck 全矩阵 14.5 min | `validation/remote/CI1-summary-12ec8a16.md` |
| ST1 状态页 | `f17b59d0` | `scripts/ci/collect.py`、`ci-results` 索引、`/status` 页、`status.yml` | — |
| TQ 测试收敛 | `b5e48bc7` | nextest、共享 Control setup、测试 KDF profile、pipeline 测试合并、fixture 共享 rendezvous | planck `test:rust` 482 → 87 s；M3 全量 check 934 → 202 s |
| 流程／CI | `4385a0c9`…`60e966a3` | process.md、changelog、workflow 拆分、四台 runner | — |

未合入：AB `perf/daemon-dataplane-runtime`（专用数据面运行时，无吞吐收益，分支已打包归档）。

## 待合入工作包

| 工作包 | 候选提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| W2 流式承载 | `90bf38e2`、`9aae0c35`、`0593e19b`（`feat/runtime-streaming-carriers`） | runtime fixture 与矩阵增加 VLESS／Trojan TCP/TLS；只有 R listener、A/B 使用所选 dialer；相同材料执行交接保留认证通道，旧输出 scope 立即失效。 | 秒级进程内红／绿回归与 T1 已通过；Planck 两行全长首跑／独立复跑尚待完成，状态见[验收范围](/v2/node/validation)。 |
| TD 测试整理 | `4a8ba975`、`704b2bd3`、`376c9176`、`52a1ac78`（`cleanup/test-dedup`） | Server 两条重复测试及重复 HTTP 断言／样板收敛；Server／transport 测试文件 ≤800 行；可执行上限检查；修复 State provider 重复关闭钩子的强引用保留与 fast 包选择。Server 物理测试行数 −1.6%，未达到 −30%；Clock 完整传播范围超过 15 文件，按边界仅完成评估。 | `.meshway/td/`：覆盖对照、Clock 评估、provider 生命周期红／绿回归、T1 `check.log`；热构建 `test:rust` 990 → 988 条，墙钟 114.7 → 128.3 s（单次观测，无提速结论）。 |
