import { defineConfig, type DefaultTheme } from "vitepress";
import { contractAssets } from "./contracts.mts";

const v2Sidebar: DefaultTheme.SidebarItem[] = [
  {
    text: "总体",
    items: [
      { text: "概览", link: "/v2/" },
      { text: "项目结构与 Shared Crates", link: "/v2/architecture" },
      { text: "开发服务、测试与环境", link: "/v2/development" },
      { text: "运行日志", link: "/v2/logging" },
      { text: "数据模型与权威来源", link: "/v2/entities" },
      { text: "操作权限", link: "/v2/permissions" },
      { text: "按需执行许可", link: "/v2/execution" },
    ],
  },
  {
    text: "Server · 控制面",
    items: [
      { text: "职责、能力与运行", link: "/v2/server" },
      { text: "HTTP 与生成客户端", link: "/v2/api" },
      { text: "注册与认证", link: "/v2/api/authentication" },
      { text: "SSE 与初始同步", link: "/v2/api/events" },
      { text: "版本与操作历史", link: "/v2/api/history" },
      { text: "Actions 与 Problem", link: "/v2/api/problems" },
      { text: "存储与查询", link: "/v2/storage" },
      {
        text: "存储字段参考",
        collapsed: true,
        items: [
          { text: "持久数据库", link: "/v2/storage/database" },
          { text: "内存容器", link: "/v2/storage/memory" },
        ],
      },
    ],
  },
  {
    text: "Node · 节点端",
    items: [
      { text: "Daemon、CLI 与组件", link: "/v2/node" },
      { text: "Server 接入组件", link: "/v2/node/control" },
      { text: "网络模式与本机运行", link: "/v2/node/runtime" },
      { text: "Mesh 路径诊断与 ping", link: "/v2/node/diagnostics" },
      { text: "数据面、TUN 与 DNS", link: "/v2/node/dataplane" },
      { text: "三节点 Docker 开发环境", link: "/v2/node/docker-lab" },
      { text: "承载会话与发送边界", link: "/v2/node/transport" },
      { text: "分层性能测量", link: "/v2/node/performance" },
      { text: "连接获取与 UDP 穿透", link: "/v2/node/connectivity" },
      { text: "插件与能力", link: "/v2/node/plugins" },
    ],
  },
  {
    text: "设计讨论",
    items: [
      { text: "讨论入口与问题", link: "/v2/discussions/" },
      { text: "Packet：字节与固定开销", link: "/v2/discussions/packet" },
      { text: "包流分层、封装与加密", link: "/v2/discussions/packet-layers" },
      { text: "EasyTier 源码对照", link: "/v2/discussions/easytier" },
    ],
  },
  {
    text: "共享 Configuration 参考",
    collapsed: true,
    items: [
      { text: "结构与字段", link: "/v2/configuration-reference" },
      { text: "整网草稿与应用对照", link: "/v2/configuration-reference/editing" },
      { text: "同步、应用与恢复", link: "/v2/configuration-reference/deployment" },
      { text: "NodeConfiguration", link: "/v2/configuration-reference/node-configuration" },
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
        collapsed: true,
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
        collapsed: true,
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
        collapsed: true,
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
    text: "共享 State 参考",
    collapsed: true,
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
];

export default defineConfig({
  vite: { plugins: [contractAssets()] },
  lang: "zh-CN",
  title: "Meshway",
  description: "以 Tailscale 为产品基线，扩展自定义承载与有序指定路径的 Rust mesh 网络。",
  cleanUrls: true,
  head: [["link", { rel: "icon", type: "image/svg+xml", href: "/meshway.svg" }]],
  themeConfig: {
    logo: "/meshway.svg",
    nav: [
      { text: "文档", link: "/v2/" },
      { text: "Server", link: "/v2/server" },
      { text: "Node", link: "/v2/node" },
      { text: "设计讨论", link: "/v2/discussions/" },
      { text: "API", link: "/v2/api" },
    ],
    sidebar: {
      "/v2/": v2Sidebar,
      "/": v2Sidebar,
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
