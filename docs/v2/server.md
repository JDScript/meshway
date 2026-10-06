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
| `--console-origin`／`MESHWAY_CONSOLE_ORIGIN` | 未设置 | 浏览器访问 Console 的 origin，见下文 |
| `--stun-listen IP:PORT` | 空 | 可重复，显式绑定独立 STUN UDP 服务；最多 4 个，端口非零，不可为多播／广播或重复地址 |
| `--stun-server KEY=HOST:PORT` | 空 | 可重复，发布 hosted 或外部 STUN 端点；KEY 为 1–64 个 ASCII 字母、数字、`_`、`-` 且不可重复，IPv6 使用方括号；最多 4 个 |

命令行显式参数优先于环境变量。这些是实例启动配置，不属于 NetworkConfiguration。日志参数见[运行日志](/v2/logging)；HTTP 日志只记录路由模板与请求关联信息。

**Console origin** 为 `https://host[:port]`，或 `http://localhost`／`http://127.0.0.1`／`http://[::1]`（可带端口）；不含路径、查询、片段或凭据。Server 据此生成交给设备的浏览器链接：设备授权 `authorizationUrl` 为 `<origin>/authorize-device/<enrollmentRequestId>`。未配置时这些字段为 null，设备得不到链接，Node 与 Console 均不自行拼接。`reset-password` 的 `--console-origin` 遵守同一规则。

空库初始化属于正常启动，不提供首用户认领／实例初始化 API，首个公开账户不取得全局管理权限。部署配置、存储和 Node 信任的切换须协调，不通过任意数据库错误自动清库。

## 运维命令

运维密码恢复使用 `reset-password` binary，须先停止占有同一 dataDir 的服务：

```sh
mise exec -- cargo run -p meshway-server --bin reset-password --locked -- \
  --data-dir /absolute/private/data-dir --email user@example.com \
  --console-origin https://console.example.com --expires-in-seconds 900
```

`--expires-in-seconds` 默认 900，范围 60–86400。命令显式交付一次性链接，兑换流程见[无邮件账户恢复](/v2/api/authentication#无邮件账户恢复)；网络 Owner／Admin 无他人全局密码重置权。

GeoIP／ASN 数据集由本机 `ip-datasets` binary 导入，见[数据集部署](/v2/state-reference/ip-info#datasets)。

本工作树的开发服务（`mise run services …`）、地址、重置与恢复见[开发服务、测试与环境](/v2/development#本机服务)。源码合并或编译完成后，仍须执行服务更新并核对实际运行实例。

## 参考

- [数据模型与权限](/v2/entities) · [权限矩阵](/v2/permissions)
- [HTTP／SDK](/v2/api) · [认证](/v2/api/authentication) · [SSE](/v2/api/events)
- [存储与查询](/v2/storage) · [数据库字段](/v2/storage/database) · [内存容器](/v2/storage/memory)
- [配置应用](/v2/configuration-reference/deployment) · [历史](/v2/api/history)
