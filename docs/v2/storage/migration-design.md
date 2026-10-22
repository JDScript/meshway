# Server 存储迁移与部署形态设计

> **设计，未实现。** v0.1.0 里程碑 B（[交付门槛](/v2/process#v0-1-gate)）的设计包 MIG-D：Server schema 升级不 reset、不重新接入，以及 Server 的系统服务、发布产物、备份与远程升级。现行 schema 与字段的唯一定义仍是[持久数据库字段](/v2/storage/database)与 `apps/meshway-server/src/store/schema/`；运行参数见 [Server](/v2/server)。本页不加入文档导航；实现后现行语义迁入上述页面与[安装](/v2/install)，本页只保留决定与理由。

## 现状 {#current}

| 项 | 当前行为 | 位置 |
| --- | --- | --- |
| 身份 | `PRAGMA application_id = 0x4d534857`（MSHW）＋`user_version = 8` | `store/schema/mod.rs` |
| 打开 | `initialize` 只接受空库（建当前 schema、绑定主密钥，同一事务）或身份精确等于 (MSHW, 8) 的库；其余一律 `SchemaError::Unsupported`，进程拒绝启动，不修改文件 | 同上 |
| 完整性 | `validate` 把 `sqlite_schema` 中每个对象的**原文 SQL** 与内存中新建的当前 schema 逐一比较（含 CHECK／FK／STRICT），再跑 `integrity_check`、`foreign_key_check` 和若干业务不变量（主密钥绑定单例、challenge 时钟、IP 数据集 head） | 同上 |
| 文件 | data-dir 内 `controller.lock`（flock 独占）、`controller.sqlite`（WAL、`synchronous=FULL`）、`master-key`（53 B，XChaCha20 密封根钥）；netmap 签名钥密封后存于 `server_signing_keys` | `directory.rs`、`secrets/` |
| 密封 | 认证加密的 AAD 只绑定领域与业务键：配置为 (network, revision, purpose)，netmap 为 (network, node, serial)，签名钥为 (id, generation)，enrollment 为 id；**不绑定表名、行号或 schema 版本** | `secrets/mod.rs` |
| 启动重编译 | 打开存储后、受理请求前按本构建重编译全部存活网络的 netmap（S4）；旧编译器写成的 delta 基准按“无基准”处理（S5） | [Netmap 设计 · Server 侧](/v2/node/netmap-design#server-侧) |
| 现场 Server | 本机 `scripts/dev.py` 在 tmux 中运行 debug 构建，data-dir 为 `.meshway/server`（2026-10-14 alpha.9 reset 后建库，schema 8，约 143 MiB），经 `https://meshway-dev-api.jdscript.dev` 对外；`update server` 保留数据重启，`reset server` 把旧 data-dir 整体移到 `.meshway/backups/server-<stamp>/` 后新建库、重新 seed | `scripts/dev.py` |
| Server 发布 | **没有**。`build.yml` 只交叉编译 `meshwayd`／`meshway`／`meshway-rules`，`release.yml` 只打包 Node tar、`install.sh` 与 `SHA256SUMS`；没有 Server unit、安装脚本或 sd_notify | `.github/workflows/`、`dist/` |
| Console 托管 | Server 不提供静态文件；`vite.config` 注释“production 时 controller serves this build”与现状不符（只是同源要求）。开发时由 Vite 代理 `/api` 到 Server | `apps/meshway-web-console/vite.config.ts` |
| TLS | Server 只监听明文 HTTP（默认 `127.0.0.1:8420`），TLS 由前置代理／隧道承担 | [Server · 运行参数](/v2/server#运行参数) |

### schema 历史回放 {#history}

把 2→8 的每次变化放到“当时若有迁移”的视角下看，决定了迁移机制必须能做什么：

| 版本 | 提交 | 变化 | 若当时要迁移 |
| --- | --- | --- | --- |
| 2（无升号） | `0d77b928`、`fe139419` | 新增 `node_netmap_inputs`、`observation_subscriber_ids` 表，**没有提升 `user_version`** | 已有库会以 `Structure("missing table …")` 拒绝而不是 `Unsupported`。迁移制度必须让“改 SQL 不升号”在 CI 失败 |
| 2→3 | `5f44fe1b` | 规则集改为全局 `MWRS` 内容寻址：新表 `rule_set_artifacts`／`records`／`catalogs`、`node_netmap_artifacts`；`dataset_artifacts.kind` 的 CHECK 收窄去掉 `ruleSet`；同时 `compilerVersion` → `meshway-netmap-2` | CHECK 收窄只能重建表；旧 `ruleSet` artifact 要么用 `meshway-rules` 转成 `MWRS` 并重建引用（Rust 变换，涉及密封的 bindings），要么明确丢弃并由资源调度重新下载（改变历史恢复语义，需写进 changelog）。netmap 由启动重编译处理 |
| 3→4 | `b6ecb5e4` | `rule_set_catalogs` 新增 `document`／`signature` **NOT NULL 且无默认值** | 旧行无法提供原文。删除行会丢失目录 sequence 下限（防回滚的安全事实）；正确做法是当时就把可迁移性纳入设计（可空列，或把下限与原文分表）。迁移制度需要评审清单项：**新 NOT NULL 列必须能由已有数据推出** |
| 4→5 | `aa679ab9` | 新表 `rule_set_record_heads` | 纯 SQL：由 `rule_set_records` 回填每个 (source, digest) 的记录，或留空待下次刷新 |
| 5→6、5→6 | `5d8ee4dd`、`4493af76` | 两个分支各自把 5 升到 6（删 `rule_set_artifacts_retained` 触发器＋新表 `rule_set_catalog_checks`；`activity_events` 增加 node 主体与选择目标，CHECK 改写），合并 `861d58d3` 时手工改号为 6、7 | **版本号冲突**：两个分支的 `0006` 迁移会静默共存。制度必须让并行分支在同一位置冲突（单一“最新版本快照”文件）并检查版本连续 |
| 7→8 | `14a0af49` | `activity_events` 的 action CHECK 与 target 约束加入 `routing.autoResolve` | SQLite 不能修改 CHECK：整表重建 |

结论：真实的历史变化多数是 **CHECK／约束改写与新表**，少数需要 **解密后变换**；`ALTER TABLE ADD COLUMN` 恰好最少。而现有完整性检查按原文 SQL 比较——这一点决定了下面的选择。

## 不变量 {#invariants}

所有方案都必须满足：

- **普通启动不清库、不猜测修复**。任何迁移失败都让 `controller.sqlite` 保持迁移前的字节内容，进程以明确原因退出。
- **迁移后的库与新建库不可区分**。迁移结果必须通过当前 `validate`——即 `sqlite_schema` 原文与新建 schema 完全一致；不为迁移放宽完整性检查。
- **不可降级**。新库旧二进制拒绝启动（现有行为：旧二进制看到更高 `user_version` 即 `Unsupported`）。
- **运行时只认当前形态**。迁移是打开前的一次性单向转换；业务代码、读取路径不出现旧形态分支（Server AGENTS“不加历史兼容轨”的含义保持不变，见[决定 3](#decisions)）。
- **秘密不出私有边界**。迁移不把明文、主密钥或签名钥写到 data-dir 之外，不记入日志；中间文件同为 owner-only。
- **迁移只管存储形状**。编译产物由 S4／S5 负责（见[第 3 节](#compiler)）。
- **fail-closed 分类**（根 AGENTS）：读到已证实的不一致（旧库不符合其版本的 schema、完整性失败、版本超出支持范围）是持久阻断；I/O 错误、磁盘满、被中断是暂态，由服务管理器按有界退避重启后同一流程重证。

## 1. 迁移机制 {#mechanism}

### 选项

| | (a) 版本化 SQL 脚本，原地执行 | (b) Rust 迁移函数，原地执行 | (c) 复制到规范新库（推荐） |
| --- | --- | --- | --- |
| 形式 | `migrations/0009_*.sql`，启动时在一个事务里从 `user_version` 逐级执行到当前 | 每级一个 `fn(&Transaction, &Sealer)`，可读写任意行 | 每级在 staging 新文件中执行**该版本的规范 schema 原文**，`ATTACH` 旧库只读，按表 `INSERT … SELECT`；需要变换的表由该级的 SQL 片段或 Rust 钩子接管 |
| CHECK／约束改写 | 需 SQLite 12 步重建（关 FK、建新表、复制、删旧、改名） | 同左 | 天然：每张表都在新文件中按新定义重建 |
| 与原文比较的 `validate` | **不兼容**：`ADD COLUMN` 把列追加到原文末尾，`RENAME TO` 把表名改写为 `"name"`（实测 SQLite 3.54），迁移后的原文与新建 schema 不同，必须放宽为结构等价比较或用 `writable_schema` 改写原文 | 同左 | 兼容：staging 中的对象就是由规范原文创建的，迁移后 `validate` 原样适用 |
| 不可变触发器 | `rule_set_artifacts_immutable`、`node_netmaps_immutable` 等会阻止原地 UPDATE，迁移需临时删再建触发器 | 同左 | 只做 INSERT；新库的触发器在复制时生效，等于逐行再校验一次 |
| 解密变换 | 不能 | 能 | 能（Rust 钩子） |
| 中断安全 | 依赖单事务；但 12 步重建需 `foreign_keys=OFF`（事务内不可切换），实际要在事务外 | 同左 | 旧文件从不被写；staging 未发布前中断即丢弃，见下表 |
| 备份 | 需另做一次完整拷贝 | 同左 | 旧文件本身就是备份（硬链接，零拷贝） |
| 成本 | 只改涉及的表 | 同左 | 每次迁移复制全库；现场 143 MiB 为秒级，磁盘需余 1× 库大小 |

### 推荐：(c) 复制到规范新库，SQL 为主、Rust 钩子为辅

理由：现有 `validate` 的强度（逐字比较约束原文）是 schema 正确性的唯一证明，(a)／(b) 都必须削弱它；历史上的变化以约束改写为主，(c) 对它们没有特殊路径；旧文件只读，备份与回退零成本。复制全库的代价在 v0.1.0 的规模（单 Server、≤ 数百 MiB）可接受。

#### 源码形态

```text
apps/meshway-server/src/store/schema/
  mod.rs              # VERSION、MIN_MIGRATABLE、initialize/check/validate（不变）、migrate 入口
  current.sql …       # 当前规范 schema（不变）
  history/v8.sql      # 每个可迁移起点版本的规范 schema 快照，提交后不可修改
  migrate/mod.rs      # 唯一迁移执行器：staging、ATTACH、按表复制、发布
  migrate/v9.rs       # 8→9：该级与默认复制不同的表（SQL 片段或 Rust 钩子）
```

- `history/v{N}.sql` 是版本 N 的完整规范 schema。当前版本的快照与 `current.sql …` 的拼接必须逐字相等（测试强制），所以**改 SQL 不升号**或**升号不留快照**都会失败；并行分支同时升号会在 `history/` 的同一文件名与 `VERSION` 常量处冲突。
- 每级的默认行为是：两个版本中同名、同列集的表按主键顺序 `INSERT INTO main.t (cols) SELECT cols FROM old.t`；该级只声明偏离（新表回填、列变换、丢弃的表），偏离之外的表不需要写代码。

#### 流程

迁移在 `Directory` 已持有 `controller.lock`、SQLite worker 打开主连接**之前**执行，由 `store::schema::migrate` 唯一实现；`serve` 与 `migrate` 子命令共用它（见[决定 4](#decisions)）。

1. **识别**：只读打开 `controller.sqlite`，读 (application_id, user_version)。等于 (MSHW, VERSION) → 正常 `initialize`；`application_id` 不是 MSHW、`user_version` > VERSION（新库旧二进制）或 < `MIN_MIGRATABLE` → 持久阻断，区分三种原因记 error；否则进入迁移。
2. **证明输入**：用 `history/v{from}.sql` 对旧库做与 `validate` 相同的完整检查（原文比较、`integrity_check`、`foreign_key_check`、业务不变量）。不符即持久阻断——迁移只接受“确实是某个已发布版本写成”的库。
3. **静止旧库**：在旧库上 `PRAGMA wal_checkpoint(TRUNCATE)` 并切换 `journal_mode=DELETE` 后关闭，确保没有 `-wal`／`-shm` 遗留（WAL 不校验归属，遗留的旧 WAL 会被重放到新文件上）。
4. **主密钥**：读取 `master-key`，以旧库 `master_key_binding` 验证同一把钥（只在有 Rust 钩子需要解密时打开 Sealer；纯 SQL 级不触碰秘密）。
5. **逐级构建**：对 from→from+1→…→VERSION 每级，创建 owner-only staging 文件 `controller.sqlite.migrating`（私有目录 API），以该级目标版本的规范 SQL 建库，`ATTACH` 上一级文件 `mode=ro`，按外键拓扑序、主键升序复制（`node_netmaps` 按 serial 升序以满足单调触发器）；复制 `sqlite_sequence`（AUTOINCREMENT 高水位，保护分页游标不复用）；schema 自带的种子行（`node_challenge_clock` 的 `(1,0)`）改为 UPDATE 为旧值；设置 `application_id`／`user_version`；在**同一事务**里运行目标版本的 `validate` 与该级的迁移后检查（每表行数、未变换表的内容摘要等于源表）；提交、`fsync`。多级时上一级的 staging 在下一级成功后删除。
6. **备份**：`link(controller.sqlite, controller.sqlite.v{from}-<stamp>)` 并 `fsync` 目录——旧 inode 原样保留为备份，不额外占用空间。
7. **发布**：`rename(controller.sqlite.migrating, controller.sqlite)` 原子替换，`fsync` 目录。任一时刻 `controller.sqlite` 都存在且是完整的旧库或完整的新库；`Directory` 持有的检查句柄随后重新打开。
8. **记录**：新库的 `schema_migrations` 表（MIG-1 的 schema 变化，见[第 6 节](#mig-1)）记 from／to、构建、开始／结束时间与备份名；info 日志 `Database migrated`（`from`、`to`、`tables`、`rows`、`elapsed_ms`、`backup`），不含行数据。
9. **保留**：`controller.sqlite.v*-<stamp>` 备份按时间保留最新 3 份（同 `STALE_ARCHIVES`）。之后正常 `initialize`（`check`）与 S4 启动重编译。

#### 中断与失败

| 时刻 | 盘上状态 | 下一次启动 |
| --- | --- | --- |
| 1–5 任一步失败或被杀 | `controller.sqlite` 为未修改的旧库；可能有 `controller.sqlite.migrating` | 删除 staging 后从步骤 1 重来（staging 永不被读取） |
| 6 之后、7 之前 | 旧库＋同 inode 备份链接＋完整 staging | 同上；多出的备份链接按保留规则回收 |
| 7 之后 | `controller.sqlite` 为已验证、已持久化的新库 | 身份等于当前版本，正常打开 |
| 步骤 2 证明失败、版本不受支持 | 不修改任何文件 | 持久阻断：以专用退出码退出，unit 的 `RestartPreventExitStatus` 阻止重启循环；需人工处理 |
| 磁盘不足、I/O 错误 | 同第一行 | 暂态：普通失败退出码，systemd `Restart=on-failure` 按 `RestartSec` 重试；开始前检查可用空间 ≥ 库大小＋10%，不足直接报出 |

#### 加密列

AAD 不含表名与 schema 版本，所以**密文默认逐字节复制**，不需要主密钥。需要解密的只有两种情况，都由该级的 Rust 钩子在第 5 步内完成，结果以同一 AAD、新 nonce 重新密封：

- 密封内容的明文格式变化（如配置 `bindings` 的结构变化）。钩子在本级模块里持有一份**冻结的旧格式解码器**，只用于这一次转换；运行时读取路径仍只有当前格式。
- 组成 AAD 的业务键变化（如 revision 编号方式变化）。这类变化应尽量避免；必须做时同一钩子负责全部受影响行。

明文只存在于迁移进程内存（`Zeroizing`），不落盘、不入日志。**netmap payload、netmap 历史与签名钥不在此列**，见[第 3 节](#compiler)。

#### 支持范围

| 选项 | 内容 |
| --- | --- |
| 只承诺上一发布版本（推荐承诺口径） | 每个发布版本保证可从上一发布版本迁移，并由发布门槛中的升级测试证明 |
| 任意旧版本链式 | 代码保留从 `MIN_MIGRATABLE` 起的全部级，单元层对整条链做 fixture 测试 |

推荐：**承诺**“每个发布版本可从上一发布版本迁移”（证据是黑盒升级测试，见[第 2 节](#testing)）；**实现**保留从 8 起的完整链，因为每级只是小片段，链式成本低，跨多版本升级的现场也能工作，只是不作为发布承诺。`MIN_MIGRATABLE` 只在 minor 版本边界上调，并写入 changelog。

## 2. 测试 {#testing}

| 层 | 内容 | 位置／时机 |
| --- | --- | --- |
| 快照守卫 | `history/v{VERSION}.sql` 等于当前 schema 拼接；`history/` 中每个文件的 SHA-256 固定在常量表里（快照不可修改）；版本从 `MIN_MIGRATABLE` 到 `VERSION` 连续且每级有迁移模块 | `store/schema` 的 cfg(test) 子模块，T1 |
| 每级 fixture | 以 `history/v{N}.sql` 建库，插入覆盖**每张表**的代表性行（测试断言旧 schema 的每张表都至少有一行，新表不会无测覆盖），密封列用固定测试主密钥和真实 Sealer 生成；迁移后 `validate` 通过，未变换表逐行相等，变换表按该级断言，`sqlite_sequence` 不回退 | 同上，T1 |
| 链式 | 从 `MIN_MIGRATABLE` 的 fixture 迁到 VERSION，结果与逐级结果一致 | 同上，T1 |
| 失败语义 | 旧库篡改一个约束 → 持久阻断且文件哈希不变；更高版本 → 拒绝且不变；在每个步骤注入失败／中断后重启 → 完成且结果相同；遗留旧 WAL 不被重放 | 同上，T1 |
| 业务读写 | 迁移后的 fixture 经真实 `App::open` 打开：登录（Argon2 哈希、会话）、读取配置原文与规范值（解密）、Node 会话认证、签名钥签发、观测历史分页 | `apps/meshway-server/tests/`，T1 |
| 从上一发布升级 | 用上一 release 的 `meshway-server`（及 `meshwayd` control-only）按脚本场景建库：注册、建网、应用配置、接入一个 Node、写入观测；停机后以当前构建启动同一 data-dir，断言同一密码登录、配置 revision 与原文不变、该 Node 不重新接入即 `sync` 成功且接受签名、观测历史条数不变、旧二进制随后拒绝新库 | `scripts/ci/server-upgrade.py`；schema 或 store 路径变化时进入 `Check`，并作为 `release.yml` 的发布门槛。场景每次由上一 release 产物重建，不提交二进制库 |

“从上一发布升级”需要上一版本已有 Server 产物，因此在 MIG-2 后的第二个发布才真正生效；此前由 MIG-1 的现场验收代替。

## 3. 与 compilerVersion／S4／S5 的关系 {#compiler}

- 迁移只改变存储形状。`node_netmaps.payload`、`node_netmap_history`、`netmap_receipts`、`server_signing_keys` 的密文**逐字节复制、从不解密或重写**；其 AAD 键（network、node、serial；id、generation）与 serial 值保持不变。每节点 serial 高水位不得因迁移降低——Node 拒绝 serial 回退（[Control · finish_sync](/v2/node/control)）。
- 迁移完成后的正常打开路径照常执行 S4 启动重编译：新构建的构建身份进入输入指纹，内容变化的节点得到新 serial，节点下一次 `sync` 即收到。`compilerVersion` 同时变化时，旧编译器的当前 netmap 变成基准，由 S5 `load_earlier` 按无基准处理。
- 因此迁移级**不得**为了“适配新编译器”去改 netmap 行；编译输入（配置、绑定、选择、规则集）若需变换，变换的是那些输入表，netmap 由 S4 从变换后的输入重编译。
- 存储迁移与 `compilerVersion` 变化是独立的两件事：schema 变化不制造断网窗口；`compilerVersion` 变化的窗口规则仍见[安装 · compilerVersion](/v2/install#compiler-version-upgrade)。changelog 的范围句分别写明两者。

## 4. 部署形态 {#deployment}

### systemd unit（拟定 `dist/systemd/meshway-server.service`）

```ini
[Unit]
Description=Meshway Server
Wants=network-online.target
After=network-online.target

[Service]
Type=notify
NotifyAccess=main
User=meshway-server
Group=meshway-server
# MESHWAY_* instance settings (listen, console origin, registration …), written by install.sh.
EnvironmentFile=/etc/meshway-server/server.env
Environment=MESHWAY_LOG_FORMAT=json RUST_LOG=info
ExecStart=/usr/local/bin/meshway-server --data-dir /var/lib/meshway-server $MESHWAY_SERVER_OPTIONS
Restart=on-failure
RestartSec=5s
# Persistent storage blocks (unsupported/forged schema, failed proof) do not loop.
RestartPreventExitStatus=78
# Migration extends this with EXTEND_TIMEOUT_USEC while it makes progress.
TimeoutStartSec=120s
TimeoutStopSec=30s
LimitNOFILE=65536

StateDirectory=meshway-server
StateDirectoryMode=0700
UMask=0077

CapabilityBoundingSet=
AmbientCapabilities=
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectHostname=yes
ProtectClock=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM

[Install]
WantedBy=multi-user.target
```

要点：

- **非 root、零 capability**：默认端口 8420 与 STUN 3478 都在 1024 以上；静态系统用户 `meshway-server`（install 脚本创建）而不是 `DynamicUser`，因为 `DynamicUser` 的 `StateDirectory` 经 `/var/lib/private` 符号链接暴露，与 data-dir “无符号链接”检查冲突，且离线命令需要以同一 UID 运行。
- **`Type=notify`**：Server 在存储打开、迁移与 S4 重编译完成、监听已绑定后发 `READY=1`；迁移期间发 `STATUS=migrating …` 与 `EXTEND_TIMEOUT_USEC`。sd_notify 现只在 `apps/meshway-daemon/src/service.rs`，按“一个实现”原则移到共享 crate 后两端复用（[决定 11](#decisions)）。
- **只读二进制**：`/usr/local/bin/meshway-server` root 所有 0755，`ProtectSystem=strict` 下进程只可写 `StateDirectory`。
- **退出码**：存储持久阻断用 78（`EX_CONFIG`），其他失败用 1；前者不被 systemd 重启，`systemctl status` 显示原因，符合“已证实不一致需人工”的分类。
- **离线命令**（`reset-password`、`rotate-netmap-key`、`ip-datasets import`）需要独占锁，即需先停服务，以 `runuser -u meshway-server --` 执行；它们遇到旧 schema 时拒绝并提示先运行 `meshway-server migrate`，不隐式迁移。
- **同机 Node**：路径与 Node 完全分开（`/var/lib/meshway-server`、`/etc/meshway-server`），可与 `meshwayd` 共存。

### 安装脚本

| 选项 | 内容 |
| --- | --- |
| (a) `install.sh --component server`（推荐） | 同一脚本，默认 `node`；下载、`SHA256SUMS` 校验、架构识别、`--version`／`--artifact` 共用，server 分支另有用户创建、env、迁移、回退与 `--backup` |
| (b) 单独 `install-server.sh` | 两个脚本互不影响；但下载／校验逻辑复制一份，`curl … \| sh` 形式无法共享函数 |
| (c) deb／rpm 包 | 维护成本高，v0.1.0 不做 |

推荐 (a)：下载与校验是安全相关逻辑，只应有一份；Node 分支保持现有行为与验收不变。

### 发布产物

- `build.yml` 增加 `-p meshway-server` 的两个 musl target（`rusqlite` bundled 由 zig cc 编译），路径过滤加入 `apps/meshway-server/**`。
- `release.yml` 新增 `meshway-server-<target>.tar.gz`：`meshway-server`、`ip-datasets`、`reset-password`、`meshway-server.service`；`export-openapi` 是开发工具，不发布。发布步骤同样执行一次 x86_64 `meshway-server --version` 并要求等于 tag。
- Console 构建产物 `meshway-console.tar.gz`（静态文件）随同发布，见下。

### Console 与 TLS

| 选项 | 内容 |
| --- | --- |
| (a) Server 直接 serve Console（`--console-dir`） | 单进程同源；但 Server 仍无 TLS，前置代理依旧需要；新增运行参数与静态文件路由，属于新功能 |
| (b) 前置反向代理（推荐） | 代理终结 TLS，`/api/`、`/agent-api/` 转发到 Server，其余路径为 Console 静态文件（SPA 回退到 `index.html`）；文档给出 Caddy 示例；Server 不变 |
| (c) 把 Console 嵌入 Server 二进制 | 版本天然一致；二进制变大，Console 更新需重发 Server |

推荐 (b)：Server 现有边界（明文 HTTP、Console 独立）不变，功能冻结下不加新路由；TLS 本来就需要代理。`vite.config` 中关于“controller serves this build”的注释在 MIG-2 改为“由同源反向代理提供”。

### 备份

| 选项 | 内容 |
| --- | --- |
| (a) 迁移前自动备份＋离线 `meshway-server backup`（推荐 v0.1.0） | 迁移自动保留旧库硬链接（最新 3 份）；`backup --data-dir … --output DIR` 持锁执行 `VACUUM INTO`，同时复制 `master-key`，输出目录 0700、文件 0600；需短暂停服务（秒级，Node 凭 24 h 租约不受影响） |
| (b) Server 进程内定时在线备份 | `--backup-interval`／`--backup-keep`，在 SQLite worker 上 `VACUUM INTO` 到 data-dir 内 `backups/`；无停机；属于新运行功能 |
| (c) 外部 `sqlite3 .backup` | 绕过 data-dir 独占所有权；拒绝 |

推荐 (a)，(b) 列入 v0.1.0 之后。风险必须写进安装文档：**库加主密钥等于全部秘密**——配置中的协议材料、以及密封在库中的 netmap 签名钥（可为全网签发 netmap）。只有库没有主密钥的备份无法解密，等于无效备份；两者一起离开主机时必须由运维另行加密（如 age），并与 Server 主机分开保管。迁移产生的备份留在 data-dir 内，与主密钥同一保护边界。

### 远程升级流程 {#upgrade-flow}

`install.sh --component server --version vX` 执行：

1. **预检**：下载并校验产物；新二进制 `--version`；读取 data-dir 的 `user_version`（只读文件头）并与新二进制支持的范围比较，超出即在停服务前失败；检查磁盘余量。
2. **停服务**：`systemctl stop meshway-server`。Node 继续使用已持有租约运行（默认 24 h），升级窗口远小于它。
3. **迁移**：`runuser -u meshway-server -- meshway-server-new migrate --data-dir …`：备份＋迁移＋`check`＋S4 重编译，不监听端口。失败时旧库原封不动：保留旧二进制、启动旧服务、报告原因，结束。
4. **换二进制**：旧二进制保留为 `/usr/local/bin/meshway-server.previous`，新二进制原子替换。
5. **启动与健康**：`systemctl start` 等待 `READY=1`，再请求 `GET /api/v1/auth/options` 成功。
6. **回退**：只有在新 Server **从未应答过任何 Node `sync`** 时才允许自动回退（恢复备份＋`.previous` 二进制）。一旦新 Server 签发过 netmap，节点已持有更高 serial；回退后旧库签发的 serial 更低，节点会拒绝（serial 回退），升级后发生的接入、签名钥轮换与配置也会丢失。此后只做前进修复（发布修正版本）。新 Server 在首次应答 `sync` 时记 info 事件，脚本据此判断回退是否仍安全。

### 日志

stdout／stderr 由 journald 收集，JSON 格式（`meshway-logging`，[运行日志](/v2/logging)）；迁移与持久阻断原因各有固定消息。凭据、配置原文与 netmap 内容不进日志；`reset-password` 的链接只写到调用者终端。

## 5. 过渡到 v0.1.0 {#transition}

- schema 8 自 alpha.9（2026-10-14）起直到 alpha.20 与当前 main 未变，现场库也是 8。**以 8 作为 `MIN_MIGRATABLE`**：`history/v8.sql` 即当前 schema。
- 承诺从**带 MIG-1 的第一个发布**开始：此后每个发布都可从上一发布迁移。在此之前的 schema 变化允许 reset，但实际上 8 之后再无未发布的变化，所以现场不需要再 reset。
- MIG-1 合入后，任何 SQL 改动必须同一提交带上 `VERSION` 提升、`history/` 快照与迁移级；`dev.py reset server` 保留为开发工具，不再是升级路径。
- 合入 MIG-1 时同步修改两处规范措辞：根 AGENTS 与 Server AGENTS 中“schema 只有当前初始化形态与完整性检查；不加历史兼容轨”改为“运行时只认当前形态；旧版本库由打开前的单向迁移转换，迁移级只读旧版本快照”，`store/schema/mod.rs` 的模块注释同步。
- [持久数据库字段](/v2/storage/database)中仍有已退出的 execution 发布／回执／需求表描述，与现行 SQL 不符；它不属于本设计，单独整理。

## 6. 切片与验收 {#slices}

### MIG-1：迁移机制与第一次真实 schema 变化 {#mig-1}

范围：

- `store/schema/history/v8.sql`、`migrate/` 执行器与 `migrate` 子命令；`serve` 打开时自动迁移；持久阻断退出码 78。
- schema 9 的变化：新增 `schema_migrations`（from_version、to_version、build、started_at、finished_at、backup），只追加、不可修改。它本身就是有用的审计记录，且任何迁移都会重建全部表，所以一张新表已覆盖全库复制路径。
- 第 2 节中 T1 的全部测试；`dev.py update server` 无需改动（启动即迁移）。
- 文档：[持久数据库字段](/v2/storage/database)加入 `schema_migrations` 与迁移规则，[Server](/v2/server)加入 `migrate` 与退出码，changelog 范围句写“Server schema 8 → 9（自动迁移，不 reset）”。

现场验收（alpha.N，schema 8 → alpha.N+1，schema 9；以 `dev.py` 运行的现场 Server）：

| 检查 | 通过条件 |
| --- | --- |
| 升级前记录 | 现场库离线副本的 `user_version`=8 与每表行数；各网络当前 revision；节点清单、各节点 serial、签名钥 generation；六台 `meshway status` 为 `running`、`dataPlane=ready` |
| 升级 | `dev.py update server` 一次完成；日志有 `Database migrated`（8→9、耗时、备份名）；不运行 `reset`，不在 Console 做任何操作 |
| 存储 | `user_version`=9；`schema_migrations` 一行；`controller.sqlite.v8-<stamp>` 存在、0600、以 v8 快照完整检查通过；除新表外每表行数与升级前相同 |
| 账户与配置 | `.meshway/dev-login.json` 原密码登录成功，已有 Console 会话仍有效；各网络 revision 与配置原文不变（解密读取成功） |
| Node | 六台不执行 `up`、不重新接入：Node ID 不变，下一次 `sync` 成功（serial 因新构建可能递增），`serverKey` 为 `current`，两两 `meshway ping` 通过；之后照常 soak ≥ 24 h |
| 拒绝降级 | alpha.N 二进制对新库的副本启动失败（`Unsupported`），副本哈希不变 |
| 中断 | 在现场库副本上于迁移中途 `kill -9`，重启后完成且结果与一次完成相同 |

### MIG-2：Server 部署形态

`dist/systemd/meshway-server.service`、`install.sh --component server`、`meshway-server backup`、共享 sd_notify、`build.yml`／`release.yml` Server 与 Console 产物、[安装](/v2/install)新增 Server 安装／升级／备份／回退一节，以及 `vite.config` 注释。验收：OrbStack／VM 上首次安装、同版本重装不重启、从 `--artifact` 升级（含迁移）、迁移失败的自动回退、`--uninstall` 保留数据，沙箱取值以 `systemctl show` 核对。

### MIG-3：现场切换与升级演练

把现场 Server 迁到 Linux systemd 主机：停 `dev.py` Server，复制已静止的 `controller.sqlite` 与 `master-key` 到新主机 `/var/lib/meshway-server`，属主改为 `meshway-server`，入口域名与 Console origin 不变，六台不重新接入。随后按[安装](/v2/install)文档完成一次 alpha.N+1 → alpha.N+2 的远程升级；若期间有 `compilerVersion` 变化的发布，同一流程覆盖断网窗口演练。此后“从上一发布升级”的 CI 测试进入发布门槛。这是里程碑 B 的退出条件。

## 决定清单 {#decisions}

| # | 决定 | 推荐 |
| --- | --- | --- |
| 1 | 迁移机制 | (c) 复制到规范新库，SQL 为主、Rust 钩子只做解密变换；不放宽 `validate` |
| 2 | 支持范围 | 承诺上一发布版本；代码保留从 8 起的整条链；`MIN_MIGRATABLE` 只在 minor 边界上调 |
| 3 | 起点与规范措辞 | schema 8 为起点；MIG-1 合入时修改根与 Server AGENTS 的“无历史兼容轨”措辞为“运行时只认当前形态＋打开前单向迁移” |
| 4 | 谁触发迁移 | `serve` 自动迁移＋显式 `migrate` 子命令，同一实现；离线管理命令遇旧库拒绝 |
| 5 | 回退边界 | 仅在新 Server 未应答任何 `sync` 前自动回退；之后只前进修复 |
| 6 | 备份 | v0.1.0：迁移前硬链接自动备份（保留 3）＋离线 `backup`（含主密钥，0700／0600）；在线定时备份延后 |
| 7 | 安装脚本 | `install.sh --component server`，共用下载与校验 |
| 8 | Console 托管 | 前置反向代理（TLS＋同源），Server 不 serve 静态文件；发布 Console 静态产物 |
| 9 | 服务用户 | 静态系统用户 `meshway-server`，不用 `DynamicUser` |
| 10 | 现场 Server 主机 | MIG-3 迁到一台专用 Linux 主机或 VM（不用 Planck 共享宿主）；具体主机由用户指定 |
| 11 | sd_notify 归属 | 从 `meshway-daemon` 移到一个共享 crate，Node 与 Server 复用 |
| 12 | MIG-1 的 schema 变化 | 新增 `schema_migrations` 审计表（schema 9） |
| 13 | 持久阻断退出码 | 78（`EX_CONFIG`），unit 以 `RestartPreventExitStatus` 阻止重启循环 |
