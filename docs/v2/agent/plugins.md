# 插件、装载与能力

**已确认：启动时动态加载，首版全部正式插件随包交付。** 本页给出模块边界；native ABI、清单字段和能力扩展编码尚待实现。

## 首版装载与更新

一个发行包包含匹配版本的 `meshwayd`、`meshway`、插件及清单。安装、版本校验和升级以整个发行包为单位；插件目录不是 Controller 配置中的任意下载 URL。

启动时，宿主发现受信发行清单中的 provider，检查兼容性和依赖后加载。增加兼容 provider 可以不重新编译 daemon 主体；本轮仍由完整发行包携带并在重启后启用。逐插件更新、Controller 远程安装代码和运行中热卸载均留待后续设计。

必需 provider 是节点执行的依赖。缺失或不兼容时应明确阻止 up；建议在宿主／IPC 仍可运行时保留本机错误查询。可选 provider 不可用时报告缺失能力，不能将其配置静默忽略后报告整份 applied。

代码装载与运行资源生命周期分开：运行中仍可按配置启停 listener、创建／回收连接和重建某一运行代际。这些操作不要求卸载对应动态库。动态库保持加载，直到其所有任务、回调、会话和拥有的 buffer 已退出或归还。

## 契约与宿主

| 层 | 职责 |
| --- | --- |
| `meshway-plugin` | 插件 SDK、描述／生命周期合同、host services 的共同抽象 |
| plugin host | 清单、版本／ABI 检查、依赖解析、加载句柄、调用与退出管理 |
| 领域接口 | transport、包入口、DNS、控制同步、诊断等各自的强类型能力 |
| provider | 实现某个领域接口，持有其运行资源，报告真实结果 |

插件 API 不使用一个通用 `execute(name, arbitrary JSON)` 代替领域合同。Config／State 的类型继续由共享模型拥有；HTTP／SSE 从 Rust 路由生成，IPC 和插件接口分别有明确版本，不复制三套 Node DTO。

描述清单建议包含：plugin ID、实现版本、host ABI 范围、目标平台、依赖服务、能力声明及配置／状态 schema 引用。字段编码待定。一个 capability 不能由多个无选择规则的 provider 同时抢占；标准包应给出确定映射，依赖环和不相容接口在执行前失败。

## Native ABI

首个实现方案建议采用版本化 native ABI，Linux 以动态库交付。Rust 内部 trait 可以用于静态类型检查，但 Rust ABI、trait object、`Vec`、`String`、Future 和 Tokio 内部值不直接跨动态库边界。

ABI 定版须明确：

- 入口与接口版本、结构尺寸及能力协商；不相容时拒绝加载。
- 不透明实例／会话／资源句柄的归属与代际。
- buffer 借用、转移、归还和释放方；同一分配的释放必须回到约定 owner。
- 有界异步操作、取消、完成及等待退出；不把取消请求当作已无回调。
- 错误／panic 边界、回调期限及线程约束；不让 Rust unwind 穿过 ABI。

加载和导出适配属于受审 native/FFI 边界，安全业务实现继续保留禁止 unsafe 的边界。具体桥接方案需要独立验证；本轮不据文档放开应用 crate 的 unsafe 或直接传递 Rust 内部布局。

进程内 native 插件按受信代码运行，不提供进程隔离或内存沙箱。窄 host services 用于清楚表达归属和减少误用，不应被宣传成可安全执行任意第三方代码的权限系统。

## 数据路径与性能

承载接口从开始就包含批量处理、buffer 所有权和背压。数据包不逐包做 JSON 编解码、反射式插件查找、全局锁调度或 CLI IPC。

控制消息可使用由共享类型生成的版本化编码；高频数据面使用明确的 buffer／batch 接口。队列、待处理请求和在途 buffer 有预算，关闭等待归还；plugin host 不把所有插件调用集中到一个会阻塞无关流的单线程业务队列。

性能验证对照同一协议实现、资源和负载，分别衡量 native ABI／适配、认证／加密、转发、TUN／用户态栈的成本。不能用普通 L4 代理和 L3 E2E 隧道的差值声称插件包装损耗。

## Transport 与连接方法

`meshway-transport` 定义承载数据接口。UDP、TCP、VLESS、Trojan 是 provider；TLS／WebSocket 参数按协议合同执行。身份认证和包的端到端保护仍由共同的数据面实现，不在每个协议重复建立一套 Meshway 授权。

connectivity 通过 transport 与 underlay 的明确扩展接口使用实际 socket。普通拨号、反向拨号、UDP 打洞、TCP simultaneous-open 分别声明；不能把 STUN 观测推广成 TCP/VLESS 可达，也不能因为 native UDP 插件存在就绕过禁止 UDP 的业务配置。

已有 UDP NAT／主备／出口代码和失败反例可用于实现审查。TCP/VLESS 外层打洞尚未完成；支持 TCP 业务包、TCP relay 或 VLESS provider 不等于支持 TCP 打洞。

## Config／State／capabilities 对齐

| 来源 | 表达什么 |
| --- | --- |
| 发行／安装清单 | 磁盘上有哪些 provider 和版本 |
| 装载结果 | 哪些 provider 通过 ABI／依赖检查，哪些失败及原因 |
| Node capabilities | 当前实例已加载的兼容实现支持哪些功能／协议角色 |
| Controller 目录与配置 | 平台认识、允许、配置并可编译哪些功能 |
| 组件 State | 实际 listener、socket、通道、栈、安装结果与错误 |

插件已安装不等于已加载，已加载不等于已有 listener，支持某功能也不等于具备当前系统权限或资源已准备好。端口占用、权限不足等是运行条件／资源错误，不能伪称功能已经 applied。

capability key 继续采用 `transport:xxx`、`feature:xxx` 等命名空间；crate 名不是 capability key。UDP/TCP 的标准包必带要求也不等于它们在所有网络都被允许。两端能力、平台允许的协议、路径与当前资源条件共同决定实际执行。

例如建议的 `meshway-transport-udp`／`meshway-transport-tcp` provider 仍对应现行 `transport:native_udp`／`transport:native_tcp` 能力名，VLESS／Trojan 对应 `transport:vless`／`transport:trojan`；库的命名整理不自动重命名配置和现行 HTTP 字段。

Controller 当前目录、配置联合和 SDK 是强类型的。新增 provider 若实现已知协议，可以沿用已知配置合同；新增未知协议仍须更新平台目录／schema／编译支持，不能靠 daemon 动态加载就绕过这些边界。

启动加载的结果用于该 Node 实例的能力声明，沿现有持钥接入／acquire 合同校验。升级重启产生新实例，旧 capability 证明、lease 和 link 不继承为当前有效状态。新的 plugin State、userspace／TUN 能力键及宿主 ABI 信息的精确字段仍是后续合同设计，不直接修改现行 NodeState。

## Host services

插件按职责取得窄接口，例如受保护 socket、私有存储、任务／取消、时钟、观测发布及所需材料。调用绑定当前运行上下文，不通过随意填写一个字符串就获得 controller/bootstrap 的底层豁免。

control 使用必要的 bootstrap 解析和保护通路，不依赖业务 DNS 已启动；业务 DNS 再按下发规则运行。TUN 捕获前须完成保护准备。普通业务请求即使访问同一个 Controller 地址，也继续受其业务规则约束。

宿主持有执行上下文的有效期与撤回入口，组件持有资源的实际生命周期。状态上报、CLI 保活、插件内部重试都不续期执行许可。插件失败后，已失效资格不能通过重建资源或重放缓存重新启用。
