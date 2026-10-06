# 开发服务、测试与环境

本页是开发环境的统一操作入口。工具链版本、锁定下载和任务入口由根 `mise.toml`／`mise.lock` 维护；各脚本拥有自己的参数、资源和清理逻辑。

## 选择入口

| 目的 | 入口与实现 | 验证边界 |
| --- | --- | --- |
| 常规源码检查 | `mise run check` | Rust fmt／严格 Clippy／行为测试、生成契约 drift、SDK 消费者、Console 类型／领域测试／构建、脚本测试、文档检查 |
| 内核 TUN 验收 | `mise run test:tun:linux` → `scripts/verify_tun.py` | 隔离 Linux 内的真实 TUN 设备和双栈包 I/O |
| 受保护包流验收 | `mise run test:packet:linux` → `scripts/verify_packet.py` | 独立进程／namespace 中的认证、完整路径和正式数据面包流 |
| daemon／CLI 综合验收 | `mise run test:runtime:linux` → `scripts/verify_runtime.py` | 正式 daemon／CLI、Underlay、TUN、系统 DNS、续租／撤销／清理；穿透模式增加独立 NAT 与 pcap 证据 |
| 常驻开发服务 | `mise run services …` → `scripts/dev.py` | 本工作树的 Server／docs 进程生命周期和健康检查 |
| 常驻三节点实验环境 | `mise run node:lab …` → `scripts/node_lab.py` | lax／sha／nyc 的独立持久状态与不可变 Linux 程序部署，详见[三节点环境](/v2/node/docker-lab) |
| Linux Node 程序 | `mise run build:node:linux` → `scripts/node_lab.py build` | 宿主交叉编译常驻 lab 使用的 daemon／CLI |
| 独立性能测量 | `mise run perf:circuit`／`perf:transport`／`perf:dataplane` → `scripts/perf/{circuit,transport,dataplane}.py` | 显式 release 测量、JSON／Markdown 报告，层级与口径见[分层性能测量](/v2/node/performance) |

三个 `verify_*.py` 是递进的真实隔离验收，各自保留失败定位所需的证据，复用正式实现。性能测量与真实 Linux 隔离验收均由开发者显式运行，不进入常规 `check`。进程健康、控制面接入、真实包流和性能结果分别读取其证据。

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

`services` 支持 `start|status|stop|restart|update [server|docs|all]`，以及显式的 `reset server`。省略服务选择时是 `all`，reset 必须写出 `server`。Server 本机地址为 `http://127.0.0.1:8420`，对外为 `https://meshway-dev-api.jdscript.dev`；Console 为 `http://127.0.0.1:3001`，docs 为 `http://127.0.0.1:5173/v2/`。健康检查和开发 seed 直接访问 loopback，不经过外部隧道。

Console 使用其所属任务运行：

```sh
MESHWAY_CONTROLLER_URL=https://meshway-dev-api.jdscript.dev mise run dev:console
```

对外域名只提供 HTTPS 转发。开发启动命令显式发布外部 STUN `dev-public=stun.l.google.com:19302`，由 Node 的实际 UDP socket 查询；该服务及容器配置的 bootstrap DNS `1.1.1.1:53` 都需要实际网络可达。托管 STUN 另需独立、显式配置的 UDP 监听／可达地址；HTTPS 域名可用不证明任何公网 UDP 映射可用。

### 生命周期与归属

- `start` 幂等启动或检查健康；缺少 Server 二进制时才构建。`restart` 重启现有程序，`update server` 构建锁定程序后停旧启新。services 的构建同时产出 `meshway-server` 和离线维护工具 `ip-datasets`。这些操作保留整个数据库、私有材料与开发登录文件，不隐式导入或替换部署数据集。
- 脚本保存真实启动的 argv、tmux 命令、pane、PID、进程启动时间、路径和端口；检查记录、当前进程命令及监听进程树后才能停止。源码中的期望参数改变时，旧记录仍能证明旧进程归属，update 随后按新命令启动。
- tmux 会话名为 `meshway-server`／`meshway-docs`。缺失或旧格式记录、空启动命令、PID 重用、路径／命令不一致、同名多 pane 会话、外部 listener 均阻断操作。旧手动服务须先按实际进程证据由操作者停止，再用此入口启动；脚本不自动迁移未知进程。
- 停止只对核验过的进程树发送 SIGINT，超时保留现场。多个工作树共用这些本机端口和会话名，记录中的工作树路径必须匹配。
- 合并源码、重新编译和更新正在运行的服务是三个独立动作。部署后用 `services status --json` 核对当前服务，再从外部 URL 和 Console 检查实际访问。

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

账户名称不授予实例全局管理员权限。网络使用 Server 的正常初始配置，没有 seed Node。三节点批准接入后，运行 `mise run node:lab configure` 提交 UDP 与节点间 ICMP 运行配置并保留 Server DNS 意图，见[Ubuntu 三节点环境](/v2/node/docker-lab)。旧账户、网络和 Node 身份属于备份库，原 Node 需要重新接入新网络；reset 不操作 Node 容器或状态卷。

登录文件的 `server`／`console` 是对外 Server URL 和 Console origin；`email`／`password` 是默认开发登录，`networkId` 是新网络，`accounts` 是上述邮箱到**密码**的映射。若旧文件存在且格式有效，显式 reset 保留这些密码作为 seed；首次创建则生成随机密码。脚本不在命令行参数或普通输出打印密码／token。

