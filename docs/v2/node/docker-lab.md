# 三节点 Docker 开发环境

三个常驻 Node 共用已构建的 Linux 二进制，各自持有独立状态卷。Docker Compose 只运行程序；Rust 编译由宿主机的固定工具链执行。

## 节点与持久状态

| 容器 | hostname | Named volume |
| --- | --- | --- |
| `meshway-lax` | `meshway-lax` | `meshway-lax-state` |
| `meshway-sha` | `meshway-sha` | `meshway-sha-state` |
| `meshway-nyc` | `meshway-nyc` | `meshway-nyc-state` |

卷挂载在 `/var/lib/meshway`，保留 Node 密钥、接入请求、Server 绑定、缓存配置和 up/down 意图。容器停止、重启或替换程序均复用原卷。运行镜像为按 digest 固定的 Alpine，程序目录只读挂载；每个容器限制为 0.5 CPU、256 MiB 内存、64 个进程，使用独立 bridge 网络。

当前 Compose 源码显式使用 `--network-mode control-only`：容器 `cap_drop: ALL`，没有 TUN 安装权限。源码修改不代表常驻容器已经部署新程序／参数；须用下方 versions／status 核对实际运行实例。

接入地址为 `https://meshway-dev-api.jdscript.dev/`，转发到本机 Server；批准链接使用 `http://127.0.0.1:3001` 的 Console。`mise run services start server` 的启动命令包含 `--console-origin http://127.0.0.1:3001`；以其他入口启动 Server 时也须配置该参数或 `MESHWAY_CONSOLE_ORIGIN`，否则 Node 如实显示缺少批准链接。Node 不自行拼接 Console URL。身份绑定仍按[Node 控制组件](/v2/node/control)检查。

## 首次启动

需要本机 Docker 和当前 Server／Console 可用。支持 Linux arm64／amd64 Docker engine；构建任务按 engine 架构选择 Rust musl target，并核对产物的 ELF 架构。

```sh
mise run build:node:linux
mise run node:lab deploy
mise run node:lab versions
mise run node:lab status
```

`build:node:linux` 的 Cargo Zigbuild／Zig 由 `mise.toml` 和 `mise.lock` 固定，只在该任务激活。复用 `target/node-linux` 缓存；下载 Rust target 后，在宿主机交叉编译一次 daemon 和 CLI。

`deploy` 使用 `docker compose up --no-build`，校验产物后启动或更新三个容器，等待本机 IPC 健康检查。初始期望状态为 down；执行 up 后按链接在 Console 中选择网络批准：

```sh
docker exec meshway-lax meshway up
docker exec meshway-sha meshway up
docker exec meshway-nyc meshway up
```

也可以进入容器使用同一 CLI：

```sh
docker exec -it meshway-lax sh
meshway status --json
```

## 更新与核对

```sh
mise run build:node:linux
mise run node:lab deploy
mise run node:lab versions
```

构建产物按不可变 release 目录保存在仓库公共目录 `.meshway/node-lab/releases/`；每份 `build.json` 记录 Git revision、dirty 标记、Rust source digest、target 和二进制 SHA-256。构建期间 Rust 源码变动则不发布该产物。`versions` 同时读取容器内清单和 `/proc/1/exe` 的实际 SHA-256，`runningDaemonMatches` 表示正在运行的 daemon 是否匹配清单。

程序更新由 Compose 重建需要更新的容器，卷继续保留；已保存的 up 意图会在新进程内重新认证并取得新 instance。`deploy` 不执行 Rust 编译，也不构建镜像。

## 状态、日志与停止

```sh
mise run node:lab status
docker logs --tail 100 -f meshway-lax
docker exec meshway-lax meshway down
mise run node:lab stop
```

`down` 保存业务下线意图；`stop` 停止三个容器并保留各自意图与卷。容器日志使用 JSON 和 Docker local driver，每个容器轮转保留最多三个 10 MiB 日志文件；字段规则见[运行日志](/v2/logging)。管理脚本核对精确资源名和所有权标签，遇到同名外部资源会停止操作。

当前健康检查表示 daemon 的本机 IPC 可响应；`controlReady` 表示已完成 Server 控制接入。本环境不安装 TUN 或验证业务包流；daemon TUN 的独立候选范围见[网络模式](/v2/node/runtime)。SOCKS5 尚未实现，三个地区名称仅作为开发节点标识，不证明真实地理部署。
