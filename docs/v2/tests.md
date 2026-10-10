# 测试清单与开发门槛

本页是全部测试与 fixture 的唯一清单：它们各是什么、验证什么、跑多久、在哪一层触发。分层与触发规则见[工程流程](/v2/process)，入口命令见[开发服务、测试与环境](/v2/development)。测试数量与文件规模经 2026-10-08 TD2 复核；T1 耗时保留 TD 的 nextest 实测（Apple M3，`mise run test:rust`，热构建），TD2 的包级 fast 墙钟另列。

## 开发时怎么约束

| 时机 | 命令 | 耗时（M3，缓存热） | 内容 |
| --- | --- | --- | --- |
| 每次改完要看结果 | `mise run test:fast -- -p <crate>` | 依 crate 而定，见下表 | 所属 crate 的 fast profile |
| 提交前 | `mise run check:fast` | ~2 分钟（macOS 首次另加约 2 分钟的 Linux 目标 Clippy，之后秒级） | 测试文件上限、fmt、严格 Clippy（macOS 另对 `aarch64-unknown-linux-musl` 跑一遍，覆盖只在 Linux 编译的 underlay／DNS／TUN 模块）、fast profile 全 workspace |
| 合入前 | `mise run check` | ~3.5 分钟 | T1 全量：含 fast 排除的慢流程与真实时间合同、codegen／drift／SDK、Console、docs |
| 合入前（触及 connectivity／control／daemon／server 执行与 State） | `mise run validate:remote --short --rows nat/off nat+fallback/off` | 提交即返回，planck 约 4 分钟 | T2 短模式门槛，结果用 `--attach` 取回 |
| 合入后 | Actions 自动 | — | T2 全矩阵、T3 固定口径，进状态页 |

fast profile 按实测维护慢测试集合，不能把耗时长等同于真实时间合同。TD3 逐项复核原 45 个名称：**A 流程与 lease 交叠 17 个、B 可用同步替代固定等待 1 个、C 真实时间／节律合同 27 个**；其中一个旧名称已失效，原集合实际排除 44 个测试。两个已报告的 lease 竞态包含在 A 类计数中。真实时间合同包括 lease 到期、HTTP 中断恢复、采样间隔和换钥过期回滚；`mise run check` 必跑。本机不跑 T2 整组、不做正式性能测量。

TD3 已将健康观察测试的失效选择器改为当前名称，覆盖其跨越 20 秒观察有效期的真实等待。按精确名称维护排除集合；改造后实测不足 5 秒的测试回归 fast，不能仅凭无 sleep 或一次高负载耗时决定去留。

全量门槛另发现 1 个原名单之外的 C 类 probe 到期／scope 交接测试，完成准备阶段重排后实测仍超过 5 秒，补入 fast 排除集合。TD3 共复核 **46 个不同测试：A 17、B 1、C 28**；当前排除集合为 46 个有效名称。

## Rust 测试：按 crate

单测 = `cfg(test)` 私有模块；集成 = `tests/`。下表保留 TD2 的 fast 排除量快照，真实时间合同按上面的 TD3 行为分类认定，不把调度／负载导致的慢测试归为时间合同。耗时是一次运行的观测值。全部自包含：需要 Server 的套件在进程内起 Server＋临时 SQLite＋随机端口，不依赖 8420 的开发服务。

