# Apple App 首个切片

## 决策与实现边界

**已确认**：`apps/apple` 使用 SwiftUI 多平台 App、NetworkExtension Packet Tunnel、UniFFI Rust 核心及 XCFramework。设备是统一的 Meshway 节点；不引入 App / 服务器节点类别或中继角色开关。OAuth 是接入方向。

**已实现**：控制器 URL 本地校验与保存、配置 / 未登录状态、真实空网络页、Rust 版本及协议信息、只读系统 VPN 配置诊断、明确拒绝启动的 Packet Tunnel 骨架。iPhone 使用 TabView，macOS / iPad 使用 NavigationSplitView；macOS 有菜单栏状态入口。

**未实现**：控制面的 OAuth、用户 / 设备身份 API、注册、凭据、peer 数据源、隧道、流量及诊断导出。保存 URL 不代表控制器可达；VPN 配置数量不代表已联网。现有管理员 bearer 与一次性 enrollment token 不冒充终端登录。

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

依赖方向：UI → `AppModel` → `CoreService` / `AuthProvider` / `TunnelController`；`RustCoreService` → 生成 Swift → UniFFI → protocol / policy。extension → `MeshwayTunnel` → `MeshwayIPC`。没有复用 Linux agent 的 TUN 实现。

`AppModel` 在 MainActor 上使用 Observation；Swift 6 严格并发。AuthProvider 的实现明确不可用；SystemTunnelController 通过 NETunnelProviderManager 只读已有本 App 配置，未提供 start / save。provider 的 startTunnel 返回 “not implemented in this build” 错误，未调用 setTunnelNetworkSettings。URL 是非敏感设置，存 UserDefaults；尚未存任何身份或凭据。

Rust API `core_info` 提供实际 crate 版本、配置 schema 与共享规则上限；`valid_tag_selector` 直接调用现有 policy 校验，不授予或分配 tag。URL 校验是 Swift 本地输入规则（HTTP(S)、host、合法端口、无用户名密码 / query / fragment），不声称是控制面契约或网络验证。

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

签名默认无 Team，bundle prefix 为 `dev.example`。需要签名时创建忽略的 `apps/apple/Config/Local.xcconfig`，设置 `DEVELOPMENT_TEAM`、`MESHWAY_BUNDLE_PREFIX`，必要时覆盖 `MESHWAY_APP_GROUP`；App Group 与 bundle ID 使用变量派生。会员未续费，当前只做无签名编译与本地启动；不能据此声称 NE entitlement、真机或隧道已验证。macOS 采用 app extension，System Extension 不在此切片范围。暂不增加 macOS CI。

本地验收：cargo fmt、bridge Clippy（含 bindgen）、workspace check、三平台 core 构建通过；Swift 行为测试覆盖 Rust FFI、URL 校验和配置状态持久化；macOS / iOS Simulator 无签名编译通过；macOS App 已短暂启动并正常退出。该机器 Xcode 提示 CoreSimulator 版本过旧及 CoreDevice 插件不匹配，未验证 Simulator 运行。NE provider 在独立测试进程中直接调用 startTunnel 未收到系统回调，不能作为有效的生命周期测试；provider 只验证编译，真实启动拒绝语义待有签名的 NE 宿主验证。

## FFI 审计边界

workspace `unsafe_code = "forbid"` 保持不变。`meshway-apple-core` 不继承此 lint，独立设为 `deny`，使 UniFFI 生成的 ABI scaffolding 能局部 allow；手写 Rust 无 unsafe。原因：forbid 不能被生成的 allow 放宽。这个例外只限桥接 crate；更改导出 API 或升级 UniFFI 时必须复查生成的 ABI / 内存所有权，重新生成与运行 FFI 行为测试。

## 可移植性验证与下一步

可移植性 spike 命令：

```sh
cargo build -p meshway-dataplane -p meshway-protocol -p meshway-policy --target aarch64-apple-ios
```

Xcode 27 / Rust 1.98 上三个 crate 均编译通过（aarch64-apple-ios），无需修改这些 crate。编译通过不代表 iOS socket、后台执行或真实 NE 数据面已经运行；agent 的 tun-rs / iproute2 集成仍是 Linux 边界。

**后续建议，尚未实现**：

1. 控制面先明确并生成 OAuth / 用户 / 设备身份契约，再接 ASWebAuthenticationSession 与真实节点身份；不复制手写 HTTP DTO。
2. 从 agent 提取公共 packet-I/O 边界，接 NEPacketTunnelFlow；保留 E2E、路径授权、revision / lease 与撤回语义。
3. Keychain 保存身份密钥，App Group 共享必要非敏感快照；protocol::local 的 Unix 私有文件模型不能替代 iOS 安全存储。
4. 会员恢复后验证 entitlement、真机 NE 生命周期、后台限制，再评估 macOS System Extension 与分发。
