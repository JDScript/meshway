# 本地联调：当前可运行流程

本页是当前开发联调。多网络、账户/邀请/权限/设备归属已实现；Agent链接登录与自动建连待交付。Web先选网，每次保存由Controller检查真实身份与本网membership。当前数据库只初始化当前schema，不自动转换旧开发数据。

## 准备与启动

需要 Rust 1.98.0、Node.js 24、pnpm 11.24.0；统一入口还需 Python 3、tmux、lsof。凭据文件当前要求 Unix 私有权限。从仓库根目录执行：

```sh
pnpm install --frozen-lockfile
pnpm dev
pnpm dev:status
```

| 服务 | 地址 |
| --- | --- |
| Controller | `http://127.0.0.1:8420` |
| Web | `http://localhost:3000` |
| 文档 | `http://localhost:5173` |

`pnpm dev` 对应 `python3 scripts/dev.py start`，仅操作本 checkout 的服务及已初始化 demo。归属、更新、停止和日志见[统一开发入口](/dev-services)。分开启动时使用：

```sh
cargo run --locked -p meshway-controller -- serve \
  --data-dir .meshway/controller --bind 127.0.0.1:8420
pnpm dev:web
pnpm dev:docs
```

每个服务使用独立终端。Controller 首次创建私有 SQLite 与管理员 token，后续复用同目录；不要与统一入口重复启动。同数据目录受独占锁保护。首次地址池默认 IPv4 `100.96.0.0/16` 与持久 ULA `/64`，已有库不因启动参数重新编号，见[地址分发](/configuration#持久双栈地址池)。

## 1. 初始化、用户登录与接入令牌

新实例在Web `/setup` 使用实例bootstrap完成首Owner初始化；显式读取该私有token：

```sh
cargo run --locked -p meshway-controller -- admin-token --data-dir .meshway/controller
```

随后日常在`/login`邮箱登录。本网Owner/Admin创建邀请/接入令牌，添加节点可选listener，后续节点配置可修改。Agent使用创建时单次返回的注册token，不能拿用户session或实例token运行。App走[用户浏览器批准/PKCE](/device-login)，Member也可批准本人设备；CLI链接登录仍待实现。

Web 默认使用 `MESHWAY_CONTROLLER_URL=http://127.0.0.1:8420`；远程凭据通道须有 TLS，cookie 配置见[部署边界](/enrollment#本地与远程部署)。

## 2. 注册并复用身份

```sh
cargo run --locked -p meshway-agent -- enroll \
  --controller http://127.0.0.1:8420 \
  --token '<Web 创建的注册 token>' \
  --name nyc --state .meshway/agent.json
cargo run --locked -p meshway-agent -- run \
  --state .meshway/agent.json --interval-seconds 15
```

Agent 在网络请求前持久保存随机节点凭据。保留同一 `--state` 及私有 sidecar，响应丢失 / 重启时复用原绑定，不通过删文件重新注册恢复。目录要求 0700，文件 0600。注册事务与恢复详情见[注册与持久化](/enrollment#注册与恢复)。

普通 `run` 发送心跳并同步配置，不自动安装 TUN。`401` / `403` 或本地文件错误退出；临时错误退避，成功重置。

## 3. 保存状态与真实运行分开查看

```sh
cargo run --locked -p meshway-agent -- sync --state .meshway/agent.json
cargo run --locked -p meshway-agent -- status --state .meshway/agent.json
```

`sync` 校验配置、私有原子发布、确认文件与目录持久性后 ACK；`status` 只读本地保存状态。地址分配、心跳、缓存版本或 ACK 都不是隧道证明。文件发布后 fsync 失败也不能报保存成功，见[提交阶段](/configuration#文件发布与持久性是两个阶段)。

具备managed feature后status、邻接ping与DNS查询提供各自证据，见[CLI](/cli)。Linux TUN显式启用见[数据面](/managed-dataplane)，唯一policy执行见[执行合同](/mesh-execution)。平台托管VLESS/WS不需要本地profiles；设备配置入口仍待进一步收敛。

## 4. 管理与撤销

Web使用真实清单、授权/路由、标签和连接配置，保存CAS冲突保留草稿。唯一policy不需激活；新console的编辑体验由独立工作流接入，完整出口尚未接线。见[Access](/access-policy)、[路由方案](/routing)。preview不发trace、不代替授权。

撤销后节点保留在清单，控制面请求被拒绝；托管运行层撤回旧代际，控制面失联则由 lease 到期终止。`last_seen` 只是联系时间。若只运行基本心跳，不应宣称建立或切断了业务隧道。

## 构建与重现检查

```sh
pnpm build:docs
pnpm --filter @meshway/docs preview
pnpm --filter @meshway/web check
pnpm test:web
pnpm build:web
```

文档 preview 与开发服务共用 5173，不应同时运行。Web UI 在开发页面手工验证，不安装浏览器自动化套件。生成契约、检查命令和隔离实验入口集中在[开发与验收](/development)；修改契约须重新生成并通过 drift check。