| crate | 测试 | 单测／集成 | 合计耗时（TD） | fast 排除（TD2） | 验证什么 |
| --- | ---: | --- | ---: | ---: | --- |
| meshway-server | 232 | 185／47 | 263 s | 7 | 账户、网络、配置 CAS、执行发布、State／SSE、观察、HTTP 路由；app 私有套件与 `tests/http`／`observations`／`accounts` 起真实 Server |
| meshway-dataplane | 125 | 80／45 | 255 s | 7 | v5 codec、Noise／AEAD／MAC 一致性、流表／replay、流水线保序与 credit；`tests/runtime` 用静态签发的许可跑 A→R→B 进程内包流、续租、撤销、诊断 |
| meshway-policy | 86 | 0／86 | 2.4 s | 0 | 配置编译：Access、递归 routing、topology、profiles、DNS 规则 |
| meshway-control | 57 | 12／45 | 149 s | 2 | Node 控制面对真实 Server：接入、认证、刷新、换钥、SSE、回执、故障注入 |
| meshway-daemon | 55 | 44／11 | 319 s | 10 | 控制周期、恢复分类、IPC、ping；`tests/lifecycle` 起真实 Server 与 daemon 进程 |
| meshway-connectivity | 52 | 52／0 | 265 s | 16 | Manager：邻接认证、选链、协调穿透、健康、资源续接、事件队列（含真实 UDP／TCP 本机 socket）；按行为拆分，共用 Server／拓扑／Manager builder 与 socket support |
| meshway-dns | 46 | 21／25 | 7.9 s | 0 | 名称权限、查询 owner、系统 DNS 安装意图 |
| meshway-transport | 40 | 40／0 | 0.9 s | 0 | Session、framing、批量提交、scope |
| meshway-transport-vless | 37 | 23／14 | 2.8 s | 0 | VLESS TCP/TLS、WS 协议 I/O 与互通向量 |
| meshway-model | 36 | 2／34 | 0.7 s | 0 | schema、严格序列化 |
| meshway-protocol | 30 | 1／29 | 0.8 s | 0 | OpenAPI 契约类型 |
| meshway-transport-spec | 23 | 0／23 | 0.3 s | 0 | 承载声明 schema |
| meshway-configuration | 20 | 9／11 | 0.3 s | 0 | 文档解析、规范值、切片格式校验 |
| meshway-control-client | 19 | 12／7 | 3 s | 0 | 生成 Rust 客户端、SSE 解码、受保护 HTTP builder |
| meshway-ipc | 17 | 9／8 | 13 s | 1 | IPC v5 编解码、流式帧、空闲限时 |
| meshway-stun | 16 | 2／14 | 0.3 s | 0 | RFC 8489 编解码与限流 |
| meshway-cli | 15 | 0／15 | 12 s | 0 | CLI 对 daemon 的 IPC 行为（起真实 daemon） |
| meshway-tls | 14 | 1／13 | 0.4 s | 0 | ring provider、系统信任、精确叶证书 |
| meshway-runtime-fixture | 13 | 0／13 | 0.5 s | 0 | fixture 自身的信号／阶段逻辑 |
| meshway-transport-trojan | 10 | 3／7 | 0.4 s | 0 | Trojan TCP/TLS |
| meshway-diagnostics | 8 | 8／0 | 38 s | 1 | 有界测量调度与计时 |
| meshway-transport-udp、-tcp、-tun、-authority、-private-storage、desired-fixture、-serde、-logging、perf 工具 | 40 | — | 3.8 s | 0 | 各自原语 |
| **合计** | **991** | | **TD：1,339 s 求和；nextest 并行 120.7 s；命令墙钟 128.3 s** | **44** | |

上表按快照时的包列出。此后 `transport-spec`／`-tcp`／`-udp` 并入 `meshway-transport`（`spec`／`tcp`／`udp`），`tls`／`transport-vless`／`transport-trojan` 并入 `meshway-transport-stream`，`ipc` 并入 `meshway-protocol`，`diagnostics` 并入 `meshway-dataplane`，`authority` 并入 `meshway-engine`，`serde` 并入 `meshway-model`；需要 Server 的 Node crate 测试经 `tools/meshway-test-server` 启动 Server。

专用 Rust 测试／support 文件共 271 个、70,584 物理行（含空行和注释，不含产品文件内的 inline 测试模块）；其中 Server 25,911 行、transport 1,935 行、connectivity 5,957 行、policy 5,225 行。此口径与下述文件上限检查一致，不能直接与去掉注释／空行的代码行统计比较。全部文件不超过 800 行，现有白名单例外为 0。

