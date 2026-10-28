# Server：控制面

Server 是账户、网络配置和执行许可的权威。它聚合 Node 报告并向当前主体提供裁剪视图；不会用库存、心跳或规划路径代替真实数据面状态。

## 功能与模块

应用位于 `apps/meshway-server`，内部按端到端职责分模块：

| 模块 | 职责 |
| --- | --- |
| accounts／networks | 账户、会话、成员、角色、加入链接与网络元数据 |
| enrollment | 持钥接入、浏览器批准、接入密钥、身份转换 |
| configuration | JSONC／typed ops、校验、私有材料准备、CAS、完整版本与恢复 |
| execution | 合法需求合并、编译发布、逐节点目标、实例／lease／回执 |
| state／observations | 报告准入、可见性、SSE、历史与短期诊断任务 |
| connectivity／stun | STUN 服务与 UDP 穿透 offer 协调 |
| resources | 规则集与 GeoIP／DNS 等可信输入的管理边界 |
| secrets | 配置密封与主密钥 |
| store | 全部 SQL（按领域分子模块，函数以领域类型为输入输出）、事务执行器、当前 schema 初始化与完整性检查 |
| http | 请求抽取、同源 API、错误和生成声明；不另写领域规则 |

领域模块只做事务编排和内存逻辑，不含 SQL 文本；store 函数不含领域决策。

### 配置提交入口

