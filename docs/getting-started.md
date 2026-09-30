# 开始使用

本切片的联调路径是：controller 创建持久状态 → 管理员签发一次性令牌 → Agent 注册并保存身份 → 持续心跳 → Web 查看与撤销。VitePress 独立提供中文设计文档。

::: info 已验证的控制面流程
下面的注册、心跳、Web 登录、重启持久化和撤销流程已通过真实服务集成测试。注册和心跳建立的是控制面凭据身份，不表示 mesh 隧道或在线状态。
:::

## 准备环境

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

响应为 `{ name, version, node_count }`。空数据库的数量为 `0`；此后统计所有持久化节点，包括已撤销节点。

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

持续发送心跳：

```sh
cargo run -p meshway-agent -- run \
  --state .meshway/agent.json \
  --interval-seconds 15
```

`run` 立即发送首次心跳，成功后按给定间隔继续；临时故障从 1 秒开始指数退避，最多 60 秒，成功后重置。HTTP `401` / `403` 会退出。退出后使用同一状态文件重启继续关联同一个节点。

查看本地注册状态而不暴露凭据：

```sh
cargo run -p meshway-agent -- status --state .meshway/agent.json
```

输出中的 `pending` / `enrolled` 是本地注册进度，`status` 不发送连通性探测；`run` 也尚不建立数据面隧道。

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

列表为 `{ nodes: [...] }`，字段包括 `id`、`name`、`agent_version`、`registered_at`、`last_seen` 与 `revoked_at`。时间采用 RFC 3339；尚未撤销时 `revoked_at` 为 `null`。`last_seen` 不能标成连通性探测结果。

在 Web 撤销测试节点，或向 `POST /api/v1/nodes/revoke` 发送管理员 bearer 及 `{ "node_id": "<实际节点 ID>" }`。此后节点仍保留在清单中，其心跳返回 HTTP `403`。登出后，清单读取及创建令牌、撤销等 server action 都受到认证检查。

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

`test:e2e` 构建 Rust 与 Next.js，使用临时数据目录和端口启动真实服务，运行 `e2e/control-plane.spec.ts`。已通过的集成覆盖：

1. 无管理员凭据不能读取清单或执行管理写入；合法登录后可签发限时令牌。
2. Agent 注册、心跳与 Web 使用同一个服务端节点 ID，状态文件重启后继续复用。
3. 使用相同数据目录重启 controller，原管理员凭据仍有效，节点 ID 保留。
4. Web 撤销后 Agent 心跳失败；匿名重放同一 Next.js 管理 action 被拒绝并跳转登录。
5. 关闭 controller 后，Web 显示连接失败；登出后受保护页面跳转登录。
6. 登录 cookie 为 HttpOnly / Strict，页面不包含管理员秘密；移动视口无水平溢出，流程无浏览器 JavaScript 错误。

重启后撤销 / 令牌消费的持久化、权限隔离、并发消费、响应丢失和本地文件失败由 Rust / SDK 测试进一步覆盖，见[完整验收矩阵](/development#注册与持久化验收矩阵)。非本地部署需要独立部署层 TLS，详见[部署边界](/enrollment#本地与远程部署)。网络转发属于后续里程碑。
