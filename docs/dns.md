# 身份 DNS 与域名上下文

**节点 DNS 解决 mesh 节点名称，互联网域名分流还需要额外的 DNS 关联机制。** 当前已有显式 loopback 权威节点 DNS 和隔离 Linux 容器的 resolver 实验接入；通用平台安装器、可配置 Split DNS 与互联网域名上下文仍是后续设计。

**已确认目标**：DNS 等设备业务配置全部由控制面管理 / 下发，下文 `--dns-listen` 与 resolver adapter 参数只描述现状 / 隔离验收，不是目标日常流程。出口路由方案与域名分流进入首版主线，具体域名上下文来源仍待确认。

## 当前可用：显式本地节点解析器

`/network` 保存 DNS 开关、zone 与 TTL，默认关闭、`meshway.test`、30 秒。启用配置后，修改现有 Agent 的启动参数，显式加入：

```sh
meshway-agent run --state /state/agent.json \
  --dns-listen 127.0.0.1:5353 --interval-seconds 2 --connection-lease-seconds 15
```

可以同时保留原来的 `--apply-dataplane` 与本地 profiles 参数。不要为同一身份启动第二个 Agent。监听限定非特权 loopback UDP/TCP；端口占用会明确失败，不会抢占其他服务或偷偷换端口。

```sh
dig @127.0.0.1 -p 5353 demo-sha.meshway.test A
dig @127.0.0.1 -p 5353 demo-sha.meshway.test AAAA +tcp
dig @127.0.0.1 -p 5353 -x <节点的实际 Mesh IPv4>
```

节点显示名能规范成唯一单标签时提供可读 alias；重名或无效名称仅保留 `id.<节点 ID 前32字符>.<后32字符>.<zone>`，不截断身份或随机挑选重名节点。AAAA 是地址元数据，不证明 IPv6 TUN 已可通信。

Agent 从生成的 `getAgentDns` 接口取得同 revision 的节点记录，校验网络 / 节点绑定、摘要及地址唯一归属，私有持久保存后才发布。磁盘缓存用于检查与防回退，不在离线启动时恢复旧解析权限。新鲜的禁用配置返回 REFUSED，不可用 / lease 过期返回 SERVFAIL；启用与关闭状态都受同样的 revision / lease 约束。没有递归转发。每个正响应 TTL 不超过剩余 lease，排队的发送也受到代际和截止时间约束。

Agent 的 nameservice 本身不修改系统 DNS、搜索域或默认上游。普通应用使用短名还需要显式 resolver 接入，不能用 `dig` 成功代替应用路径验收。

### 已验证的本机隔离验收

`experiments/managed-dns/run.py` 使用独立 controller、两个 keyless DNS-only 节点和一个同进程 TUN + DNS 节点。原生 ARM64 镜像 `sha256:83a725fbde106f34fef4f42be3f63af236fe5b218619b8226f7b9343385c0de5` 已通过完整验收，运行 ID 为 `mupehmrf-285d57c8e26c99ff`，公共结果文件包含 8 阶段证据，资源清理无错误。

验证覆盖真实 UDP/TCP、`dig`、A/AAAA/PTR、重复 alias 抑制、zone 更换、禁用/恢复、TTL 不超过 lease、控制面断联、带缓存离线重启、同数据库恢复和撤销。测试限定本机独立容器，没有修改宿主 DNS 或接入登记的远端主机。

## 容器内系统解析接入

M1 采用单独的实验 adapter，将新建 Linux 容器的 loopback `:53` 接到 Agent 的 `:5353`，并为容器自身添加搜索域。来源位于 `experiments/dns-resolver`；完整验收通过 `experiments/managed-dns/run.py --system-resolver` 运行。

- 节点 zone 按标签边界匹配，Mesh 地址池内的反向查询也固定走 nameservice；拒绝或失效的 Mesh 查询不尝试外部上游。
- 接管前的 nameserver 继续负责外部域名，controller 与 transport 的引导连接使用独立底层地址。启用期间只使用 Mesh 搜索域；原搜索域随原文件保留，在 down 时恢复，避免同名外部 search 结果接管节点短名。
- 搜索失败后的裸单标签查询在本地拒绝，避免短名落到同名底层服务；外部服务使用完整的多标签名称。
- 只有 UDP/TCP 都绑定成功后才修改 resolv.conf；私有 journal 保存原始内容和文件身份。正常退出、显式 down、崩溃后的恢复均检查所有权，文件被其他程序修改时报告冲突。
- 接入仅管理显式允许的容器文件；zone / 地址池为启动参数，变更后 down 再 apply。这不是通用 systemd-resolved 或跨平台安装器。

验收使用 glibc `getent`、真实 `curl` 响应与 Mesh IP / TUN 路由证明应用通路；Access 默认拒绝时，同样的名称可解析但 HTTP 访问失败。控制面失联后继续检查 lease 到期与内核拒绝路由，区分 DNS 可见性、解析器运行和业务授权。

