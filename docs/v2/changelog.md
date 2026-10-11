# Changelog

按合入 main 的顺序记录工作包、提交与证据 ID。规则见[工程流程](/v2/process)；性能数字的口径见[分层性能测量](/v2/node/performance)，验收状态见[验收范围](/v2/node/validation)。

## v0.1.0-alpha.1

首个 Linux 预发布，用于验证发布与安装流程，不承诺兼容性。

- 产物：`meshway-x86_64-unknown-linux-musl.tar.gz`、`meshway-aarch64-unknown-linux-musl.tar.gz`（`meshwayd`、`meshway`、`meshwayd.service`），`install.sh`，`SHA256SUMS`。
- 安装：`install.sh` 校验并安装为 systemd 服务（`Type=notify`、capability 限于 NET_ADMIN／NET_RAW／NET_BIND_SERVICE／SYS_PTRACE、`ProtectSystem=strict`），支持 `--version`、`--artifact`、`--uninstall [--purge]`；见 [Linux 安装与系统服务](/v2/install)。
- 新命令 `meshway version`：程序版本、git 描述、IPC 6、circuit wire 6、`meshway-netmap-1`。
- 范围：Linux Mesh-only TUN、Native UDP／TCP、VLESS／Trojan TCP/TLS、协调 UDP 穿透、resolved／resolv-conf 系统 DNS、`meshway ping`、签名 netmap 租约内离线运行、IPv4 出口；验收状态见[验收范围](/v2/node/validation)。
- 已知限制：systemd-resolved 重启或停机后 DNS 归属冲突、数据面不自动恢复，需重启机器，见[当前限制](/v2/install#当前限制)。

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

## 2026-10-08

| 工作包 | 合入提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| 脚本整理 | `ead029e6` | 删除无引用的 lab OS 检查；fixture 只记录负载不等待空闲；`perf:crypto:attribute` 任务 | — |
| 流程 | `d91e9ffc`、`a5982814` | 整组验收异步化、靶证据定位；`profile.fast`／`check:fast`；[测试清单](/v2/tests) | M3 fast 945 项 51 s |
| V2 NAT／resolv-conf 回归 | `da736c2b`…`641c27ca` | STUN 观测刷新与邻接查询分开调度；State 采样先处理排队 target 并确认回执；退役需求取消；fixture 识别 v5 帧 | planck 四行各两次通过 `20261008T032743Z-897a549e`、`20261008T050147Z-0c5809cf`；回归行 `…-abbd4545`、`…-7a35b80e`；`.meshway/validation/v2/20261008-nat-regressions/` |
| TD 测试整理 | `6371afbc`…`214df852` | Server／transport 测试文件 ≤800 行与可执行上限检查；2 条真重复测试删除；修复 State provider 重复关闭钩子强引用；`test:fast -- -p` 生效 | `.meshway/td/TD-receipt-20261008.md`；Server 测试行 −1.6%（去重假设基本不成立） |
| main 全矩阵 | `1271be5d` | v5 矩阵首次 9/9 通过（含 `udp/resolved`） | planck `20261008T053753Z-0f9ac9d2`，20.7 分钟 |
| L5 复测 | `5837bd33` | lab 2 CPU／节点 sha→lax→nyc：TCP 1321／1525 Mbps、UDP 475 Mbps | `.meshway/perf/l5/20261008T055333Z-main-1271be5d-2cpu` |
| CI runner 可移植 | `49dc2e8e`…`e9c7a376`、`4e8a6f9d`、`5c2a012f` | DNS 文件后端测试自探测可信目录；`DOCKER_CONTEXT`／buildx builder 跟随 context；状态收集器改标准库 HTTP；`MISE_GLOBAL_CONFIG_ROOT` 排除 runner 个人配置 | mars-14900k／jp-tyo 四类失败各一 |
| W2 流式承载 | `46c65e47`…`36840b3b` | runtime fixture 与矩阵增加 `vless/off`、`trojan/off`；相同材料执行交接保留流式认证通道 | planck 两行各两次通过 `20261008T060724Z-15ece3bb`、`20261008T061606Z-9875d228`；`.meshway/w2-streaming/` |
| TD2 测试拆分 | `d9baecb0`…`06f33739` | connectivity／policy 六个超限测试文件按行为拆分，共用 harness，白名单清零（271 文件 0 例外） | `.meshway/td2/`；测试数 52／86 不变 |
| W4 边级承载 | `cd536833`…`8ca49b0f` | `topology[].transports`、双向拨号交集、`edge_transport_unavailable` 定位规则／边、执行候选过滤、OpenAPI／SDK | policy `tests/execution/topology.rs`、`tests/configuration/topology.rs`；`.meshway/w4-edge-transports/` |
| W3 lab VLESS 边 | 本提交 | lab revision 7 验证 sha→lax VLESS TCP/TLS、双向原路径诊断与高负载 L5；部署版 `1271be5d` 缺 W2 续接修复导致流式链路重建，回滚 revision 8 | L5 `20261008T063234Z-main-1271be5d-2cpu-vless`；`.meshway/w3-lab-vless/`；复现分支已打包归档 |
| V3 中继交接丢回复 | `6af24659`…`c31541bd` | retained 执行交接时中继保留诊断请求关联与原期限；maintenance 保留有界待交接关联；重放保护与旧 scope 失效不变 | CI 证据 `37762142607` `udp/resolved` 首探测超时；复现 `retained_relay_delivers_inflight_diagnostic_and_business_reply_exactly_once`；planck `20261008T170200Z-bbdb811a` 三行通过；`.meshway/validation/v3/` |
| V4 重启不卡死 | `fix/underlay-restart-journal` 系列 | journal 的 boot／netns 与当前不一致时原子归档为 `underlay.json.stale-*` 并按无 journal 启动；当前 netns 内活 owner 冲突仍拒绝但有 1–30 s 退避；损坏 journal 归 Storage | 现场 `.meshway/w3-lab-vless/incident-20261008T1655/`；隔离测试 `container_restart_archives_previous_namespace_journal`、`live_tun_owner_conflict_retries_and_recovers_without_new_up`；planck `20261008T183321Z-9b32fb72` 两行通过；`.meshway/validation/v4/` |
| W5 节点自报端点 | `feat/node-reported-endpoints` 系列 | daemon 在 State `sockets[].localEndpoints` 上报 listener 的本机接口地址（≤8，排除 loopback／link-local／Mesh／TUN）；Server 编译候选 configured→local，带来源；Manager 全部承载按候选拨；`acquisition: localEndpoint`；管理员 `endpoints` 变为可选；lab `configure` 不再写 bridge IP；compilerVersion `meshway-demand-5`、Control record v3、IPC v5 | planck `20261008T185410Z-1d4429c8` 四行通过（`udp/off` 为 reported-only）；`.meshway/w5-endpoints/` |
| V5 运行中接口变化 | `fix/live-interface-change` 系列 | kernel monitor 忽略非内核来源 multicast（原误判为永久保护冲突，使 control 瞬时失败不恢复）；接口／地址／路由外部变化 → 关旧控制连接重连并重枚举本机端点；自有 suspend 的 DelAddress／DelLink 不算外部变化；localEndpoints 按自有接口集合排除 TUN／DNS 锚点；fixture 保留已观测执行截止 | 现场 `.meshway/validation/v5/analysis.md`；`control_reconnects_after_live_interface_replacement`、`owned_suspend_keeps_control_connection_and_reports_no_network_change`；planck `20261008T205449Z-84befa85` 四行通过 |
| X0 Exit/NAT 设计 | `e3ba2519` | `docs/v2/node/exit-design.md`：对照 Tailscale（router_linux／linuxfw／peerapi DoH／table 52）的首版方案：Linux IPv4 内核转发＋masquerade、独立出口 TUN、nft CLI 批事务、ip_forward 只检测、经 Mesh 的出口 resolver、X1–X7 拆包 | 研究基线 tailscale `ab2380a6` |
| X7a Exit 验收准备 | `31f8880f`…`f492fe5d` | `exit/off` 隔离拓扑（Controller／A／R／E＋外部服务＋E LAN 服务）、镜像锁定 nftables／conntrack／tcpdump、fixture 八阶段 `pending-x2-x5`、证据链；默认矿阵不含该行 | planck 短跑 `20261008T222757Z-a9263023` 以 `unsupported` 如实结束；`.meshway/exit/x7a/` |
| X1 Exit 契约与联合编译 | `feat/exit-contract` 系列 | `EgressCapturePlan`／`ExitRolePlan`／`ExitPermit`／`ExitCapability`，first-match CIDR 互斥分割，`is_internet_address` 唯一公网边界，能力不可用投影不回退 direct，compilerVersion `meshway-demand-6`；Control record v4 退役 demand-5 缓存；Console 原因文案 | policy `tests/execution/exit_contract.rs`；`.meshway/exit/x1/` |
| X5 单出口 DNS 契约与客户端 owner | `ef68c08a`、`7b4519ec` | `DnsServicePermit`、`DnsEgress::ViaExit`／`ServeExitResolver`，选择顺序 MagicDNS→useWithExit split→出口 resolver，联合许可与到期交集，客户端密封 `MeshResolverRoute`，默认适配拒绝安装（出口侧接线等 X2／X3／X4） | `.meshway/exit/x5/{receipt,interfaces}.md` |
| X2 dataplane 出口流 | `09da0d7f`…`9dec9b9a` | IPv4 出口 flow 按有界内核投影门控（`ExitProjection` Install／Renew／Retire＋ACK 前不放首包）、回程经唯一 FlowTable 原路径、Replacement 结束旧流、tombstone ≥120 s、容量 1024 流／128 投影／4096 槽；无 Underlay 依赖 | `tests/runtime/exit_lifecycle.rs` 13 项；`.meshway/exit/x2/` |
| V6 NAT 退路偶发 | `957aa298`…`5629fc06` | STUN 刷新复用有效 discovery context、截止前 8 s 续取元数据；已选流式通道结束后按来源优先级重新获取而不被 local UDP 黑洞阻塞 | CI `37849292927` 与 `20261008T230914Z-5380356e` 两次失败证据；`live_discovery_context_refreshes_same_socket_without_another_metadata_rpc`、`selected_tcp_fallback_reconnects_before_blackholed_local_udp_after_peer_health_skew`；planck `20261008T235755Z-8600ca5d` 两行通过；`.meshway/validation/v6/` |
| TD3 测试 lease 赛跑 | `test/lease-race-hardening` 系列 | 46 个真实时间测试逐个分类（A 流程跨 lease 17／B 固定等待 1／C 真到期 28）；A 类改为按产品方式续租或把断言靶近，B 改条件等待；两条已出事测试在 load 15–52 下连续 5 次通过；不改产品代码 | `.meshway/td3/{inventory,verification}.md` |
| X3／X3a Underlay 出口 owner | `cb464ae0`…`d86a80e5` | 出口前置探测（ip_forward／TUN／rp_filter／nft 真事务／conntrack）；`Host::install_exit_role`、`ExitRole::{install→ProjectionAck, confirm→CtConfirmation, renew, retire_flow, reply_reader, retire}`；私有 `inet meshway_exit_*`／`ip meshway_exit_nat_*` 表，仅 NEW∧未归属∧资格命中打 ct mark，masquerade 限所属 ct，精确 ct 删除，`underlay-exit.json` journal 与 SIGKILL 同 namespace 恢复；不要求 nf_conntrack_acct | planck 隔离 `exit_nat_udp_real_reply_qualification_confirm_retire_and_expiry`、`exit_recovers_real_sigkill_live_ct_in_same_namespace` 等 10 项通过；`.meshway/exit/x3/X3a-receipt.md` |
| X2b 精确关流 | `7799b955` | `SecureNode::fail_exit_projection(&FlowIncarnation)`：CT 确认失败后按 incarnation 关流，不影响共享 CID 的其他流 | `exit_confirm_failure_isolates_shared_cid_and_survivor_can_renew` 等 2 项红→绿 |
| V7 诊断抖动与首包延迟 | `fix/diagnostic-publication-churn` 系列 | 切片不变时复用 publication；安装门槛改为"本节点至少参与一条完整候选路径"（可选中转误拒）；`state_scope_changed` 作为正常 scope 刷新不进 Retrying；去掉 5 s 执行通知延迟、1 s 首轮等待、100 ms 握手轮询、Server 250 ms 全量重授权改事件驱动 | planck `20261009T042152Z-24bad550` 两行通过；lab 60 s 诊断 Retrying 4→0、Dataplane failure 6→0；直连首包 7.6–12.3 s→0.6–2.0 s（4/5 ≤1.5 s），一跳中继 3/5 ≤2.5 s；订阅延期仍产生 publication（留 N1）；`.meshway/validation/v7/` |
| X3c Underlay 接口补齐＋X3b capture | `feat/exit-underlay-x3c` | `Host::probe_exit`（已开 Host 上探测，成功 Ready/ipv4）；`ExitRole::send` 带每包 `ExitSendGuard`；`ExitIncarnation([u8;32])`；ICMP 按 identifier＋sequence 资格共享 CT；中继 R 无 exit role 可装外部 permit；`Host::install_egress_capture`／`EgressCapture::switch`（`/0`→客户端 TUN、direct 例外、IPv6 sink、并集负向门切换、SIGKILL 后负向保留） | planck 隔离 9/9＋真实 Control 场景；`.meshway/exit/x3c/receipt.md` |
| X3d 出口 role 合法交接 | `1a3ac88e`…`8ee294b5` | predecessor guard 失效时 role 进入 retained（拒旧 I/O、不删 CT／资格），直接后继续接，retention 截止才清理；停止的 role 先 reconcile 再探测；nft 高位 mark prefix 回读规范化（原误判所有权冲突） | X4 消费者测试 `daemon_exit_control_handoff_preserves_projection`、`daemon_exit_dns_role_refresh_preserves_applied` planck 绿；`.meshway/exit/x3d/` |
| X4 daemon 出口装配 | `8f10572c`…`ec56f8bc` | Control 绑定 role／能力到当前 instance；authority 保留投影原截止；`worker.rs::install` 用 `probe_exit` 上报能力、role 与 capture 安装进原子 applied；`runtime/network/exit/` 桥接 Install→ACK→guarded send→confirm→Renew／Retire 与回程 `exit_reply`；DNS 变体单独 unavailable；IPC v5 可省略 `exit` 字段，`meshway status` Exit 行 | planck 隔离 4 项＋X3 套件 22 项通过（真实 Server＋三 Control＋SecureNode＋真实 NAT）；`.meshway/exit/x4/{receipt,x7b-handoff}.md` |
| 2026-10-09 code review | `.meshway/review-20261009/` | 四份只读审查（Server／node-control／dataplane-connectivity／underlay-dns），93 条 findings：remove 29、simplify 33、keep 31；一致结论：保留安全与一致性内核，不再在逐 demand publication 链上加功能，N1 是最有价值的单一改动 | `SYNTHESIS.md` |
| N0 netmap 设计 | `08410a1a`、`e9f7a47f` | `docs/v2/node/netmap-design.md`：netmap 形状（内容 digest 与 24 h 租约分离）、稳定策略／动态事实两条线、本地准入、`SecureNode::update`、邻接按需、撤销与到期、迁移、remove 清单对照（消失 26／变形 3／保留 0）、拆包 N1a–N1j；五个产品决定已记录 | — |
| TD4 低风险清理 | `ea9d626e`…`7bec72de` | review 中不依赖 N1 的 11 项：死 wrapper／不可达分支、仅测试的 pause/resume、被 streaming 取代的 Batch API、refresh 双入口、复制的一致性循环、丢弃结果的 D-Bus RPC、无消费者的 DNS Owner API、egress 重复 stage、四份 private-write 原语共用、`Shared`→`EpochContext`、`Scope`→`ProtectionScope`；净 −113 行 | planck underlay／DNS 84 项＋X3 17 项通过；`.meshway/td4/receipt.md` |
| X7b `exit/off` 验收行 | `feat/exit-acceptance` 系列 | 八阶段真跑：能力／前置／安装／capture／投影 NAT CT／续期关流／原路径回程／drain 恢复；隔离公网正例 `11.77.98.3` 经 E masquerade 的 UDP／TCP 真实往返；行保持显式选择 | planck 全长 `20261009T080945Z-9fc2ac9c` **未通过**：定位到 X4 产品缺陷（安装期间 `sample()` 1 s 超时被当作 `Report::stopped` 上报，Server 撤回出口资格，三端关流）；`.meshway/exit/x7b/{receipt,x4-sampling-blocker}.md` |
| X4b 采样超时不算 stopped | `fix/state-sample-timeout` | daemon State 采样区分 `Timeout`（跳过本轮完整上报，不改序号／observedAt／lease）与 `Closed`／真实 stopped（如实上报）；X7b 定位的"安装忙→伪 stopped→出口资格撤回→三端关流"消除 | `busy_installation_skips_state_without_withdrawing_exit_then_stopped_withdraws` 红→绿；planck X4 四项通过；`exit/off` 全长仍未通过（新阻塞：A 首流前 20 次 scope 轮换、applied 为空，归因待定）；`nat+fallback/off` 短门槛恢复后 protected relay I/O 超时（归因待定）；`.meshway/exit/x4b/` |
| T1 边级承载顺序 | `topology/transport-order` | `topology[].transports` 为有序承载偏好（重复报 `invalid_transport/repeated_transport`），省略用平台顺序 udp→tcp→vless→trojan；dialer 删除 `priority`，只表达启用与进阶选项；`DialCandidate.priority` 由边序生成，Manager 不变；compilerVersion `meshway-demand-7`，demand-6 缓存退役 | policy `tests/execution/topology.rs`、`tests/configuration/topology.rs`；server `app::tests::cache` demand-6 |

## 2026-10-11

| 工作包 | 合入提交 | 内容 | 证据 |
| --- | --- | --- | --- |
| S1 系统服务与发布 | `s1/install-release` | systemd unit（`Type=notify`、最小 capability、沙箱）、`install.sh`（下载校验／本地产物／升级幂等／卸载）、`meshway version`、`release.yml`（tag＝workspace 版本、复用 `build.yml`、`SHA256SUMS`、changelog 段作说明）；workspace 版本 `0.1.0-alpha.1` | OrbStack Ubuntu 24.04 安装／重启（含 Server 不可达）／resolver 间隙实测，`.meshway/node-lab/s1/` |
