# Meshway Apple

iOS / macOS SwiftUI App 与 Packet Tunnel 骨架。设备使用统一 Meshway 节点模型；当前不支持登录、注册或真实隧道。

首次打开 `Meshway.xcodeproj` 或运行 Swift package 检查前：

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
bash apps/apple/scripts/build-core.sh
bash apps/apple/scripts/check.sh
```

以上命令从仓库根执行。`build-core.sh` 生成被忽略的 Swift 绑定与 arm64 三平台 XCFramework；未生成时 Swift package 无法解析 binary target。`check.sh` 以 `CODE_SIGNING_ALLOWED=NO` 编译 macOS 与 iOS Simulator。

签名及 bundle prefix 的本地覆盖放在 `Config/Local.xcconfig`；默认不指定 Team。真实设备 NE 验证需恢复会员并配置 entitlement。详细结构、FFI 边界与后续工作见 [`docs/apple-app.md`](../../docs/apple-app.md)。
