# Node：节点端

Node 由长期运行的 **`meshwayd`** 和本机 CLI **`meshway`** 组成。新实现处于设计阶段，按此处的职责重新组织；旧节点源码将退出活跃树，复用通过[白名单与回归审查](/v2/architecture#旧实现退出与复用)进行。

## 运行形状

```text
meshway CLI ── 本机 IPC ──► meshwayd
                            ├─ plugin host / 私有持久化 / 生命周期
                            ├─ control          接入与 Server 同步
                            ├─ dataplane        准入、加密、路径与包流
                            ├─ connectivity     邻接、主备与打洞
                            ├─ underlay         底层 socket／系统资源保护
                            ├─ transport
                            │   ├─ udp / tcp    标准发行必带
                            │   └─ vless / trojan / ...
                            ├─ tun / netstack   两种模式的包接入
                            ├─ dns              业务解析
                            └─ diagnostics      状态与获准测量
```

`meshway-control` 是节点侧控制连接组件，消费生成客户端；服务端应用保持独立。`meshway-plugin` 是共同契约／SDK，plugin host 才是装载者。crate 与目录的唯一清单见[总体结构](/v2/architecture)。

## 功能边界

| 组件 | 拥有的责任 |
| --- | --- |
| host | 进程、IPC、装载句柄、私有存储原语、执行上下文和停止协调 |
| control | 持钥接入、目标同步、有效实例／lease 与 State 上报 |
| dataplane | 业务准入、完整逻辑路径、状态型回复、端到端保护与转发 |
| connectivity | 获准下一跳的建连、通道复用、承载主备、打洞尝试与回收 |
| transport | 协议建连／监听、封装、batch／buffer 数据接口 |
| underlay | 用途受限 socket、bootstrap 解析、capture／sink、转发／NAT 资源归属 |
| tun／netstack | 系统 TUN 或用户态网络栈的包接入／交付 |
| dns／diagnostics | 下发的业务解析和真实测量；读取其他组件的窄接口 |

Server 决定许可和有界候选；Node 在获准集合内结合实时状态运行。连接方向不等于业务方向，承载切换不等于逻辑改路，回复沿原会话完整路径返回。

## 已确定的运行方向

- CLI 主要提供 up／down／ping／status 与网络切换；业务配置在云端管理。
- 支持无 TUN userspace networking 与系统 TUN 模式，Meshway 加密／承载仍在用户态。
- 插件启动时动态加载；首版随发行包一起安装、校验、升级，后续再考虑独立更新。
- UDP／TCP 实现必带不授予业务使用权限，VLESS-only 配置继续有效。

运行语义见[模式与 IPC](/v2/node/runtime)，装载与能力见[插件](/v2/node/plugins)。userspace 的应用入口、默认模式、CLI flags、IPC 编码和 native ABI 精确接口尚待对齐。
