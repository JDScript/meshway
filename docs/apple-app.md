# Apple App：浏览器接入

## 决策与实现边界

**已确认**：`apps/apple` 使用 SwiftUI 多平台 App、NetworkExtension Packet Tunnel、UniFFI Rust 核心及 XCFramework。设备是统一的 Meshway 节点；不引入 App / 服务器节点类别或中继角色开关。

**已实现**：单一服务器地址（Web 控制台 origin）、系统浏览器授权、Keychain 节点凭据与待兑换状态、生成 Rust 客户端兑换与心跳。Home 展示真实接入阶段、节点名称 / 短 ID / Mesh IP、最近心跳结果和时间。前台每 30 秒至多一次心跳，请求限时 10 秒、禁止重叠；心跳仅表示控制面联系。保留真实空网络页、Rust 信息、只读系统 VPN 诊断及明确拒绝启动的 Packet Tunnel 骨架。iPhone 使用 TabView，macOS / iPad 使用 NavigationSplitView；macOS 有菜单栏入口。

**未实现**：外部 OIDC、X25519 公钥发布、嵌入 agent 生命周期、peer 数据源、NE 包流适配、隧道、流量及诊断导出。浏览器当前使用控制台管理员登录 / 批准；App 不获得管理员凭据。保存 URL、注册及心跳均不代表隧道已建立。

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

状态协调器在 MainActor 上使用 Observation；Swift 6 严格并发。`ASWebAuthenticationSession` 使用前台 window anchor、`meshway` scheme，非 ephemeral 以复用控制台人类登录 cookie；App 不读取 cookie。回调严格核对 scheme / host / path、唯一且相等的 state、互斥 code / error。取消 / 拒绝保留节点凭据；网络失败重试原绑定。首次无效 / 过期（401）可显式重开新尝试并保留凭据；已消费的相同绑定即使过期也恢复原节点。403 标为撤销，409 不更换身份。重置需用户确认，只清本地，**不撤销服务端节点**。

节点凭据、PKCE verifier / state、收到但尚未兑换的 code 放在一个原子更新的 Keychain journal，accessibility 为 `AfterFirstUnlockThisDeviceOnly`、不同步。journal 同时保留恢复所需公开绑定与节点快照；非敏感 server / node / IP / phase 另写 Application Support 的原子 JSON 镜像。先提交 journal，再写镜像；重启以 journal 修复镜像。打开浏览器前保存凭据 + 完整尝试；兑换前保存 code；成功后一次提交节点 + 清 pending。任何写入失败阻止后续请求 / 自动生成，重启恢复或显式重置；Keychain 不可用显示真实错误，无明文 fallback。保留公开身份却缺少秘密时要求显式重置，不当作新设备；重置先删镜像再删 Keychain，途中失败 / 崩溃由原 journal 恢复，用户可重复重置。前台自动恢复 `redeeming`；`awaitingBrowser` 由按钮重开原尝试。

SystemTunnelController 仍只读 VPN preferences；provider 的 startTunnel 返回 “not implemented in this build”，不设置网络、路由或 DNS。

Rust FFI：`begin_authorization`、`new_node_credential`、异步 `exchange_device` / `heartbeat`，以及既有 `core_info` / `valid_tag_selector`。PKCE S256 与随机 credential 复用 protocol；HTTP DTO / 操作复用生成客户端。一个单 worker、bridge-owned Tokio runtime 执行请求，Swift executor 只等待任务；净化错误枚举不携带 source、body 或秘密。URL 只允许 HTTPS origin（开发回环 HTTP），无 path / userinfo / query / fragment，不声称已验证网络可达。

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

真实接入 smoke（先 `cargo build -p meshway-controller --locked`、`pnpm install --frozen-lockfile`、`pnpm build:web`）：

```sh
pnpm exec tsc -p apps/apple/scripts/tsconfig.json
node apps/apple/scripts/check-signin.ts
pnpm exec playwright test e2e/device-login.spec.ts
```

smoke 自行分配回环端口与临时 controller 数据，使用真实 Keychain 的唯一测试 item，完成 Swift → FFI → 生成 Rust client → Web proxy → controller 的接入、重启读取、心跳、撤销；退出时删除测试 item / 数据并停止子进程。可在命令末尾传入独立 bundle prefix 构建的 macOS executable 路径，短暂启动 unsigned App（同样由脚本停止）。批准由 test-only 生成客户端 fixture 完成，不注入 App 管理员权限、不将 code 写日志；它不验证 ASWebAuthenticationSession UI。Playwright 单独验证真实 Web 登录 / 批准 / 拒绝、proxy 幂等兑换 / 心跳、错误透传与 admin 路由未暴露。

签名默认无 Team，bundle prefix 为 `dev.example`。需要签名时创建忽略的 `apps/apple/Config/Local.xcconfig`，设置 `DEVELOPMENT_TEAM`、`MESHWAY_BUNDLE_PREFIX`，必要时覆盖 `MESHWAY_APP_GROUP`。`MESHWAY_KEYCHAIN_ACCESS_GROUP` 默认空，unsigned 使用默认 Keychain；需要 extension 共享时设置此变量并为 App / extension 添加匹配的 `keychain-access-groups` entitlement 与有效 provisioning。App Group 不能替代 Keychain access group。会员未续费，不能据编译声称签名权限、真机 NE 或隧道已验证。

本地验收：Rust bridge + 真实 controller ephemeral-port 联测；Swift 覆盖严格 callback、各持久化 phase 恢复、镜像写入中断修复、原绑定重试、Keychain 错误 / 丢失、取消 / 拒绝、过期重开、撤销与重置中断；真实 Keychain / FFI / proxy smoke 和 Web Playwright 通过。三平台 XCFramework、macOS / iOS Simulator unsigned build 通过；独立 bundle prefix 的 macOS App 已对隔离 console 短暂启动并退出。原生浏览器 UI 自动化因 macOS Accessibility 未启用无法执行，不能声称已看到 Home 的真实接入显示。该机器 CoreSimulator / CoreDevice 插件版本仍不匹配，未验证 Simulator 运行；真机 NE 生命周期待有签名宿主验证。

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
