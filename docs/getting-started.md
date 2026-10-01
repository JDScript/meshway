# 开始使用

基础联调路径是：controller 创建持久状态 → 管理员签发一次性令牌 → Agent 注册并保存身份 → 持续心跳 → Web 查看与撤销。本轮在此基础上增加地址分配、配置同步 / 回执和策略预览 / 条件保存。VitePress 独立提供中文设计文档。

::: info 已验证的控制面流程
注册、心跳、Web 登录、重启持久化和撤销，以及新增双栈地址、策略编辑和配置回执均已通过真实服务集成测试。注册建立的是控制面凭据身份；地址、心跳和配置 ACK 都不表示 mesh 隧道或在线状态。
:::

## 准备环境

已有开发环境可直接使用 `pnpm dev` 启动本地服务，`pnpm dev:status` 查看真实状态，`pnpm dev:update` 构建并更新。完整命令及已有 Docker demo 的保留方式见[统一开发入口](/dev-services)。下文保留首次初始化和各组件的单独操作说明。

- Rust 1.98.0 与 Cargo；仓库的 `rust-toolchain.toml` 固定工具链版本。
- Node.js 24 与 pnpm 11；精确 pnpm 版本见根 `package.json` 的 `packageManager`。
- 当前凭据存储要求 Unix 私有目录 / 文件权限，适用于 macOS、Linux 等 Unix 环境。
- 在仓库根目录执行以下命令；各开发服务分别使用一个终端。

```sh
pnpm install
pnpm protocol:generate
```

生成步骤让 Rust Agent 客户端、TypeScript SDK 与控制面契约保持同步。修改协议后还需 `pnpm protocol:check` 做漂移检查，不能直接改生成文件。

## 1. 启动控制面

```sh
cargo run -p meshway-controller -- serve \
  --data-dir .meshway/controller \
  --bind 127.0.0.1:8420
```

首次启动在私有数据目录创建 `controller.sqlite3`、`admin.token` 与 `controller.lock`；常规启动日志不输出秘密。同一数据目录只允许一个 controller 使用；重启使用同一个 `--data-dir` 保留管理员凭据、节点、已消费令牌及撤销状态。

### 地址池与开发数据库升级

新网络默认采用 `100.96.0.0/16` 与首次生成后持久保存的 ULA IPv6 `/64`。首次创建网络配置时，可以在 `serve` 后传入 `--ipv4-prefix` 和 `--ipv6-prefix`，例如 `--ipv4-prefix 100.96.0.0/16 --ipv6-prefix fd12:3456:789a::/64`。IPv4 必须是规范网络前缀且有可用主机地址；IPv6 要求 `fd00::/8` 内规范 `/64`。