旧文件的顶层默认登录可以是另一个旧 Owner，不一定属于四个种子账户；只有顶层邮箱属于这四个账户时，才要求其密码与 `accounts` 中对应值一致。reset 只导入 `accounts` 中精确四个密码，新文件的主登录固定为 `admin@example.com`（Development Owner）；旧非种子 Owner 及原登录文件保留在整库备份中。

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

Python 回归验证快照、停服／归档／导入／启动／seed 顺序和失败保留行为；真实数据可用性仍需正式 import 后重启及实际查询验证。维护工具短时打开 App 不代表 Node execution 或真实包流验收。

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

`--artifacts` 指向 `--build-only` 输出的 `bin` 目录；脚本验证 source manifest、目标架构和二进制摘要，不能把不同源码的产物当作当前结果。各承载／DNS 组合的实际验收范围见[数据面](/v2/node/dataplane)，daemon／CLI 行为见[运行合同](/v2/node/runtime)。

Ubuntu 基础镜像的唯一声明在 `dev/node-lab/assets.lock.json`。纯内核／packet 验收使用固定 base；完整 runtime 的常规 actors 使用显式 `image-build` 选择的 Ubuntu 工具镜像，并覆盖其 entrypoint，仅运行 fixture。DNS 与穿透需要的额外 OS／pcap 支持分别归 `dev/dns-fixture/`、`dev/traversal-fixture/`；共享软件包下载／依赖解析由 `dev/dns-fixture/build.py` 维护，校验锁定的包索引／软件包摘要后以无网络 Docker build 产出 `image-manifest.json`。具体穿透拓扑与调用参数见 `dev/traversal-fixture/README.md`。这些目录提供 OS、namespace 和证据读取支持，业务行为使用正式 Rust 实现。

DNS 模式显式传入 `--dns-backend resolved|resolv-conf` 和 `--dns-image-manifest /absolute/private/os/image-manifest.json`；`off` 模式不传 DNS 镜像。穿透模式 `--traversal nat` 要求 `--transport native-udp` 和 `--traversal-image-manifest`（以 `dev/dns-fixture/build.py --lock-file dev/traversal-fixture/assets.lock.json` 构建）；`--traversal-fallback` 另预授权 A–R 的 TCP 退路。OS 输出目录由 `--output` 指定，保留锁定输入、下载包和镜像清单；image ID／manifest 本身不代表行为验收成功。

脚本为运行生成唯一 runId，限制 namespace、资源和挂载，以精确容器／网络身份清理。异常退出后先对照该 run 的报告和资源标签调查，再清理遗留资源；不全量 prune，也不改未知归属的宿主路由、DNS、防火墙或 Docker daemon。

## 数据、报告与回收

服务目录相对于**执行脚本的工作树**；Node lab、隔离验收和 transport 性能报告相对于 `git --git-common-dir` 对应的**仓库公共根目录**，circuit／dataplane 性能报告位于执行脚本的工作树。

| 所属入口 | 数据／报告／缓存位置 | 清理边界 |
| --- | --- | --- |
| 本机 services | 工作树 `.meshway/server/`、`.meshway/dev-login.json`、独立 `.meshway/ip-datasets/` 部署资产；`.meshway/services/` 的 launch 记录、reset journal、私有日志和 `dev.lock`；`target/` 构建缓存 | stop 保留数据；reset 整库移入 `.meshway/backups/`，并保存本次资产快照；独立部署资产保留，备份人工保留或回收 |
| 隔离 TUN／packet／runtime | 公共根 `.meshway/validation/{tun,packet,runtime}/<runId>/` 的报告、二进制、进程日志、场景数据和证据；工作树 `target/node-kernel-tests/` | 正常 finally 清理精确容器／网络，报告保留；失败查报告及 Docker 实际资源 |
| OS fixture | 显式 `--output` 目录中的包、Dockerfile、image manifest；Docker 的不可变 image ID | 先确认没有容器／构建引用，再回收该输出和对应镜像 |
| 常驻 Node lab | 公共根 `.meshway/node-lab/releases/`、`selected`、`operation.lock`、`deploy.json`；工作树 `target/node-linux/`；`meshway-{lax,sha,nyc}-state` named volumes | stop 先清理 TUN 并保留原意图与卷；deploy 核验 namespace 停止证明、复用身份卷并更换不可变程序；完整重新接入时按标签和精确资源身份单独处理 |
| perf | transport：公共根 `.meshway/perf/transport/<runId>/`；circuit／dataplane：工作树 `.meshway/perf/{circuit,dataplane}/<runId>/`；各含原始样本、清单、JSON／Markdown 报告；工作树 `target/{transport,circuit,dataplane}-perf/` | 保留可追溯报告；运行结束且无引用后回收相应缓存 |

大型构建前检查磁盘，至少保留 50 GiB；`CARGO_INCREMENTAL=0`，同任务复用指定 target。三个 Linux 验收入口共享 `target/node-kernel-tests`，不同 feature 的可执行测试不并发使用这个 target。回收工作树前确认没有 WIP、进程或挂载引用，并保留已提交分支和必要证据。
