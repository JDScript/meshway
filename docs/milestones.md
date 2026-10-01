# 后续里程碑

2026-10-01 路线建议，非本轮全量实现承诺。已有功能与官方依据见 [Tailscale 对照](./tailscale-alignment.md)，这里仅维护可执行交付。已完成的 network v7、管理员 tags、端点 ACL/精确路径编译、IPv4 TUN、多 transport/反向 dial、双编辑器与前端 spacing/IA 不再作为待开发里程碑。

## 产品对齐后的交付优先级

用户已确认首版同时完成 **Mesh 私网互访与按域名 / IP 的互联网分流**。平台托管全部承载配置、默认按需建连、统一节点模型是共同前提。公网出口与域名上下文进入首版闭环，不再以旧 M4 编号作为延后交付的依据。

控制面还需支持类似 tailnet 的多网络隔离。这是已确认的产品基础要求，将与接入、配置下发和策略作用域共同设计；既有单网络、身份与地址必须无损迁移。

产品以 Tailscale 为基线，扩展自定义 transport 与四层策略（连接 / 授权 / Mesh 内路由 / 出口路由），详见[产品对齐记录](/product-alignment)。

### 首版交付阶段

| 阶段 | 内容 | 依赖 |
| --- | --- | --- |
| **P1a 多网络** | 节点、接入密钥、IPAM、DNS、策略、连接与运行报告按网络隔离；现有网络无损迁移为第一张网络；所有 API 在服务端限定所选网络，Web 增加网络选择；设备一次只接入一张网络 | — |
| **P1b 策略引擎 v3** | 四层 schema、校验与编译：连接图 allow/block（阻断优先）、grants 并集、Mesh 内路由具体度与冲突检测、出口路由最长匹配；在允许的连接图上自动选路与中转；v2 → v3 迁移及无法等价部分的报告。纯 Rust，暂不接入 controller | — |
| **P1c 出口数据面** | 出口 NAT、状态型回包、撤回终止存量流、DNS 泄漏边界的 RFC，以及 Linux 隔离 Docker 原型与验收 | — |
| **P2a 承载托管** | 添加节点时可选 listener，节点页可修改；平台生成并下发参数与凭据；Agent 不再依赖本地 profiles，现有节点无损迁移 | P1a |
| **P2b 用户与登录** | 用户、设备归属、按用户 / 组授权；Agent 链接登录（类似 `tailscale up`），并入 App 浏览器 SSO | P1a、问题 10 |
| **P3a 策略 v3 上线** | controller 与 Web 接入四层策略，图形与 JSONC 共享草稿 / CAS，迁移 live 策略 | P1a、P1b |
| **P3b 自动建连** | 按授权流量与路由按需建立连接、自动中转；手工连接页收敛为高级覆盖 | P2a、P3a |
| **P3c 出口与域名分流** | 出口路由接入数据面，域名上下文与默认出口 | P1c、P3a |
| **P3d 打洞** | 原 M3：候选分发、认证探测与打洞，失败时经中转 | P3b |

P1 三项文件边界基本独立，并行推进；协议、生成产物、根 manifest 与 lockfile 由协调者合并。下面 M0–M4 保留技术依赖与已实现证据。前端收敛、统一开发入口和性能定位继续并行。

## M0 · 显式本地节点 DNS（已实现并验收）

