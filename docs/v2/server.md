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
| resources | 规则集与 GeoIP／DNS 等可信输入的管理边界 |
| store | 私有 SQL、事务、当前 schema 初始化与完整性检查 |
| http | 请求抽取、同源 API、错误和生成声明；不另写领域规则 |

配置提交、Node 接入／删除、成员移除和必要执行退出共用事务边界。身份和权限在最终事务重检；HTTP waiter 取消不丢弃已接纳事务的提交结果。每个事务把它的 effects 和结果一起返回，执行器在 COMMIT 之后统一应用到内存 journal，见[数据库执行与提交](/v2/storage#数据库执行与提交)。

## 当前能力

| 状态 | 范围 |
| --- | --- |
| 已接通 | 账户／会话、网络／成员、加入链接、密码恢复、配置编辑／预览／CAS／历史、私有配置加密 |
| 已接通 | Node 持钥／PKCE 接入、实例／lease、按需执行目标、回执、永久撤销 |
| 已接通 | State／历史、用户与 Node SSE、固定 revision 应用进度、限时观察任务 |
| 已接通 | 规则集 HTTPS 下载、精确 artifact 绑定、单次 apply 准备和周期更新 |
| 未完成 | 完整类型化配置诊断／影响预览、地址重分配／材料轮换操作、非管理员 Tag owner 的局部入口 |
| 未完成 | 可信 DNS／GeoIP 输入与自动方案闭环、规划 evaluate／部分管理执行视图、版本兼容展示和 Node 发布安装链 |

Server HTTP／SDK 和持钥测试 Node 的验证止于相应控制流程；新 Node 程序、真实网络安装与包流另行验收。网络删除／账户注销的生命周期仍待确定。具体可用操作见[生成契约](/v2/contracts/openapi.json)。

## 运行与维护

```sh
mise exec -- cargo run -p meshway-server --bin meshway-server --locked -- \
  --data-dir /absolute/private/data-dir --listen 127.0.0.1:0 \
  --registration-enabled true --session-ttl-seconds 86400
```

运行目录使用私有、无符号链接的绝对路径并独占锁。端口 0 分配临时监听端口；注册默认开放，可由实例配置关闭。空库初始化属于正常启动，不提供首用户认领／实例初始化 API，首个公开账户不取得全局管理权限。

运维密码恢复使用 `reset-password` binary，须先停止占有同一 dataDir 的服务：

```sh
mise exec -- cargo run -p meshway-server --bin reset-password --locked -- \
  --data-dir /absolute/private/data-dir --email user@example.com \
  --console-origin https://console.example.com --expires-in-seconds 900
```

命令显式交付一次性链接；网络 Owner 无他人全局密码重置权。部署配置、存储和 Node 信任的切换须协调，不通过任意数据库错误自动清库。

## 参考

- [数据模型与权限](/v2/entities) · [权限矩阵](/v2/permissions)
- [HTTP／SDK](/v2/api) · [认证](/v2/api/authentication) · [SSE](/v2/api/events)
- [存储与查询](/v2/storage) · [数据库字段](/v2/storage/database) · [内存容器](/v2/storage/memory)
- [配置应用](/v2/configuration-reference/deployment) · [历史](/v2/api/history)
