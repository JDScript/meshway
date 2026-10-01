import {
  ActivityIcon,
  CableIcon,
  GlobeIcon,
  LayoutDashboardIcon,
  RouteIcon,
  ServerIcon,
  SettingsIcon,
  ShieldCheckIcon,
  TagIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react"

import { sectionSegment, type ConsoleRoutes } from "@/lib/routes"

export { sectionSegment }

/** Editors whose unsaved work is kept across navigation. */
export type DraftKind = "access" | "routing" | "connections"

export type SubNavItem = {
  /** Path segment under the network, e.g. "access". */
  segment: string
  label: string
  icon: LucideIcon
  description: string
  href: (routes: ConsoleRoutes) => string
  draft?: DraftKind
  keywords: string[]
}

export type NavTab = {
  key: string
  label: string
  icon: LucideIcon
  href: (routes: ConsoleRoutes) => string
  /** Path segments (first segment after the network) owned by this tab. "" is the overview. */
  segments: string[]
  children?: SubNavItem[]
  keywords: string[]
}

export const policyNav: SubNavItem[] = [
  {
    segment: "access",
    label: "访问授权",
    icon: ShieldCheckIcon,
    description: "哪些节点之间可以访问",
    href: (routes) => routes.access(),
    draft: "access",
    keywords: ["access", "acl", "grants", "authorization", "shouquan", "授权"],
  },
  {
    segment: "routing",
    label: "路由",
    icon: RouteIcon,
    description: "流量经过哪些节点、从哪里出去",
    href: (routes) => routes.routing(),
    draft: "routing",
    keywords: ["routing", "policy", "via", "exit", "luyou", "路由策略"],
  },
  {
    segment: "tags",
    label: "标签",
    icon: TagIcon,
    description: "管理员维护的节点分组",
    href: (routes) => routes.tags(),
    keywords: ["tags", "groups", "biaoqian"],
  },
  {
    segment: "connections",
    label: "手工连接",
    icon: CableIcon,
    description: "当前的监听与主动连接配置（高级）",
    href: (routes) => routes.connections,
    draft: "connections",
    keywords: ["connections", "listeners", "dial", "transport", "lianjie", "高级连接"],
  },
]

export const diagnosticsNav: SubNavItem[] = [
  {
    segment: "diagnostics",
    label: "诊断工作台",
    icon: ActivityIcon,
    description: "按问题选择检查入口",
    href: (routes) => routes.diagnostics(),
    keywords: ["diagnostics", "monitoring", "netcheck", "whois", "ping", "zhenduan"],
  },
  {
    segment: "topology",
    label: "拓扑与流量检查",
    icon: WaypointsIcon,
    description: "已上报邻接与按已保存策略编译的路径",
    href: (routes) => routes.topology(),
    keywords: ["topology", "map", "flow", "path", "tuopu"],
  },
]

export const navTabs: NavTab[] = [
  {
    key: "overview",
    label: "概览",
    icon: LayoutDashboardIcon,
    href: (routes) => routes.overview,
    segments: [""],
    keywords: ["overview", "dashboard", "gailan"],
  },
  {
    key: "machines",
    label: "节点",
    icon: ServerIcon,
    href: (routes) => routes.machines(),
    segments: ["machines"],
    keywords: ["machines", "nodes", "devices", "jiedian", "设备"],
  },
  {
    key: "policies",
    label: "访问控制",
    icon: ShieldCheckIcon,
    href: (routes) => routes.access(),
    segments: policyNav.map((item) => item.segment),
    children: policyNav,
    keywords: ["policy", "access controls", "celue", "策略"],
  },
  {
    key: "dns",
    label: "DNS",
    icon: GlobeIcon,
    href: (routes) => routes.dns(),
    segments: ["dns"],
    keywords: ["dns", "magicdns", "zone", "ttl", "名称解析"],
  },
  {
    key: "diagnostics",
    label: "诊断",
    icon: ActivityIcon,
    href: (routes) => routes.diagnostics(),
    segments: diagnosticsNav.map((item) => item.segment),
    children: diagnosticsNav,
    keywords: ["diagnostics", "monitoring"],
  },
  {
    key: "settings",
    label: "设置",
    icon: SettingsIcon,
    href: (routes) => routes.settings,
    segments: ["settings"],
    keywords: ["settings", "network", "name", "prefix", "shezhi", "网络设置"],
  },
]

export function activeTab(pathname: string, base: string): NavTab | undefined {
  const segment = sectionSegment(pathname, base)
  if (segment === null) return undefined
  return navTabs.find((tab) => tab.segments.includes(segment))
}

export function activeSubItem(pathname: string, base: string): SubNavItem | undefined {
  const segment = sectionSegment(pathname, base)
  return activeTab(pathname, base)?.children?.find((item) => item.segment === segment)
}
