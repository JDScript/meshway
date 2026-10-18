# macOS System Extension 验收 VM

`meshway-mac-vm` 是 [macOS App 设计](/v2/node/apple-design)的独立 Tart 验收机。App 在宿主 Xcode 构建，在 VM 内安装、激活与运行；网络使用 Tart 默认 NAT。VM 的 SIP／AMFI 开发设置只用于无 provisioning profile 的平台实验，不是发布方式。

## A3 Node 接线与当前部署

当前源码链接 `meshway-apple-bridge` 的 XCFramework，并由 extension 内唯一 Node 驱动 Apple Host。VM 仍安装下文的 A2 **0.1/2**，没有 VPN 配置；源码构建不代表 VM 已更新。通过 SSH 请求卸载旧 extension 时返回 `AuthorizationCreate failed: ... no user interaction was possible`（退出 70），须在来宾 GUI 完成旧 extension 的停用／授权后再部署。没有替换现有 App，没有停止 Tart。A3 私有证据在工作树 `.meshway/macos-lab/a3/`。

宿主构建（先检查磁盘余量超过 50 GiB）：

```sh
df -h /
MESHWAY_CORE_PROFILE=debug bash apps/meshway-apple/scripts/check.sh
MESHWAY_CORE_PROFILE=debug bash apps/meshway-apple/scripts/sign-lab.sh
```

`build.sh` 先运行 `build-core.sh`：mise 锁定 Rust、`CARGO_INCREMENTAL=0`，默认复用当前工作树 `target/`（可用 `CARGO_TARGET_DIR` 指定），目标 `aarch64-apple-darwin`、deployment target 26.0；生成 Swift、头文件和 `build/MeshwayCore.xcframework`。`MESHWAY_CORE_PROFILE` 默认 release，debug 用于 lab。生成目录被忽略；Xcode 直接引用生成物。

完成 GUI 授权与本页的升级清理后，用 `MESHWAY_VM_SSH_CONFIG` 和 `deploy-vm.sh` 部署。来宾 Node 命令（尚未完成 VM 验收）：

```sh
MESHWAY_SERVER_URL=https://192.168.64.1:PORT /Applications/Meshway.app/Contents/MacOS/Meshway --lab start-private
/Applications/Meshway.app/Contents/MacOS/Meshway --probe status
/Applications/Meshway.app/Contents/MacOS/Meshway --probe up --enrollment-key @/absolute/private/enrollment-key
/Applications/Meshway.app/Contents/MacOS/Meshway --probe down
```

`start-private` 保留 lab action 名，当前捕获完全由签名 netmap 决定，不再固定 `10.203.0.0/24`；旧默认路由 lab action 在此包拒绝。Server origin 经 NE providerConfiguration 交给 Node，远端必须 HTTPS 并通过平台信任；未重新设置环境变量时保留已有 VPN 配置的 origin，供 stop/start 离线恢复。`--probe up --server ORIGIN` 只核对正在运行的 Node origin，不改变 IPC 13 或切换网络。每个 probe 请求新建一条 XPC／Rust IPC 会话，完整帧上限 64 KiB＋4 字节；回包消费后 ACK，5 秒无 ACK 关闭。状态目录为 root 0700 的 `/Library/Application Support/Meshway`，日志为 os_log `dev.meshway.node`／`node`。

