# Meshway

Rust 控制面与 Agent、React pure SPA 控制台、VitePress 文档站。

## 项目边界

- `apps/controller`：控制面服务。`apps/agent`：节点进程。
- `crates/`：按清楚职责拆分的 Rust 库。用户明确支持把 Agent 内可独立构建、测试的能力抽出（如 `agent-dns`），不为压低 crate 数量把实现堆回 App；也不按临时阶段机械拆包。Rust 生成客户端位于 `crates/control-client/src/generated`，由 Agent 与 Apple core 直接消费。
- `apps/console`：pure SPA 控制台，由用户独立工作流维护。旧 `apps/web` 已按用户要求删除。`apps/apple`：iOS / macOS App。`packages/`：生成的前端协议与客户端。
- `docs`：VitePress 产品设计、架构和开发文档。
- `references`：独立的上游参考 checkout；研究版本见其 README，默认不修改。
- 旧 Meshing 项目已废弃，不将其代码、记忆或合同作为本项目依据。

## 协议与类型安全

- 用户最新决定：项目仍处于大规模开发期，当前策略统一命名为 `policy`，不保留 `policy-v3`、`policy_v3`、`legacy_v2` / `mesh_v3` 双轨或为旧策略构建兼容层。可破坏式更新当前契约并同步所有现行消费者；数据库采用当前 schema 初始化，允许在协调切换时重置本机开发数据，不再以旧数据无损迁移作为这次收敛前置。重置是明确部署操作，不能让普通启动因任意数据库错误静默清库。现行鉴权、网络隔离、CAS、密钥/凭据保护与同 revision 执行仍须保持。
- Rust 契约及实际路由声明是 HTTP API 的权威来源，生成 OpenAPI 和客户端。
- Controller ↔ agent、controller ↔ web 均使用生成接口，不手工复制 DTO 或客户端 endpoint。
- Web 对外部响应使用生成的运行时 schema 校验；TypeScript 类型声明不能代替校验。
- 禁止通过 `any`、未验证的类型断言、宽泛 JSON 值掩盖协议不一致。
- 修改协议必须同时重生成产物并通过 drift check；不要直接编辑生成文件。
- WASM 可用于后续共享策略逻辑，不是 HTTP 通信的必需依赖。

## 实现与验证

