import { defineConfig } from "vitepress";

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
    ],
    sidebar: [
      {
        text: "产品",
        items: [
          { text: "概览、概念与使用流程", link: "/concepts" },
          { text: "网络管理", link: "/network-management" },
          { text: "访问授权（Access）", link: "/access-policy" },
          { text: "Mesh 路由与出口方案", link: "/routing" },
          { text: "控制台", link: "/web-product" },
          { text: "Apple App", link: "/apple-app" },
          { text: "产品对齐记录", link: "/product-alignment" },
          { text: "交付里程碑", link: "/milestones" },
        ],
      },
      {
        text: "架构",
        items: [
          { text: "总览与实现边界", link: "/architecture" },
          { text: "代码边界审查与收敛", link: "/architecture-review" },
          { text: "模型与 SPA 接口审查", link: "/model-api-audit" },
          { text: "SPA 会话、查询与轮询", link: "/spa-api-audit" },
          { text: "控制面注册与持久化", link: "/enrollment" },
          { text: "控制面发布与预算", link: "/controller-publication" },
          { text: "浏览器登录与节点接入", link: "/device-login" },
          { text: "本地账户领域与 SSO 接线", link: "/local-accounts" },
          { text: "地址与配置分发", link: "/configuration" },
          { text: "公钥与连接意图", link: "/connections" },
          { text: "Agent 连接运行层", link: "/connection-runtime" },
          { text: "托管 Mesh IPv4 数据面", link: "/managed-dataplane" },
          { text: "四层策略引擎 v3", link: "/policy-v3" },
          { text: "出口数据面 RFC 与原型", link: "/exit-dataplane" },
          { text: "系统 socket 保护通路", link: "/underlay-routing" },
          { text: "DNS 与域名上下文", link: "/dns" },
          { text: "Transport、插件与主备", link: "/transports" },
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
          { text: "Apple UI 验收", link: "/apple-ui-verification" },
        ],
      },
      {
        text: "参考",
        items: [
          { text: "CLI 与手动验证", link: "/cli" },
          { text: "协议与类型安全", link: "/protocol" },
          { text: "Netcheck 底层观测", link: "/netcheck" },
          { text: "真实承载 endpoint 观测", link: "/endpoint-discovery" },
          { text: "Tailscale 对照", link: "/tailscale-alignment" },
          { text: "参考源码", link: "/references" },
        ],
      },
    ],
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
