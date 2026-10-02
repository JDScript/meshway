# Apple App：浏览器接入

## 决策与实现边界

**当前优先级**：暂停 Apple 新增开发，先推进 Linux 自动组网、打洞和跨地域 Docker 验收；已有登录、PKCE 和生成客户端接口继续保持兼容。下文构建与验收是现有能力记录，不表示本轮继续扩展 NE。

**已确认**：`apps/apple` 使用 SwiftUI 多平台 App、NetworkExtension Packet Tunnel、UniFFI Rust 核心及 XCFramework。设备是统一的 Meshway 节点；不引入 App / 服务器节点类别或中继角色开关。

**已实现**：多网络接入保存、一次一个活动接入、系统浏览器授权、每接入独立 Keychain 身份与待兑换状态、生成 Rust 客户端兑换、心跳和只读设备 / DNS 快照。macOS 为默认 500 × 680 的单列普通窗口与 window-style 菜单栏面板；iOS 使用 NavigationStack、insetGrouped 设备列表、搜索、下拉刷新、侧滑 / 上下文复制与详情。静态标题与服务器 host 保持不变；macOS 窗口、菜单栏面板与 iOS 的 ⋯ 菜单提供保存的网络（活动勾选、Pending / Revoked）、Add network…、详情与本地移除。退出当前选择后的登录页也列出保存接入。右上角原生 mini switch 禁用、off，旁边为 Not connected，明确隧道未实现。

前台出现 / 接入成功时读取，自动刷新跨窗口每 30 秒至多一次；手动刷新可立即发起，整个心跳 → 配置 → DNS 流程禁止重叠，各 HTTP 请求限时 10 秒。自身成功 contact 只展示一行真实最近时间，失败显示 orange reason；section header 显示 updated / previous fetch。peer 展示名称、IPv4、真实 friendly DNS alias 与管理员 country；canonical DNS / IPv6 留在详情 / copy action。行使用真实名称 initials 与 node-ID 稳定 tint 的 monogram，不表示设备类型或在线状态。DNS 仅在配置同 revision、ID / 地址一致时合并。403 进入持久撤销状态并清除库存。快照只用于显示，**不 ACK、不持久化配置、不激活数据面**。

**未实现**：外部 OIDC、X25519 公钥发布、嵌入 agent 生命周期、NE 包流适配、隧道、流量及诊断导出。浏览器批准已接入 controller 用户会话、所选网络 membership 和设备归属，App 不获得密码、会话或管理员凭据。节点端尚无网络显示名 / Owner 身份接口，保存条目使用真实 DNS zone、IPv4 prefix、服务器 host，未取得配置时使用已接入节点 IPv4；同来源标签相同时补充已保存 node ID 区分接入，不冒充网络名。待接入显示本地尝试标识，不能本地重命名网络。保存 URL、注册及心跳均不代表隧道已建立。

## 结构与依赖

```text
apps/apple/
├── App/                         # 薄 @main SwiftUI 入口
├── PacketTunnel/                # 薄 extension principal class
├── Meshway.xcodeproj/            # synchronized folders，多平台两个 target
├── Config/                      # xcconfig、entitlements、extension Info.plist
├── MeshwayKit/
│   ├── Sources/MeshwayCore/      # Rust FFI 的安全 Swift 包装
│   ├── Sources/MeshwayIPC/       # App ↔ extension 类型（尚未接线）
│   ├── Sources/MeshwayModel/     # Observation 状态、服务协议
│   ├── Sources/MeshwayUI/        # 原生 SwiftUI 功能视图
│   ├── Sources/MeshwayTunnel/    # NEPacketTunnelProvider 骨架
│   └── Tests/                   # FFI、URL、状态持久化验证
├── core/                        # 独立 Rust FFI 边界 crate
├── scripts/                     # 构建和检查
└── build/                       # 忽略的产物
```

依赖方向：UI → `AppModel` / `EnrollmentCoordinator` → `EnrollmentService` / `BrowserAuthorizing` / `SecretStore` / `TunnelController`；`RustEnrollmentService` → 生成 Swift → UniFFI → `meshway-agent::generated`。bridge 使用 agent 默认 features，没有启用 Linux / TUN 功能。extension → `MeshwayTunnel` → `MeshwayIPC`。

状态协调器在 MainActor 上使用 Observation；Swift 6 严格并发。`ASWebAuthenticationSession` 使用前台 window anchor、`meshway` scheme，非 ephemeral 以复用控制台人类登录 cookie；App 不读取 cookie。回调严格核对 scheme / host / path、唯一且相等的 state、互斥 code / error。Add network… 进入新接入的浏览器登录流程；每次新接入产生独立身份，同服务器的不同网络也不复用凭据。取消 / 拒绝保留可继续或明确移除的 Pending 条目；网络失败重试原绑定。401 可显式重开尝试并保留凭据；403 只撤销当前接入，409 不更换身份。移除需确认，仅清本地，**不撤销服务端节点**；移除活动接入后选择另一已接入网络，否则回到未选择状态。

