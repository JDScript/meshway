import { defineConfig, type DefaultTheme } from "vitepress";

const currentSidebar: DefaultTheme.SidebarItem[] = [
  {
    text: "产品",
    items: [
      { text: "概览、概念与使用流程", link: "/concepts" },
      { text: "网络管理", link: "/network-management" },
      { text: "访问授权（Access）", link: "/access-policy" },
      { text: "Mesh 路由与出口方案", link: "/routing" },
      { text: "控制台", link: "/web-product" },
      { text: "Apple App", link: "/apple-app" },
      { text: "产品要求", link: "/product-alignment" },
      { text: "当前状态与剩余工作", link: "/milestones" },
    ],
  },
  {
    text: "架构",
    items: [
      { text: "总览与实现边界", link: "/architecture" },
      { text: "控制面注册与持久化", link: "/enrollment" },
      { text: "控制面发布与预算", link: "/controller-publication" },
      { text: "浏览器登录与节点接入", link: "/device-login" },
      { text: "本地账户领域与 SSO 接线", link: "/local-accounts" },
      { text: "地址与配置分发", link: "/configuration" },
      { text: "公钥与连接意图", link: "/connections" },
      { text: "Agent 连接运行层", link: "/connection-runtime" },
      { text: "托管 Mesh IPv4 数据面", link: "/managed-dataplane" },
      { text: "Mesh 执行合同", link: "/mesh-execution" },
      { text: "E2E 重启恢复", link: "/e2e-session-recovery" },
      { text: "四层策略引擎", link: "/policy" },
      { text: "出口数据面 RFC 与原型", link: "/exit-dataplane" },
      { text: "系统 socket 保护通路", link: "/underlay-routing" },
      { text: "DNS 与域名上下文", link: "/dns" },
      { text: "Transport、插件与主备", link: "/transports" },
      { text: "托管 listener 与材料", link: "/managed-transports-api" },
      { text: "承载目录与协商边界", link: "/transport-registry" },
      { text: "邻接延迟历史", link: "/latency-history" },
      { text: "加密边界与真实追踪", link: "/security-and-tracing" },
    ],
  },
  {
    text: "开发",
    items: [
      { text: "本地联调", link: "/getting-started" },
      { text: "统一开发入口", link: "/dev-services" },
      { text: "环境、检查与验收", link: "/development" },
      { text: "Linux 传输与性能实验", link: "/transport-lab" },
      { text: "Linux 跨地域验收", link: "/linux-field-validation" },
    ],
  },
  {
    text: "参考",
    items: [
      { text: "CLI 与手动验证", link: "/cli" },
      { text: "协议与类型安全", link: "/protocol" },
      { text: "模型与 schema 按需导出", link: "/model-api-audit" },
      { text: "SPA 会话、查询与轮询", link: "/spa-api-audit" },
      { text: "Netcheck 底层观测", link: "/netcheck" },
      { text: "真实承载 endpoint 观测", link: "/endpoint-discovery" },
      { text: "Tailscale 对照", link: "/tailscale-alignment" },
      { text: "参考源码", link: "/references" },
    ],
  },
];

