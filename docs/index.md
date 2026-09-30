---
layout: home

hero:
  name: Meshway
  text: 用身份连接节点，用策略定义路径。
  tagline: Rust 控制面与 Agent · 有序多跳 · 多传输 · 可验证的网络诊断
  image:
    src: /meshway.svg
    alt: Meshway 节点路径标志
  actions:
    - theme: brand
      text: 运行开发演示
      link: /getting-started
    - theme: alt
      text: 阅读架构设计
      link: /architecture

features:
  - title: 身份与域名上下文
    details: 面向节点身份、来源位置及域名 / CIDR 分类，区分节点发现 DNS 与互联网域名分流。
    link: /dns
    linkText: DNS 设计
  - title: 显式路径，多种承载
    details: 把有序中间节点、出口选择与相邻节点间的传输候选分开建模，让失败行为可解释。
    link: /routing
    linkText: 路径设计
  - title: 加密与真实追踪
    details: 优先端到端加密与真实数据面 mesh trace；信任模式的变化必须显式决定。
    link: /security-and-tracing
    linkText: 安全边界
---

## 当前阶段：控制面注册与持久化

Meshway 已实现 **单 controller SQLite 持久化、一次性注册令牌、Agent 持久凭据与心跳、Web 管理员登录及撤销**的纵向切片。Rust / SDK 行为测试、生成漂移检查，以及生产构建 Next.js + 真实 controller + Rust Agent 的浏览器端到端测试已通过。

**注册与心跳不等于 mesh 连通。** 本切片的身份是 HTTP 控制面凭据身份；mesh 公钥认证、E2E 握手、TUN、MagicDNS、有序转发、多传输与 `meshway trace` 仍待实现。最近联系时间不能作为在线或隧道健康状态。

文档区分 **已确认**的用户产品方向、**当前实现选择**、**建议默认**、**待决策**与有源码及验收依据的**已实现**。SQLite 与令牌协议属于本切片的实现选择，不表示用户逐项确认了所有协议细节。详见[注册与持久化](/enrollment)、[架构状态表](/architecture#设计状态)和[开发路线](/development)。
