# 插件与能力

本文的动态装载／ABI 是已确定的设计方向，尚未实现。当前 daemon 直接链接 Rust 组件与正式 stream providers；实际 crate 和运行边界见[总体结构](/v2/architecture#node-组件库)。规划中的 netstack 与 plugin host 不构成已安装能力。

## 交付与装载

首版动态装载 transport providers。control、dataplane、connectivity、underlay、TUN/netstack、DNS 和 diagnostics 保持独立职责的 Rust 库，由 daemon 链接；crate 边界不自动成为动态库边界。

首版发行包携带匹配版本的 daemon、CLI、全部正式 provider 和清单，整体安装／升级。daemon 启动时检查兼容性和依赖并加载；增加兼容 provider 不要求重新编译 daemon 主体，首版仍随完整发行包交付并重启启用。

独立插件更新、Server 分发可执行代码、运行中热卸载留待后续。配置启停 listener、重建连接或代际不等于卸载动态库。

必需 provider 缺失或不兼容时阻止 up；宿主／IPC 尚能运行则保留错误查询。可选能力缺失如实报告，不忽略其配置后宣称 applied。动态库的任务、回调、会话和 buffer 必须先退出／归还，再释放代码。

## 接口

`meshway-plugin` 定义共同 SDK；plugin host 处理装载／生命周期；首版承载接口由 `meshway-transport` 定义。其余组件使用库内的领域接口，后续增加插件种类时再定义对应 ABI。公共描述建议包含 ID、版本、ABI 范围、平台、依赖、capabilities 和 schema 引用，精确编码待定。

Stream／Datagram、发送所有权、背压、取消和连接复用的设计边界见[承载会话](/v2/node/transport)。

Config／State 类型继续由共享库拥有，不用通用 `execute(name, arbitrary JSON)` 替代合同。依赖环和无选择规则的重复 provider 在启动检查中失败；组件不自行加载任意同名库。

## Native ABI

建议采用版本化 native ABI。Rust trait object、`Vec`、`String`、Future 和 Tokio 内部值不直接跨动态库边界；Rust 类型依赖不是 ABI 稳定性保证。

需要一次明确的接口约定：

- 版本、结构尺寸、入口和兼容判断。
- 不透明实例／会话／资源句柄的归属与代际。
- buffer 的借用／转移／释放方，以及有界 batch／背压。
- 异步执行器归属、取消、完成、回调期限与 join。
- 错误和 panic 边界，禁止 Rust unwind 穿过 ABI。

loader／导出薄入口是受审 FFI 边界，业务实现保持安全约束。进程内插件按受信代码运行，不提供内存或崩溃隔离；窄 host services 用于表达归属，不声称能安全执行任意第三方代码。

## 数据面与 host services

高频数据面使用明确的 buffer／batch，不逐包 JSON 编解码、反射查找或 CLI IPC。控制消息可使用共享类型的版本化编码。比较性能时固定同一协议实现和负载，分开计量 ABI、加密、转发与包入口成本。

host services 提供有上下文的 socket、存储、任务、时钟、观测和必要材料接口。底层豁免不能由任意字符串指定用途；control 的 bootstrap 不依赖业务 DNS 已启动，TUN 捕获前先完成保护。

dataplane 保持共同身份／端到端保护；transport 执行协议封装，connectivity 管理实际邻接与 socket 尝试。provider 的底层资源需求、显式建连／协调 UDP 穿透的公共边界及 MASQUE 范围统一见[连接获取](/v2/node/connectivity)。连接的收发方向不能替代业务授权。

## Config／State 对齐

| 层 | 含义 |
| --- | --- |
| 安装清单 | 磁盘上的 provider／版本 |
| 装载结果 | ABI／依赖检查及失败原因 |
| Node capabilities | 本实例已加载的兼容实现支持哪些功能／协议角色 |
| Server 目录与 Config | 平台识别、允许、配置和可编译的功能 |
| 组件 State | 实际资源、安装、通道与错误 |

已加载不等于端口可绑定或功能已安装。协议能力继续使用 `transport:native_udp`、`transport:native_tcp`、`transport:vless`、`transport:trojan` 等现行 key；库名整理不自动重命名配置。标准发行必带 UDP/TCP 也不放宽 VLESS-only 限制。

新增未知协议仍需 Server 目录、schema 与编译支持。启动加载结果随新实例重新声明，旧能力、lease 和 link 不继承为当前有效证明。plugin State、userspace／TUN 能力键和 ABI 信息的精确字段尚待定；执行期限不由状态上报、插件重试或 CLI 保活延长。