配置版本只有一个提交入口。管理编辑、Node 接入／身份转换、成员移除和邮箱维护是它的四种 **intent**：intent 决定谁有权、如何从当前版本写出目标草稿，以及允许的差异范围（编辑不新增节点、身份只能改为非空 Tag；接入只动被接纳的节点；移除只删除该成员的 User 设备；邮箱维护只改可读名字和绑定）。之后的材料、规范化、版本写入与执行发布对所有 intent 相同，流水线与 effects 见[数据库执行与提交](/v2/storage#数据库执行与提交)。

## 当前能力

| 状态 | 范围 |
| --- | --- |
| 已接通 | 账户／会话、网络／成员、加入链接、密码恢复、配置编辑／预览／CAS／历史、私有配置加密 |
| 已接通 | Node 持钥／PKCE 接入、实例／lease、按需执行目标、回执、永久撤销 |
| 已接通 | State／历史、用户与 Node SSE、固定 revision 应用进度、限时观察任务、STUN 与 UDP 穿透协调 |
| 已接通 | 规则集 HTTPS 下载、精确 artifact 绑定、单次 apply 准备和周期更新 |
| 未完成 | 完整类型化配置诊断／影响预览、地址重分配／材料轮换操作、非管理员 Tag owner 的局部入口 |
| 未完成 | 可信 DNS／GeoIP 输入与自动方案闭环、规划 evaluate／部分管理执行视图、版本兼容展示；Node 安装范围见 [Node](/v2/node) |
| 未完成 | 诊断续租跨执行发布的连续测量交接（现行行为见[实时观察](/v2/state-reference/realtime)）、相关 Node 控制请求的专项性能验收 |

网络删除与账户注销的生命周期尚未确定。具体可用操作见[生成契约](/v2/contracts/openapi.json)。

## 运行参数

```sh
mise exec -- cargo run -p meshway-server --bin meshway-server --locked -- \
  --data-dir /absolute/private/data-dir --listen 127.0.0.1:0 \
  --registration-enabled true --session-ttl-seconds 86400 \
  --console-origin https://console.example.com
```

| 参数／环境变量 | 默认 | 行为 |
| --- | --- | --- |
| `--data-dir`／`MESHWAY_DATA_DIR` | 必填 | 私有、无符号链接的绝对路径，独占锁 |
| `--listen`／`MESHWAY_LISTEN` | `127.0.0.1:8420` | HTTP 监听地址；端口 0 分配临时端口 |
| `--registration-enabled`／`MESHWAY_REGISTRATION_ENABLED` | `true` | 是否允许自助注册新账户；关闭后已有账户仍可登录、建网和接受加入链接 |
| `--session-ttl-seconds`／`MESHWAY_SESSION_TTL_SECONDS` | `86400` | 用户会话寿命，整秒，60–2592000 |
| `--console-origin`／`MESHWAY_CONSOLE_ORIGIN` | 未设置 | 浏览器访问 Console 的 origin，见下文；同时放行该 origin 的[跨源访问](#cors) |
| `--delegation-client NAME=ORIGIN`／`MESHWAY_DELEGATION_CLIENTS` | 空 | 可重复（环境变量以逗号分隔），声明可取得[委托会话](/v2/api/authentication#delegation)的浏览器客户端；ORIGIN 遵守 Console origin 规则，同时放行[跨源访问](#cors) |
| `--public-origin`／`MESHWAY_PUBLIC_ORIGIN` | 未设置 | 客户端访问本 Server 的 origin，随委托会话交给客户端；规则同 Console origin |
| `--stun-listen IP:PORT` | 空 | 可重复，显式绑定独立 STUN UDP 服务；最多 4 个，端口非零，不可为多播／广播或重复地址 |
| `--stun-server KEY=HOST:PORT` | 空 | 可重复，发布 hosted 或外部 STUN 端点；KEY 为 1–64 个 ASCII 字母、数字、`_`、`-` 且不可重复，IPv6 使用方括号；最多 4 个 |
| `--netmap-validity`／`MESHWAY_NETMAP_VALIDITY` | `86400` | 每次 [netmap 同步](/v2/execution#netmap-同步)签发的租约时长，整秒，60–86400 |
| `--netmap-sync-hold`／`MESHWAY_NETMAP_SYNC_HOLD` | `50` | 无变化时 `sync` 的名义挂起时长，整秒，1–54，另加 ±10% 抖动 |

命令行显式参数优先于环境变量。这些是实例启动配置，不属于 NetworkConfiguration。作为系统服务时它们写在 `/etc/meshway-server/server.env`，data-dir 固定为 `/var/lib/meshway-server`，见[安装 · Server](/v2/install#server)。日志参数见[运行日志](/v2/logging)；HTTP 日志只记录路由模板与请求关联信息。

60 秒下限是防止运维误设过短导致 Server 抖动即全网 fail closed 的可用性保护，不是安全边界；更短的 validity 只会让撤销更快生效。接收端仍校验租约为正且不超过 24 h，不因短于配置下限扩大授权。

**Console origin** 为 `https://host[:port]`，或 `http://localhost`／`http://127.0.0.1`／`http://[::1]`（可带端口）；不含路径、查询、片段或凭据。Server 据此生成交给设备的浏览器链接：设备授权 `authorizationUrl` 为 `<origin>/authorize-device/<enrollmentRequestId>`。未配置时这些字段为 null，设备得不到链接，Node 与 Console 均不自行拼接。`reset-password` 的 `--console-origin` 遵守同一规则。

### 跨源访问 {#cors}

浏览器从 Console origin 或任一委托客户端 origin 调用 Server 时，Server 回复 CORS 头；Console 与 Server 同源（反向代理）时浏览器不需要它们，托管在其他 origin（如 Cloudflare Pages，见[安装 · Console 托管](/v2/install#server-proxy)）时必须把 Console 的 origin 设为 `--console-origin`。

| 项 | 规则 |
| --- | --- |
| 放行的 origin | `--console-origin` 与各 `--delegation-client` 的 origin，逐字比较；其他 origin 不带任何 CORS 头 |
| 路径 | 只有 `/api/v1/` 下的路径（含用户事件流 SSE）；`/agent-api/v1` 与其他路径不放行 |
| 方法 | `GET`、`POST`、`DELETE` |
| 请求头 | `Authorization`（bearer 会话）、`Content-Type`、`Last-Event-ID` |
| 暴露的响应头 | `x-request-id`、`Date` |
| 凭据 | 不放行 cookie（无 `Access-Control-Allow-Credentials`）；认证只靠 `Authorization` |
| 预检缓存 | `Access-Control-Max-Age: 600` |

两者都未配置时 Server 不发送任何 CORS 头。CORS 只决定浏览器是否把响应交给页面，不是授权：每个请求仍按其会话检查权限。

空库初始化属于正常启动，不提供首用户认领／实例初始化 API，首个公开账户不取得全局管理权限。部署配置、存储和 Node 信任的切换须协调，不通过任意数据库错误自动清库。

服务启动遇到较早 schema 版本（不早于本构建支持的最低版本）的数据库时，先把它[迁移](#存储迁移)到当前版本再打开；不受支持、伪造或损坏的库一律拒绝，不修改文件。

| 退出状态 | 含义 |
| --- | --- |
| 0 | 正常结束 |
| 78（`EX_CONFIG`） | 存储持久阻断，服务、`meshway-server` 的子命令与 `reset-password`／`ip-datasets` 相同：数据目录路径／归属／权限不合法，schema 版本不受支持（非 Meshway 库、比本构建新、早于最低可迁移版本），库与其版本的规范 schema 不符或完整性检查失败，SQLite 报告库文件损坏或不是数据库（`SQLITE_CORRUPT`／`SQLITE_NOTADB`），主密钥与库不匹配（迁移在发布之前即证明），或离线命令遇到需要迁移的旧库。重启不会改变结果，需人工处理；unit 以 `RestartPreventExitStatus=78` 不重启 |
| 1 | 其他失败（I/O、库被占用（busy）、磁盘空间不足、监听失败、`backup` 的输出目录已存在等）：暂态，再次启动重新执行同一检查与迁移 |

**就绪通知**：设置了 `NOTIFY_SOCKET`（systemd `Type=notify`）时，服务在存储已打开（含迁移与启动重编译）且监听已绑定后发送 `READY=1` 与 `STATUS=Serving on <地址>`，开始停止时发送 `STOPPING=1`。存储打开期间由一个续期任务每 30 秒发送 `EXTEND_TIMEOUT_USEC=120000000`，步骤变化时随之更新 `STATUS=`（`Migrating the database from schema <N> to <M>: …`、`Compiling stored netmaps with this build: network <i> of <n>` 等）；打开完成或失败时续期任务停止并结束。单个步骤（例如复制一张大表）耗时多久都不会被启动超时杀掉。没有 `NOTIFY_SOCKET` 时不发送；发送失败只记 warn，不改变行为。

**版本**：`meshway-server --version` 输出构建身份、本构建写入的 schema 版本与最低可迁移版本，以及 target；`-V` 只输出第一行的版本号。升级脚本据第二行在停止服务之前检查 data-dir：

```text
meshway-server 0.1.0-alpha.25 (v0.1.0-alpha.25-0-g1234abcd)
schema: 9 (migrates from 8)
target: x86_64-unknown-linux-musl
```

**served 记录**：进程在发出第一个 Node `sync` 回答**之前**，把 data-dir 的 `served` 原子替换（0600、fsync）为本进程的记录（`boot`：每个进程新生成的 UUID；`build`；`at_ms`），每个进程只写一次；写入失败或结果未知时该 sync 以 503 失败、不发出租约，下一次 sync 重写。从此节点可能持有它签发的租约，升级不再能回退；`served` 是[回退边界](/v2/install#server-rollback)的唯一依据。同时记 info `First Node sync answered by this Server process`（`network_id`、`node_id`，只记一次），仅作诊断。

每次打开存储（服务启动及下列离线命令）在受理请求之前按本构建重编译全部存活网络的已存 netmap：同一构建不写任何行，换了构建只有内容变化的节点得到新 serial，升级后无需手动触发重编译。规则与耗时见 [Netmap 设计 · Server 侧](/v2/node/netmap-design#server-侧)，升级顺序见 [Server 升级](/v2/install#server-upgrade)。

## 运维命令

离线命令须先停止占有同一 dataDir 的服务，并以 data-dir 的属主运行；系统服务安装下为 `sudo systemctl stop meshway-server` 后 `sudo runuser -u meshway-server -- <命令>`，`reset-password` 与 `ip-datasets` 位于 `/usr/local/libexec/meshway-server/`。

运维密码恢复使用 `reset-password` binary：

```sh
mise exec -- cargo run -p meshway-server --bin reset-password --locked -- \
  --data-dir /absolute/private/data-dir --email user@example.com \
  --console-origin https://console.example.com --expires-in-seconds 900
```

`--expires-in-seconds` 默认 900，范围 60–86400。命令显式交付一次性链接，兑换流程见[无邮件账户恢复](/v2/api/authentication#无邮件账户恢复)；网络 Owner／Admin 无他人全局密码重置权。

GeoIP／ASN 数据集由本机 `ip-datasets` binary 导入，只供自动路由方案使用，State 不返回查询结果，见[数据集部署](/v2/state-reference/ip-info#datasets)。

### 存储迁移

`meshway-server migrate` 把数据库迁移到本构建的 schema 版本，并按本构建重编译已存 netmap（与服务启动相同），然后退出、不监听端口。它与服务启动共用同一份迁移实现：服务启动遇到旧库时自动执行同样的迁移；本命令用于在启动新版本服务前单独完成并确认迁移。须先停止占有同一 dataDir 的服务。

```sh
mise exec -- cargo run -p meshway-server --bin meshway-server --locked -- \
  migrate --data-dir /absolute/private/data-dir
```

stdout 先是一行 `Migration state: <状态>`，供脚本判断，然后成功时为 `Database migrated from schema 8 to 9 in <ms> ms; the previous database is kept as <path>.`，库已是当前版本时为 `Database is already at this build's schema; nothing migrated.`。失败时状态行由命令在失败后从 data-dir 读回（库文件头与 `schema_migrations` 中本次开始以后的记录），不由失败的方式推断；退出状态见[运行参数](#运行参数)。

| 状态行 | 含义 |
| --- | --- |
| `current` | 库已是当前版本，没有迁移 |
| `published <备份路径>` | 本次迁移已发布：`controller.sqlite` 是新版本库，旧库保存在该路径（成功时，或发布之后才失败时） |
| `unpublished` | 失败，什么也没有发布：`controller.sqlite` 仍是原来的库 |
| `unknown` | 失败，且 data-dir 无法读回（被占用、库不可读或两者都不是）：由运维检查 |其他离线命令（`rotate-netmap-key`、`backup`、`reset-password`、`ip-datasets`）遇到需要迁移的旧库时拒绝并提示先运行本命令，不隐式迁移，均以 78 退出。

| 项 | 行为 |
| --- | --- |
| 支持范围 | 每个发布版本保证可从上一发布版本迁移；代码保留从最低可迁移版本（当前为 schema 8）起的整条链，跨多个版本时逐级执行 |
| 过程 | 先以旧版本的规范 schema 完整检查旧库并证明 `master-key` 与旧库的主密钥绑定匹配（不匹配即以 78 退出，什么也不发布），再在 data-dir 内的 staging 文件中按目标版本规范 schema 新建库、逐表复制全部行、同一事务内完整检查，最后原子替换 `controller.sqlite`；规则见[持久数据库字段 · 迁移](/v2/storage/database#migration) |
| 备份 | 旧库文件原样保留为 `controller.sqlite.v<旧版本>-<Unix 毫秒>`（0600，与主密钥同在 data-dir），保留最新 3 份；它只能由对应旧版本的 Server 打开 |
| 中断 | 任一时刻 `controller.sqlite` 都是完整的旧库或完整的新库；中断后下一次启动丢弃 staging 并重新迁移 |
| 空间 | 开始前要求 data-dir 所在文件系统可用空间不少于库大小的 110%（跨多个版本时为 2 倍），不足即暂态失败 |
| 秘密与 netmap | 密文逐字节复制，不需要也不解密主密钥保护的内容；netmap、netmap 历史与签发钥不改写，serial 不回退 |
| 日志 | info `Database migration started`（`from`、`to`）与 `Database migrated`（`from`、`to`、`tables`、`rows`、`elapsed_ms`、`backup`），不含行数据 |
| 回退 | 只在新 Server 尚未应答任何 Node `sync` 前，可停止新服务、把备份改回 `controller.sqlite`（新库的 `-wal`／`-shm` 随新库移开）后启动旧版本；一旦新 Server 签发过 netmap，节点已持有更高 serial，回退会被节点拒绝，只能前进修复。系统服务的自动与手动回退见[回退边界](/v2/install#server-rollback) |

### 备份

`meshway-server backup` 把数据库与主密钥一起复制到一个新目录，供灾难恢复与离线快照。须先停止占有同一 dataDir 的服务。

```sh
mise exec -- cargo run -p meshway-server --bin meshway-server --locked -- \
  backup --data-dir /absolute/private/data-dir --output /absolute/private/backups/20261022T120000Z
```

| 项 | 行为 |
| --- | --- |
| 输出 | `--output` 必须是尚不存在的绝对路径，由命令以“仅新建”方式创建（同名已存在即拒绝，从不接管已有目录）；与 data-dir 相同，每一级上级目录的属主须为 root 或运行用户、且不可被其他用户写入（粘滞位目录除外），父目录须可由运行用户写入；创建为 0700，内含 `controller.lock`、`controller.sqlite`、`master-key`，文件均 0600。输出目录本身就是一个 data-dir，Server 可直接以它启动 |
| 一致性 | 持有源 data-dir 的独占锁，不写源库；先以当前 schema 完整检查源库并证明主密钥与库匹配，再以 SQLite `VACUUM INTO` 复制，复制主密钥，最后对副本做同样的完整检查与主密钥绑定检查并 `fsync` |
| 拒绝 | 需要迁移的旧库、不受支持或损坏的库、主密钥不匹配：以 78 退出（同其他离线命令）；data-dir 中没有数据库也拒绝。输出已存在、上级目录不满足上述条件或空间不足（需库大小 110%）：以 1 退出 |
| 失败 | 中途失败时删除本次写出的文件（库及其日志、主密钥、锁）与本次创建的输出目录，不留下部分复制的秘密；目录中出现了别的文件时只删自己的、保留目录并报告 |
| 输出行 | `Backed up the database (schema 9, <bytes> bytes) and the master key to <path>.` |

库加主密钥是这台 Server 的全部秘密，含 netmap 签名钥；只有库没有主密钥的备份无法解密。保管与恢复的约束见[安装 · 备份与恢复](/v2/install#server-backup)。

### Netmap 签发钥轮换

`meshway-server rotate-netmap-key` 轮换签署 [netmap 租约](/v2/execution#netmap-同步)的 Ed25519 钥。它与上面的命令一样离线运行：须先停止占有同一 dataDir 的服务，完成后再启动；停机期间节点按已持有租约继续运行，sync 恢复后即重连。

```sh
mise exec -- cargo run -p meshway-server --bin meshway-server --locked -- \
  rotate-netmap-key --data-dir /absolute/private/data-dir --window 48h
```

| 参数 | 默认 | 行为 |
| --- | --- | --- |
| `--data-dir`／`MESHWAY_DATA_DIR` | 必填 | 同服务参数 |
| `--window` | `48h` | 交接窗口，整数加单位 `s`／`m`／`h`／`d`，60 秒–30 天；默认是最长租约的 2 倍 |

命令在一个事务内生成下一代（generation + 1）钥 K′ 并使其成为当前钥，把被替换钥 K 的 `retired_at` 设为窗口结束时刻，输出两把钥的 generation／keyId 与窗口结束时间。窗口仍开时再次执行被拒绝（一次只有一个窗口）；窗口状态只存于数据库，重启后不变。

| 时段 | 接入响应 `serverSigningKey` | sync 回复 `signature` | `keyRotation` |
| --- | --- | --- | --- |
| 窗口内 | K′ | K | `{ next: K′, notBefore, endorsement, leaseSignature }`：K 对 K′ 的背书与 K′ 对同一租约的签名 |
| 窗口后 | K′ | K′ | null |

节点如何跟随交接、错过窗口时的表现见[Netmap 发布](/v2/node/control#server-签名钥轮换)。节点库存视图的 `serverKey` 显示每个节点最近一次 sync 报告的钉住钥 generation 与 `status`：`current`、`rotating`（窗口内仍钉 K，下一回复即跟随）或 `keyMismatch`（错过窗口或其他钥；须保留身份重新接入，见下）。它是本进程内观测，Server 重启后在节点下一次 sync 前为 null。旧钥永久保留在表中供审计，不删除。

错过窗口的节点保持已持有租约直至到期，之后 fail closed；恢复方式是保留身份的重新接入 `meshway up --reenroll`（接入意图 `existing`，须 Console 批准，enrollment key 不适用），兑换响应即钉住当前钥，Node ID、密钥与配置引用不变，见[保留身份的重新接入](/v2/node/control#保留身份的重新接入)。当前不做自动轮换，也不支持多 Server 实例共享钥。

本工作树的开发服务（`mise run services …`）、地址、重置与恢复见[开发服务、测试与环境](/v2/development#本机服务)。源码合并或编译完成后，仍须执行服务更新并核对实际运行实例。

## 参考

- [数据模型与权限](/v2/entities) · [权限矩阵](/v2/permissions)
- [HTTP／SDK](/v2/api) · [认证](/v2/api/authentication) · [SSE](/v2/api/events)
- [存储与查询](/v2/storage) · [数据库字段](/v2/storage/database) · [内存容器](/v2/storage/memory)
- [配置应用](/v2/configuration-reference/deployment) · [历史](/v2/api/history)