原注册基线的 v1 SQLite 数据库进行事务增量升级，保留管理员凭据、节点身份、令牌绑定和撤销记录，按稳定 ID 顺序为既有节点分配地址；无需清空目录或重新注册。已有地址池的数据库拒绝不同前缀参数，不会自动重新编号。范围见[开发数据库升级](/configuration#当前开发数据库升级)。

在另一终端显式读取管理员令牌：

```sh
cargo run -p meshway-controller -- admin-token --data-dir .meshway/controller
```

也可读取 `.meshway/controller/admin.token`。以下示例在当前终端变量中加载令牌，再用管理员 bearer 读取概览：

```sh
ADMIN_TOKEN="$(cat .meshway/controller/admin.token)"
curl --fail-with-body \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  http://127.0.0.1:8420/api/v1/overview
```

响应包含 `name`、`version`、`node_count`；本轮增加当前全局 `configuration_revision`。空数据库的节点数量为 `0`；此后统计所有持久化节点，包括已撤销节点。

## 2. 创建一次性注册令牌

```sh
curl --fail-with-body \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  --data '{"expires_in_seconds":600}' \
  http://127.0.0.1:8420/api/v1/enrollment-tokens
```

有效期允许 `60..86400` 秒。响应中的 `token_id` 用于标识令牌，`token` 才是注册 bearer，`expires_at` 是到期时间。将刚收到的 `token` 用于下一步，不要把管理员令牌交给 Agent。

## 3. 注册 Agent 并发送心跳

```sh
cargo run -p meshway-agent -- enroll \
  --controller http://127.0.0.1:8420 \
  --token '<刚创建的注册 token>' \
  --name nyc \
  --state .meshway/agent.json
```

Agent 在请求前原子保存随机节点凭据，注册后复用这一私有状态文件。节点 ID 由 controller 建立，不能用名称或自行指定的 UUID 代替身份。响应丢失时保留状态文件，按[恢复语义](/enrollment#注册与恢复)恢复；不能通过删除状态文件自动生成另一份身份来重试。

状态文件和旁路锁文件以 `0600` 创建，父目录必须由当前用户持有并为私有 `0700`。默认 `.meshway` 不存在时自动私有创建；自定义 `--state` 路径也应放在专用私有目录中。已有目录或文件权限不符时命令会拒绝使用。

发送一次心跳：

```sh
cargo run -p meshway-agent -- heartbeat --state .meshway/agent.json
```

持续发送心跳并同步配置：

```sh
cargo run -p meshway-agent -- run \
  --state .meshway/agent.json \
  --interval-seconds 15
```

`run` 立即开始一轮工作，在心跳后执行配置同步，成功后按给定间隔继续。临时故障从 1 秒开始指数退避，最多 60 秒，成功后重置。HTTP `401` / `403` 和本地文件错误会退出；文件恢复后可另行重试。退出后使用同一状态文件重启继续关联同一个节点。

查看本地注册状态而不暴露凭据：

```sh
cargo run -p meshway-agent -- status --state .meshway/agent.json
```

输出中的 `pending` / `enrolled` 是本地注册进度。本轮 `status` 增加 `stored_revision`、`stored_digest` 和已缓存的双栈地址；无缓存时版本为 `0`，其余缓存字段为 `null`。它只读本地文件，不向 controller 查询最新 ACK，也不发送连通性探测。

### 显式同步配置

发送一次配置同步：

```sh
cargo run -p meshway-agent -- sync --state .meshway/agent.json
```

Agent 通过生成客户端读取自身配置，验证绑定、schema、摘要、网络和策略，私有原子发布并确认文件和父目录 fsync 成功后再 ACK 当前 revision / digest。默认配置路径是 `.meshway/agent.json.config.json`，即 `<state>.config.json`，v1 身份文件继续使用。摘要未变时不重写缓存，但每次 ACK（包括重启后）仍重新确认持久性；凭据使用前也执行确认。

成功输出的 `stored_revision` 和摘要表示本次已保存配置且收到匹配的 ACK 响应；本地 `status` 只表示可见缓存，不能推出持久性已确认、服务端回执成功或仍为最新。更旧版本、同版本不同摘要及损坏快照在发布前被拒绝，保留原缓存且不 ACK。

写入失败按提交阶段区分：`NotCommitted` 保留旧文件；`CommittedNotDurable` 表示新文件已经可见、崩溃持久性未确认，Agent 内存保留已发布值但返回错误、不 ACK。不要通过删除身份或缓存“重试”；恢复文件条件后重新运行命令，由文件和目录同步成功后继续。详情见[同步与回执](/configuration#校验、私有落盘、再-ack)。这一步不会创建 TUN、应用 OS 网络或建立隧道。

## 4. 在 Web 查看并撤销

```sh
pnpm dev:web
```

打开 `http://localhost:3000`，使用管理员令牌登录。Next.js 验证成功后设置 `HttpOnly`、`SameSite=Strict` 的登录 cookie，8 小时后需要重新登录；服务端通过 `@meshway/control-client` 读取与修改控制面数据。默认 controller 地址为 `http://127.0.0.1:8420`，可用 `MESHWAY_CONTROLLER_URL` 调整。未登录不显示管理数据；controller 不可达时保留会话并显示真实错误。

本地 HTTP 可使用默认 `MESHWAY_WEB_COOKIE_SECURE=auto`；远程 HTTPS 部署可明确设为 `true`。完整取值与代理行为见[Secure 配置](/enrollment#web-cookie-的-secure-配置)。

管理员也可直接读取节点清单：

```sh
curl --fail-with-body \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  http://127.0.0.1:8420/api/v1/nodes
```

列表为 `{ nodes: [...] }`，字段包括身份、版本、注册 / 联系 / 撤销时间，本轮扩展双栈地址、必填可空 `country`、当前配置版本及节点回执版本。时间采用 RFC 3339；尚未撤销时 `revoked_at` 为 `null`。`last_seen` 不能标成连通性探测结果，回执也不能标成网络已应用。

在 Web 撤销测试节点，或向 `POST /api/v1/nodes/revoke` 发送管理员 bearer 及 `{ "node_id": "<实际节点 ID>" }`。此后节点仍保留在清单中，其心跳返回 HTTP `403`。登出后，清单读取及创建令牌、撤销等 server action 都受到认证检查。

### 国家与策略管理

Web 节点清单可通过 CAS 编辑国家元数据；配置列的“已校验保存”表示当前版本的回执，不表示在线或网络已应用。打开 `http://localhost:3000/routing` 使用引导式策略编辑器。使用真实节点 ID，来源国家从清单派生；预览互联网国家只是显式提示，没有自动 GeoIP。

1. 页面显示只读网络前缀、当前策略及草稿基准 revision；初始策略为空规则、默认 `reject`，可选择 direct / reject。
2. 编辑 suffix-only、CIDR / 节点 / 国家条件及有序 hop，选择来源与预览目标，点击“编译预览”查看匹配规则、逻辑路径和诊断。预览不保存、不推进草稿 CAS 基准，不发 DNS 请求，也不测试网络连通。IPv4-mapped IPv6 输入返回 `invalid`，不会隐式转换后进入默认动作。
3. 点击“保存策略”，请求携带草稿的 `expected_revision`。策略非法返回 `422`；清单、国家或策略已变化导致 `409` 时保留草稿。基准仅在保存成功或显式“重新读取”后更新；较新的预览结果不能自动覆盖基准。
4. 让 Agent 同步后查看回执版本。撤销使引用非法时，管理端保留原策略及诊断，节点收到整体 deny-all 有效策略。

这一浏览器闭环和匿名保存 action 重放拒绝已由 `e2e/routing.spec.ts` 验证。HTTP 路由和生成请求类型见[协议页](/protocol#配置与策略-api)。

## 5. 启动文档

另一个终端启动本文档：

```sh
pnpm dev:docs
```

打开 `http://127.0.0.1:5173`。也可以直接运行文档包：

```sh
pnpm --filter @meshway/docs dev
pnpm build:docs
pnpm --filter @meshway/docs preview
```

`preview` 在 `127.0.0.1:5173` 提供构建结果，与文档开发服务共用端口，应先停止开发服务。

## 重现集成验收

首次运行浏览器测试前安装 Chromium，然后执行端到端测试：

```sh
pnpm exec playwright install chromium
pnpm test:e2e
```

`test:e2e` 构建 Rust 与 Next.js，使用临时数据目录和端口启动真实服务。当前 2 项测试均已通过：`e2e/control-plane.spec.ts` 与 `e2e/routing.spec.ts`。注册流程覆盖：

1. 无管理员凭据不能读取清单或执行管理写入；合法登录后可签发限时令牌。
2. Agent 注册、心跳与 Web 使用同一个服务端节点 ID，状态文件重启后继续复用。
3. 使用相同数据目录重启 controller，原管理员凭据仍有效，节点 ID 保留。
4. Web 撤销后 Agent 心跳失败；匿名重放同一 Next.js 管理 action 被拒绝并跳转登录。
5. 关闭 controller 后，Web 显示连接失败；登出后受保护页面跳转登录。
6. 登录 cookie 为 HttpOnly / Strict，页面不包含管理员秘密；移动视口无水平溢出，流程无浏览器 JavaScript 错误。

配置 / 路由测试覆盖三 Agent 唯一双栈地址和国家标签、逻辑多跳及 suffix / IPv6 目标预览、草稿保存 / 匿名拒绝 / 409 保留、私有缓存先于 ACK、重复同步 mtime 不变、心跳 / ACK 不增版、controller 重启后地址 / 策略 / 回执稳定，以及撤销必经节点后的源策略保留和 deny-all 配置回执。

数据库升级、权限隔离、并发消费、响应丢失、映射地址拒绝和文件发布 / 持久性失败由 Rust / SDK 测试进一步覆盖，见[注册验收矩阵](/development#注册与持久化验收矩阵)与[配置验收证据](/development#新切片验收清单)。这些测试使用独立数据目录；既有运行实例的升级需由其实际启动执行。非本地部署需要独立部署层 TLS，详见[部署边界](/enrollment#本地与远程部署)。网络转发属于后续里程碑。