- **可见交付**：显式开启 loopback:5353 后，真实 UDP/TCP A/AAAA/PTR 可查；这是 nameservice，不改变 OS resolver。
- **协议/UI/CLI**：沿用 node DNS snapshot/version/digest/limits；网络页显示 intent，Agent `run --dns-listen` 显式启用。无新增 DNS ACK；ACK saved 不表示 applied。
- **不变量**：fresh lease 与 revision coherent；禁递归、禁公网监听、禁把 AAAA 当 IPv6 数据面。
- **验收**：UDP/TCP 正反查、禁用/rename/节点撤销、超限/无效 digest、长 HTTP 阻塞期间 lease 仍到期；旧记录不因重启而恢复服务。
- **证据**：完整本机隔离 Docker 验收已通过，详见 [DNS](/dns#已验证的本机隔离验收)。待发送旧响应的代际 fencing 与禁用后持久 self-address pin 已覆盖行为回归；节点 DNS 不会因此自动接管系统 resolver。

## M1 · 容器内可用 DNS + 只读 CLI（已交付受控容器切片）

- **可见交付**：隔离 Linux 容器内 `getent hosts <短名/FQDN>` 后真实访问已授权 Mesh IPv4 服务；`status --live --json/--peers` 与 `dns status/query` 给出可解释诊断。
- **协议/UI/CLI**：DNS adapter 在同一 netns 提供 53 端口，接入现有 5353 nameservice，添加 search domain；外部查询走接管前的上游，不递归计算。区分 intent/saved/applied、DNS listener/OS integration、运行代际/lease、邻接 transport 与业务方向。网络页集中配置，节点详情展示执行结果。
- **不变量**：显式 opt-in，只管理容器自身 resolver；保存/恢复仅自己拥有的配置；拒绝自身上游循环，bootstrap 与 Mesh DNS 解耦。DNS 可见不等于 Access allow；撤回不允许 Mesh 流量落入底层默认路由。
- **验收**：glibc 与实际应用短名解析、A/AAAA/PTR、TCP 查询；外部域名保留原解析；重复 apply/down/失败回滚；上游指回自身报错；改名旧 alias 消失；lease 到期停止旧 DNS；daemon 停止、DNS 端口占用/权限不足均真实报告。
- **已交付**：受控容器 resolv.conf adapter 与真实 glibc / curl / TUN 验收通过，含外部搜索域诱饵、拒绝访问、过期和所有权恢复；网络 / 节点 DNS 名称与诊断入口已归并。见 [DNS](/dns#容器内系统解析接入)。
- **后续边界**：adapter 的 zone / 地址池需显式 stop/reapply；systemd-resolved 平台探测和 controller 汇总节点 DNS 运行报告仍需后续切片。Web 目前显示保存意图与本地诊断入口。

### 已启动的独立 CLI slice

已实现 `status --live`、可读摘要 / JSON、peer 视图、`whois`、邻接 `ping --json` 与 `dns status/query`，见 [CLI](/cli)。DNS-only 与 managed 模式共用私有 daemon IPC；DNS 查询使用真实 UDP/TCP，状态和答复证据分开呈现。

`apps/agent/src/diagnostics.rs` 的只读 IPC 复用单一 Rust 契约与封闭 enum；当前 JSON 使用 `schema_version`、证据来源、generation/revision、lease 和明确的未知状态。`whois` 查询当前可信 inventory 中的节点 ID / 名称 / Mesh IP，不提供尚未发布的 tag 映射。无 daemon 不回退 cached-live；同 UID、私有目录/socket、帧大小/并发/timeout 限制保留。后续跨语言消费者接入前再生成其 schema，不手写第二套 DTO。

当前兼容退出码为 0 成功、1 运行失败、2 参数错误；更细的失败种类放在版本化 JSON error code，stdout 只放结果，stderr 放诊断。验收覆盖 daemon 在/停/代际切换、过期 snapshot、无 peer、畸形/超大 IPC、权限不符、JSON 单对象及旧 ping 协议兼容。现有 `ping` 继续明确 neighbor-probe；up/set/down/login/logout、netcheck 网络探测、trace 或新管理 mutation 仍是后续切片。

## M2 · Endpoint 发现与 netcheck（承载观测与有期限上报已验收）

- **可见交付**：知道 UDP 是否可用、观测到的 underlay endpoint、映射是否随目的变化、探测时间与失败原因；尚不宣称打洞成功。
- **协议/UI/CLI**：typed observation 带来源（local/manual/STUN）、时间/有效期与候选状态；节点能力、监听、endpoint、dial policy 各自建模。`netcheck --json` 与节点连通性面板共用诊断语义；IPv6 探测能力不等于 IPv6 overlay。
- **不变量**：STUN 不证明节点身份；候选必须经认证 channel 确认；unknown 不写成 false；候选变更不改 logical path。控制 HTTP/IPC 类型从契约生成，Web 校验运行时 schema。
- **验收**：独立本机 fixture 两个 STUN 观察点、对称映射、受限 NAT、UDP blocked、endpoint 变化；检查旧候选过期、测量超时/不可测值，不触碰宿主路由或提供的远端。
- **依赖/并行**：依赖 M1 诊断边界；发现实现、schema/UI、fixture 可按文件所有权并行，fixture 网络执行串行。

第一片已交付独立临时 UDP socket 的 CLI 观测、两个实际 STUN 观察点和容器内 kernel NAT 的 7 场景验收，详见 [Netcheck](/netcheck)。第二片已将采集绑定到真实 carrier socket，接入有身份、代际与期限的报告及 Web 展示，并通过四容器 8 阶段验收，见[承载 endpoint 观测](/endpoint-discovery)。Web `/monitoring` 保留原 URL 与节点上下文深链，将独立诊断与真实承载报告分别呈现。

接下来的独立交付是面向已获准邻接的候选分发、peer 连接验证与自动打洞。当前观测不会自动添加或改写连接配置，也不表示 peer 已经可达。

## M3 · 本机打洞 + 同邻接 fallback（待开工）

- **可见交付**：可打洞场景建立真实通道；困难 NAT/UDP blocked 有明确失败或同 peer 的备用通道，并可观察切换/恢复。
- **协议/UI/CLI**：认证探测、限速/重放防护、候选优先级主备、退避/重测及 relay binding/lease；显示 selected transport、underlay relay、逻辑邻接与 mandatory via。CLI probe 报本次实测，不输出假 trace。
- **不变量**：UDP→TCP/VLESS→授权 relay 都只替换同一邻接的承载；E2E 与完整路径授权不降级，不跳必经节点，不做同流聚合。先验证现有反向 VLESS，再加入最小 relayfallback。
- **验收**：M2 fixture 扩展受限/双对称 NAT、UDP blocked、反向 VLESS、endpoint 重绑定、relayfallback same-peer；检查优先级恢复、撤销/lease 终止旧 channel、relay 不读端点明文。无法直连也是有效证据。
- **依赖/并行**：依赖 M2；relay 适配和打洞可独立实现，契约统一后集成。本阶段仅独立 Docker netns，无 host network/privileged、宿主 DNS/防火墙修改；完成后才安排远端主机联测。

## M4 · IPv6 / 出口 / 真实 trace（后续分片）

- **状态/交付**：均未由 AAAA、relay 或路径预览完成。IPv6 首片交付真实双栈访问；出口片交付显式授权的公网 egress 与 DNS；trace 片交付可验证的 overlay hop 诊断。
- **协议/UI/CLI**：IPv6 地址分配/route/packet/MTU 与 capability；出口职责、选择/批准、默认路由及 exit DNS 分开；trace 定义 authenticated hop 观测或 TTL/ICMP 映射。UI 标注预览与实测，CLI 区分 neighbor/overlay/ICMP 层。
- **不变量/验收**：IPv6 双栈 ACL、PMTU、lease fail-closed 后才允许声称可达；出口需 stateful return 设计、NAT/回包/撤销/DNS leak 隔离验收；trace 需多 hop/拒绝/超时/反向 dial 验证，不复用预览结果。
- **依赖/并行**：IPv6 依赖 M1/M3，可与 trace 协议研究并行；出口依赖职责授权及状态回包片，真实 trace 依赖诊断与转发层，不强制等出口完成。

## 排期与语义决策

多网络已由用户确认纳入首版产品基础，成员、IPAM、授权和配置均需按网络隔离。已有独立 controller / stable network ID 可继续支持隔离 fixture 与性能研究；多网络实现同时推进真实作用域与无损迁移。同一设备一次只接入一张网络、可切换（参照 Tailscale 客户端），不做多网并行传输；具体管理权限仍在主线程逐项对齐。

tags 管理与前端整体重组可持续并行：网络设置归网络，节点身份/标签归节点详情，Access 与 Routing 各自编辑、共享诊断入口；只补真实撤回/冲突状态，不重复实现 tag catalog 或双 editor。验收 tag removal/revoke、新成员在默认 allow/reject 下的匹配、未知 tag/group 报错、图形/文本切换保留无效草稿与注释。

仅以下语义变化需用户另行确认，当前路线不依赖确认：

1. 是否未来引入 user-owned/device-tag 身份替换与委派 tagOwners；推荐现阶段维持管理员分配属性。
2. 是否将来允许全局 DNS override/split upstream/exit DNS 接管；推荐先保留原上游、仅容器内 opt-in。
3. stateful return 的授权、超时与策略撤回如何终止存量流；推荐独立 RFC/验收后实现，不从显式双向推导。