每接入有稳定本地 ID、server URL、独立原子 Keychain journal（凭据、PKCE、未兑换 code、节点记录）及原子公开镜像；`profiles.json` 保存非敏感目录 / 真实标签，`active-profile.json` 单独保存活动指针。Keychain 为 `AfterFirstUnlockThisDeviceOnly`、不同步、无明文 fallback。先提交 journal，再镜像 / 目录；重启由 journal 修复公开状态。打开浏览器前保存凭据 + 完整尝试；兑换前保存 code；成功后一次提交节点 + 清 pending。缺少秘密且仍有公开身份时阻止重新生成。

旧单接入无损迁移到固定 `legacy` profile：先复制原 journal、修复镜像、写目录及活动指针，再移除旧镜像 / Keychain，最后标记完成；每步可重复恢复，不重新接入、不改变凭据 / 节点 / 待兑换绑定。新接入先写目录意图，未完成首次 journal 的条目明确 Pending。移除先提交目录 tombstone，再更新活动指针、删除镜像 / journal，重启完成未结束的清理。

切换先递增运行 generation、取消浏览器并立即清库存，再等待旧 heartbeat / configuration / DNS / exchange 整段完成，才加载新身份。每个异步结果同时核对 profile ID 与 generation；新旧请求不重叠，迟到响应不能显示在新网络下。清除旧 contact、错误、节流时间和设备详情；新接入独立保存撤销状态。当前没有数据面可切换，未来嵌入 agent / NE 时须在此边界停止旧隧道后再启动新隧道。

SystemTunnelController 仍只读 VPN preferences；provider 的 startTunnel 返回 “not implemented in this build”，不设置网络、路由或 DNS。

Rust FFI：`begin_authorization`、`new_node_credential`、异步 `exchange_device` / `heartbeat` / `get_device_configuration` / `get_device_dns`，以及既有 `core_info` / `valid_tag_selector`。`core/src/inventory.rs` 调用 agent 生成 `get_agent_configuration` / `get_agent_dns`，将有类型 wire DTO 转换为权威 protocol 类型并验证 digest、自身 ID、schema / revision、库存 bounds / 唯一身份 / 地址与 policy；DNS 复用 `NodeDnsConfiguration::validate`。Agent 完整缓存验证器为私有，未作为显示层公开；摘要直接复用 protocol 函数，不重写密码逻辑。Swift 另绑定已持久节点 IP、阻止 revision 回退 / 同 revision mutation，合并 DNS 时检查同 revision / ID / IP。

PKCE S256 与随机 credential 复用 protocol；HTTP DTO / 操作复用生成客户端。一个单 worker、bridge-owned Tokio runtime 执行请求，Swift executor 只等待任务；净化错误枚举不携带 source、body 或秘密。URL 只允许 HTTPS origin（开发回环 HTTP），无 path / userinfo / query / fragment，不声称已验证网络可达。

## 构建与运行

环境：Xcode 27 / Swift 6.4、仓库 Rust 1.98。Apple targets 显式本地安装，不加入 rust-toolchain.toml。

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
bash apps/apple/scripts/build-core.sh
swift test --package-path apps/apple/MeshwayKit
xcodebuild -project apps/apple/Meshway.xcodeproj -scheme Meshway \
  -configuration Debug -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath apps/apple/build/DerivedData CODE_SIGNING_ALLOWED=NO build
xcodebuild -project apps/apple/Meshway.xcodeproj -scheme Meshway \
  -configuration Debug -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath apps/apple/build/DerivedData CODE_SIGNING_ALLOWED=NO ARCHS=arm64 build
