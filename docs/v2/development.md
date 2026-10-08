# 开发服务、测试与环境

本页是开发环境的统一操作入口；何时运行哪一层、合入与分支规则见[工程流程](/v2/process)。工具链版本、锁定下载和任务入口由根 `mise.toml`／`mise.lock` 维护；各脚本拥有自己的参数、资源和清理逻辑。

## 选择入口

| 目的 | 入口与实现 | 验证边界 |
| --- | --- | --- |
| 常规源码检查 | `mise run check` | Rust fmt／严格 Clippy／行为测试、生成契约 drift、SDK 消费者、Console 类型／领域测试／构建、脚本测试、文档检查 |
| 内核 TUN 验收 | `mise run test:tun:linux` → `scripts/verify_tun.py` | 隔离 Linux 内的真实 TUN 设备和双栈包 I/O |
| 受保护包流验收 | `mise run test:packet:linux` → `scripts/verify_packet.py` | 独立进程／namespace 中的认证、完整路径和正式数据面包流 |
| daemon／CLI 综合验收 | `mise run test:runtime:linux` → `scripts/verify_runtime.py` | 正式 daemon／CLI、Underlay、TUN、系统 DNS、续租／撤销／清理；穿透模式增加独立 NAT 与 pcap 证据 |
| 并行验收矩阵 | `mise run validate:matrix` → `scripts/validate_matrix.py` | 一次构建、按行隔离、失败继续；支持诊断短模式 |
| 远程验收矩阵 | `mise run validate:remote` → `scripts/validate_remote.py` | 已推送 revision、远程 detached 执行、断线续等和本地证据副本 |
| 常驻开发服务 | `mise run services …` → `scripts/dev.py` | 本工作树的 Server／docs 进程生命周期和健康检查 |
| 常驻三节点实验环境 | `mise run node:lab …` → `scripts/node_lab.py` | lax／sha／nyc 的独立持久状态与不可变 Linux 程序部署，详见[三节点环境](/v2/node/docker-lab) |
| Linux Node 程序 | `mise run build:node:linux` → `scripts/node_lab.py build` | 宿主交叉编译常驻 lab 使用的 daemon／CLI |
| 独立性能测量 | `mise run perf:circuit`／`perf:transport`／`perf:dataplane` → `scripts/perf/{circuit,transport,dataplane}.py` | 显式 release 测量、JSON／Markdown 报告，层级与口径见[分层性能测量](/v2/node/performance) |

三个 `verify_*.py` 是递进的真实隔离验收，各自保留失败定位所需的证据，复用正式实现。性能测量与真实 Linux 隔离验收均由开发者显式运行，不进入常规 `check`。进程健康、控制面接入、真实包流和性能结果分别读取其证据。

## Rust 测试

`mise run test:rust` 先以锁定版本的 cargo-nextest 执行 workspace／all-targets／all-features 测试，再以 `cargo test --workspace --all-features --locked --doc` 执行 doctest。nextest 本身不执行 doctest；文档示例因此有独立入口。

```sh
mise install --locked
mise run test:rust
NEXTEST_PROFILE=ci mise run test:rust
mise exec -- cargo nextest run -p meshway-control --test control
```

`.config/nextest.toml` 的 default／ci profile 均不重试失败，单例运行超过 60 秒时报警；全局并发默认使用可用逻辑 CPU 数。真实 Server／SQLite／HTTP fixture 使用有界 test group，避免跨进程的 KDF、文件同步和后台 worker 同时拥塞。每个测试保留独立临时数据库、网络和 listener，全部自包含；nextest 每个 case 独立进程，进程内 singleton 不构成跨 case 的共享 Server。ci profile 收集全部失败并写入 `target/nextest/ci/junit.xml`。

测试任务为深层异步 fixture 设置 `RUST_MIN_STACK=8388608`，并用 `umask 077` 创建私有测试资源。系统 DNS 文件后端的测试要求 fixture 根目录的全部祖先通过正式私有存储检查（owner-only，无 group／world 写位）；fixture 依次探测 `TMPDIR`、`XDG_RUNTIME_DIR`、`HOME` 与当前目录，取第一个通过者，并把自身目录设为 `0700`，不依赖调用方的 umask 或 `/tmp`。测试 profile 只优化 Argon2／Blake2 依赖，保留生产 KDF 参数和业务代码的 debug assertions。

