import { defineConfig } from "vitepress";

export default defineConfig({
  lang: "zh-CN",
  title: "Meshway",
  description: "面向身份、策略路径与多传输的 Rust mesh 网络。",
  cleanUrls: true,
  head: [["link", { rel: "icon", type: "image/svg+xml", href: "/meshway.svg" }]],
  themeConfig: {
    logo: "/meshway.svg",
    nav: [
      { text: "开始使用", link: "/getting-started" },
      { text: "架构设计", link: "/architecture" },
      { text: "开发状态", link: "/development" },
    ],
    sidebar: [
      {
        text: "项目",
        items: [
          { text: "开始使用", link: "/getting-started" },
          { text: "架构与设计状态", link: "/architecture" },
          { text: "Web 产品与交互", link: "/web-product" },
        ],
      },
      {
        text: "网络设计",
        items: [
          { text: "身份 DNS 与域名上下文", link: "/dns" },
          { text: "分类、授权与有序路径", link: "/routing" },
          { text: "多传输与故障切换", link: "/transports" },
          { text: "邻接、建连与协议插件", link: "/transport-connections" },
          { text: "公钥与连接配置", link: "/connections" },
          { text: "连接应用、观测与地址", link: "/connection-runtime" },
          { text: "手动验证连接", link: "/testing-connections" },
          { text: "Agent CLI 与运行证据", link: "/cli" },
          { text: "Netcheck 与底层网络", link: "/netcheck" },
          { text: "承载 socket 与 endpoint 观测", link: "/endpoint-discovery" },
          { text: "托管 Mesh IPv4 数据面", link: "/managed-dataplane" },
          { text: "端点访问授权与 JSONC", link: "/access-policy" },
          { text: "Network 管理边界", link: "/network-management" },
          { text: "Linux 传输实验", link: "/transport-lab" },
          { text: "加密边界与真实追踪", link: "/security-and-tracing" },
        ],
      },
      {
        text: "实现与研究",
        items: [
          { text: "控制面注册与持久化", link: "/enrollment" },
          { text: "地址与配置分发", link: "/configuration" },
          { text: "协议与类型安全", link: "/protocol" },
          { text: "开发路线与验收", link: "/development" },
          { text: "后续里程碑", link: "/milestones" },
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
