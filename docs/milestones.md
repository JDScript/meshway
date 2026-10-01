# 后续里程碑

2026-10-01 路线建议，非本轮全量实现承诺。已有功能与官方依据见 [Tailscale 对照](./tailscale-alignment.md)，这里仅维护可执行交付。已完成的 network v7、管理员 tags、端点 ACL/精确路径编译、IPv4 TUN、多 transport/反向 dial、双编辑器与前端 spacing/IA 不再作为待开发里程碑。

## M0 · 显式本地节点 DNS（已实现并验收）

- **可见交付**：显式开启 loopback:5353 后，真实 UDP/TCP A/AAAA/PTR 可查；这是 nameservice，不改变 OS resolver。
- **协议/UI/CLI**：沿用 node DNS snapshot/version/digest/limits；网络页显示 intent，Agent `run --dns-listen` 显式启用。无新增 DNS ACK；ACK saved 不表示 applied。
- **不变量**：fresh lease 与 revision coherent；禁递归、禁公网监听、禁把 AAAA 当 IPv6 数据面。
- **验收**：UDP/TCP 正反查、禁用/rename/节点撤销、超限/无效 digest、长 HTTP 阻塞期间 lease 仍到期；旧记录不因重启而恢复服务。
- **证据**：完整本机隔离 Docker 验收已通过，详见 [DNS](/dns#已验证的本机隔离验收)。待发送旧响应的代际 fencing 与禁用后持久 self-address pin 已覆盖行为回归；节点 DNS 不会因此自动接管系统 resolver。

## M1 · 容器内可用 DNS + 只读 CLI（CLI 并行推进）

- **可见交付**：隔离 Linux 容器内 `getent hosts <短名/FQDN>` 后真实访问已授权 Mesh IPv4 服务；`status --live --json/--peers` 与 `dns status/query` 给出可解释诊断。
- **协议/UI/CLI**：DNS adapter 在同一 netns 提供 53 端口，接入现有 5353 nameservice，添加 search domain；外部查询走接管前的上游，不递归计算。区分 intent/saved/applied、DNS listener/OS integration、运行代际/lease、邻接 transport 与业务方向。网络页集中配置，节点详情展示执行结果。
- **不变量**：显式 opt-in，只管理容器自身 resolver；保存/恢复仅自己拥有的配置；拒绝自身上游循环，bootstrap 与 Mesh DNS 解耦。DNS 可见不等于 Access allow；撤回不允许 Mesh 流量落入底层默认路由。
- **验收**：glibc 与实际应用短名解析、A/AAAA/PTR、TCP 查询；外部域名保留原解析；重复 apply/down/失败回滚；上游指回自身报错；改名旧 alias 消失；lease 到期停止旧 DNS；daemon 停止、DNS 端口占用/权限不足均真实报告。
- **依赖/并行**：DNS adapter 依赖 M0；CLI 与 UI 可独立并行，协议/根 manifest/lock 合并归协调者。第一片先用受控容器 resolv.conf adapter，systemd-resolved 平台探测后续补齐。

### 已启动的独立 CLI slice

本轮已实现 `status --live`、可读摘要 / JSON、peer 视图、`whois` 和邻接 `ping --json`，见 [CLI](/cli)。DNS status/query 等 DNS 运行边界稳定后接入；容器 OS resolver adapter 仍是下一片。

在已有 `apps/agent/src/diagnostics.rs` 私有 Unix socket 上增加**只读** live-status/DNS-status 请求，复用单一 Rust 契约与 enum/严格校验；生成 schema 给后续消费者，禁止 CLI 与 daemon 各自手写 DTO。保留原 saved `status` 默认输出，新增 `--live`、`--json`、`--peers` 与 human summary；`dns query` 发真实本地查询，`whois` 只查当前可见 IP→node/tag 映射。JSON 带 schemaVersion、source、observedAt、generation/revision、lease、运行状态与 unknown 字段；无 daemon 不回退 cached-live。沿用同 UID、私有目录/socket、帧大小/并发/timeout 限制，不读取或输出私钥。

当前兼容退出码为 0 成功、1 运行失败、2 参数错误；更细的失败种类放在版本化 JSON error code，stdout 只放结果，stderr 放诊断。验收覆盖 daemon 在/停/代际切换、过期 snapshot、无 peer、畸形/超大 IPC、权限不符、JSON 单对象及旧 ping 协议兼容。现有 `ping` 继续明确 neighbor-probe；up/set/down/login/logout、netcheck 网络探测、trace 或新管理 mutation 仍是后续切片。

## M2 · Endpoint 发现与 netcheck（待开工）

- **可见交付**：知道 UDP 是否可用、观测到的 underlay endpoint、映射是否随目的变化、探测时间与失败原因；尚不宣称打洞成功。
- **协议/UI/CLI**：typed observation 带来源（local/manual/STUN）、时间/有效期与候选状态；节点能力、监听、endpoint、dial policy 各自建模。`netcheck --json` 与节点连通性面板共用诊断语义；IPv6 探测能力不等于 IPv6 overlay。
- **不变量**：STUN 不证明节点身份；候选必须经认证 channel 确认；unknown 不写成 false；候选变更不改 logical path。控制 HTTP/IPC 类型从契约生成，Web 校验运行时 schema。
- **验收**：独立本机 fixture 两个 STUN 观察点、对称映射、受限 NAT、UDP blocked、endpoint 变化；检查旧候选过期、测量超时/不可测值，不触碰宿主路由或提供的远端。
- **依赖/并行**：依赖 M1 诊断边界；发现实现、schema/UI、fixture 可按文件所有权并行，fixture 网络执行串行。

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

多 network/多租户**不应阻塞 NAT 实测**：已有 stable network ID 足够隔离 fixture；每个快照/endpoint/binding 保留 network scope。多租户涉及登录、IPAM、授权和数据分区，本轮未确认需求，另立产品任务。

tags 管理与前端整体重组可持续并行：网络设置归网络，节点身份/标签归节点详情，Access 与 Routing 各自编辑、共享诊断入口；只补真实撤回/冲突状态，不重复实现 tag catalog 或双 editor。验收 tag removal/revoke、新成员在默认 allow/reject 下的匹配、未知 tag/group 报错、图形/文本切换保留无效草稿与注释。

仅以下语义变化需用户另行确认，当前路线不依赖确认：

1. 是否未来引入 user-owned/device-tag 身份替换与委派 tagOwners；推荐现阶段维持管理员分配属性。
2. 是否将来允许全局 DNS override/split upstream/exit DNS 接管；推荐先保留原上游、仅容器内 opt-in。
3. stateful return 的授权、超时与策略撤回如何终止存量流；推荐独立 RFC/验收后实现，不从显式双向推导。
