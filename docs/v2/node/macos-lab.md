# macOS System Extension 验收 VM

`meshway-mac-vm` 是 [macOS App 设计](/v2/node/apple-design)的独立 Tart 验收机。App 在宿主 Xcode 构建，在 VM 内安装、激活与运行；网络使用 Tart 默认 NAT。VM 的 SIP／AMFI 开发设置只用于无 provisioning profile 的平台实验，不是发布方式。

## 资源与边界

| 项 | 取值 |
| --- | --- |
| VM 名称 | `meshway-mac-vm` |
| 基础镜像 | `ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:eeec54bfe1f076e27786c5d92b89187a05b1d109b5071eb2dcdf02d596e34640` |
| 镜像形态 | Cirrus Labs 官方 Tahoe arm64，无 Xcode／Homebrew／Tart Guest Agent；比同系列 base 更小 |
| 镜像大小 | OCI 压缩约 22.34 GiB，虚拟磁盘 50 GB；clone 使用 APFS copy-on-write |
| 用户 | 官方镜像默认 `admin`，密码 `admin`；仅用于此开发 VM |
| 网络 | Tart 默认 shared NAT；不传 `--net-bridged`、`--net-softnet`，不修改宿主 DNS／路由／防火墙 |
| 构建 | `apps/meshway-apple/build/DerivedData`，macOS 26+、arm64、Swift 6 严格并发 |
| 安装 | VM 的 `/Applications/Meshway.app`；extension 位于 App 的 `Contents/Library/SystemExtensions` |
| 当前程序 | A2 Swift 薄壳；XPC echo／定时 fixture，未链接 Node／Rust bridge |

本机建立记录（2026-10-17）：Tart 2.32.1 官方发布包校验通过；镜像拉取前余量 178 GiB，clone 后 140 GiB。`meshway-mac-vm` 为 4 CPU／4096 MiB，Tart 能报告 running，但正常启动的 SSH 未建立，Recovery 的自动显示检查只取得黑屏／光标，未取得可操作界面。SIP、AMFI boot-args、System Extension developer mode **均未取得来宾实际状态**，对应修改命令没有成功执行回执。VM 当前已停止，保留磁盘，需从本机交互式终端启动 Tart 并完成下面的 Recovery 步骤。已签名构建产物保留在宿主构建目录；只读目录共享已配置试启动，但来宾安装未完成。

## 安装 Tart 与创建 VM

```sh
df -h /
brew install cirruslabs/cli/tart
tart pull ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:eeec54bfe1f076e27786c5d92b89187a05b1d109b5071eb2dcdf02d596e34640
tart clone ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:eeec54bfe1f076e27786c5d92b89187a05b1d109b5071eb2dcdf02d596e34640 meshway-mac-vm
tart set meshway-mac-vm --cpu 4 --memory 4096
df -h /
tart run meshway-mac-vm
```

拉取前按压缩层、解压磁盘和 clone 写入预算保留至少 50 GiB；拉取／clone 后再次核对。已有同名 VM 时先检查 `tart list`／`tart get meshway-mac-vm`，不覆盖。资源在 `~/.tart`，不要全量 prune 其他 VM／镜像。

Homebrew 7 如拒绝 tap 2.32.1 formula 的重复 `depends_on :macos`，可使用该 formula 指向的官方发布包；不用修改 Homebrew tap 或宿主网络配置：

```sh
mkdir -p .meshway/macos-lab/tools
curl -fL https://github.com/cirruslabs/tart/releases/download/2.32.1/tart.tar.gz \
  -o .meshway/macos-lab/tools/tart.tar.gz
echo '8554ab4f7fc12afe52f9b7e3093a935673cbac737a83973d2db7a0683c814529  .meshway/macos-lab/tools/tart.tar.gz' | shasum -a 256 -c -
tar -xzf .meshway/macos-lab/tools/tart.tar.gz -C .meshway/macos-lab/tools
export TART="$PWD/.meshway/macos-lab/tools/tart.app/Contents/MacOS/tart"
"$TART" --version
```

