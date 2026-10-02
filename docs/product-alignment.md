# 已确认产品要求

本页保留稳定决策与边界；[概念](/concepts)说明四层模型和流程，[当前状态](/milestones)单独说明实现。常规产品以Tailscale为基线，差异是可插拔承载和域名/IP的有序指定路径；需要偏离时说明原因，不把研究上游wire合同直接当作Meshway合同。

## 网络、账户与节点

- 一个controller托管多张隔离网络，成员、IPAM、DNS、Access、路由/配置各自作用域；网络Owner不获跨网权限。一设备可保存多接入，同一时间只启用一网。
- 首版本地邮箱密码、安全哈希、仅邀请注册入网，不发邮件，链接手动转交。日常用户登录，Owner/Admin/Member是网络角色；实例凭据只用于初始化/恢复及必要兼容。OIDC后续。
- Web/App/CLI共用controller用户与浏览器会话；设备只取得一次性接入结果，不收集密码或持有人类/管理员凭据。用户设备归属membership；服务器可用预分配tag接入密钥。
- 统一节点模型，不按服务器Agent/App预设中继资格，不增加产品中继角色开关；路径/任务、实际能力、授权与可达性决定职责。出口资格须管理员在Web显式开启，Agent按下发完成转发/NAT。

## 平台托管与承载

- listener、承载/凭据、DNS、出口及业务策略由Web管理、Controller下发。设备只保留接入/登录和允许范围内路由方案选择；profiles/`--dns-listen`/`--stun-server`是待收敛兼容现状，不新增本地配置流程。
- 添加节点必须有可选listener入口，之后可在节点配置修改；bind、公告endpoint、TLS/SNI与WebSocket Host/path分别建模。平台生成默认材料，允许Web显式覆盖。
- 默认按授权流量/路径/可达性按需建连，手工连接约束是高级入口，不先要求逐对配候选。承载按平台优先级主备，不交付首版按流负载分担/同流聚合。
- VLESS/WS、Shadowsocks等用于Meshway内部承载，不做普通代理客户端账户/订阅服务。代理UUID/password不替代Meshway节点身份。
- 承载变化与逻辑路径变化分开，反向dial与业务方向分开；双向通道不等于双向访问许可。连接失败不能降低E2E或绕过必经点。
- NAT协调接口transport-neutral，UDP/TCP打洞、反向拨号、中继分别表达。VLESS TCP/TLS与WS/TLS外层为TCP，UDP映射不能推广为TCP可达；仅允许VLESS不静默启用UDP业务。TCP打洞范围须明确，不能因工程优先UDP自动排除。

## 四层策略与出口方案

连接allow/block和Access grants顺序无关；Mesh内路由与出口路由都是从上到下first-match，不用具体度覆盖。连接block优先；grants取并集、未匹配拒绝，新网络模板成员全部允许。Mesh路由可指定有序必经节点，无规则自动选路；命中路径失败不回退绕过，编译提示完全/部分遮挡和示例流量。

Mesh外规则=公共规则（始终最前）+当前方案有序规则+兜底（出口/direct/不接管）。匹配DNS标签边界suffix、IP段、规则集，支持出口候选主备和出口前中转。内置仅Mesh、全局经X、自动；用户在允许范围切换，管理员可锁定，切换中断旧出口存量连接。

- 使用出口/外部目的地须grants，含未授权目标的方案分配保存报错；自动引用方案也不得扩权限。
- 出口覆盖互联网与节点所在局域网，不另设subnet-router产品分类；“互联网”仅公网，排除RFC1918、ULA、CGNAT和链路本地，局域网须单独授权具体段。
- 规则集由controller下载/管理/更新/编译，设备不下载；首版只精确支持域名后缀/IP段，无法转换条目须报告。设备安装等价不重叠前缀表。
- 旧初始routing reject废止；已有策略无损迁移或明确报告不能等价转换，不自动放宽/改身份。

## 观测、安全与生命周期

listener/拨号socket分别展示bind、外部候选、来源、验证、期限与代际；STUN、打洞与UPnP/NAT-PMP/PCP显式映射分开，UPnP范围未因此确认。自动方案取有效底层公网IP→Controller GeoIP，保留来源/时间/数据版本，不用controller来源IP、MeshIP、出口IP或管理员country补位置；缺测/过期/冲突走兜底。

系统保护按内部socket用途豁免controller/probe/carrier/bootstrap DNS，不按进程/目标全局放行；先准备保护再捕获/换通道，双栈一致。业务配置/路径/公钥同revision，lease/撤回不等待HTTP，TUN停止保留Mesh sink。ACK、心跳、规划路径、实际业务分别表达。

真实邻接测量与端到端路径测量分开；缺测/超时不填零，重复心跳/读取不造样本或丢包率。地图只用有来源位置与新鲜连接证据，无位置明确待定位，动效不造流量/健康。

## 当前执行优先级

Linux真实组网、用途保护、自动建连/打洞与跨地域验证优先；Apple新增开发暂停，已有接口与必要消费者兼容保留。`apps/console` pure SPA由用户另一工作流维护；后端不主动UI重做。固定链路与恢复已验，第二轮仍须两端仅出站的真实穿透/中继边界，不能把NYC防火墙或WARP路径省略。[跨地域验收](/linux-field-validation)保留事实与证据。

域名上下文的实际来源（DNS映射/Fake-IP/嗅探）仍需实施阶段比较确认；完整出网与Mesh私网同属首版，不以后端技术里程碑将出口推迟为附属目标。参考：[Tailscale tailnet](https://tailscale.com/docs/concepts/tailnet)、[grants](https://tailscale.com/kb/1324/grants)、[exit nodes](https://tailscale.com/kb/1103/exit-nodes)、[sing-box rule-set](https://sing-box.sagernet.org/configuration/rule-set/)、[mihomo rule-providers](https://wiki.metacubex.one/en/config/rule-providers/)。