const v2Sidebar: DefaultTheme.SidebarItem[] = [
  {
    text: "Meshway V2 · 设计中",
    items: [
      { text: "概览", link: "/v2/" },
      { text: "配置与状态模型", link: "/v2/network-configuration" },
      { text: "数据模型与权威来源", link: "/v2/entities" },
      { text: "操作权限", link: "/v2/permissions" },
      { text: "按需执行许可", link: "/v2/execution" },
    ],
  },
  {
    text: "Configuration",
    items: [
      { text: "结构与字段", link: "/v2/configuration-reference" },
      { text: "整网草稿与应用对照", link: "/v2/configuration-reference/editing" },
      { text: "Addressing · 地址与分配", link: "/v2/configuration-reference/addressing" },
      { text: "Defaults", link: "/v2/configuration-reference/defaults" },
      { text: "字段复用与默认继承", link: "/v2/configuration-reference/inheritance" },
      { text: "Groups / Tags", link: "/v2/configuration-reference/identity" },
      { text: "Access · 访问权限", link: "/v2/configuration-reference/access" },
      { text: "Topology · 组网规则", link: "/v2/configuration-reference/topology" },
      { text: "Routing · 流量路由", link: "/v2/configuration-reference/routing" },
      { text: "Nodes", link: "/v2/configuration-reference/nodes" },
      { text: "Transport 与支持状态", link: "/v2/configuration-reference/transports" },
      {
        text: "通用字段",
        collapsed: false,
        items: [
          { text: "Listen Fields", link: "/v2/configuration-reference/shared/listen" },
          { text: "Dial Fields", link: "/v2/configuration-reference/shared/dial" },
          { text: "TLS", link: "/v2/configuration-reference/shared/tls" },
          { text: "WebSocket", link: "/v2/configuration-reference/shared/websocket" },
        ],
      },
      {
        text: "Listeners",
        link: "/v2/configuration-reference/listeners",
        collapsed: false,
        items: [
          { text: "Native UDP", link: "/v2/configuration-reference/listeners/native-udp" },
          { text: "Native TCP", link: "/v2/configuration-reference/listeners/native-tcp" },
          { text: "VLESS", link: "/v2/configuration-reference/listeners/vless" },
          { text: "Trojan", link: "/v2/configuration-reference/listeners/trojan" },
        ],
      },
      {
        text: "Dialers",
        link: "/v2/configuration-reference/dialers",
        collapsed: false,
        items: [
          { text: "Native UDP", link: "/v2/configuration-reference/dialers/native-udp" },
          { text: "Native TCP", link: "/v2/configuration-reference/dialers/native-tcp" },
          { text: "VLESS", link: "/v2/configuration-reference/dialers/vless" },
          { text: "Trojan", link: "/v2/configuration-reference/dialers/trojan" },
        ],
      },
      { text: "DNS", link: "/v2/configuration-reference/dns" },
      { text: "Rule Sets", link: "/v2/configuration-reference/rule-sets" },
    ],
  },
  {
    text: "State",
    items: [
      { text: "State Reference", link: "/v2/state-reference" },
      { text: "NodeState", link: "/v2/state-reference/node" },
      { text: "Capabilities", link: "/v2/state-reference/capabilities" },
      { text: "Resources / Links", link: "/v2/state-reference/resources" },
      { text: "Endpoint Candidates", link: "/v2/state-reference/endpoints" },
      { text: "GeoIP 与 ASN", link: "/v2/state-reference/ip-info" },
      { text: "按需连接与保活", link: "/v2/state-reference/link-lifecycle" },
      { text: "上报与新鲜度", link: "/v2/state-reference/reporting" },
      { text: "NetworkState", link: "/v2/state-reference/network" },
      { text: "查询、裁剪与订阅", link: "/v2/state-reference/query" },
      { text: "Realtime Observation", link: "/v2/state-reference/realtime" },
      { text: "观测与历史", link: "/v2/state-reference/observations" },
    ],
  },
  {
    text: "分发与 API",
    items: [
      { text: "API 分区与操作", link: "/v2/api" },
      { text: "注册与认证", link: "/v2/api/authentication" },
      { text: "SSE 与初始同步", link: "/v2/api/events" },
      { text: "配置版本与操作历史", link: "/v2/api/history" },
      { text: "Actions 与 Problem", link: "/v2/api/problems" },
      { text: "NodeConfiguration", link: "/v2/configuration-reference/node-configuration" },
      { text: "配置应用与进度", link: "/v2/configuration-reference/deployment" },
      { text: "同步与回滚", link: "/v2/configuration-reference/synchronization" },
    ],
  },
  {
    text: "Agent · 新设计",
    items: [
      { text: "Daemon、CLI 与组件", link: "/v2/agent" },
      { text: "运行模式与本机 IPC", link: "/v2/agent/runtime" },
      { text: "插件、装载与能力", link: "/v2/agent/plugins" },
      { text: "Crates 与目录", link: "/v2/agent/structure" },
    ],
  },
  {
    text: "存储与工程",
    items: [
      { text: "持久数据库字段", link: "/v2/storage/database" },
      { text: "Controller 内存字段", link: "/v2/storage/memory" },
      { text: "存储与访问", link: "/v2/storage" },
      { text: "配置查询与缓存", link: "/v2/configuration-reference/storage-api" },
      { text: "技术栈与代码结构", link: "/v2/architecture" },
    ],
  },
];

export default defineConfig({
  lang: "zh-CN",
  title: "Meshway",
  description: "以 Tailscale 为产品基线，扩展自定义承载与有序指定路径的 Rust mesh 网络。",
  cleanUrls: true,
  head: [["link", { rel: "icon", type: "image/svg+xml", href: "/meshway.svg" }]],
  themeConfig: {
    logo: "/meshway.svg",
    nav: [
      { text: "产品", link: "/concepts" },
      { text: "架构", link: "/architecture" },
      { text: "开发", link: "/getting-started" },
      { text: "参考", link: "/cli" },
      {
        text: "文档版本",
        items: [
          { text: "当前实现", link: "/" },
          { text: "V2 · 设计中", link: "/v2/" },
        ],
      },
    ],
    sidebar: {
      "/v2/": v2Sidebar,
      "/": currentSidebar,
    },
    search: {
      provider: "local",
      options: {
        locales: {
          root: {
            translations: {
              button: { buttonText: "搜索文档", buttonAriaLabel: "搜索文档" },
              modal: {
                displayDetails: "显示详细列表",
                resetButtonTitle: "清除搜索",
                backButtonTitle: "关闭搜索",
                noResultsText: "没有找到相关内容",
                footer: { selectText: "选择", navigateText: "切换", closeText: "关闭" },
              },
            },
          },
        },
      },
    },
    outline: { label: "本页目录", level: [2, 3] },
    docFooter: { prev: "上一页", next: "下一页" },
    darkModeSwitchLabel: "外观",
    darkModeSwitchTitle: "切换至深色模式",
    lightModeSwitchTitle: "切换至浅色模式",
    sidebarMenuLabel: "目录",
    returnToTopLabel: "返回顶部",
    footer: { message: "设计先明确边界，实现以可验证的纵向切片推进。" },
  },
});