诊断时可回退到 libtest，命令覆盖与 nextest 相同的目标，并保留独立 doctest：

```sh
umask 077
RUST_MIN_STACK=8388608 mise exec -- cargo test --workspace --all-targets --all-features --locked -- --test-threads=4
mise exec -- cargo test --workspace --all-features --locked --doc
```

单 crate 修改优先执行 `mise exec -- cargo test -p <crate>` 或对应 nextest filter。完整计时记录源码提交、runner、宿主 load、构建是否已热身及测试墙钟；测试总数、最长单例与并发运行时的累计测试时间分别记录。

## 本机服务

在要部署的工作树执行：

```sh
mise run setup
mise run services status --json
mise run services start server
mise run services start docs
mise run services update server
mise run services stop docs
```

`services` 支持 `start|status|stop|restart|update [server|docs|all]`，以及显式的 `reset server`。省略服务选择时是 `all`，reset 必须写出 `server`。Server 本机地址为 `http://127.0.0.1:8420`，对外为 `https://meshway-dev-api.jdscript.dev`；Console 为 `http://127.0.0.1:3001`，docs 为 `http://127.0.0.1:5173/v2/`。Server 以 `--console-origin http://127.0.0.1:3001` 启动，设备批准链接指向本机 Console。健康检查和开发 seed 直接访问 loopback，不经过外部隧道。

Console 使用其所属任务运行：

```sh
MESHWAY_CONTROLLER_URL=https://meshway-dev-api.jdscript.dev mise run dev:console
```