DNS 地址决定见 [Apple Host](/v2/node/apple-design#apple-host)。决定前使用明确关闭 DNS 的隔离网络做 Mesh IP 验收；需要 DNS 的 netmap 会阻断 applied。A3 目标环境为独立 Server 数据目录／随机端口和带所有权标签的 Linux peer 容器，不使用另一工作流的 `127.0.0.1:8420`。接入、双向包流、租约／停机／恢复和离线载入当前均未取得 VM 证据。

下文 A2 0.1/2 的平台实验步骤与结果描述的是薄壳版本；其中 echo／定时 fixture 与固定路由开关已退出当前源码。保留这些结论仅用于界定已经验证的平台前提。

## 资源与边界

| 项 | 取值 |
| --- | --- |
| VM 名称 | `meshway-mac-vm` |
| 实际系统 | macOS 26.6.2 (25G83) arm64；4 CPU／4096 MiB |
| 基础镜像 | `ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:eeec54bfe1f076e27786c5d92b89187a05b1d109b5071eb2dcdf02d596e34640` |
| 镜像形态 | Cirrus Labs 官方 Tahoe arm64，无 Xcode／Homebrew／Tart Guest Agent；比同系列 base 更小 |
| 镜像大小 | OCI 压缩约 22.34 GiB，虚拟磁盘 50 GB；clone 使用 APFS copy-on-write |
| 用户 | 官方镜像默认 `admin`，密码 `admin`；仅用于此开发 VM |
| 网络 | Tart 默认 shared NAT；不传 `--net-bridged`、`--net-softnet`，不修改宿主 DNS／路由／防火墙 |
| 构建 | `apps/meshway-apple/build/DerivedData`，macOS 26+、arm64、Swift 6 严格并发 |
| 安装 | VM 的 `/Applications/Meshway.app`；extension 位于 App 的 `Contents/Library/SystemExtensions` |
| 当前程序 | A2 Swift 薄壳；XPC echo／定时 fixture，未链接 Node／Rust bridge |

当前 VM 正常运行，IP 为 `192.168.64.2`，SSH 专用密钥和屏幕共享 `vnc://192.168.64.2` 可用。来宾实际读回 `csrutil status=disabled`、`kern.bootargs=amfi_get_out_of_my_way=1`（已重启）、`systemextensionsctl developer=on`、`kern.hv_vmm_present=1`。App／extension 0.1/2 已安装，extension 为 **activated enabled**；另有一个旧 0.1/1 实例为 `terminated waiting to uninstall on reboot`，由环境 owner 安排的来宾重启清理。用户批准 extension 和 VPN 后，真实 tunnel、root XPC、fd 发现和本页路由实验均已完成。最终 VPN 配置已移除，没有运行中的 shell tunnel；保留 extension 启用状态和 App，普通 HTTP 已恢复，默认路由为 en0→192.168.64.1。

Tart 2.32.1 官方发布包校验通过；镜像拉取前余量 178 GiB，clone 后 140 GiB。常驻 Tart 进程由环境 owner 启动并挂载只读构建目录；验收操作使用 SSH，不停止或替换该进程。需要重启来宾时在来宾内执行 `sudo shutdown -r now`。

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

常驻 VM 已配置专用密钥和独立 SSH config（位于仓库公共私有目录，不随工作树复制）：

```sh
export MESHWAY_VM_SSH_CONFIG=/Users/zeyu/Developer/meshway/.meshway/macos-lab/ssh/config
ssh -F "$MESHWAY_VM_SSH_CONFIG" meshway-mac-vm
scp -F "$MESHWAY_VM_SSH_CONFIG" artifact.zip meshway-mac-vm:artifact.zip
```

下面的 Recovery 设置只用于首次建立或重新 clone；当前 VM 已完成。

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
    sysctl kern.bootargs
   sudo systemextensionsctl developer on
   systemextensionsctl developer
   ```

`csrutil status` 必须显示 disabled，`nvram boot-args` 证明参数已写入，重启后的 `sysctl kern.bootargs` 证明当前启动使用了该参数；AMFI 对 NE entitlement 的实际放行仍以激活／加载结果判断。developer mode 不代替用户对 extension／VPN 的批准。

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
# 使用上面的专用 SSH config 时，不要求 tart 在 PATH 中：
MESHWAY_VM_SSH_CONFIG="$MESHWAY_VM_SSH_CONFIG" bash apps/meshway-apple/scripts/deploy-vm.sh
```

部署脚本默认从 Tart 解析 `meshway-mac-vm` 的 IP；指定 `MESHWAY_VM_SSH_CONFIG` 时，scp／ssh 都使用该 config 的 `meshway-mac-vm` Host。脚本检查 guest hypervisor 标记，再安装到 VM `/Applications`。若已有 App 则停止；更新前在 VM 内 Stop、移除本 App 的 VPN 配置、Deactivate extension、退出 App，并把旧 App 移到具名备份目录后重试。不要让宿主 Launch Services 打开构建产物。

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
sudo /usr/bin/log show --last 5m --style compact --predicate 'category == "shell" OR category == "xpc-shell"'
```

探针要求实际 echo、发送 ACK 和至少两个独立连续 push 才退出 0；4 秒截止，断线／缺帧返回非零。fixture 为 `shell-fixture: runtime=absent`，**不是 IPC 13 Status**。服务端最多 8 连接、每帧 1 MiB、每连接一个未 ACK push，5 秒 ACK 超时取消连接；连接中断撤销 timer 和将来 Rust stream 的取消入口。

无 GUI 的 VM 可通过同一 App 控制器触发实验：

```sh
/Applications/Meshway.app/Contents/MacOS/Meshway --lab activate
/Applications/Meshway.app/Contents/MacOS/Meshway --lab start-private
/Applications/Meshway.app/Contents/MacOS/Meshway --lab remove
```

`--lab` 检查 `kern.hv_vmm_present`，只接受一个 action：`activate`、`start-private`、`start-default`、`start-all`、`start-default-local`、`start-all-local`、`app-frame`、`remove`。`start-default` 捕获默认 IPv4 路由，`start-all` 额外启用 `includeAllNetworks`；`-local` 后缀设置 `excludeLocalNetworks=true`，优先用此组合。`app-frame` 验证与调试视图共用的控制器 send／状态更新及连续推送，随后关闭 XPC。start 退出 0 只代表请求被接受，实际 connected、provider PID／UID、NE 设置和探测日志仍须分别读回。

退出 77 表示需要 GUI 批准，78 表示要求重启，124 表示系统请求在 60 秒内未回答。不要反复提交代替用户批准。首次保存 VPN 配置如出现“Meshway would like to add VPN configurations”，仍需用户在 VM 中点 **Allow／允许**。当前 VM 已完成这两项批准。

外连实验分别在私网路由、默认 IPv4 路由、默认路由＋`includeAllNetworks` 下重启 tunnel。extension 在设置路由前完成一次 URLSession HTTPS 请求，在设置完成后再做一次（`https://1.1.1.1/cdn-cgi/trace`，10 秒截止），只记录 HTTP 状态或错误，不记录响应正文。对照普通 VM 进程访问同地址与 tunnel 包捕获，才能区分 provider 绕行、VM 物理网络不可用或请求落入隧道。URLSession 结果不能证明 Rust 普通 socket／UDP 的全部行为。

无 VPN 的来宾公网对照：`curl https://example.com` 与 `curl https://1.1.1.1/cdn-cgi/trace` 均返回 HTTP 200。每次扩大路由范围前先验证对照，失败则不继续 capture 实验。utun 只读观察已取得 provider UID 0 的 fd 5／7、接口 `utun4`；不读写包、不向 Rust 交接。kernel control socket 不一定通过 `getsockname` 暴露预期的本地地址族，因此扫描直接查询 `UTUN_OPT_IFNAME` 并验证返回名称。

### 扩大 capture 与独立回滚

**用户／协调者报告的观察**：用户曾在两个 lab 路由开关均开启时点击 Start tunnel 并批准 VPN，随后 SSH／VNC／ARP 均失联，协调者报告宿主 bridge100／vmenet0 正常，并重启 Tart 恢复。该事件的 VNC／ARP 细节没有由本实验独立复现。独立测试已复现 `includeAllNetworks=true`、`excludeLocalNetworks=false` 时的 SSH 中断，并证明来宾内定时回滚可以恢复；不能依赖 SSH／VNC 手动停掉这个不处理捕获包的 shell。

任何默认路由／`includeAllNetworks` 实验前，在 **来宾内**安排一个不依赖 SSH 连接存活的回滚进程。以下为已使用的 40 秒模式；确认进程存在后才开始：

```sh
mkdir -p "$HOME/meshway-a2-shell-evidence"
nohup /bin/sh -c 'sleep 40; /usr/sbin/scutil --nc stop "Meshway Shell Lab"' \
  </dev/null >"$HOME/meshway-a2-shell-evidence/rollback.log" 2>&1 &
rollback_pid=$!
kill -0 "$rollback_pid"
/Applications/Meshway.app/Contents/MacOS/Meshway --lab start-all-local
```

先测 `-local`，再单独测试不排除本地网络的组合。SSH 失联时等待独立回滚，再核对 Disconnected、普通 HTTP 恢复及回滚日志，不用重启 Tart 代替清理。实验实际使用 `nohup` 独立脚本记录 stop 退出码和状态，三种扩大 capture 组合均回滚成功。

| Capture | includeAllNetworks | excludeLocalNetworks | provider NE 设置前／后 HTTPS | 普通来宾 HTTPS（前／中／回滚后） | SSH 专项观察 |
| --- | --- | --- | --- | --- | --- |
| `10.203.0.0/24` | false | false | 200／200 | capture 期间 200 | App 控制器／CLI 可连接 |
| 默认 IPv4 | false | true | 200／200 | 200／8 秒超时／200 | 未专项统计 |
| 默认 IPv4 | true | true | 200／200 | 200／8 秒超时／200 | 60 秒内 29 次真实 SSH 命令均成功 |
| 默认 IPv4 | true | false | 200／200 | 200／8 秒超时／200 | 约第 2 秒起 SSH 超时，40 秒回滚后第 42 秒采样恢复 |

includeAll／excludeLocal 值从来宾实际保存的 NE preferences 读回；capture 生效另以 Connected／主接口及普通进程请求超时对照。上述外连证据限 IPv4 URLSession HTTPS，不外推到 Rust UDP／TCP、IPv6 或漫游。`excludeLocalNetworks` 仅在这些 lab action 中显式选择，不据此决定正式 Apple Host 的业务策略。

## 停止、重置与清理

日常停止：App Stop → VM `sudo shutdown -h now`。无响应时宿主 `tart stop meshway-mac-vm`；该命令是 VM 强制停止，不能当作 extension 有序关闭的验收。

重置先保留所需证据、移除本 App 的 VPN 配置并停机，再为旧 VM 具名备份：

```sh
tart rename meshway-mac-vm meshway-mac-vm-backup
tart clone ghcr.io/cirruslabs/macos-tahoe-vanilla@sha256:eeec54bfe1f076e27786c5d92b89187a05b1d109b5071eb2dcdf02d596e34640 meshway-mac-vm
```

新 clone 需要重新执行 Recovery 设置。确认具名备份无运行／挂载引用且不再需要后用 `tart delete meshway-mac-vm-backup` 精确回收。完全销毁本 lab 时停机后 `tart delete meshway-mac-vm`；不要用全局 `systemextensionsctl reset` 或全量 Tart prune 清理其他项目。

## 验收范围

结论维护在 [macOS App 平台验证](/v2/node/apple-design#unverified)。Swift 平台部分已有 VM 中真实激活／tunnel、root 对端、认证负例、连续回包、正常关闭与 SIGTERM 取消证据。App 控制器通过 `--lab app-frame` 验证共用数据路径，不代替 GUI 人工视觉验收。当前薄壳尚无 Node 运行时，因此完整 A2 的 control-only Status 和 Rust 分派仍未交付。

Xcode 27.1 上 unsigned App＋extension、`sign-lab.sh`（真实开发签名、无 profile）和静态 Team／identifier requirement 校验通过；普通 `build.sh` 因缺少 NE／System Extension provisioning profile 失败。VM 内签名校验、真实运行和平台实验通过的精确范围见上表及设计页。私有原始证据位于仓库公共 `.meshway/macos-lab/`，索引为 `evidence-index.md`：`v2-private-xpc.log`／`xpc-negative.log` 记录帧与认证；`route-cases.log` 记录 capture 对照；`*.ssh-management.log` 记录实际 SSH 中断／恢复；`completed-final-state.log` 记录最终私网重连、双入口回包、VPN 移除、普通 HTTP 恢复与 extension 保留；用户／协调者的先前观察单列 `reported-full-capture-observation.md`。初次 Python raw-socket 管理探测的 baseline 无效，保留原始记录但不用于结论。