- 用户明确强调代码洁癖：源码组织、命名和职责须干净可解释；关注 `apps/controller/src/store` 中散落的 tests、`policy_v3` 与 `schema_v*`。完整 dump / 审查不等于已完成源码清理，不再混报。生产领域模块按职责命名，避免阶段名、临时修复名和重复实现。当前权限、数据完整性、所有权与生命周期回归须保留；历史迁移与旧模式 fixture 可随最新明确废弃的兼容层一并删除。
- 全仓 Rust 手写代码统一模块与测试组织：单文件模块 `foo.rs`；有子模块使用 `foo/mod.rs`，避免 `foo.rs` 旁再开仅含子测试的 `foo/`。私有实现单元测试作为所属模块的 `#[cfg(test)]` 子模块；公共 API 黑盒测试放 crate 顶层 `tests/`，入口遵守 Cargo 自动发现（`tests/*.rs` 或 `tests/<suite>/main.rs`），共享 helper 放子目录。不得为搬测试扩大生产可见性、复制实现或用临时 path/autotests 配置绕过边界；生成代码与 vendor 按各自权威来源维护。
- DNS、TUN、underlay、连接运行、节点执行、诊断与生成控制客户端分别位于 `agent-dns`、`agent-tun`、`agent-underlay`、`agent-connections`、`agent-dataplane`、`agent-diagnostics`、`control-client`。库不反向依赖 App，底层加密/转发仍由 `dataplane` 唯一实现。Apple core 直接消费 control-client，不再为 HTTP 客户端依赖 Agent。
- Connections 库不持有 TUN/上层 PacketPlan；Diagnostics 经窄 provider 获取状态/探测。App 负责跨组件编排、HTTP、私有身份/profile 文件、CLI/IPC 投影与日志，underlay 库负责 socket/capture/sink/journal。拥有的 TCP 也须经过实际用途/代际/namespace/mark acceptance；拆分不得静默开启 TLS 默认 features 或扩大生产 test helper 可见性。Agent 已统一消费当前必填 execution snapshot，库存不携带业务授权；单一 policy 的正常三节点 Linux TUN、必经中继、撤回与同身份恢复已验，自动组网和完整出口仍未交付。
- 文档只保留当前用法、架构、契约和真实未完成项；合并重复说明，删除过时交接、逐次修复流水账和已被权威生成物替代的旧 dump。Git 历史与私有原始证据负责追溯，不持续新建清理报告 / changelog；必要的现行安全和兼容边界保留。
- 前后端按契约推进真实纵向功能；空状态和错误状态必须真实，不伪造节点、流量、健康状态。
- 将提案、用户已确认需求和已实现功能明确区分。控制面注册和心跳不代表隧道已建立。
- 管理员、注册令牌、节点凭据的权限分开；Web 的每个管理 mutation 都必须在服务端检查登录身份。
- 注册令牌与节点身份在同一事务内绑定；幂等恢复不能换身份或恢复已撤销节点。本地凭据原子、私有地保存，不输出到普通日志或 Web 客户端。
- 端到端加密优先；逐跳可信路由是显式的架构选择，不允许因连接失败静默降低加密语义。
- Multi-transport 切换与逻辑路径变更是不同操作，不得为了可达性偷偷绕过必经节点。
- 建连方向与业务方向独立：下一跳可以主动 dial 当前节点，已认证的双向通道供两个方向复用；通道双向不代表访问授权双向。
- 节点能力、监听器、可达 endpoint、主动连接策略与中继 / 出口职责分别建模。代理 UUID / password 不替代 Meshway 节点身份。
- 用户已明确：Agent 首次接入后，V2Ray / Shadowsocks 等协议配置、凭据、监听及后续变更由平台管理和下发，日常操作应在 Web 完成；手工维护节点 profiles 不是目标产品流程。具体配置体验正在主线程逐项对齐，未确认部分不能擅自当作需求实现。
- 用户已确认这些协议用于 Meshway 内部传输承载；后续终端设备通过 Meshway App 接入。普通代理客户端直接接入、代理账户及订阅服务不属于当前产品目标。
- 用户已确认节点邻接默认按访问授权、路径规则和可达性按需自动建立，并支持 Web 手动约束；无需先手工配置每对节点连接。自动建连不改变 Access 授权，也不能绕过明确的必经节点；当前手工连接流程需逐步收敛为高级入口。
- 用户已确认承载默认由平台提供协议优先级并自动选择 / 主备切换，默认参数与凭据由平台生成和下发，Web 支持允许协议、优先级与逐段约束覆盖；添加节点时必须提供可选 listener 配置入口，后续可在 Web 修改，不能因自动化隐藏这项控制能力。
- 用户要求统一节点模型，以简单逻辑为准：不按服务器 Agent / App 划分节点类别或默认中继资格，不额外引入一套中继角色开关；节点在路径中的职责由路径 / 任务及现有授权表达，平台依据实际能力和可达性选择。底层能力差异留在能力模型内，不变成用户必须维护的产品分类。
- 用户已确认首版同时覆盖 Mesh 私网互访和按域名 / IP 经指定路径访问互联网，两者同为核心闭环；公网出口与域名分流应进入首版交付主线，不再仅按旧技术里程碑排到后期。具体授权、路径与状态回包边界仍按已确认语义实现。
- 用户已确认一个 controller 管理多个相互隔离的网络，采用类似 tailnet 的产品边界；成员、地址、DNS、Access、路由及相关配置按网络隔离，Web 操作限定在所选网络。v9 多网络存储、显式类型化作用域与既有数据无损迁移已实现；Apple 保存隔离多接入及活动网络切换已实现，CLI 尚待实现。目标为同一设备一次只启用一张网络、不同时在多网传输，Apple 当前没有实际隧道可切换。
- 用户已确认需要用户身份：设备归属登录用户，服务器类设备可用预分配 tag 的接入密钥；Agent 提供类似 `tailscale up` 的链接登录，App 使用系统浏览器 SSO。登录 / 接入凭据与管理员权限保持分离。首版账户由 controller 本地管理：邮箱 + 密码注册登录，密码仅存安全哈希；OIDC 后续再接，不阻塞首版。不开放自助注册，仅凭管理员 / 网络 Owner 生成的邀请链接注册并加入指定网络，现有管理员迁移为第一个 Owner；首版不发邮件，邀请与重置链接在 Web 生成后手动转交。
- 本地账户仍须提供 Meshway 内部统一浏览器登录与设备授权：Web/App/CLI 共用 controller 用户体系与 Web 会话，App/CLI 不收集密码、不取得浏览器会话或管理员凭据，只取得绑定本次接入的一次性结果并建立节点身份。P2b 需将用户会话、网络成员权限、设备归属接入已有 App PKCE 兑换；CLI 链接登录复用同一授权服务，返回结果的方式单独适配。外部 OIDC 是未来认证来源，不是完成上述 SSO 的前提；不隐含要求实现通用第三方 OAuth/OIDC 授权服务器。
- 用户已确认产品定位：常规场景以 Tailscale 为基线，Meshway 的差异是可插拔的自定义 transport 承载，以及按域名 / IP 经有序指定路径的 routing。网络、设备 / 用户、接入、授权、DNS、出口、控制台与客户端等产品定义默认参照 Tailscale 官方做法；只有这两项扩展需要时才偏离，偏离须说明理由。既有确认项与 Tailscale 冲突时逐项向用户复核，不擅自推翻。
- 用户已确认 Apple 终端 App 位于 `apps/apple`：SwiftUI 多平台 + NetworkExtension Packet Tunnel，Rust 核心经 UniFFI 复用，FFI unsafe 仅限受审计的桥接 crate。App 经系统浏览器跳转 controller Web 完成用户登录 / 授权，以一次性 PKCE 结果和本地持久化节点身份接入。用户账户、网络权限、设备归属、Keychain、多接入、生成客户端兑换与心跳、只读清单已接通；隧道与 NE 包流未实现。用户最新决定暂停 Apple 新增开发，优先 Linux 自动组网、打洞及跨地域 Docker 联测；保留已有接口兼容与必要消费者检查，真机 NE 待后续恢复及签名条件满足。
- 用户已确认先推进 Linux 传输实验；协议按编译期插件边界扩展，批量与 buffer 所有权属于公共接口。实验密钥、授权和网络必须与现有控制面身份及运行数据隔离。
- 用户最新再次强调纽约位于防火墙后，质疑固定可达链路联测对真实打洞的价值，要求按已有 Tailscale 对照推进核心连接能力。原KR→NYC配置目标128.238.177.131:28443但KR转发FIB为CloudflareWARP/table65743；Docker端口暴露不证明上游防火墙放行，原PASS只覆盖固定底层可达下TUN/E2E/多跳/恢复。P3b/P3d最小闭环与underlay接线独立并行：两端仅出站，无发布端口/手工对端候选，正常Controller下发、认证探测/打洞、实际直连与UDP blocked中继兜底；不以既有VPN或STUN映射冒充Meshway直连，不承诺穿透任意防火墙。
- 用户进一步质疑把打洞限定为UDP，以及仅允许VLESS时是否不能打洞。通用候选/协调接口不得把UDP写成产品必选：NAT穿透取决于外层socket能力，当前VLESS TCP/TLS和WS/TLS外层为TCP；TCP simultaneous-open在部分环境可能可用，但Meshway尚未实现/验证其socket协调与上层角色接线。UDP打洞、TCP打洞、反向拨号和中继分别表达，不能把UDP映射推广到VLESS/TCP或静默启用被禁止的业务承载；具体TCP打洞交付范围不因工程上优先UDP而被擅自排除。
- 用户进一步要求审查 multi-transport 扩展架构，重点收敛 controller/agent 的 transport registry、控制面提供的可用目录与配置校验、以及 Agent 版本/能力协商。审查需明确控制面允许使用的协议与 Agent 实际编译支持的能力，保留 Agent 执行前的防御性和本机资源校验；不把 registry 理解为动态下载任意执行代码。具体统一注册接口与兼容迁移方案见承载扩展审查，尚未实现部分不得写成现状。
- 性能目标优先衡量 Meshway 包装的增量损失：同一协议 / 实现、资源和负载建立匹配基线，分开记录适配层、认证 / 加密、转发与 TUN 成本；不得用原生 L4 流代理与 L3 E2E 隧道的差值直接声称包装损耗。
- 性能深挖与新 transport 实现可委派独立 worktree 并行推进；共享 Docker VM 的压测通过 `scripts/performance_lock.py` 的同一把锁串行执行，保留 FD 继承，避免互相干扰。
- 用户反馈 Docker 原生网络约 100 Gbps、Meshway 隧道约 300 Mbps 且中转更低，属于待复现的用户测量。优先排查真实直连 / 多跳吞吐瓶颈；区分环境基线、完整隧道成本与包装增量，保留负载、CPU、MTU、并发和加密条件。
- 用户已确认首版 multi-transport 采用优先级主备；按流负载分担和同流多路径聚合不属于首版要求。
- 用户已确认四层策略模型。连接、授权、Mesh 内路由属于整张网络的一套配置，对所有设备相同；出口路由按路由方案组织。权限类（连接、授权）与规则顺序无关，路由类（Mesh 内路由、出口路由）均为从上到下的有序 first-match：
  1. 连接：listener 决定可被拨入，无 listener 只主动拨出；默认模板允许可达节点直连，node / tag 阻断规则优先于允许规则，删除默认允许即为手动连接图；实际连接按授权流量需要按需建立，直连受阻时经允许连接自动中转并保持 E2E。设备所在位置造成的可达性差异由自动中转处理，不另写配置。
  2. 授权：叠加 grants。
  3. Mesh 内路由：可选的有序必经节点，只用于“必须经过某节点”的要求；无规则时平台自动选路。用户已确认规则从上到下 first-match（不采用具体度匹配或禁止冲突），编译时提示完全被遮挡的规则，以及被前序规则部分抢先的流量示例，供调整顺序。
  4. 出口路由（Mesh 外，含公网与局域网）：以**路由方案（routing profile）**组织。设备生效规则 = 公共规则（管理员定义、所有方案都生效、排在最前）+ 当前方案的有序规则 + 方案兜底（出口 / `direct` / 不接管）。规则采用与 sing-box / mihomo 一致的有序 first-match，匹配域名 suffix（按 DNS 标签边界）/ IP 段 / 规则集，目标为候选出口（主备）或 `direct`，可指定出口前中转。
     - 当前方案由用户在 App / CLI 从管理员允许的方案中切换（属于使用选择，不是设备配置），管理员可为用户 / 设备锁定。内置方案：仅 Mesh（只有公共规则）、全局经 X（每个允许的出口自动生成，等同 Tailscale 选择 exit node）、自动（按 STUN 观测到的设备公网 IP 所属国家 / 地区选择方案，不使用控制面连接来源 IP；无法判断时走其兜底方案）。
     - 方案不能绕过授权：使用出口或网段须 grants 允许，含未授权目标的方案分配时保存报错。切换方案会中断经旧出口的存量连接。
     - 节点须由管理员在 Web 开启“可作为出口”后才能成为出口（服务器与用户设备相同），设备侧不做通告或本地配置，转发 / NAT 由 Agent 按下发自动完成。不单独做 subnet router：出口路由覆盖所有离开 Mesh 的流量，目标节点转发到其本地网络（局域网或互联网）；授权目的地可写“互联网”或具体 IP 段 / 域名后缀；“互联网”按 Tailscale 基线只含公网地址（排除 RFC1918、ULA、CGNAT、链路本地），局域网须单独授权具体网段。
     - 规则集（IP / 域名）由 controller 下载、管理、更新并编译下发，设备不下载数据集；格式与来源参考 sing-box rule-set、mihomo rule-providers；首版仅域名后缀与 IP 段，无法精确转换的条目必须报告。控制面把有序规则编译为设备可安装的等价不重叠前缀表，并提示被前序规则完全遮蔽的规则。
     - 原 routing 初始空策略 reject 废止；开发期统一使用当前策略模型，旧策略兼容与迁移按上面的最新清库/破坏式更新决定收敛。