2026-10-01 最终系统 resolver / DNS CLI 集成验收已通过：运行 ID `mupq9edz-37f8cdbf841d8edf`，共 12 个阶段、16 项系统接入检查；原生 ARM64 镜像 `meshway-manageddns:m1-20261001-b`，ID 为 `sha256:8d089b9bb1c67d3ef54d6b00acc90e6b32869e97516ebb8d158f79ed0c60df3a`，包含关闭状态 lease 修复与真实 DNS CLI。原 resolver 内容完整恢复，独立容器 / 网络清理无错误。结果文件还记录 TCP 占用 / 只读文件 apply 失败、SIGKILL 恢复、第三方编辑冲突、CLI 过期状态及 SERVFAIL 答复证据、daemon 退出后的显式 unreachable 路由。

## MagicDNS：从节点清单到本地解析器

推荐的数据流是：控制面分发有权限可见的节点清单，Agent 更新本地解析器，再通过各操作系统支持的方式接入系统 DNS。

| 能力 | 建议设计 | 边界 |
| --- | --- | --- |
| `A` / `AAAA` | 节点名称映射到 mesh IPv4 / IPv6 地址 | 已实现本地权威回答；AAAA 不等同 IPv6 数据面 |
| `PTR` | mesh 地址的反向节点名解析 | 已实现，只回答快照中实际分配的地址 |
| 搜索域 | 允许使用节点短名称 | 容器实验只安装当前 Mesh search，退出恢复原配置，避免外部同名接管 |
| Split DNS | 按域后缀分配 mesh / 企业 / 公网解析器 | 后缀按 DNS 标签匹配，避免误劫持 |
| OS 集成 | 安装、更新并恢复系统 DNS 配置 | 已验收受控 Linux 容器文件 adapter；平台 resolver 仍需分别实现和测试 |

当前后缀由网络管理员配置，重名 alias 被抑制，TTL 受新鲜配置 lease 限制；记录可见性与现有单网络公共 inventory 一致，不赋予业务访问权。跨组织可见性与 OS 集成仍需设计，不能仅修改一个通用 `resolv.conf` 就声称跨平台支持完成。

## 互联网域名路由需要什么

普通 IP 包只有地址，没有访问的域名。为了在数据面执行首版 suffix-only 规则，Agent 必须获取带有来源和有效期的域名上下文。本轮纯策略预览使用显式 `domain` 输入，不执行 DNS 查询；exact / prefix 已从首版范围移除。

可以评估以下输入机制，首版取舍未定：

| 机制 | 能提供什么 | 主要限制 |
| --- | --- | --- |
| 受控 DNS 查询与响应关联 | 查询名、真实地址、客户端、TTL 与策略版本的映射 | 一个 IP 可能属于多个域名；缓存和重绑定需要处理 |
| FakeIP | 为域名分配合成地址，连接时反查原域名 | 需要地址池、映射生命周期、例外与应用兼容性设计 |
| 显式代理输入 | 代理协议携带的目标主机名 | 只覆盖通过该代理的流量 |
| 协议嗅探 | 某些明文主机名或握手信息 | 不能作为完整、可靠的域名来源 |

**MagicDNS 不是 Clash 式 Fake-IP。** 给 `nyc` 解析一个 mesh 地址，并不会自动让 Agent 知道某个互联网 IP 对应哪个网站。

### 不能假定域名始终可见

- 应用自行使用 DoH / DoT 时，Agent 未必能获得 DNS 查询与连接的关联。
- ECH 会隐藏原本可能用于嗅探的握手主机名。
- 共享 IP、CDN、DNS 缓存和多个并发查询会让地址反查具有歧义。
- 直接连接 IP 的应用可能根本没有域名意图。

建议将真实域名上下文来源与过期 / 歧义信息保留；这些概念字段尚未进入 API。当前旧版纯预览在域名缺失时不匹配域名条件，真实 TUN 对可能改变路径的未知条件失败关闭。目标出口规则按方案有序匹配与兜底，不能继续用旧初始 reject 定义首版行为，也不能猜测域名后当作确定事实授权。

## DNS 路径与数据路径一致

以 NYC 经 LAX、从 SHA 出口访问某域名为例：在纽约解析得到的 CDN 地址可能不适合上海出口。需要让解析器选择与数据路径使用同一策略意图，并记录解析时采用的出口 / 区域及策略版本。

建议实现顺序：

1. 根据来源身份和查询域名确定解析策略；避免依赖尚未解析出的目标 IP 形成循环。
2. 选择与预期出口一致的 DNS 上游和到上游的可达路径。
3. 缓存域名关联、结果 TTL 与解析策略上下文。
4. 新流建立时执行最终分类和授权；策略变更或上下文过期后重新求值。

“DNS 经 SHA”与“业务从 SHA 出口”是两个需协调的动作，并非配置其中一个就自动成立。

### Bootstrap 与防回环

控制面地址、认证服务、传输对端、DNS 上游本身也可能需要解析。Agent 必须先有可用的引导解析与底层连接，才能安装依赖隧道的 DNS / 路由。

建议为控制 socket、传输 socket 和必要的 bootstrap DNS 设置明确的底层出口或平台 socket 标记，防止它们再次进入自身 TUN。网络切换和重新连接时也要验证这一点。

## 地理集合不能混用

以下集合需要独立定义、维护和版本化：

- 中国相关域名集合：业务或数据集定义的域名分类。
- 中国 IP 集合：地址段及其地理数据来源。
- `.cn`：DNS 顶级域后缀。

三者既不等价，也不能相互完整推导。规则与诊断应展示实际使用的集合及版本，避免把地理标签表现为身份事实。