对外域名只提供 HTTPS 转发。开发 Server 以 `--stun-server dev-public=stun.l.google.com:19302` 发布外部 STUN，由 Node 的实际 UDP socket 查询，需要实际网络可达；托管 STUN 另需显式的 `--stun-listen`。HTTPS 域名可用不证明任何公网 UDP 映射可用。lab 容器的 bootstrap DNS 见[三节点环境](/v2/node/docker-lab#节点与持久状态)。

### 生命周期与归属

- `start` 幂等启动或检查健康，缺少 Server 二进制时才构建；`restart` 重启现有程序；`update server` 构建锁定程序（同时产出 `meshway-server` 与离线维护工具 `ip-datasets`）后停旧启新。这些操作保留数据库、私有材料与开发登录文件，不隐式导入或替换部署数据集。
- 脚本记录实际启动命令与进程身份（tmux 会话 `meshway-server`／`meshway-docs`）；记录、进程或工作树路径与预期不符，或端口被外部 listener 占用时，拒绝操作而不迁移未知进程。
- `stop` 只对核验过的进程树发送 SIGINT，超时保留现场。
- 合并源码、重新编译和更新正在运行的服务是三个独立动作；部署后用 `services status --json` 核对，再从外部 URL 和 Console 检查实际访问。

### 显式重置整个开发库

```sh
mise run services reset server
```

此命令只针对当前工作树的 `.meshway/server`，顺序如下：

1. 核验私有目录／文件权限、已记录服务、端口、数据目录使用者和 seed 工具链；拒绝外来进程、符号链接、硬链接或非私有数据树。若部署了[独立 GeoIP／ASN 资产](#独立-geoip-asn-部署资产)，先校验并复制到本次私有备份中的 `ip-datasets/`，缺少所需二进制时在停服前构建。
2. 停止已核验 Server，确认端口释放和数据目录无人使用，并取得 Server 使用的 `controller.lock` 排他锁。
3. 将**整个** `.meshway/server` 原子移到 `.meshway/backups/server-<UTC时间>-<runId>/server`，保留数据库、WAL 和所有私有密钥；将旧 `.meshway/dev-login.json` 移到同一备份目录。备份目录权限为 `0700`，不对活动 SQLite 逐文件删除。
4. 进入 `datasets` 阶段；如有资产，按 `geoip`、`asn` 顺序通过正式 `ip-datasets` 工具将已校验快照导入 fresh `.meshway/server`。每次导入前确认 Server 未运行、端口释放，已创建的新目录没有使用者；App 持有自己的目录锁，完成 key、MMDB 类型／范围校验和事务写入后退出。
5. 用当前已构建程序启动 fresh Server，使用生成 TypeScript SDK 及其严格运行时 schema，在 loopback 创建固定开发账户和一个 `Development` 网络。
6. 核验成员／网络结果、确认加入链接已被对应成员单次消费并注销临时会话后，原子发布 `0600` 的 `.meshway/dev-login.json`。

| 账户 | Development 角色 |
| --- | --- |
| `admin@example.com` | Owner |
| `member@example.com` | Member |
| `carol@example.com` | Member |
| `dave@example.com` | Member |

账户名称不授予实例全局管理员权限。网络使用 Server 的正常初始配置，没有 seed Node；三节点接入后的运行配置见[三节点环境](/v2/node/docker-lab#首次启动)。旧账户、网络和 Node 身份属于备份库，原 Node 需要重新接入新网络；reset 不操作 Node 容器或状态卷。

登录文件的 `server`／`console` 是对外 Server URL 和 Console origin；`email`／`password` 是默认开发登录（固定为 `admin@example.com`），`networkId` 是新网络，`accounts` 是上述邮箱到**密码**的映射。若旧文件存在且格式有效，reset 只导入其 `accounts` 中四个种子账户的密码；首次创建则生成随机密码。脚本不在命令行参数或普通输出打印密码／token。

当前 reset 状态保存在 `.meshway/services/server-reset.json`，每次备份另有 `reset.json`；阶段为 `stopping`、`archiving`、`datasets`、`starting`、`seeding`、`complete` 或 `failed`，失败时记录 `failedAt`。journal 的 `datasets` 按种类记录 `sha256`、`source` 和 `installed`；只有对应导入命令成功退出才设为 `true`，`false` 表示未确认完成，不能据此认定失败事务未写入。失败返回非零，保留已有备份、资产快照及部分初始化的新库，不自动重试导入／seed、不自动再次清库。`status` 对未完成 reset 显示 `ready=false`，即使 HTTP 已能响应。

恢复时先检查两个 journal、私有服务日志及当前进程归属。正常 start／update 不会修复或重做 seed。若决定重新执行 reset，这是新的整库重置，会再次归档当前部分初始化目录；若恢复旧备份，应先核验并停止服务，将当前目录另行保留，再以同一批次的整个 `server` 和 `dev-login.json` 恢复，避免数据库与 key／凭据交叉配对。备份不会被脚本自动回收。

### 独立 GeoIP／ASN 部署资产

部署方将已取得的真实 MMDB 文件保存在当前工作树 `.meshway/ip-datasets/`，与业务库 `.meshway/server/` 分开。MaxMind 授权、账户和下载由部署方处理；reset 只复用这些本地文件，不自动下载、生成替代数据或从旧业务库回退选取数据。独立资产不随业务库 reset 删除。

目录中的固定文件为 `manifest.json`、`geoip.mmdb`／`asn.mmdb`。清单结构如下，摘要占位符必须替换为对应文件实际的 64 位小写十六进制 SHA-256：

```json
{
  "version": 1,
  "datasets": {
    "geoip": {
      "sha256": "<geoip.mmdb 的 SHA-256>",
      "source": "GeoLite2-City deployment snapshot"
    },
    "asn": {
      "sha256": "<asn.mmdb 的 SHA-256>",
      "source": "GeoLite2-ASN deployment snapshot"
    }
  }
}
```

- 顶层只接受 `version`、`datasets`；`version` 为整数 `1`。`datasets` 只接受 `geoip`／`asn`，至少一项；每项只接受 `sha256`、`source`，没有 `file` 字段，不接受重复 JSON 键。
- `source` 为 1–256 UTF-8 字节的非控制字符字符串，作为正式导入的来源记录。每个声明文件大小必须在 `(0, 128 MiB]` 内，实际摘要必须匹配清单。
- 资产目录为当前 UID 所有、权限 `0700`；清单和声明文件为当前 UID 所有的普通文件、权限 `0600`，禁止符号链接和硬链接。
- 整个资产目录缺失时，reset 保持无数据集的初始化行为；未提供种类的查询仍真实返回 `datasetUnavailable`。存在目录时，清单、声明文件、大小、摘要或权限检查失败均在停服前拒绝，保留原服务及业务库。
- 校验快照保存在 `.meshway/backups/server-<UTC时间>-<runId>/ip-datasets/`，含本次规范化 `manifest.json` 和已核对摘要的文件；目录／文件同样为 `0700`／`0600`。停服后仅从这一批快照读取，部署方后续更新源目录不会混入本次 reset。

归档后、fresh Server 启动前，脚本执行 `target/debug/ip-datasets --data-dir <fresh-path> import --kind <geoip|asn> --file <backup-snapshot> --source=<source>`，通过正式 App owner 导入；不会直接写 SQL，也不在运行中 Server 的数据目录执行导入。内容不符合 MMDB 类型／范围或导入失败时，journal 记录 `failedAt=datasets`，保留现场供检查。

## 隔离 Linux 验收与镜像

```sh
mise run test:tun:linux
mise run test:packet:linux
mise run node:lab image-build
mise run test:runtime:linux --build-only
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --dns-backend off
mise run test:runtime:linux --artifacts /absolute/private/bin --transport native-udp --traversal nat \
  --traversal-image-manifest /absolute/private/os/image-manifest.json
```

`--artifacts` 指向 `--build-only` 输出的 `bin` 目录；脚本验证 source manifest、目标架构和二进制摘要，不能把不同源码的产物当作当前结果。各承载／DNS／NAT 组合的验收状态见[验收范围](/v2/node/validation)，daemon／CLI 行为见[运行合同](/v2/node/runtime)。

| 目录 | 用途 |
| --- | --- |
| `dev/node-lab/assets.lock.json` | Ubuntu 基础镜像的唯一声明；纯内核／packet 验收使用固定 base，完整 runtime 的常规 actors 使用显式 `image-build` 选择的工具镜像并覆盖其 entrypoint |
| `dev/dns-fixture/` | DNS 场景的额外 OS 支持；`build.py` 维护共享软件包下载／依赖解析，校验锁定的包索引与摘要后以无网络 Docker build 产出 `image-manifest.json` |
| `dev/traversal-fixture/` | 穿透场景的 NAT 路由器与 pcap 支持；拓扑与调用参数见其 `README.md` |

这些目录只提供 OS、namespace 和证据读取支持，业务行为使用正式 Rust 实现。

DNS 模式显式传入 `--dns-backend resolved|resolv-conf` 和 `--dns-image-manifest /absolute/private/os/image-manifest.json`；`off` 模式不传 DNS 镜像。穿透模式 `--traversal nat` 要求 `--transport native-udp` 和 `--traversal-image-manifest`（以 `dev/dns-fixture/build.py --lock-file dev/traversal-fixture/assets.lock.json` 构建）；`--traversal-fallback` 另预授权 A–R 的 TCP 退路。OS 输出目录由 `--output` 指定，保留锁定输入、下载包和镜像清单；image ID／manifest 本身不代表行为验收成功。

脚本为运行生成唯一 runId，限制 namespace、资源和挂载，以精确容器／网络身份清理。异常退出后先对照该 run 的报告和资源标签调查，再清理遗留资源；不全量 prune，也不改未知归属的宿主路由、DNS、防火墙或 Docker daemon。

### 一次构建、并行矩阵 {#validation-matrix}

```sh
mise run validate:matrix --list
mise run validate:matrix --parallel 4
mise run validate:matrix --parallel 2 --short --rows nat/off nat+fallback/off
```

`scripts/validate_matrix.py` 的 `ROWS` 是可执行行的唯一声明，与[验收矩阵](/v2/node/validation#验收矩阵)的九个隔离入口对应。`--rows` 使用行 ID，省略时运行全部；重复 ID 只执行一次。runtime 构建一次，七个 runtime 行和 packet 的内嵌 daemon 检查都使用同一 `--artifacts`；packet／TUN 各构建一次。三个入口共享 `target/node-kernel-tests/build.lock`，只在构建及复制产物阶段持锁；导入已经冻结且逐文件校验的产物与执行阶段不持构建锁。

构建阶段从可变 Cargo target 复制并冻结产物。导入阶段仍校验源码／目标／二进制摘要，同一文件系统以硬链接共享只读可执行文件，避免每行复制多 GiB 的 debug 程序；跨文件系统才复制。容器对程序目录只读挂载，可写的外来产物不作为冻结输入接受。

`--parallel N` 默认是可用 CPU 数除以 8 向下取整、最少 1。依据是每个 runtime 行的 controller／A／R／B 各限制 2 CPU；NAT 行另外包含两个轻量路由器，需为宿主及路由器留出余量。并发不改变 fixture 的进程内期限，不保证高负载下仍通过。每行记录启动 `uptime`、1／5／15 分钟 load、耗时、报告 ID、失败阶段摘录和遗留资源；standalone 的 `verify_runtime.py`／`verify_packet.py` 只记录负载，不设空闲门槛；功能验收不以负载作为前提，矩阵用有界并发控制并发度。与历史串行数据比较时应同时读取负载和源码指纹。

各行、packet 内嵌 daemon、容器、bridge、挂载目录、私有 D-Bus／DNS 都有独立 run ID／namespace；固定监听端口只存在于各自 namespace，不发布宿主端口。IPv4 地址由 Docker IPAM 分配，IPv6 为每个 bridge 的随机 documentation prefix；共享 `/dev/net/tun` 不共享接口。实际并发受 CPU／内存、Docker 地址池剩余 bridge 数及 IO 影响；单个 NAT 行最多三个 bridge。失败不取消其他行，退出码表示全部选中行及清理是否通过。

Linux rootful Docker 的容器 root 与非 root 宿主 UID 不同：fixture 只为私有 bind 访问增加 `DAC_OVERRIDE`，保持目录／秘密文件私有权限；写入者退出后，用本 run 同标签、无网络、仅挂载该报告的 `CHOWN`／`DAC_OVERRIDE` helper 将证据归还宿主 UID。运行中的 ready marker 只导出精确单个文件；递归导出不跟随符号链接。helper 同样必须清理，不修改其他运行、宿主网络或系统文件。

启用 Docker AppArmor 的 Linux 上，仅 DNS actor 使用 `apparmor=unconfined`，使既有 `SYS_ADMIN` 能力可以执行其私有 mount namespace 中的挂载；保留 capability 限制、只读根、no-new-privileges 及 Docker 默认 seccomp，其他 actor 仍使用默认 AppArmor。

`underlay-negative` 内核测试故意把远端 underlay peer 地址加入 sink；其独立 namespace 的 IPv4 `rp_filter` 固定为 0（含 Docker endpoint 的接口设置），避免宿主继承的 loose RPF 在 marked-socket 检查之前丢弃回复。其他 actor 保留其默认设置，不改宿主 sysctl。Planck 上同一正式内核测试的隔离 A/B 已证明：`rp_filter=2` 超时，`0` 通过；此设置是该负向 fixture 的环境基线。

矩阵根据 Docker Linux 架构选择 `amd64`／`arm64` 的锁定 OS 输入；默认在工作树 `.meshway/validation/os/` 按输入摘要构建并缓存所需镜像。也可显式传 `--runtime-image-manifest`、`--dns-image-manifest`、`--traversal-image-manifest`；runtime 镜像使用 node-lab 的正式 provenance 校验，不改变常驻 lab 的选中镜像。TUN／packet 直接入口也支持 `--build-only`、`--artifacts`；所有入口可用 `--report-dir` 指定新的私有目录。

`--short` 将 runtime 持续／纯接收包流窗口由 125 秒改为 30 秒，退路触发由 30 秒改为 5 秒。报告及矩阵 `scope=diagnostic-short`，其余业务场景、真实 instance 到期等待、DNS、撤销恢复和清理仍执行。短模式不要求已经越过原需求期限；未观测到需求延长时 `laterExpiryMs=null`，不填造测量。它**不构成验收**，且整行耗时仍可能为数分钟。单独入口同样可运行 `mise run test:runtime:linux --short …`。

### 远程矩阵与断线续等

```sh
git push planck tooling/validate-matrix
mise run validate:remote --ref tooling/validate-matrix --parallel 4
mise run validate:remote --short --rows nat/off nat+fallback/off
mise run validate:remote --attach 20261007T232151Z-35296f56
```

`--host` 默认 `zj2631@ai4ce-planck.dyn.poly.edu`，`--path` 默认 `/mnt/data/Meshway/src`；`--ref` 默认本地 HEAD。入口先用 `git ls-remote planck` 校验 commit 可从已推送 ref 到达，再让远程 clean clone fetch，并在其 `.worktree/validation-matrix/` 专用 clean worktree 中 detached checkout。公共 clone 的普通 checkout 不会替换正在验证的源码。该专用 worktree 保留固定 Cargo cache；报告及 OS cache 仍位于 `--path` 下的 `.meshway/validation/`，通过 `MESHWAY_VALIDATION_ROOT` 传给矩阵。远程需具备 SSH、Git、Python、mise 锁定工具链、普通 Docker builder 和至少 50 GiB 空闲磁盘；本地使用 rsync 拉回证据。其余参数透传给矩阵。

远程 supervisor 使用独立 session，记录 `.meshway/validation/jobs/<run>/` 下的请求、PID、实际 worktree、日志及最终状态；工作树操作锁从 fetch／checkout 持有到矩阵及其子进程全部结束，第二个远程任务不会切换专用 worktree 中的源码。SSH 断连不停止已启动任务。`--timeout` 是本地等待期限（默认 14400 秒），到期退出 2 后可用启动时打印的 `--attach <run>` 续等，不能用 attach 改变参数。任一行失败或本 run 带标签的容器／网络仍有遗留均返回非零；其他工作流的容器不纳入该 run 清理。

本地每 15 秒读取一次远程进度；行状态或实际 fixture 阶段变化时立即显示，阶段不变时每分钟显示一次经过时间。进度只投影排队／运行／通过／失败和结构化 `phase`，不打印私有错误正文，也不把读取进度记为新的验收样本。

远程结果拉回工作树 `.meshway/validation/remote/<host>-<run>/` 后打印 `summary.md`；包含矩阵、各行原始日志／JSON／pcap、远程状态与 supervisor 日志，不复制构建目录及大体积可执行文件，原二进制与 hash 清单保留在远程。目标已存在时拒绝覆盖；下载中断将部分目录另存为 `*-partial-<id>`，再次 attach 使用新的完整目标。

## 数据、报告与回收

服务目录相对于**执行脚本的工作树**；Node lab、隔离验收和 transport 性能报告相对于 `git --git-common-dir` 对应的**仓库公共根目录**，circuit／dataplane 性能报告位于执行脚本的工作树。

| 所属入口 | 数据／报告／缓存位置 | 清理边界 |
| --- | --- | --- |
| 本机 services | 工作树 `.meshway/server/`、`.meshway/dev-login.json`、独立 `.meshway/ip-datasets/` 部署资产；`.meshway/services/` 的 launch 记录、reset journal、私有日志和 `dev.lock`；`target/` 构建缓存 | stop 保留数据；reset 整库移入 `.meshway/backups/`，并保存本次资产快照；独立部署资产保留，备份人工保留或回收 |
| 隔离 TUN／packet／runtime | 公共根 `.meshway/validation/{tun,packet,runtime}/<runId>/` 的报告、二进制、进程日志、场景数据和证据；工作树 `target/node-kernel-tests/` | 正常 finally 清理精确容器／网络，报告保留；失败查报告及 Docker 实际资源 |
| matrix／remote | 执行工作树 `.meshway/validation/matrix/<run>/matrix.json`、`summary.md`、`rows/<row>/<reportId>/`、构建日志及产物；本地远程副本 `.meshway/validation/remote/<host>-<run>/` | 每行独立精确清理；保留失败证据；不覆盖历史目录 |
| OS fixture | 显式 `--output` 目录中的包、Dockerfile、image manifest；Docker 的不可变 image ID | 先确认没有容器／构建引用，再回收该输出和对应镜像 |
| 常驻 Node lab | 公共根 `.meshway/node-lab/releases/`、`selected`、`operation.lock`、`deploy.json`；工作树 `target/node-linux/`；`meshway-{lax,sha,nyc}-state` named volumes | stop 先清理 TUN 并保留原意图与卷；deploy 核验 namespace 停止证明、复用身份卷并更换不可变程序；完整重新接入时按标签和精确资源身份单独处理 |
| perf | transport：公共根 `.meshway/perf/transport/<runId>/`；circuit／dataplane：工作树 `.meshway/perf/{circuit,dataplane}/<runId>/`；各含原始样本、清单、JSON／Markdown 报告；工作树 `target/{transport,circuit,dataplane}-perf/` | 保留可追溯报告；运行结束且无引用后回收相应缓存 |

大型构建前检查磁盘，至少保留 50 GiB；`CARGO_INCREMENTAL=0`，同任务复用指定 target。三个 Linux 验收入口共享 `target/node-kernel-tests`，构建锁串行化不同 feature 的构建；冻结产物导入后可按上述矩阵并行执行。回收工作树前确认没有 WIP、进程或挂载引用，并保留已提交分支和必要证据。
