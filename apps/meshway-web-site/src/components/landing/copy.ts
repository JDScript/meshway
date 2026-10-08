import type { Lang } from "@/lib/i18n"

type Feature = { title: string; body: string }

type LandingCopy = {
  eyebrow: string
  title: [string, string]
  intro: string
  docs: string
  signIn: string
  heroNote: string
  mesh: {
    caption: string
    source: string
    relay: string
    destination: string
    protection: string
    route: string
  }
  rules: {
    eyebrow: string
    title: string
    intro: string
    items: [Feature, Feature, Feature]
    labels: [string, string, string]
  }
  connection: Feature & { eyebrow: string; protocols: string[] }
  dns: Feature & { eyebrow: string; command: string; note: string }
  observation: {
    eyebrow: string
    title: string
    intro: string
    items: [Feature, Feature, Feature]
    caption: string
    globe: string
    nodes: string
    links: string
  }
  closing: { title: string; body: string; footer: string }
}

export const landingCopy: Record<Lang, LandingCopy> = {
  zh: {
    eyebrow: "LINUX 节点 · MESH 网络",
    title: ["连接节点，", "也定义连接的边界。"],
    intro:
      "Meshway 将节点连成端到端保护的 Mesh 网络。谁能访问、如何连接、经过哪里，都由明确的规则决定。",
    docs: "阅读文档",
    signIn: "登录控制台",
    heroNote: "端到端保护 · 显式访问授权 · 必经路径",
    mesh: {
      caption: "MESH / 路径示意",
      source: "源节点",
      relay: "中继节点",
      destination: "目标节点",
      protection: "端到端保护",
      route: "中继按会话转发，业务流仅在端点准入。",
    },
    rules: {
      eyebrow: "明确的规则",
      title: "每一层连接，都有依据。",
      intro:
        "服务端管理配置与授权，节点在获准范围内运行网络。访问权限、直接连接与业务路径各有自己的规则。",
      items: [
        {
          title: "谁可以访问",
          body: "访问规则按源、目标和协议／端口授予权限。默认拒绝，允许连接不等于允许业务访问。",
        },
        {
          title: "节点如何连接",
          body: "组网规则决定哪些节点可以直接连接，并可限定每条连接允许使用的承载协议。",
        },
        {
          title: "流量经过哪里",
          body: "Mesh 路由可指定有序必经节点。切换承载仍保留完整路径，回复沿原路径返回。",
        },
      ],
      labels: ["访问权限", "组网规则", "Mesh 路由"],
    },
    connection: {
      eyebrow: "连接获取",
      title: "多种承载，同一套边界。",
      body: "借助 STUN 发现候选端点，协调 UDP 打洞。打洞不可用时，可使用获准的 TCP 退路，仍遵守必经路径。",
      protocols: ["Native UDP", "Native TCP", "VLESS", "Trojan"],
    },
    dns: {
      eyebrow: "MAGICDNS + CLI",
      title: "用名称找到节点。",
      body: "MagicDNS 解析网络内有权访问的节点名称。使用 meshway ping，查看获准 Mesh 路径的认证往返与耗时。",
      command: "meshway ping peer-name",
      note: "接受节点名或 Mesh IP；探测遵守访问权限与必经路径。",
    },
    observation: {
      eyebrow: "控制台",
      title: "从规则，看到网络的实际状态。",
      intro:
        "在同一个控制台中，查看节点、连接与路径，让配置和运行事实各自清晰。",
      items: [
        {
          title: "实时状态",
          body: "订阅服务端 State，在地球视图中查看已定位的节点与连接。",
        },
        {
          title: "延迟观测",
          body: "查看节点返回的实时探测结果与延迟历史；没有样本的时段保留为空。",
        },
        {
          title: "路径检测",
          body: "按当前配置或草稿查看路径规划与匹配规则。规划结果不代表实测连通。",
        },
      ],
      caption: "网络视图示意 · 非实时数据",
      globe: "地球视图",
      nodes: "节点",
      links: "连接",
    },
    closing: {
      title: "了解网络如何连接。",
      body: "从文档了解 Meshway，或登录控制台查看你的网络。",
      footer: "连接有路径，访问有边界。",
    },
  },
  en: {
    eyebrow: "LINUX NODES · MESH NETWORKING",
    title: ["Connect your nodes.", "Define the boundaries."],
    intro:
      "Meshway connects nodes in an end-to-end protected mesh. Explicit rules govern access, connections, and the paths traffic takes.",
    docs: "Read the docs",
    signIn: "Sign in to the console",
    heroNote: "End-to-end protection · Explicit access · Required paths",
    mesh: {
      caption: "MESH / PATH ILLUSTRATION",
      source: "Source",
      relay: "Relay",
      destination: "Destination",
      protection: "End-to-end protection",
      route:
        "Relays forward by session. Traffic is admitted only at endpoints.",
    },
    rules: {
      eyebrow: "EXPLICIT RULES",
      title: "A reason for every connection.",
      intro:
        "The server manages configuration and authorization. Nodes run within those permissions, with distinct rules for access, direct connections, and traffic paths.",
      items: [
        {
          title: "Who can access",
          body: "Access rules grant permissions by source, destination, and protocol or port. Access is denied by default; a connection alone grants none.",
        },
        {
          title: "How nodes connect",
          body: "Topology rules determine which nodes may connect directly and can limit the transport protocols allowed on each connection.",
        },
        {
          title: "Where traffic goes",
          body: "Mesh routes can require nodes along an ordered path. Transport changes preserve the full path, and replies follow it back.",
        },
      ],
      labels: ["Access", "Topology", "Mesh routing"],
    },
    connection: {
      eyebrow: "CONNECTION ACQUISITION",
      title: "Different transports. Shared boundaries.",
      body: "STUN discovers candidate endpoints for coordinated UDP hole punching. When punching is unavailable, an allowed TCP fallback can carry traffic along the required path.",
      protocols: ["Native UDP", "Native TCP", "VLESS", "Trojan"],
    },
    dns: {
      eyebrow: "MAGICDNS + CLI",
      title: "Find a node by name.",
      body: "MagicDNS resolves the names of nodes you may access in your network. Use meshway ping to see authenticated round trips and timings along an allowed Mesh path.",
      command: "meshway ping peer-name",
      note: "Accepts a node name or Mesh IP; probes respect access rules and required paths.",
    },
    observation: {
      eyebrow: "THE CONSOLE",
      title: "From the rules to the running network.",
      intro:
        "See nodes, connections, and paths in one console, with a clear view of configuration and runtime facts.",
      items: [
        {
          title: "Live state",
          body: "Subscribe to State from the server. See located nodes and their connections on the globe.",
        },
        {
          title: "Latency observations",
          body: "View live probe results reported by nodes and latency history. Periods without samples stay blank.",
        },
        {
          title: "Path checks",
          body: "Inspect planned paths and matching rules for the current configuration or a draft. A plan is not a connectivity measurement.",
        },
      ],
      caption: "Network illustration · Not live data",
      globe: "Globe view",
      nodes: "Nodes",
      links: "Connections",
    },
    closing: {
      title: "Get to know your mesh.",
      body: "Explore Meshway in the docs, or sign in to see your network.",
      footer: "Defined paths. Explicit access.",
    },
  },
}