open apps/apple/build/DerivedData/Build/Products/Debug/Meshway.app
```

全检查：`bash apps/apple/scripts/check.sh`。首次打开 Xcode 或运行 swift test 前必须先 build-core；脚本锁定 UniFFI 0.32.2，构建 arm64 macOS / iOS / iOS Simulator 静态库，生成 Swift/header/modulemap 并组装 XCFramework。生成绑定与二进制均忽略，不手工编辑。当前不包含 Intel macOS / x86_64 Simulator slice。

真实接入 smoke（先 `cargo build -p meshway-controller --locked`、`pnpm install --frozen-lockfile`；隔离 Web build）：

```sh
pnpm exec tsc -p apps/apple/scripts/tsconfig.json
MESHWAY_WEB_DIST_DIR=.next-apple-check pnpm build:web
MESHWAY_WEB_DIST_DIR=.next-apple-check node apps/apple/scripts/check-signin.ts
```

smoke 自行分配回环端口与临时 controller 数据，使用真实 Keychain 的唯一测试 item，完成 Swift → FFI → 生成 Rust client → Web proxy → controller 的接入、重启读取、心跳、第二个真实节点的配置 / DNS 库存、撤销；退出时删除测试 item / 数据并停止子进程。可在命令末尾传入独立 bundle prefix 构建的 macOS executable 路径，短暂启动 unsigned App（同样由脚本停止）。批准由 test-only 生成客户端 fixture 完成，不注入 App 管理员权限、不将 code 写日志；它不验证 ASWebAuthenticationSession UI。辅助进程/HTTP helper 独立于浏览器框架，Web 登录与批准界面改为手工验证。

真实 macOS 窗口视觉验收（无需 screen recording）：

```sh
xcodebuild -quiet -project apps/apple/Meshway.xcodeproj -scheme Meshway \
  -configuration Debug -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath apps/apple/build/DerivedData CODE_SIGNING_ALLOWED=NO \
  MESHWAY_BUNDLE_PREFIX=dev.meshway.snapshot build
MESHWAY_WEB_DIST_DIR=.next-apple-check node apps/apple/scripts/snapshot.ts \
  "${TMPDIR%/}/opencode/apple-saved-networks"
```

脚本启动隔离端口/临时数据和唯一Keychain/defaults namespace，经真实API创建第二张网络，分别接入并验证保存/切回/移除fallback。Debug-only AppKit hook捕获实际窗口/sheets；无Accessibility时原生菜单用同真实条目的sheet验证。不注入假节点/隧道，结束清理独立Next构建目录及临时配置改动。当前保留必要兼容检查，Apple新增开发暂停。

签名默认无 Team，bundle prefix 为 `dev.example`。需要签名时创建忽略的 `apps/apple/Config/Local.xcconfig`，设置 `DEVELOPMENT_TEAM`、`MESHWAY_BUNDLE_PREFIX`，必要时覆盖 `MESHWAY_APP_GROUP`。`MESHWAY_KEYCHAIN_ACCESS_GROUP` 默认空，unsigned 使用默认 Keychain；需要 extension 共享时设置此变量并为 App / extension 添加匹配的 `keychain-access-groups` entitlement 与有效 provisioning。App Group 不能替代 Keychain access group。会员未续费，不能据编译声称签名权限、真机 NE 或隧道已验证。

本地验收：Rust bridge + 真实 controller ephemeral-port 联测；Swift 覆盖严格 callback、各持久化 phase 恢复、镜像中断修复、原绑定重试、Keychain 错误 / 丢失、取消 / 拒绝、过期重开。多接入专项覆盖旧身份迁移的每个写入 / 删除中断、凭据隔离、Pending 恢复、移除与活动 fallback、heartbeat / configuration / DNS / exchange 迟到结果 fencing。真实 Keychain / FFI / proxy smoke、三平台 XCFramework、macOS / iOS Simulator unsigned build 通过；双网络真实窗口截图使用隔离数据与生成客户端。Web 与系统浏览器 UI 由用户手工验证；未安装 Simulator runtime，未验证 native iOS 运行；真机 NE 生命周期待有签名宿主验证。

## FFI 审计边界

workspace `unsafe_code = "forbid"` 保持不变。`meshway-apple-core` 不继承此 lint，独立设为 `deny`，使 UniFFI 生成的 ABI scaffolding 能局部 allow；手写 Rust 无 unsafe。原因：forbid 不能被生成的 allow 放宽。这个例外只限桥接 crate；更改导出 API 或升级 UniFFI 时必须复查生成的 ABI / 内存所有权，重新生成与运行 FFI 行为测试。

## 可移植性验证与下一步

可移植性 spike 命令：

```sh
cargo build -p meshway-dataplane -p meshway-protocol -p meshway-policy --target aarch64-apple-ios
```

Xcode 27 / Rust 1.98 上三个 crate 均编译通过（aarch64-apple-ios），无需修改这些 crate。编译通过不代表 iOS socket、后台执行或真实 NE 数据面已经运行；agent 的 tun-rs / iproute2 集成仍是 Linux 边界。

**后续建议，尚未实现**：

1. 私有持久化 X25519 身份并经生成 `publishConnectionIdentity` 不可替换地发布。
2. 嵌入 agent 生命周期，再从 agent 提取公共 packet-I/O 边界接 NEPacketTunnelFlow；保留 E2E、路径授权、revision / lease 与撤回语义。
3. 有签名时验证 Keychain access group 和必要的 App Group 非敏感快照共享。
4. 会员恢复后验证 entitlement、真机 NE 生命周期、后台限制，再评估 macOS System Extension 与分发。
