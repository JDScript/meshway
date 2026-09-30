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
    details: 面向节点身份、来源国家及域名后缀 / 双栈 CIDR 分类，区分节点发现 DNS 与互联网域名分流。
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

## 当前阶段：从持久身份到配置与路由意图

Meshway 已实现 **单 controller SQLite 持久化、一次性注册令牌、Agent 持久凭据与心跳、Web 管理员登录及撤销**的纵向切片。Rust / SDK 行为测试、生成漂移检查，以及生产构建 Next.js + 真实 controller + Rust Agent 的浏览器端到端测试已通过。

**配置与路由控制意图切片也已实现并验证：** 持久双栈 mesh IPAM、保留现有数据的开发数据库增量升级、带摘要的版本化配置与私有缓存 / ACK、纯 Rust 策略编译，以及 `/routing` 引导式编辑 / 预览 / 条件保存。详见[地址与配置分发](/configuration)。

首版[路由语义](/routing)已修订为 **域名 suffix-only、first-match、可配置 direct / reject 默认动作（初始 reject）**。国家为管理员元数据，互联网目的国家是预览提示；没有自动 DNS / GeoIP 查询。

**注册、地址分配与配置回执不等于 mesh 连通。** 当前身份仍是 HTTP 控制面 bearer 身份；mesh 公钥认证、E2E 握手、TUN、MagicDNS、有序转发、多传输与 `meshway trace` 仍待实现。逻辑预览不是实测路径，最近联系时间不是隧道健康状态。

本轮已通过独立审查修复后的 policy 26 项与受影响核心 30 项测试、SDK 5 项测试、完整 TypeScript 检查及 2 项真实服务 Playwright 测试。测试分组、文件发布失败语义和覆盖范围见[开发路线](/development#配置与路由切片)。里程碑 1 的控制意图部分已完成，mesh 公钥身份、ACL 与网络数据面仍待实现。