后续命令中的 `tart` 可替换为 `"$TART"`。工作树共用仓库公共 `.meshway/macos-lab` 中的工具与证据；不把 VM 磁盘放进 Git。

## 登录与 Recovery 设置

正常启动后从宿主进入 VM：

```sh
tart ip meshway-mac-vm
ssh admin@"$(tart ip meshway-mac-vm)"
```

SSH 密码为 `admin`。首次核对并保存该 VM 的 SSH host key；重建 VM 后只更新对应 VM 的 known_hosts 条目。vanilla 没有 guest agent，不能用 `tart exec` 代替 SSH。Recovery 没有正常系统的 SSH 服务，使用 Tart GUI。

1. 在 VM 内执行 `sudo shutdown -h now`，等 `tart run` 退出。
2. 宿主执行 `tart run --recovery meshway-mac-vm`。
3. 在 **VM 窗口**选择恢复用户 `admin` 并输入 `admin`（若询问），进入 Recovery 后从菜单 **Utilities → Terminal** 打开终端。
4. 在 Recovery 终端执行：

   ```sh
   csrutil disable
   nvram boot-args="amfi_get_out_of_my_way=1"
   ```

   按 `csrutil` 提示确认并输入 VM 管理员凭据。记录命令结果；不要在宿主终端执行这些命令。
5. 在 VM 的 Apple 菜单选择关机，待 Tart 退出，再由宿主运行 `tart run meshway-mac-vm`（不带 `--recovery`）。
6. 在正常启动的 **VM 内**执行：

   ```sh
   csrutil status
   nvram boot-args
   sudo systemextensionsctl developer on
   systemextensionsctl developer
   ```

`csrutil status` 必须显示 disabled，boot-args 必须读回对应值；boot-args 仅证明参数已写入，AMFI 对 NE entitlement 的实际放行仍以激活／加载结果判断。developer mode 不代替 SIP／签名校验，也不代替用户对 extension／VPN 的批准。

## 构建与签名

两个 target：`Meshway`（SwiftUI App）和 `PacketTunnel`（System Extension 可执行程序）。`Shared/FrameProtocol.swift` 只定义不透明 `Data` 帧和签名 requirement。`App --probe` 与 GUI 是同一个签名可执行文件；未来 CLI 加入独立 identifier 后须显式扩展 allowlist。

```sh
bash apps/meshway-apple/scripts/check.sh
bash apps/meshway-apple/scripts/build.sh
```

`check.sh` 强制 `CODE_SIGNING_ALLOWED=NO`，检查 App 与 extension 编译及 plist／shell 语法。`build.sh` 使用 `Config/Shared.xcconfig` 和被忽略的 `Config/Local.xcconfig`。默认无 Team；本地配置示例：

```text
DEVELOPMENT_TEAM = YOUR_TEAM_ID
MESHWAY_BUNDLE_PREFIX = dev.yourteam.lab
CODE_SIGN_IDENTITY = Apple Development
```

Team ID 取证书 subject 的 OU 或 Developer 账户 Team ID，不是证书显示名称括号内的个人标识。App Group／Mach service 从 Team 和 bundle prefix 派生；XPC 双向校验 `anchor apple generic`、相同 Team 和精确 executable identifier。unsigned 构建不能通过这一认证。

具备匹配 provisioning profile 时正常 `build.sh` 签名。无 profile 的 VM 实验显式使用：

```sh
bash apps/meshway-apple/scripts/sign-lab.sh
```

该命令以 Local.xcconfig 构建，再用所选真实开发证书签署 extension 和 App，保留 NE entitlement，不嵌入 provisioning profile。它不公证、不声明有效分发权限。证书／本地设置不提交，签名成功不等于 VM 能加载 NE。

## 部署与检查

```sh
TART="${TART:-tart}" bash apps/meshway-apple/scripts/deploy-vm.sh
```

部署脚本只从名称 `meshway-mac-vm` 取得 IP，使用 scp 上传、SSH 检查 guest hypervisor 标记，再安装到 VM `/Applications`。若已有 App 则停止；更新前在 VM 内 Stop、移除本 App 的 VPN 配置、Deactivate extension、退出 App，并把旧 App 移到具名备份目录后重试。不要让宿主 Launch Services 打开构建产物。

