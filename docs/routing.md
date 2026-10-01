# Mesh 内路由与出口路由方案

**已确认模型将 Mesh 内路由和出口路由分开。** 路由不授予访问权限；自动选路、主备与出口选择均受连接图和 grants 约束。模型原文见[产品对齐](/product-alignment)，简要说明见[概念](/concepts)。

## 已确认：Mesh 内路由

一张网络的一套规则，对所有设备相同，从上到下、命中第一条生效：

- 无规则时平台自动选路；可指定必须依次经过的节点。
- 编译时提示完全被前序规则遮挡的规则，以及部分抢先的具体流量示例；提示依赖当前节点 / 标签库存，供管理员调整顺序。
- 必经节点是有序路径，不是候选集合。节点失效时不能偷偷删除 hop 或直连终点。
- 建连方向与业务方向独立；连接主备只换同一邻接的承载，路径变更是不同操作。

## 已确认：出口路由方案

出口覆盖所有离开 Mesh 的流量，包括互联网与目标节点的局域网。管理员在 Web 开启“可作为出口”，转发 / NAT 由 Agent 按下发完成。

设备生效规则 = **公共规则 + 当前方案的有序规则 + 方案兜底**。公共规则始终在前、所有方案都生效；其后有序 first-match 匹配域名后缀、IP 段或规则集，目标为候选出口（主备）或 `direct`，可附有序出口前中转。兜底为出口 / `direct` / 不接管。

用户从允许范围切换方案，管理员可锁定：

| 内置方案 | 行为 |
| --- | --- |
| 仅 Mesh | 只有公共规则 |
| 全局经 X | 每个允许出口自动生成，等同选择 exit node |
| 自动 | 按 STUN 观测的设备公网 IP 所属国家 / 地区选择方案，无法判断使用兜底方案 |

自动不依据控制面连接来源 IP。方案切换是使用选择，切换会中断经旧出口的存量连接。方案分配须检查 grants；含未授权出口 / 网段时保存报错。

例如「CN」方案 `openai.com → 经 hk → 出口 sg`，「US」方案 `openai.com → direct`；「自动」按设备地区选择。地区选择不改变网络内授权，也不需要每个位置维护一套连接 / Mesh 路由。

## 匹配与规则集

### 系统保护通路

控制面通信、STUN/认证建连探测、transport 外层 socket 与它们必要的 bootstrap DNS，
使用独立于 Meshway 自身 TUN 的底层通路。此项在业务规则之前由系统处理，不是用户
必须手工放在第一条的 `direct` 规则，也不随 routing profile 切换。

豁免按 Agent 内部 socket 用途识别（Linux 可用 socket mark 与策略路由，其他平台用
等效机制），不能把整个 Agent 进程或 controller / peer 的 IP 全部放行。普通应用访问
同一 IP、同一 CDN 上的其他域名仍按公共规则、方案规则和兜底处理。transport 只把
外层包交给当前合法下一跳；业务内层依然经过已授权的完整路径。

换网、DNS 重解析或 peer endpoint 变化时，先准备新底层通路，再启用新 socket 和
相关观测；控制面断连或观测过期时不拿隧道出口地址补齐地区。这里的底层网络是运行
环境原有的网络路径，不自动绕过其他系统 VPN 或企业网络策略。

P1c 原型的固定 /32 例外仍会影响同地址的普通业务；它已记录为原型限制，正式集成须
改为 socket 级保护，见[出口数据面](/exit-dataplane)。

### 业务匹配

域名先做 IDNA、ASCII 小写与末尾点规范化，再按 DNS 标签边界匹配 suffix：`example.com` 命中根域与 `a.example.com`，不命中 `badexample.com`。首版不支持 exact、keyword 或 regex；URL 与字面 IP 不能伪装成域名。

IP 使用规范化 CIDR。规则集由 controller 下载、管理、更新并编译下发，设备不下载数据集；参考 sing-box rule-set / mihomo rule-providers。首版仅域名后缀和 IP 段，无法精确转换的条目须报告。控制面把有序规则编译为等价不重叠前缀表，提示完全被前序规则遮蔽的规则，不采用最长匹配替代 first-match。

TUN 包没有域名；DNS 关联、Fake-IP 与嗅探的取舍仍待数据面设计确认，见[域名上下文](/dns#互联网域名路由需要什么)。`.cn`、国内域名集合和国内 IP 集合不是同一个输入。

## 当前实现与迁移边界

当前 `crates/protocol/src/routing.rs` 与 `crates/policy` 仍实现旧版 `RoutingPolicy`：Mesh / internet / any 混合、有序 first-match、`default_action` direct/reject、来源管理员国家元数据，以及 `via` / `exit_node_id`。旧初始空策略 reject 已被目标模型废止；这里的字段只描述现状与迁移输入，不是目标四层 schema。

旧版纯预览已贯通 controller、生成 SDK、Agent 缓存和 Web 条件保存。托管 TUN 只执行已注册节点间的 Mesh IPv4 路径，使用独立 Access 与路径绑定 E2E；互联网出口、路由方案、规则集和可信域名上下文未由该切片接通，IPv6 CIDR 预览也不证明 IPv6 数据面。

新版独立实现见 [v3 策略引擎](/policy-v3) 与 [出口数据面原型](/exit-dataplane)。两者已合入，正式下发与旧配置迁移仍在后续集成阶段。

迁移须无损保留旧策略行为；混合 scope、国家条件或拒绝语义不能等价表达时须明确报告，不静默改变授权或删除必经节点。

### 当前预览与保存约束

- 明确节点目标必须有效；同时提供 IP / 国家时须与清单一致。字面 Mesh IP 解析为相应节点，池内无归属地址拒绝，不退化为互联网。
- IPv4-mapped IPv6 在分类前为 `invalid`，不隐式转换绕过拒绝；预览不发 DNS / GeoIP 查询。
- 结果区分 route、reject、invalid；有效拒绝不是“网络已连通”，规划路径不是实测 trace。
- 保存用 `expected_revision` CAS：无效策略 `422`、过期基准 `409`；预览不写库、不推进草稿基准。图形 / JSONC 共用草稿，见[Access](/access-policy#标签与草稿)。
- 撤销使引用非法时，保留源策略和诊断，分发整体拒绝的有效快照，不跳过坏规则落入 direct。

当前规则 / selector / 路径容量见[配置分发](/configuration#容量边界)，测试入口见[开发与验收](/development#配置与路由切片)。真实 ICMP / HTTP 与撤回证据见[托管数据面](/managed-dataplane)，真实逐跳追踪仍见[安全与 trace](/security-and-tracing)。
