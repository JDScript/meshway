---
layout: home

hero:
  name: Meshway
  text: 以身份互联，以策略选择路径。
  tagline: Tailscale 产品基线 · 自定义 transport · 有序指定路径 · Rust 控制面与 Agent
  image:
    src: /meshway.svg
    alt: Meshway 节点路径标志
  actions:
    - theme: brand
      text: 了解产品与概念
      link: /concepts
    - theme: alt
      text: 运行本地联调
      link: /getting-started

features:
  - title: 网络、用户与设备
    details: 目标为多网络隔离、仅邀请账户与统一节点；设备一次只接入一张网络，配置由平台管理。
    link: /concepts
    linkText: 已确认目标与当前差距
  - title: 四层策略与路由方案
    details: 连接图、叠加 grants、Mesh 内必经路径、出口有序方案各司其职；用户在允许范围内选择方案。
    link: /routing
    linkText: 路由模型
  - title: 自定义承载与 E2E
    details: 将承载主备与逻辑路径分开，连接方向独立于业务方向，中间节点转发端到端密文。
    link: /transports
    linkText: 承载与插件边界
---

## 当前实现

已交付单网络 SQLite、管理员 bearer / 一次性节点接入、App 浏览器 PKCE 兑换、IPAM、版本化配置与 ACK、公钥认证邻接、Linux Mesh IPv4 TUN 与 Noise E2E，以及节点 DNS 和只读诊断。真实 ICMP / HTTP、反向 TLS-VLESS、授权 / 必经节点 / lease 撤回均有隔离验收，入口见[开发与验收](/development)。

**已确认但仍待交付**：多网络真实隔离、本地邮箱密码 / 仅邀请用户体系、平台托管全部配置、grants 与四层模型迁移、按需自动建连、出口 / 路由方案 / 规则集与可信域名上下文。旧 first-match Access、混合 Routing 和本地 profiles 仅是迁移前现状，不能当作目标流程。

IPv6 地址和 AAAA 不代表 IPv6 TUN；Apple App 接入 / 库存不代表 App 隧道。配置 ACK 是持久保存回执，心跳是联系记录，规划路径不是实测 trace。实现边界见[架构](/architecture)，首版闭环和依赖见[里程碑](/milestones)。

## 阅读入口

- **产品**：[概念与流程](/concepts) → [网络](/network-management)、[Access](/access-policy)、[路由方案](/routing)；决策依据为[产品对齐](/product-alignment)。
- **架构**：[总览](/architecture) → [控制面身份](/enrollment)、[配置](/configuration)、[Agent](/connection-runtime)、[数据面](/managed-dataplane)。
- **开发**：[本地联调](/getting-started) → [统一入口](/dev-services)、[检查与验收](/development)、[实验](/transport-lab)。
- **参考**：[CLI](/cli)、[协议 / API](/protocol)、[Tailscale 对照](/tailscale-alignment)、[参考源码](/references)。
