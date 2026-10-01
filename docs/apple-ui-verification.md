# Apple UI 真实窗口视觉验收

2026-10-01，分支 `feat/apple-ui-polish`。主验收是 **真实 unsigned Debug macOS App 的 NSWindow**，包含系统标题栏、原生 toolbar、Menu、disabled switch、TextField、ScrollView 和原生 sheet；不再用 ImageRenderer 的静态 chrome 代替主窗口。macOS 默认 500 × 680，最小 440 × 520，宽窗口 680 × 680；菜单栏内容宽 360。iOS 保留 NavigationStack / insetGrouped List。

## 可复现的真实窗口流程

```sh
pnpm install --frozen-lockfile
cargo build -p meshway-controller --locked
pnpm build:web
bash apps/apple/scripts/build-core.sh
xcodebuild -quiet -project apps/apple/Meshway.xcodeproj -scheme Meshway \
  -configuration Debug -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath apps/apple/build/DerivedData CODE_SIGNING_ALLOWED=NO \
  MESHWAY_BUNDLE_PREFIX=dev.meshway.snapshot build
pnpm exec tsc -p apps/apple/scripts/tsconfig.json
node apps/apple/scripts/snapshot.ts \
  "${TMPDIR%/}/opencode/apple-ui-polish/round-4"
```

脚本约需 2–3 分钟，可在输出目录参数后传入独立构建的 App executable。要求 `dev.meshway.snapshot.meshway` bundle ID，避免写入开发者 App 的窗口偏好；Debug configuration 显式定义 `DEBUG`。`App/WindowSnapshots.swift` 整体由 `#if DEBUG && os(macOS)` 包围，Release 不包含 hook。

- controller / Web 使用动态回环端口与私有临时数据目录；不操作既有服务或 Docker。
- 五个 peer 经生成的真实 enrollment API 创建，覆盖短名称、长名称、Unicode；真实保存 admin country 和启用 DNS。DNS 的 hash fallback 名称也是服务端真实结果。
- App 使用真实 Keychain 的唯一 `dev.meshway.snapshot.<UUID>` service、私有快照目录及独立 defaults suite。外部脚本批准真实 device flow；App 仅通过私有 IPC 交出授权 URL、接收 callback，随后经 Swift → UniFFI → 生成 Rust client → Web proxy 兑换和读取；App 不拥有 admin token。
- 明 / 暗 × 默认 / 最小 / 宽窗口，各捕获 signed-out、devices、device detail / selection、network details、菜单栏内容、无匹配搜索、停止 controller 后的真实刷新错误、admin revoke 后的真实撤销，以及 signed-out / revoked 菜单栏内容。
- 等待真实数据读取结束和窗口布局后，通过 `contentView.superview` 的 `bitmapImageRepForCachingDisplay` / `cacheDisplay` 写 PNG；sheet 单独捕获。PNG 使用当前显示屏的 native backing scale，不强制统一像素倍率。
- 正常退出与 SIGINT / SIGTERM 中断清理脚本创建的进程、临时数据、Keychain 和 defaults namespace；不输出凭据或 callback code。强制 SIGKILL 无法执行 finally，私有目录中的 `namespace` 只保存该次运行的公开清理标识。

## 四轮观察与修正

1. **Round 1**：Connect 标题消失、switch 过大，最小宽度变成 toolbar overflow；重复窗口标题挤占空间；DNS / IPv6 使行过高；section / separator edges 不一致；两种 sheet 底部被 footer 遮挡。
2. **Round 2**：紧凑 switch 与名称 + IPv4 / 次行 DNS 修正行密度，IPv6 只在详情；统一 `LayoutMetrics`、完整 560pt 内容高度的 sheet、hover / selection feedback 和撤销说明。发现隐藏标题后 Connect 跑到左侧、无 country 的 IPv4 列移位、面板末行贴近 viewport。
3. **Round 3**：native flexible toolbar spacer 保持 Connect 右上角；保留 dropdown indicator、固定 country 列，面板恢复底部留白。六种窗口组合与原生 sheet 全部比较，默认窗口可同时显示五个 peers；最小高度正常滚动。iPhone-width 的 accessibility-size 共享行采用上下布局，修正辅助 renderer 的透明背景 / 垂直居中。
4. **Round 4**：扩展 signed-out / revoked 菜单栏内容检查，统一居中和真实 reset 引导，并保留 tunnel notice。最终真实窗口矩阵无有意义的对齐、截断或拥挤缺陷；图片读取工具直接查看关键 PNG，并用 contact sheets 比较完整矩阵。