### TD2 包级复核

同一 Apple M3 Pro、同一 target、`CARGO_INCREMENTAL=0`；每个包先预热，再测一次 `mise run test:fast -- -p <crate>` 的完整命令墙钟。前值来自 `e9c7a376`，测试选择保持一致；以下是观测值，不作性能结论。

| crate | 测试数：前 → 后 | 专用测试／support 行数：前 → 后 | fast 墙钟：前 → 后 |
| --- | ---: | ---: | ---: |
| meshway-connectivity | 52 → 52 | 6,009 → 5,957 | 47.990 s → 52.011 s |
| meshway-policy | 86 → 86 | 5,180 → 5,225 | 0.861 s → 0.874 s |

全仓专用测试／support 为 70,587 → 70,584 行、241 → 271 个文件；其中还含 DNS 既有测试 helper 的 4 行纯格式增量。文件拆分、入口与 builder 增加结构行，共用搭建代码减少重复；没有删除测试。

另有 17 个 `#[ignore]`：13 个要求隔离 Linux namespace／NET_ADMIN／TUN（由 T2 的 Linux 入口显式运行，不在本机），1 个五分钟真实 Server session 到期，1 个只读历史报告检查，2 个显式 opt-in。

## 隔离 Linux 整组（T2）

`exit/off` 是默认矩阵行；使用 `mise run validate:remote -- --ref <commit> --rows exit/off --short` 单独诊断真实 Server／daemon 出口路径。隔离拓扑、组件／进程断言边界与公网分类见 `dev/exit-fixture/README.md`，实际结果见[出口场景](/v2/node/validation#exit-scenario)。

| 入口 | 行 | 单行耗时 | 验证什么 |
| --- | --- | ---: | --- |
| `test:tun:linux` | tun | 秒级 | 真实 TUN 设备、双栈包 I/O、GSO/GRO 能力 |
| `test:packet:linux` | packet | ~9 分钟 | A→R→B 受保护包流：小包、满 MTU、MTU+1 拒绝、多 MSS TCP、原路径回复、内嵌真实 daemon 整组 |
| `test:runtime:linux` | udp/off、tcp/off、udp/resolv-conf、udp/resolved、nat/off、nat+fallback/off、nat/resolved | 各 ~8 分钟（NAT 行失败时更短） | 真实 meshwayd／CLI＋Server：接入、续租、125 秒流、HTTP 中断恢复、Access 撤销恢复、lease 失效、离线清理；DNS 行加系统解析；NAT 行加双路由器穿透与 pcap 证据 |
| `test:runtime:linux --traversal exit` | exit/off | 短模式诊断／全长验收 | 真实 Controller／A／R／E、隔离公网分类目标与私有 LAN；能力／安装、NAT tuple、完整路径、长流／仅接收、撤回／到期／journal／精确清理；内部 ACK／截止边界单独调用正式组件测试 |
| `test:runtime:linux` | vless/off、trojan/off | 各 ~8 分钟 | 同一 daemon 共同场景；A/B 只经所选 TCP/TLS 承载拨入 R，SecureNode 包流与原路径回复；独立 TLS 材料与精确叶证书验证 |

11 行由 `validate:matrix` 统一定义；planck（32 核）默认并发 11 行同时执行，全量约 13.5 分钟（其中构建约 4 分钟，最长行约 9.5 分钟），此前并行 4 约 28 分钟；`--short` 把流缩到约 30 秒只作诊断。当前每行状态见[验收范围](/v2/node/validation)。

## Fixture 与工具

| 目录 | 性质 | 被谁用 |
| --- | --- | --- |
| `tools/meshway-desired-fixture` | 测试／基准用的编译期望内容与节点密钥材料；不签发任何租约或许可 | dataplane `tests/runtime`、circuit-perf、dataplane-perf |
| `tools/meshway-packet-fixture` | packet 整组的 A／R／B 进程、rendezvous、满 MTU 用例 | `verify_packet.py`、runtime-fixture |
| `tools/meshway-runtime-fixture` | runtime 整组的 controller／actor 编排、信号、阶段、请求计数 | `verify_runtime.py` |
| `tools/meshway-circuit-perf`、`-dataplane-perf`、`-transport-perf`、`-crypto-bench` | L0／L2–L4／L1／原语测量程序 | `perf:*` |
| `dev/node-lab` | 常驻 lab 镜像、compose、运行时装配 | `node:lab`、packet／runtime 的 OS 镜像来源 |
| `dev/dns-fixture` | resolved／resolv-conf 场景的 OS 支持与 namespace 脚本 | runtime DNS 行 |
| `dev/traversal-fixture` | NAT 路由器、pcap、穿透证据分类器 | runtime NAT 行 |

每个 fixture 只提供 OS、namespace、编排与证据读取；业务行为全部走正式 Rust 实现，不复制实现。

## 收敛规则

- 新测试先问归属：原语与状态机进单测；跨 crate 的行为进所属 crate 的 `tests/<suite>/`；需要内核／namespace 的进 T2 行，不新增入口。
- 同一行为只保留一处断言最严格的测试；工作包补回归测试时先搜已有套件。
- HTTP 层断言路由、鉴权／主体边界、序列化、错误码与 SSE 帧；CAS、lease、吊销、成员生命周期、执行发布和回执排序由 app／所属状态机套件负责。删去重复覆盖时必须指出保留的测试及其更严格的断言；共享场景不等于重复覆盖。
- Rust 专用测试文件（含 `tests/` 下 support、`tests.rs`、`*_tests.rs`）最多 **800 行**，按行为拆为普通子模块，共享 fixture 归所属套件的 `support.rs` 或 `support/`。不得压行、扩大生产可见性或用 `include!`／`#[path]` 绕过限制。
- `mise run check:rust`（因此也包括 `check:fast`／`check`）运行 `scripts/check_test_files.py`，同时扫描已跟踪及未忽略的新文件。`scripts/test_file_limits.json` 只记录既有超限文件的冻结上限；不能增长，降到 800 行后删除条目。Server 与 transport 不允许例外。
- 真实时间合同测试保持真实时间，并注明到期语义；实测 ≥5 s 的测试纳入 `profile.fast` 排除名单，改造后实测 <5 s 则移出（名单在 `.config/nextest.toml`）。耗时变化不改变 A／B／C 的行为分类。
- **测试不得假设流程总时长小于任何产品 lease**。需要跨 lease 的流程由测试主体按产品方式续租，遵守返回的 `renew_after`／续租窗口；续租引发发布时，先续租再获取上报 configuration，不能把旧输出换绑到新权限。无关准备工作在创建短期对象前完成，针对创建、重试和取消的断言靠近相应操作。
- **等待事件用条件，不用固定 sleep**：优先 Notify、事件流或有截止的条件轮询，上界来自产品合同或实际返回的 deadline。真实到期、节律与负向观察仍使用真实时间，并在测试顶部注明所等常量与不能缩短的原因；到期前的必要准备必须在其一半预算内完成，否则重排。禁止修改产品时间常量来迁就测试。
- 不新增依赖外部服务的测试；需要 Server 的测试在进程内起。

## 结构变更记录

| 日期 | 变更 |
| --- | --- |
| 2026-10-08（TD2） | Connectivity 的接收／回执、重接、穿透与 policy 的配置／执行／准入按行为拆分；共用真实 Server／拓扑、Manager、socket、STUN 与已应用 gateway 的搭建代码。六个文件上限例外清零；connectivity 52 个、policy 86 个测试均保留，policy 测试正文不变。 |
