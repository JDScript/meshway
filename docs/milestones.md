# 当前交付状态与首版剩余工作

**Mesh 私网互访与按域名 / IP 经指定路径出网同为首版核心。** 产品语义见[产品概念](/concepts)与[产品要求](/product-alignment)，这里仅列最新实现边界。

## 已交付

| 能力 | 当前边界 |
| --- | --- |
| 网络与身份 | 多网络 SQLite、显式作用域、IPAM、注册/心跳/撤销、公钥绑定；本地邮箱账户、邀请、网络 RBAC、设备归属与 App PKCE |
| 配置与策略 | 网络内 revision/CAS、JSONC 草稿、digest/私有保存/ACK；四层纯引擎及正常 Controller→Agent→Linux TUN 的 IPv4 Mesh 执行 |
| 承载 | 平台托管 Native UDP/TCP、VLESS TCP/TLS 与 WebSocket/TLS listener/凭据；保留旧 profiles 等价私有导入 |
| 数据面 | 路径绑定 Noise E2E、反向 dial、节点 DNS、daemon 诊断；认证有界 E2E resync 与运行报告/历史槽位恢复修复 |
| 观测 | 真实邻接 probe、滚动 latency history 存储/查询、UDP carrier socket STUN 与有期限上报；端到端路径测量未实现 |
| 系统保护 | Linux 正常接入/daemon/TUN 使用用途受限 socket、独立 controller/carrier 与 Journal v3；隔离 namespace 内包流、退出/恢复及资源保留已验，远端未部署 |
| Agent 领域库 | DNS、TUN、underlay、连接、节点执行、诊断与生成控制客户端已独立，统一消费当前 policy 执行合同 |
| 自动邻接首片 | Controller 按已授权策略的选定路径静态生成托管邻接；同逻辑邻接 UDP/TCP 主备、双向 TUN、撤回与恢复已验。首包触发、真实 NAT/直连仍未完成 |
| 状态型 Mesh 回包 | TCP/UDP/ICMP echo 在单向 grant 下可回包，反向新流仍拒绝；原路径、丢包/乱序、流水发送、容量、长连接和半关闭已验，完整出口仍待接线 |
| 远端固定链路 | 固定 `4c9a5be` 同身份直连与四地有序路径恢复复验 PASS；[证据及限制](/linux-field-validation) |
| 用户入口 | Controller 账户/成员/节点配置接口已实现，控制台由独立工作流的 `apps/console` 接入；Apple 多接入/Keychain/浏览器接入和库存已实现，隧道未实现 |

ACK 只表示校验与持久保存；心跳、配置候选、逻辑路径和邻接探测都不能替代真实业务包流。

## 当前工作

1. **优先：自动组网、主备与打洞**：接续已有 `nat-connectivity` 候选，串通当前 Controller/Agent 的发现意图、真实 socket 候选、认证协调、按需邻接和协议优先级。首个闭环为两台仅出站节点、不发布端口/手填对端候选，证明实际直连和获准中继兜底；UDP/TCP 打洞、反向拨号和中继分别验收，必经节点和 E2E 保持。[承载扩展](/transport-registry)。
2. **并行：完整出网**：状态型 Mesh 回包已关闭审查并通过组合验收，继续正式出口授权、转发/NAT和路由方案，再接域名/IP分流、规则集与撤回。正常场景或纯编译器通过不能替代异常时序与真实 TUN 验收。

本机当前 schema 与三个 Docker demo 已完成切换、六方向 TUN 互通通过；原容器与可写层工具保留。该固定底层可达验证不代表上述自动组网、打洞或出网工作已经完成。新镜像/安装入口还需固化本轮核对的运行依赖及非 root 启动准备。

## 首版剩余工作

| 工作 | 必须完成的真实闭环 |
| --- | --- |
| 系统保护 | 更多宿主/VPN与 resolver 环境、真实 endpoint/DNS/网络变化；旧资源须有可验证 ownership 才能迁移，Mesh sink 不落回底层 |
| 自动组网与优先级主备 | 按 grants/路径/可达性按需连接，直连受阻经允许连接中转，手工候选收敛为高级覆盖；不绕过必经节点 |
| NAT 连接能力 | 两端仅出站、无发布端口/手工对端候选，在可打洞环境证明真实转直连；UDP blocked 中继兜底保持 E2E。UDP/TCP 打洞、反向拨号与中继分别表达 |
| 完整出口与分流 | 公共规则+方案规则+兜底、域名/IP/规则集、出口开关、grants、NAT/回包、方案切换与撤回；Linux 原型不等于正式下发 |
| 托管发现与地区 | 平台默认观察点/发现意图未完成，仍有本地 `--stun-server` 兼容入口；GeoIP 查询/更新/来源 API 未实现，自动方案真实执行未接通 |
| 客户端 | CLI 链接登录、多接入与允许方案选择待实现；Apple 新增开发暂停，NE 包流待恢复 |

NAT 接口不能把 UDP 写成产品必选；VLESS TCP/TLS、WS/TLS 外层为 TCP，TCP simultaneous-open 的 socket 协调与上层角色尚未实现/验证。仅允许 VLESS 时不能静默启用 UDP 业务承载。IPv6 overlay、真实逐跳 trace、OIDC、邮件与 HA 分别后续推进。

## 验收与发布

- 后续联调优先已核对的 Mac Controller `https://meshway-dev-api.jdscript.dev`；构建优先 Mac Linux ARM64 / NYC AMD64，普通受限容器，不用 privileged/host network。
- 第二轮四地验证覆盖自动建连、主备及真实打洞，不能重复固定链路 PASS 代替。NYC 在防火墙后，原 KR→NYC FIB 经 WARP，第一轮不是纯公网直连/穿透证明。
- 远端持久 latency history 查询未验；持续 runtime 新鲜报告不替代历史存储查询。
- `apps/console` pure SPA 由用户另一工作流维护；本轮后端仅维护契约兼容与消费者检查，Apple 新增开发暂停。性能本批已结束，保留产物，不整分支部署 offload/NEON 实验。
- 验收走正常 Controller 下发、Agent 生命周期和真实 TUN 包流；未审定候选不部署。检查入口见[开发与验收](/development)。
