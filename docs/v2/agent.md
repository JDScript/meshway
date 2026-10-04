# Agent：Daemon、CLI 与插件

新 Agent 按 V2 Config／State 与当前节点产品模型重新设计。**本页记录已确认方向及组件提案，尚未实现新的 meshwayd、CLI 或动态插件。** 现有 Agent、NAT 和出口代码及回归用例是待评估的复用资产，不预先要求保留其目录或旧 HTTP 接口。

## 已确认的方向

| 项目 | 决定 |
| --- | --- |
| 进程 | `meshwayd` 是长期运行的节点进程，`meshway` 是通过本机 socket 操作它的 CLI |
| CLI | 主要提供 `up`、`down`、`ping`、`status`，以及网络切换；日常业务配置由云端管理 |
| 网络模式 | 无 TUN 的 userspace networking，以及通过系统 TUN 接入 OS 网络栈的模式 |
| 插件装载 | 启动时动态加载，增加兼容插件不要求重新编译 daemon 主体；运行中热卸载不在首版范围 |
| 交付 | 首版所有正式插件随发行包一起安装、验证和升级；之后再讨论逐插件更新 |
| 配置与状态 | 插件使用平台下发的配置并报告实际状态，能力目录与 Config／State 对齐 |

“系统 TUN 模式”不表示 Meshway 的加密、承载或路径逻辑进入内核。两种模式的区别主要是应用流量如何进入 Meshway、由哪个 TCP/IP 栈处理；详见[运行方式与本机接口](/v2/agent/runtime)。

## 组件形状

下面是**逻辑职责图**；具体 Cargo 包名是建议，不是当前 workspace 清单。

```text
meshway (CLI)
    │ 本机 IPC：命令、查询、事件
    ▼
meshwayd
    ├─ plugin host / 私有持久化 / 本机 IPC / 运行上下文
    ├─ meshway-control                       必需：连接 Controller
    ├─ meshway-dataplane                     必需：准入、加密、路径与包流
    ├─ meshway-connectivity                  必需：邻接、承载选择与打洞协调
    ├─ meshway-underlay                      必需：底层 socket 与系统资源保护
    ├─ meshway-transport                     必需：统一承载接口
    │   ├─ meshway-transport-udp             标准发行必带
    │   ├─ meshway-transport-tcp             标准发行必带
    │   ├─ meshway-transport-vless
    │   ├─ meshway-transport-trojan
    │   └─ ...
    ├─ meshway-tun                           系统 TUN 模式的包入口
    ├─ meshway-netstack                      userspace 模式的网络栈与适配
    ├─ meshway-dns                           业务 DNS
    └─ meshway-diagnostics                   本机查询与获准测量
```

两个命名区别：

- `meshway-controller` 已是服务端应用。节点侧连接它的组件建议叫 **`meshway-control`**，直接消费生成的 `meshway-control-client`，不把服务端、数据库或网络策略编辑器装进每个 daemon。
- **`meshway-plugin` 是插件契约／SDK，不是一个需要再被加载的业务插件。** daemon 的 plugin host 使用它装载各 provider；`meshway-transport` 则是承载领域的公共接口，不为每种协议重建一个框架。

首版“插件一起安装”和“功能可选”可以同时成立：TUN、netstack、DNS 和各协议的实现都可随包交付，是否运行由本机条件、下发配置及任务决定。标准发行必带 UDP/TCP 实现不授予使用它们的业务权限；VLESS-only 配置仍不能偷偷回退到 native UDP/TCP。

## 职责边界

| 组件 | 拥有的职责 | 对外提供的结果 |
| --- | --- | --- |
| daemon host | 进程与实例上下文、本机 IPC、私有存储原语、插件装载、任务与资源生命周期 | 可查询的本机状态、组件句柄与受限 host services |
| control | 节点接入、持钥认证、Config／执行目标同步、lease 取得、State 上报 | 一致的已授权执行目标、有效身份／lease 上下文 |
| dataplane | 业务端点准入、完整逻辑路径、状态型回复、端到端加密、转发、按需流量需求 | 获准包流、真实路径状态与计数 |
| connectivity | 下一跳邻接、dial/listen 协调、通道复用、优先级主备、打洞尝试和空闲回收 | 已认证可用通道及对应的 socket／尝试证据 |
| transport providers | 协议自身的建连／监听、封装与数据传递；TLS／WS 等协议参数执行 | 统一的承载会话与本协议错误／实际限制 |
| underlay | 用途受限的底层 socket、bootstrap 解析、路由保护、系统 capture／sink／转发与 NAT 资源归属 | 可验证的资源句柄、绑定／代际及清理结果 |
| tun / netstack | 相应模式的 IP 包入口、出口与协议栈适配 | 统一包接口及实际安装／运行状态 |
| dns | 下发的 Mesh 名称、业务解析规则及有期限的域名关联 | 真实解析结果、来源／期限和错误 |
| diagnostics | 读取其他组件的窄观测接口，执行获准的 ping／观察任务 | 有明确来源的测量与状态，不制造健康数据 |

Controller 决定业务授权和有界候选／完整路径。Agent 根据实时状态选择、建立和执行，但不能自行扩大 access 或绕过必经节点。承载主备切换与逻辑路径变更分开；回复继续沿原会话的完整路径返回。

`meshwayd` 可以像一个干净的持久运行宿主，但仍须有明确的协调责任：一个活动网络、有效执行上下文、撤回／期限、任务取消与退出顺序不能分散成“每个插件自行猜测”。宿主不重新实现网络策略编译器；领域算法保留在相应组件中。

## 继续对齐的内容

- [运行方式、命令与本机 IPC](/v2/agent/runtime)：up/down 的目标语义、网络切换和持久状态。
- [插件与能力](/v2/agent/plugins)：启动装载、ABI、整体交付、Config／State 边界。
- [Crates 与目录](/v2/agent/structure)：应用、共享契约、安全实现与 native 入口的分工。

userspace 的应用接入方式、默认模式、CLI flags、IPC 编码，以及 native ABI 的异步执行器归属、精确签名和批次接口仍需工程／产品对齐。文档设计不自动修改当前 Controller API、Agent 源码、workspace 或运行服务。