- 用户要求设备不做本地配置：节点能力、listener、承载、出口、DNS 等全部由控制面管理和下发，设备侧只保留接入 / 登录，以及用户在允许范围内切换路由方案这类使用选择；现有 Agent 本地 profiles 与 `--dns-listen`、`--stun-server` 等参数是待收敛的实现现状，新功能不得再引入需要在设备上手工配置的入口。
- 配置 ACK 只表示 Agent 已校验并持久保存，不等同于数据面生效；逻辑路径预览不等同于真实 trace 或访问授权。
- 托管 IPv4 数据面显式启用。Access 采用端点授权模型，与 routing 的路径选择分开。当前模型参照 Tailscale grants：只写允许规则、未匹配即拒绝、规则取并集且与顺序无关，新网络默认模板为成员全部允许；旧有序 allow/reject 不再保留兼容层。selector 支持当前 network 内的 `*`、管理员分配的 tag、node，后续加入用户 / 组；每条规则可显式双向。内部完整路径授权由控制面联合编译，不跳过必经节点；TCP/UDP/ICMP echo 状态型回复基于运行时流资格沿原路径返回，不授予反向新流权限。
- Access 与 routing 的图形化及带 schema 的 JSONC 编辑器必须共享同一份草稿和 CAS 基准；无效文本、注释和未保存编辑不得被切换视图静默覆盖。
- 用户已确认控制台可整体推翻重做，按已确认产品模型重整产品逻辑与视觉，但 App 依赖的公共接入端点与 Controller 鉴权边界必须保持；独立任务按需要使用 worktree，根 manifest、lockfile 和合并验证由协调者维护。前端一致性与信息架构整理应和后端功能主线并行推进，独立 console 工作流按其会话约定执行。
- 已交付账户 / Web 首批重整：日常仅用户账户登录，所有者 / 管理员 / 成员是网络内角色；实例凭据收敛到专门初始化 / 恢复入口，不扩大网络 Owner 的跨网权限。成员 / 邀请 / 角色 / 设备归属、我的账户、登录初始化及节点列表 / 详情 / 配置已接通；导航为概览 / 节点 / 成员 / 访问控制 / 路由 / 设置，保留地图方向。
- 用户最新明确删除旧 `apps/web`、专属依赖与启动/验收入口，不保留旧 Web 兼容层。后端集中完成 Rust、生成契约/SDK和真实 Linux 组网，新 console 由用户独立工作流处理。Controller 鉴权、网络作用域及 App/CLI 浏览器接入等公共契约仍须保持；Apple 现有原生测试直接使用隔离 Controller API，不依赖已删除的 Next 代理。验收必须走正常 Controller 下发、Agent 生命周期和真实 TUN 包流，独立编译器或 fixture 原型通过不算真实组网闭环。
- `apps/console` 是用户与另一个 agent 进行的新 Web 设计工作，当前后端线程忽略该目录并保护其相关 workspace / lockfile 改动。后端验收可使用独立、固定依赖的快照，不重置或替换另一工作流的依赖，不因它的未完成状态扩展前端任务。
- 用户已确认新 console 为 pure SPA，并要求完整导出 / 审查 model、字段、schema 与 HTTP API。接口独立于 Next Server Actions，明确会话 / 网络权限、错误、配置 CAS、观测新鲜度、轮询与分页；已发布契约和开发中 v3 候选分开。字段可省略、可 null、未知 / 不适用 / 已失效是不同语义，不为清理而统一删除空值。
- 已核对用户在 console 线程确认的方案：沿用不透明 `mws_` 用户会话，不改 JWT；LocalStorage 持久化，标签页捕获到内存后以 Authorization 携带，storage 变化时锁定 / 重载并隔离缓存和草稿；实例管理员令牌只放内存。新控制台无 BFF，同源由 Controller 从运行时配置目录提供静态 dist，API / App 路径保持有效，静态导航 fallback 不得吞掉 API / 缺失资源错误；Rust 构建与测试不依赖前端构建。Cookie 会话是未采用的备选审查提案，不再当作此迁移前置要求；Controller 仍逐请求 / 事务检查真实身份与网络权限。
- 用户最新要求停止使用 `openai/gpt-6.1-sol`，本线程后续 subagents 统一使用 `openai/gpt-6-astra-ultrafast`，包括两个现有开发任务、审查及任务续接；基于已有提交和 WIP 继续，不重置或重做，保留正在执行的构建与清理边界。控制台导航偏好侧边栏，精简重复说明，状态、错误与诊断信息保持真实可查。
- 控制台使用 shadcn `sidebar-07`（`npx shadcn@latest add sidebar-07`）作为基础布局参考，但用户最新明确视觉与交互不应被 shadcn 模板限制，希望更现代的整体产品设计。继续复用现有 Base UI 组件与无障碍能力，可定制画布、排版、层级与过渡；模板示例数据不进入产品页面，网络作用域、草稿/CAS、鉴权和观测新鲜度边界保持有效。
- 用户明确保留概览，并希望登录后以点状地图、节点点位和连接状态为视觉中心，角落显示精简总体状态；通过缩放/过渡展开 sidebar 管理界面。listener、承载及节点相关设置归入节点配置，移除日常“设置地区”等手工维护入口。地图仅使用有来源的地区/位置和真实连接证据，无定位数据的节点明确待定位；旧管理员 country 不能冒充实测位置，地图动效不伪造流量或健康状态。具体首屏/管理模式切换与其余导航收敛在 Web 设计中落实。
- 用户希望节点间延迟检测与历史趋势形成真实且统一的可视化体验。真实邻接探测采样、滚动历史存储/查询和 Web 曲线已合入，需新版 Agent 产生样本；端到端路径探测仍待实现。区分方向、承载、路径和测量来源，缺测/超时不填零；不得用重复心跳或浏览器重复读取制造历史样本与丢包率，探测耗时包含排队及可能的内部重试等待。
- P2a 承载托管需覆盖便捷的 VLESS 配置（含用户要求的 WebSocket 承载）；监听 bind 地址 / 端口、对外 endpoint、TLS 与 WebSocket 参数分别建模，由控制面管理并在 Web 提供显式覆盖。TUN 接管 IP 包，NAT 打洞属于 transport socket；打洞观测不等于可达监听，也不自动替代 VLESS / WebSocket 的服务端入口。
- 用户要求控制台可见每个 listener / 实际承载 socket 的绑定地址、对外候选、发现来源、验证与过期状态；拨号 socket 的候选也须正确关联，不能统一假定为 listener。STUN 观测、NAT 打洞与 UPnP/NAT-PMP/PCP 显式映射不同，UDP 的映射不能推广成 TCP/VLESS 可达性。自动路由方案的地区判断复用当前底层公网 IP 观测，由控制面 GeoIP 推导并展示来源/时间/选中方案；不得把 Mesh IP、所选出口 IP 或管理员国家标注当作设备的实测地区，缺少有效观测时使用已定义兜底。UPnP 实现范围尚未由本条确认。
- 控制面通信、STUN/认证建连探测、transport 外层 socket 及必要 bootstrap DNS 必须使用不进入 Meshway 自身 TUN 的系统保护通路，优先于公共/方案业务规则且不可被用户排序覆盖。以 Agent 内部 socket 用途和路由标记/平台等效机制限定豁免，不按整个进程或目标 IP/域名全局放行；普通业务访问相同地址仍受其策略约束。只豁免外层传输，不改变逻辑路径/Access/必经节点。网络与endpoint变化时先准备底层保护再启用新通道；IPv4/IPv6一致，缺少有效底层观测时不伪报地区。P1c的全IP /32 pin仅为已记录原型限制，正式出口下发前须收敛。
- 业务配置、连接和端点公钥须来自同一 revision 的快照；撤回和 lease 到期独立于 HTTP 请求终止旧运行代际，TUN 停止后 Mesh 网段不得落回底层默认路由。
- 用户已确认把第一轮远端联测提前到打洞之前：先用已验证主线，在已登记 Linux 主机的独立 Docker 容器核对部署、固定链路、跨地域 TUN 互访和重启恢复，先两台再四台；自动建连 / 打洞 / 主备完成后另做第二轮。测试保留原 SSH/管理连接，不手工改宿主机路由、DNS、防火墙或 Docker daemon 配置，不用 host network/privileged；Docker 创建的测试网络、端口及数据目录必须隔离且可精确清理，不恢复旧配置快照覆盖用户工作。
- 用户早已提供本机 Mac Controller `:8420` 映射至 `meshway-dev-api.jdscript.dev` 的计划，部署选址必须先核对该入口，不能遗漏后默认另设控制面。本轮 LAX Controller 是隔离停机 / 恢复验收的临时安排，不是产品部署要求；已运行的同身份复验先收尾，不中途手改 controller URL / 身份。用户最新希望构建集中在本地 / NYC，允许评估跨平台编译，不再把东京 / 首尔目标机原生构建当作硬性前置；优先评估 Mac 的 Linux ARM64 环境与 NYC 的 AMD64 环境或固定交叉工具链，具体新流水线尚未实施。
- Mac 域名已从四地核对可达，生成 GET 确认与本机8420为同一网络；这不等于原生Agent接入验收。原LAX测试身份和数据独立保留。
- 本机 Controller `:8420` 已按用户指令显式重置为当前 schema 13 和唯一 `/policy`；公网映射已核对同一新网络，旧 `/policy-v3` 返回404。三个 Docker demo Agent 已刷新二进制并重新接入，实际六方向 TUN ping 通过；原容器与可写层工具保留，旧库和节点状态私有归档。部署须核对实际进程与接口，不以磁盘 binary 重新编译代替重启；最新私有回执位于 `.meshway/services/policy-reset-*/{deployment,final-verification}.json`。初始化临时 Owner 密码已被后续更新替代，不覆盖其他工作流的账户变更。
- no-host-network / no-privileged 同样覆盖构建器。已披露并移除的两个特权 buildx builder 不再启动；原证据保留。使用既有普通 builder 构造环境 / runtime 层，Rust 在非 root、cap-drop-ALL、CPU/内存有界的普通容器内编译，不开放 insecure / host-network entitlement、不改 daemon。
- E2E 重启恢复已采用端到端认证的有界 `resync-v1`；快速路径要求参与节点支持扩展，旧节点及丢包 / 竞争保留原 TTL 边界，首个 stale packet 不重放。邻接恢复不证明旧 packet 会话恢复；不缩 TTL、无条件清反向 cache 或降低加密掩盖失败，与状态型 grants 回包分开跟踪。
- Linux managed 构建已接上正常 bootstrap/control/daemon/TUN 的用途受限保护，在隔离 namespace 验证通过，远端未部署。Journal v3 使用 boot + 非零 netns cookie；旧无 cookie / 不明归属 journal 保留并拒绝，不按 inode/形状接管。无绑定时仅 guard-only Controller bootstrap，真实网络绑定私有落盘后才转 bound owner；业务 DNS 每次发布核对该绑定。DNS错误/取消必须 join 原运行代际后撤 TUN route、保留 sink，输出背压不能阻塞清理或退出。受审 socket2 单 API 补丁限 vendor，应用 crate 继续 forbid unsafe。
- Rust 使用 `cargo fmt`、Clippy 和必要的行为测试；JS 使用严格 TypeScript 与各包检查。修改协议、HTTP 路由或生成客户端时，合并前检查 Rust workspace/all-features（含 Agent 与 Apple core）、生成 drift、SDK、Apple 脚本及 `experiments/*/tsconfig.json` 消费者。Console 的 UI/构建由独立工作流负责，后端提供兼容的生成契约与变更说明。
- 用户已明确移除 Meshway 的浏览器自动化检查：不保留或默认重建 Playwright / 浏览器 E2E 套件，不以浏览器全量回归阻塞日常开发或部署。UI 由用户在开发页面手工验证；协议、鉴权、迁移和数据面仍运行各自必要的非浏览器行为测试。
- 并行任务明确文件所有权；根 workspace 配置、lockfile 和整体验证由协调者维护。
- 不同 feature 组合的 CLI 行为测试与 SDK / 原生联测构建不得并行共用同一 Cargo target 目录；后者可能替换前者正在执行的 `target/debug/meshway-agent`。使用独立 target 或串行执行。
- 构建缓存须有总量边界：只保留主工作区、当前活跃任务及当前验证所需的 target；相同任务/feature 复用指定缓存，仍保持上述并发 feature 隔离。一次性审查优先 `CARGO_INCREMENTAL=0`，审查或整合完成后及时删除专用 target。追溯保留源码/patch、日志、manifest 与必要运行产物，不把完整 incremental/deps 目录当证据长期留存。启动大型构建前检查磁盘，可用空间不足 50 GiB 时先清理已结束且无进程/挂载引用的缓存，不继续扩张；不对未知归属 Docker volumes 做全量 prune。
- 用户已在磁盘清理后明确恢复 demo 和两个开发任务：自动连接主线继续，状态型回包先关闭独立审查缺陷再续接出口。后续按最新模型选择基于各自既有 HEAD/WIP 续接，继续执行构建空间和缓存回收约束。
- 所有后续 worktree 统一创建在本仓库 `.worktree/<任务名>`，不再散放上级目录。独立任务完成、提交已整合且无未提交 / 未跟踪工作后及时移除 worktree；保留分支和测量产物。活跃任务须在不影响进程与构建的安全边界迁移，不能直接删改其目录。
- 当前状态：`4c9a5be` 同身份直连 / 四地有序路径恢复复验 PASS，测试资源已清理，私有身份 / 源配置 / 旧 FAIL 及失败窗口保留。远端持久 latency history 查询未验；KR→NYC 底层 FIB 经 WARP，不能当公网直连或打洞证据。
- Controller 按 `http/`、`contracts/`、`accounts/`、`store/` 分责；`store/schema/` 拥有当前 schema 原子初始化与结构/数据完整性校验，历史升级链已删除，普通启动不自动重置。共享测试辅助在 `test_support/`；网络存在性类型位于中性 `network_scope`，不代替权限；单一策略源/CAS与执行epoch/ACK分别归 `network_policy`、`mesh_execution`。不为测试读取SQL扩大生产可见性。
- 当前主线：单一 policy、自动邻接首片及状态型 Mesh 回包已通过独立审查和正常 Linux 组合验收。当前回包合同为执行 schema 3 / `Ipv4StatefulExactPathV3` / `StatefulIpv4V2`、Controller schema 15；本机运行中的 schema 13 服务尚未随此提交切换，部署另行协调。继续推进真实首包按需建连 / NAT、完整出口 / 域名分流、平台默认 STUN、GeoIP 与自动地区方案。Apple 新增开发暂停，console 由用户独立工作流维护。未审定候选不部署。
- 用户最新确认优先继续自动组网、优先级主备和真实打洞（主线1），并行推进状态型回包及完整出口/域名-IP分流（主线2）。连接主线从已有 `nat-connectivity` 候选续接当前 Agent 领域库和唯一 policy；第一份闭环须两端仅出站、无发布端口/手填对端候选，并验证实际直连与获准中继兜底。TCP/VLESS 打洞仍在范围内，不以 UDP 优先实现替代其交付。共享 Node/HTTP 契约、根 manifest/lock 与最终合并由协调者收敛，各并行任务先交可消费的契约 checkpoint，不能复制 DTO 或各自建立兼容轨。
- 用户已明确状态型回包路径：同一会话沿建立时选定的完整逻辑路径反向返回。若 A→B 必经 C、B→A 必经 D，A 发起会话的回复走 B→C→A；经 D 的反向规则用于 B 主动发起的新会话。回包不重新匹配反向路由规则，也不因此授予 B 新主动流权限；回包路径的必经点不能因故障被绕过。
- 用户正在逐项对齐 Controller 的 Entity／Config／State：主要实体 Node、User；相对静态、类似 ROM 的配置真实持久化并保留完整版本供回滚，频繁变化、类似 RAM 的运行 State 与之区分。最新 NetworkConfiguration 讨论基线为 groups、tagsOwners、grants、nodes[NodeId]（hostname、tags、listeners、dialers）、dns；hostname 在该配置中是规范 DNS 名称来源。共享 configuration crate、逐节点编译视图及 node-specific delta 是已认可方向，精确字段、Tag 身份、版本／状态与回滚依赖仍待确认。用户要求优先用简洁配置参考逐项对齐，入口为 docs/v2/configuration-reference.md，背景说明为 docs/v2/network-configuration.md；不得把此前六块配置树或纯 Tailscale 照搬当已定需求，不凭讨论稿直接改 schema／API 或部署。
- 用户进一步明确 Config 与 State 分离、分别下发；要求先按 sing-box 式“结构／字段／示例”补充配置参考并整理文档导航。具体字段/default 由 assistant 提案后逐项确认；大 JSON 查询、可重建派生索引、版本化分页与类型化局部 API 属于待确认存储方案，不据此新增独立 Tag／Link 实体或直接修改数据库。相关草案位于 docs/v2/configuration-reference/ 与 docs/v2/state-reference.md。
- 用户最新要求配置参考按以后公开文档编写：grants／connections 去掉用户维护的 id，改为 remark（按非 null 字符串、缺省空字符串理解）；增加 defaults 讨论兜底；listen／dial 通用字段与各协议 listener／dialer 分页，并较完整覆盖 VLESS／Trojan 及上游字段差异，不能将独立库或未实现扩展写成已托管。用户认为万级 Node 的静态 JSON 全量加载可以先保持简单：优先完整版本与按版本内存缓存，派生数据库索引在实测需要时再加；defaults 的具体权限范围与新增协议字段仍是文档提案，不直接实施新 schema／API。
- 用户进一步指出 dialers 过复杂、extend／下发说明重复且“protocol fields 不在 configuration”表达错误，并认可管理员直接编辑协议参数。文档按“显式值优先，省略时平台补全并持久保存；NodeConfiguration 包含完整协议值和必要材料”修正；配置加密／日志脱敏与能否编辑分开，节点身份仍独立保护。dialers 直接以协议名为 key、去掉重复 type／streams 的简化，以及通用字段／默认集合继承规则，是待用户确认的下一版格式提案；当前材料 API 权限未随文档自动改变。
- 用户已确认下一版配置三部分命名为 `access`／`topology`／`routing`，界面名称为“访问权限／组网规则／流量路由”：access 定义业务端点权限，topology 定义允许存在的图与建连约束，routing 定义获准业务在图上的路径；实时 `links`／`paths` 属于 State。配置参考同步替换此前 grants／connections 命名及 defaults 对应 key；这不是直接重命名现行 Rust／HTTP 契约的授权，dialers 是否保留和具体连接计划结构仍待对齐。
- 用户要求 State 也按配置参考方式写文档：Node 上报自身 State，Controller 聚合一份网络全局 State，Node 可主动 query 其他相关节点状态，也可订阅／接收推送，不必预先复制逐节点 State。服务端裁剪须结合身份、access 及 topology／routing 的执行依赖（含必要中转），不能由客户端自行决定可见范围，不能把中转可见性当业务授权。NodeState／NetworkState／query/watch 的具体字段、序号和期限是 docs/v2/state-reference/ 中的下一版提案，尚不直接实施 API。
- 用户建议 State capability key 使用 `transport:xxx` 等命名空间以便扩展其他能力，认为 dial/listen 通常同时存在；文档建议完整实现可省略 roles（默认双向），单向构建须显式表达，最终默认规则仍待确认。用户正在讨论是否需维持 link 和历史上报；“按需建立／复用／无活跃依赖后空闲回收、历史消费真实连接／probe 事件而不另建连接凑曲线”是当前推荐方案，不凭此修改现行保活／遥测生命周期或宣称已实现统一历史。
- 用户要求连接 active 状态如实上报，并提出后续 Console 可 watch 指定 path 的实时往返延迟、流量、丢包指标。docs/v2/state-reference/realtime.md 将其列为未实现的限时主动观察任务，区别于读取既有 State／History；建议复用有界测量任务与短租约，业务字节不重复累计中转或诊断流量，先明确“探测未回应率”而非伪称底层／业务包丢包率。具体任务权限、采样与丢包口径仍待确认，不自动新增浏览器自动化或实现接口。
- 用户要求本轮新的配置／State／下发设计文档统一放入 `docs/v2/`，站点入口为 `/v2/`，与当前实现文档使用独立侧栏及版本导航。V2 是文档分区和设计方向，不代表已发布新 API，也不重新引入 policy-v2／policy-v3 运行时双轨。
- 用户单独指定 V2 Config／State 对照 Tailscale 的设计审查使用普通 `openai/gpt-6-astra`，仅此审查覆盖默认 Ultrafast 选择；只读评价模型完整性与进入 API 设计的阻断项，不直接实现接口或重写文档。
- 用户要求用 question 工具集中确认产品决策，已完成本批问答，选择后直接更新 V2 文档。Tag 身份、离网／重新登录、权限、默认值、执行分工、出口／DNS 和回滚决定见下一条；工程参数与精确 API 编码由后续设计补齐，不据此直接更改现行代码。
- V2 批量问答已确认：最后一个 Tag 需设备重新用户登录后移除；非管理员 Tag owner 只可给自己的 User 身份设备赋所拥有标签，不扩整网／秘密权限；邮箱变更自动同步可读引用，历史和授权绑定稳定 User ID。默认 access=deny，新网络显式生成 Mesh 互访允许模板；出站覆盖缺省继承、显式空集合禁用；删除后同名重建 listener 是新资源，生成新自动材料。Controller 按合法需求发布有界候选／完整路径与一致依赖，设备结合 State 选择，不下发任意图让设备自授权限。access 约束业务目的，允许方案／routing 决定出口，联合 exitEnabled/topology 编译接收许可；分配 allowed 取并集，default/locked 冲突拒绝；direct/unmanaged 本地出网不额外套 Mesh access。域名授权采用可信解析地址集合并承认共享 IP 无严格服务隔离；普通 DNS 默认随方案出口，显式解析器例外保留，bootstrap 独立。保存用户方案选择，重启时仍 allowed/unlocked 才恢复，否则当前默认；回滚遇已吊销／不可用历史依赖明确失败要求替代。普通成员只读相关 State/History，Console 主动诊断仅自己的 User 身份设备；Owner/Admin 管理本网络。规范见 docs/v2/permissions.md、execution.md 及各字段页；这是 V2 文档决定，不直接改现行实现。
- V2 API 最新决定替代此前仅邀请注册的产品目标：Controller 默认开放邮箱／密码注册，实例 .env/config 可关闭；不做邀请注册或邀请绕过关闭开关。有效登录用户可创建自己的网络并成为本网 Owner，既有用户登录／建网与成员管理分开，公开首用户不自动获实例管理员权限。批量问答已确认用户继续不透明会话而非 JWT；Node 采用本地持久公私钥身份，具体持钥认证握手／短期凭据仍待技术定版，与数据面密钥分责。Console SSE 首次全量同步当前可见 Config／State，随后增量、配置变更提示及获准 path observation；不覆盖本地草稿，保留 CAS、权限重检、cursor/reset 与真实观测。用户已纠正 API 根路径为 `/api/v1`（Console）与 `/agent-api/v1`（Node），不用 `/api/v1/user`、`node`、`instance` 分区；Controller 正常启动后普通注册／建网，不需要实例初始化／首用户认领 API，空库结构初始化和部署运维仍独立。OpenAPI 可描述 SSE，但当前 TS JSON emitter 与 Rust enable_sse_client=false 尚未支持本合同。草案位于 docs/v2/api.md、docs/v2/api/，不据此改现行 schema、接口或部署。
- 用户要求继续审阅数据库 Entity、全部数据的存储访问、技术栈和 layout/shared crates。本轮提案位于 docs/v2/entities.md、storage.md、architecture.md：User／Node 身份与 Network／Membership、完整配置版本、身份／材料／执行恢复记录、持久使用选择、有界历史和 RAM State 分责；建议沿用 SQLite/rusqlite，完整配置按版本缓存，必要 SQL 约束／索引保留。model、configuration、transport-spec、private-storage 的职责和 protocol／policy 依赖收敛属于待审工程方案，不是已实现 crate；不能凭文档改数据库、搬代码或新增 console 工作。
- 用户提出 State endpoints 同时涵盖接口地址和配置声明，并按来源区分，建议初始尝试顺序为 self claim → interface → stun。文档以 configured／interface／stun 表达；configured 须匹配 Controller 配置，来源不是可达性或信任等级，尝试前应用权限／地址／资源约束并有界竞速，已认证健康通道不因来源顺序无条件替换。具体优选算法仍待确认，见 docs/v2/state-reference/endpoints.md，不将 UDP 观测推广为 TCP 可达。
- 自动邻接首片已完成并独立批准：Controller 从托管 listener、节点能力和已授权策略的编译路径静态推导邻接，不写回 source candidates；同逻辑邻接 UDP→TCP→UDP、双方认证探测、双向 TUN HTTP、撤回/恢复已验。尚不代表基于首包的按需调度、NAT或真实直连，后续继续。状态型回包的重传/乱序、容量、流水发送、半关闭、活跃会话及第三 ACK 携带数据等审查缺口已关闭；整合验收另覆盖自动邻接下的单向 grant 回复和反向新连接拒绝。原失败证据保留，不改写为成功。
- 性能本批已结束，不整分支部署 offload / NEON 实验或扩大矩阵；未知归属或仍有进程 / 未提交工作的树须保留。
- 独立完成的工作形成清晰提交。提交前检查生成产物、文档和实际运行命令一致。
