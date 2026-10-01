# 统一开发入口

在仓库根目录使用：

```sh
pnpm dev
pnpm dev:status
pnpm dev:restart web
pnpm dev:update
pnpm dev:stop
```

以上别名对应同一个 Python 入口，也可以直接使用：

```sh
python3 scripts/dev.py start
python3 scripts/dev.py status
python3 scripts/dev.py restart web
python3 scripts/dev.py update
python3 scripts/dev.py stop
```

形式为 `python3 scripts/dev.py <start|status|stop|restart|update> [controller|web|docs|demo|all]`。
省略服务等同于 `all`：controller、Web、文档站，以及**本 checkout 已初始化**的 Docker demo。
脚本从自身位置确定仓库和 `.meshway`，不依赖当前工作目录；独立 worktree 不会访问主 checkout 的运行数据。

## 本地服务

需要 Python 3、tmux、lsof、Rust、Node 24 和 pnpm 11。首次安装依赖：

```sh
pnpm install --frozen-lockfile
python3 scripts/dev.py start
```

| 服务 | 地址 | 持久 tmux session |
| --- | --- | --- |
| controller | `http://127.0.0.1:8420` | `meshway-controller` |
| web | `http://localhost:3000/login` | `meshway-web` |
| docs | `http://localhost:5173` | `meshway-docs` |

Controller 数据位于 `.meshway/controller`。Web 自动连接上述 controller，使用 `MESHWAY_WEB_COOKIE_SECURE=auto`。
日志位于 `.meshway/services/meshway-{controller,web,docs}.log`，目录 0700、文件 0600。
服务在 tmux 中持续运行，开发入口退出后仍然可用。

`start` 幂等；已有 session 必须同时匹配 checkout、启动命令、实际进程和监听端口归属，才能接管。
同名外部 session 或其他进程占用端口时明确报错。不同 worktree 使用相同默认 session 名和端口，因此不会同时启动两套本地服务。
`stop` 向已核实的服务进程树发送 SIGINT，等待退出；超时保留现场并报错。

Controller 二进制缺失时，`start controller` 自动执行锁定依赖的 managed 构建。
`update controller` 在重启前执行同一构建：

```sh
cargo build --locked -p meshway-controller -p meshway-agent \
  --bin meshway-controller --bin meshway-agent \
  --features meshway-agent/managed-connections
```

`restart` 不编译；`update web/docs` 重启各自开发服务器，继续使用 HMR。
SQLite 升级使用当前数据向前迁移，实时策略保留。构建失败不会进入重启步骤。

## 状态与退出码

```sh
python3 scripts/dev.py status
python3 scripts/dev.py status demo --json
```

状态区分 session 是否运行、端口是否监听、真实 readiness 和检查错误。
Controller readiness 是由已核实进程提供的公开 401/Bearer 响应；Web 是 `/login` 的 HTML 200；文档站是首页的 HTML 200。
运行 session、保存的配置、二进制部署 receipt 均不能替代 readiness。
Demo 的 `ready` 要求容器运行、注册有效、当前且未过期的 `applied` 运行报告；TUN 证据另列为 `dataplane`，缺少证明时明确显示 `not-proven`。日志 rotation 可能移除旧启动证明，入口不会据此捏造数据面状态或无故重启 Agent。实际隧道访问仍用单独的流量验证。
选择的服务未 ready、检查失败或生命周期操作失败时返回非零。
`all` 中尚未初始化 demo 是提示，不影响基础服务的成功；显式 `status demo` 返回非零。
JSON 只输出公开、精简的状态，不输出管理员 bearer、节点凭据、密钥或完整进程环境。

