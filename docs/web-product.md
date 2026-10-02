# Console 产品与合同边界

`apps/web` 已deprecated，由独立工作流的 `apps/console` pure SPA替代。当前后端提供实际Rust路由、生成SDK/运行时schema和网络内权限；不将旧Web页面或候选设计当成新console已完成。

日常使用controller本地用户账户。Owner/Admin/Member是网络内角色，实例bootstrap仅用于专门初始化/恢复；合法NetworkId不授予权限。会话、浏览器接入与跨标签页缓存边界见[SPA合同](/spa-api-audit)和[本地账户](/local-accounts)。

已确认产品以地图概览为视觉中心，侧边栏承载节点、成员、访问控制、路由等管理。只显示有来源的地区、真实连接与探测证据，缺测/过期显示未知，节点联系或配置ACK不伪称隧道健康。listener、承载和socket信息归节点配置，设备不手工维护日常配置。

策略编辑使用唯一NetworkPolicy：连接/授权是无序权限层，Mesh/出口是有序first-match路由层。GET `.../policy`、POST `.../policy/save`使用同一源和revision CAS；JSONC与图形视图共享逐网络草稿，切换不丢注释、无效文本或未保存修改。新网络已有默认policy，无迁移/激活按钮。当前完整出口/自动方案尚未交付，保存入口拒绝未实现语义。

管理请求显式网络作用域，Controller每次/事务检查真实会话和当前角色。错误、CAS冲突、观测新鲜度、缺失与null以生成合同为准。UI由用户手工验证，不保留浏览器自动化套件；后端协议/鉴权/数据面采用必要非浏览器行为测试。
