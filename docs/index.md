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

## 当前阶段：基础契约与本地演示

Meshway 正在建立 **Rust controller / agent、生成客户端与 Next.js 管理界面**的第一个纵向切片：Agent 报告进程信息，控制面提供概览与列表，Web 读取真实 API。

**这还不是可转发流量的 mesh。** 节点认证、TUN、MagicDNS、有序转发、多传输、端到端加密与 `meshway trace` 都属于后续网络实现。进程报告记录不等于已认证、已入网或在线的节点。

本文档统一使用四种状态：**已确认**是用户明确的产品方向；**建议默认**是尚可调整的设计；**待决策**尚未冻结；**已实现**必须以当前代码和验收结果为依据。详见[架构状态表](/architecture#设计状态)和[开发路线](/development)。