SSH 尚不可用时可先挂只读产物目录，在恢复正常启动并登录后从 VM Finder 拷贝：

```sh
tart run meshway-mac-vm \
  --dir="meshway-build:$PWD/apps/meshway-apple/build/DerivedData/Build/Products/Debug:ro"
```

来宾目录为 `/Volumes/My Shared Files/meshway-build/Meshway.app`。在 VM 内复制到 `/Applications` 后打开；不要直接从共享目录激活 System Extension。已有 `.app` 的归属与更新要求同 SSH 部署。共享目录仍被运行 VM 引用时，不回收对应工作树／构建产物。

在 VM App 中依次选择 **Activate extension**、在 System Settings 批准、**Start tunnel** 并批准 VPN 配置，再发 XPC 帧。默认路由仅 `10.203.0.0/24`、MTU 1280；薄壳丢弃捕获包，不安装 DNS。默认关闭两个扩大捕获范围的 lab 开关。

VM SSH 会话中：

```sh
systemextensionsctl list
codesign -d --entitlements :- /Applications/Meshway.app
/Applications/Meshway.app/Contents/MacOS/Meshway --probe
log show --last 5m --style compact --predicate 'category == "shell" OR category == "xpc-shell"'
```

探针要求实际 echo、发送 ACK 和至少两个独立连续 push 才退出 0；4 秒截止，断线／缺帧返回非零。fixture 为 `shell-fixture: runtime=absent`，**不是 IPC 13 Status**。服务端最多 8 连接、每帧 1 MiB、每连接一个未 ACK push，5 秒 ACK 超时取消连接；连接中断撤销 timer 和将来 Rust stream 的取消入口。

外连实验分别在私网路由、默认 IPv4 路由、默认路由＋`includeAllNetworks` 下重启 tunnel。extension 在设置路由前完成一次 URLSession HTTPS 请求，在设置完成后再做一次（`https://1.1.1.1/cdn-cgi/trace`，10 秒截止），只记录 HTTP 状态或错误，不记录响应正文。对照普通 VM 进程访问同地址与 tunnel 包捕获，才能区分 provider 绕行、VM 物理网络不可用或请求落入隧道。URLSession 结果不能证明 Rust 普通 socket／UDP 的全部行为。

## 停止、重置与清理

日常停止：App Stop → VM `sudo shutdown -h now`。无响应时宿主 `tart stop meshway-mac-vm`；该命令是 VM 强制停止，不能当作 extension 有序关闭的验收。

重置先保留所需证据、移除本 App 的 VPN 配置并停机，再为旧 VM 具名备份：

```sh
tart rename meshway-mac-vm meshway-mac-vm-backup
tart clone ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:eeec54bfe1f076e27786c5d92b89187a05b1d109b5071eb2dcdf02d596e34640 meshway-mac-vm
```

新 clone 需要重新执行 Recovery 设置。确认具名备份无运行／挂载引用且不再需要后用 `tart delete meshway-mac-vm-backup` 精确回收。完全销毁本 lab 时停机后 `tart delete meshway-mac-vm`；不要用全局 `systemextensionsctl reset` 或全量 Tart prune 清理其他项目。

## 验收范围

结论维护在 [macOS App 待验证项](/v2/node/apple-design#unverified)。VM 启动、代码签名和 unsigned 编译分别记录；只有 VM 中真实激活／加载、对端认证正负例、流式回包和取消证据才能完成 A2 平台验收。当前薄壳尚无 Node 运行时，因此不能交付 control-only Status 或 Mesh 包流。

Xcode 27.1 上 `check.sh`（unsigned App＋extension）、`sign-lab.sh`（真实开发签名、无 profile）和静态 Team／identifier requirement 校验通过；普通 `build.sh` 因缺少 NE／System Extension provisioning profile 失败。`mise run check:docs` 通过。上述结果不包含 VM 内 App 安装、System Extension 激活、XPC 实际回包或 provider 外连。