最终产物目录（90 张真实窗口 / sheet / panel-content PNG，`png-paths.txt` 列出完整路径）：

```text
/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/apple-ui-polish/round-4/
```

命名：`mac-<state>-<light|dark>-<default|min|wide>.png`；sheet 加 `-sheet`，panel 内容加 `-panel`。`detail` 的主窗口图记录原生 sheet dimming 和所选设备背景；不是把 sheet 人工拼在主图上。

尺寸为请求的布局 points；原生标题栏 / footer 与最小内容限制可能增加捕获窗口高度，不裁掉 chrome 来伪装为固定尺寸。

## 辅助 iPhone-width 检查与边界

```sh
MESHWAY_RENDER_DIR="${TMPDIR%/}/opencode/apple-ui-polish/round-4/secondary" \
  swift test --package-path apps/apple/MeshwayKit --filter renderAppleScreens
```

测试 fixtures 仅链接测试 target。ImageRenderer 渲染真实共享内容，在 393pt 宽度检查 onboarding、设备行与 `.accessibility3` 的上下布局；辅助产物中旧 macOS mock chrome 不再用于判断真实窗口质量。

- MenuBarExtra 无 Accessibility 自动化时不能可靠展开，使用独立 borderless NSWindow / NSHostingView 捕获 **真实 MeshwayMenuView 内容及原生控件**；未验证 MenuBarExtra 的系统定位、阴影或外部 popup menu。网络 dropdown label 和实际 network details sheet 已覆盖，原生 dropdown popup 未捕获。
- AppKit window-tree 缓存不包含 WindowServer 外部阴影，也不自动合成 attached sheet；主窗口和 sheet 分别保留。
- 本机没有 iOS Simulator runtime，没有下载；**native iOS chrome、实际 iOS Dynamic Type、Simulator / 真机运行未验证**。辅助 iPhone-size 图不等于 iOS 截图。
- 浏览器系统授权 UI、NE、真实隧道 / 设备连通性未验证。瞬时 loading 没有人为延长或注入；其共享空状态仅作辅助渲染检查。

## 行为与实现边界

- 数据来自 node bearer 的生成配置 / DNS GET；protocol 类型、共享摘要 / DNS 校验 / policy 编译、自身 ID / 已保存地址及 revision 绑定。显示读取不发送 ACK。
- 自身 heartbeat 时间 / 结果真实；peer 列表显示 controller 库存的名称、IPv4、DNS 名与 admin country，IPv6 在详情 / copy action。配置读取成功不代表数据面可用。
- 网络 Menu 当前列唯一真实接入；标题为启用的 DNS zone 或服务器 host。详情、复制 URL、本地重置是真实操作；本地重置有确认且不撤销服务端节点。
- Connect switch 始终 off、disabled，tooltip 和正文明确隧道未实现。多网络保存 / 切换尚未实现，没有空操作按钮。
- 自动前台读取跨窗口 30 秒节流；手动刷新立即读取、整段流程禁止重叠；保留旧库存时明确标示，403 持久撤销并清库存。

## 验证

- `cargo fmt --check`，通过。
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`，通过。
- `cargo test -p meshway-apple-core --locked`：2 unit + 1 real-controller integration，通过。
- `bash apps/apple/scripts/check.sh`：crate Clippy、workspace check、三平台 XCFramework、Swift tests（live opt-in test 跳过）、macOS / iOS Simulator unsigned build，通过。
- `pnpm exec tsc -p apps/apple/scripts/tsconfig.json`、`pnpm build:web`，通过。
- `node apps/apple/scripts/snapshot.ts …/round-4`：六组真实窗口、生成客户端接入 / 库存 / DNS / 撤销、清理，通过。中断 smoke 额外确认自己的进程、Keychain item、defaults 与临时目录被清理。
- `renderAppleScreens`，通过；iPhone-width / accessibility-size 辅助图已查看。
- macOS unsigned Release build，通过；`strings` 检查 Release executable 不含 `MESHWAY_SNAPSHOT_` 或 `MeshwaySnapshotAction`。
