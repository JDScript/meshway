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

已交付多网络 SQLite、用户账户/邀请/网络 RBAC/设备归属、App PKCE、IPAM/revision/CAS/ACK、平台托管 Native UDP/TCP 与 VLESS TCP/TLS、WebSocket/TLS、Linux Mesh IPv4 正常执行和 Noise E2E、节点 DNS/daemon 诊断与真实邻接历史。固定同身份直连/四地有序路径恢复复验 PASS，边界见[Linux 验收](/linux-field-validation)。

**仍待交付**：完整出口/域名分流、首包按需建连、真实打洞、默认 STUN/GeoIP 与 CLI 链接登录。单一 policy、状态型 Mesh 回包及自动邻接/主备已有正常 Linux 包流验收；固定底层可达的证据不能证明公网穿透，原跨地域链路还含既有 WARP。

IPv6 地址和 AAAA 不代表 IPv6 TUN；Apple App 接入 / 库存不代表 App 隧道。配置 ACK 是持久保存回执，心跳是联系记录，规划路径不是实测 trace。实现边界见[架构](/architecture)，首版闭环和依赖见[里程碑](/milestones)。

## 阅读入口

- **正在对齐**：[配置参考](/configuration-reference)按字段分页面，另有[State](/state-reference)、[同步与回滚](/configuration-reference/synchronization)、[存储／查询 API](/configuration-reference/storage-api)；新增格式为设计草案。
- **产品**：[概念与流程](/concepts) → [网络](/network-management)、[Access](/access-policy)、[路由方案](/routing)；决策依据为[产品对齐](/product-alignment)。
- **架构**：[总览](/architecture) → [控制面身份](/enrollment)、[配置](/configuration)、[Agent](/connection-runtime)、[数据面](/managed-dataplane)。
- **开发**：[本地联调](/getting-started) → [统一入口](/dev-services)、[检查与验收](/development)、[实验](/transport-lab)。
- **参考**：[CLI](/cli)、[协议 / API](/protocol)、[Tailscale 对照](/tailscale-alignment)、[参考源码](/references)。
