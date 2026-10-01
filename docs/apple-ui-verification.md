# Apple UI 离屏验收

2026-10-01，分支 `feat/apple-ui-redesign`。macOS 采用单列、默认 500 × 680 的可滚动设备窗口，设备规模和当前功能不需要 sidebar；菜单栏面板宽 360。iOS 保留原生 NavigationStack / insetGrouped List。共同视觉：indigo 品牌 glyph、轻描边分组、SF Symbols 中性节点图标、等宽地址、清晰的 self / peers 层级；没有状态点或虚构的连通性。

## 生成与观察

```sh
MESHWAY_RENDER_DIR="${TMPDIR%/}/opencode/apple-ui" \
  swift test --package-path apps/apple/MeshwayKit --filter renderAppleScreens
```

样本仅在 `apps/apple/MeshwayKit/Tests/MeshwayVisualTests/RenderTests.swift`，没有进入 App target。ImageRenderer 渲染真实共享 `DeviceContentView.content`、`OnboardingLayout`、`SignInButtonLabel` 和 `NetworkLabel`。macOS 上原生 Menu / Toggle / TextField / ScrollView 不支持 ImageRenderer：测试替换 native chrome 为静态表示、去掉滚动外壳。实际 App 使用原生 toolbar、disabled `.switch`、Menu、TextField，按钮操作和复制仍真实。

所有 16 张最终 PNG 已用图片读取工具逐张观察。第一轮发现原生控件占位、scroll 内容为空；第二轮提取共享内容；最后调整卡片边界、深色地址对比、loading indicator 及长名称换行。库存截图扩展高度以同时展示 5 peers；实际默认窗口滚动。iPhone-size 截图使用 macOS ImageRenderer 验证共享内容的窄屏布局，**不是 iOS 原生导航 / List、Dynamic Type 运行或 Simulator 截图**。iOS 原生分支通过 unsigned Simulator build；未声称 Simulator / 真机运行。

产物目录：

```text
/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/apple-ui/
```

下表文件均在该目录，尺寸为 points，PNG 为 2×：

| 画面 | 尺寸 | Light | Dark |
| --- | --- | --- | --- |
| macOS signed-out | 500 × 680 | `mac-signed-out-light.png` | `mac-signed-out-dark.png` |
| macOS inventory | 500 × 1000 | `mac-devices-light.png` | `mac-devices-dark.png` |
| macOS refresh error | 500 × 1100 | `mac-error-light.png` | `mac-error-dark.png` |
| Menu panel | 360 × 590 | `menu-panel-light.png` | `menu-panel-dark.png` |
| iPhone-size onboarding | 393 × 852 | `iphone-onboarding-light.png` | `iphone-onboarding-dark.png` |
| iPhone-size inventory | 393 × 1000 | `iphone-devices-light.png` | `iphone-devices-dark.png` |
| macOS revoked | 500 × 680 | `mac-revoked-light.png` | `mac-revoked-dark.png` |
| macOS loading | 500 × 680 | `mac-loading-light.png` | `mac-loading-dark.png` |

## 行为与实现边界

- 数据来自 node bearer 的生成配置 / DNS GET；protocol 类型、共享摘要 / DNS 校验 / policy 编译、自身 ID / 已保存地址及 revision 绑定。显示读取不发送 ACK。
- 自身 heartbeat 时间 / 结果真实；peer 只显示 controller 库存的名称、IPv4 / IPv6、DNS 名与 admin country。配置读取成功不代表数据面可用。
- 网络 Menu 当前列唯一真实接入；标题为启用的 DNS zone 或服务器 host。详情、复制 URL、本地重置是真实操作；本地重置有确认且不撤销服务端节点。
- Connect switch 始终 off、disabled，tooltip 和正文明确隧道未实现。多网络保存 / 切换尚未实现，没有空操作按钮。
- 自动前台读取跨窗口 30 秒节流；手动刷新立即读取、整段流程禁止重叠；保留旧库存时明确标示，403 持久撤销并清库存。

## 验证

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test -p meshway-apple-core --locked`：2 unit + 1 real-controller integration，通过。
- `bash apps/apple/scripts/check.sh`：crate Clippy、workspace check、三平台 XCFramework、Swift tests、macOS / iOS Simulator unsigned build，通过。
- `pnpm typecheck`、`pnpm lint`、`pnpm test:web`（128 tests）、`pnpm build:web`，通过。
- `pnpm exec playwright test e2e/device-login.spec.ts`：真实批准 / 兑换 / 心跳、第二节点库存、DNS、GET 401、错误方法 405、admin paths 404，通过。
- `pnpm exec tsc -p apps/apple/scripts/tsconfig.json` 与 `node apps/apple/scripts/check-signin.ts`，通过；后者在隔离端口 / 临时数据中，以真实 Keychain → FFI → generated Rust → Web proxy 验证两个真实节点、peer inventory、DNS revision、恢复与撤销。测试清理自己的 Keychain items / 子进程 / 数据，不操作现有服务或 Docker。
- 原生 ASWebAuthenticationSession UI、系统隧道和实际设备连通性不在本次验证范围。