Demo 的 TUN 证明读取当前容器 `StartedAt` 以来的**全部可用**日志流，只保留最后一条 `managed_dataplane` 事件，再按当前生成快照的 node/revision/digest 校验；后来的撤回、lease expiry 或设备错误会使旧 ready 失效。
读取上限为单行 64 KiB、stdout/stderr 合计 16 MiB、10 秒，不将完整历史装入内存。读取失败、超限、截断或畸形 stdout 返回 `not-proven`；仍同时要求新鲜 runtime、ACK、内核接口/路由、UID/capability 和现有 epoch fences。
现有 Docker 日志保留约 1 MiB/1 文件；如果 generation proof 已被 rotation 删除，状态仍为 `not-proven`，不会用 ACK 或邻接探测替代真实 TUN 证明，也不会为状态检查自动重启 Agent。

## 已有 Docker demo

统一入口只操作 `.meshway/docker-demo/demo.json` 已绑定的 demo。
首次初始化仍使用仓库 `experiments/managed-connections/DEMO.md` 的现有流程：显式提供镜像、控制面地址和私有管理员 token 文件，完成原有授权、身份注册及隔离网络设置。
未初始化的 worktree 不会自动创建或 seed demo。

```sh
python3 scripts/dev.py start demo
python3 scripts/dev.py restart demo
python3 scripts/dev.py status demo
python3 scripts/dev.py update demo
# 已预建、验证的 native managed image，也可显式指定：
python3 scripts/dev.py update demo --image meshway-managedtest:VALIDATED_TAG
```

对已有 Ubuntu **development/writable** demo：

- `start/stop/restart` 按 metadata 和完整 ownership labels 找到原容器，保留容器 ID、apt 软件及自定义文件；不会重建容器。
- `update` 默认从当前 checkout 的 `Dockerfile.dev` 构建唯一标签镜像，或使用 `--image` 指定的本机架构 Linux managed 镜像。
- 从一个停止、`network=none` 的临时容器提取 Agent ELF 到已批准临时根目录下的独占 0700 子目录（文件 0600），完成后按 inode/ownership 清理；逐个备份旧 binary，再校验新旧 digest、架构和功能。
- 将新 binary 暂存到原容器，在原路径同文件系统 atomic rename，SIGINT 优雅停止，再启动同一容器。确认 live CLI、相同容器 ID、apt inventory、CLI alias 以及原始 identity/key/profiles 不变。
- 原始 `demo.json.image_id` 仍是容器 base image pin。源 image、binary SHA、各容器验证结果和旧 binary 保留在 `.meshway/services/binary-update-*/deployment.json` 同目录；二进制更新不冒充 base image 更新。
- 完成部署后通过现有 SDK-backed demo status 检查当前 runtime readiness，并单独呈现数据面证明。部署 receipt 的 live IPC 结果不等同于隧道连通性。

只读 demo 的 `update` 会明确拒绝。要升级其镜像，请显式使用原有 `demo.py upgrade-image`；这会重建容器，保留绑定的数据目录，但丢弃容器可写层。
只读 demo 已 ready 时 `start` 仅检查状态；需要恢复的 `start` 以及 `restart/stop` 使用原有生命周期流程，恢复或重启可能重建容器。
开发模式的容器缺失、未完成 enrollment 或有待恢复的 mode/image transition 时，统一入口报错，需先按现有 demo 文档处理。

所有 Docker 操作共用现有 performance lock，默认路径：

```text
/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode/meshway-performance.lock
```

直接委托 `demo.py` 时由它持锁；原位 binary 和 writable 生命周期由入口持锁并向 Docker 子进程继承 FD。
与压测共享 Docker VM 时会等待已有任务完成。可通过 `MESHWAY_PERF_LOCK` 显式统一到另一私有绝对路径，所有 runner 必须一致。
失败部署保留旧 binary 和 receipt 供检查；不会恢复旧业务策略或身份快照。临时容器和暂存文件只在 ownership 仍匹配时清理。

## Host-only 验证

```sh
python3 -B -m unittest discover -s scripts -p 'test_dev.py' -v
python3 scripts/dev.py --help
node --test experiments/managed-connections/packet-log.test.ts
```

测试使用临时私有文件和模拟进程/Docker，不接触 live 服务。
