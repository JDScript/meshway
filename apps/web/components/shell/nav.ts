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

export type NavItem = {
  /** First path segment under the network, e.g. "access"; "" is the overview. */
  segment: string
  label: string
  icon: LucideIcon
  /** One line, shown as the sidebar tooltip and in the command menu. */
  description: string
  href: (routes: ConsoleRoutes) => string
  draft?: DraftKind
  /** Small qualifier after the label, e.g. an advanced entry. */
  badge?: string
  keywords: string[]
}

export type NavGroup = {
  key: string
  /** Groups without a label sit at the top of the sidebar. */
  label: string | null
  items: NavItem[]
}

export const navGroups: NavGroup[] = [
  {
    key: "network",
    label: null,
    items: [
      {
        segment: "",
        label: "概览",
        icon: LayoutDashboardIcon,
        description: "待处理事项与网络摘要",
        href: (routes) => routes.overview,
        keywords: ["overview", "dashboard", "gailan"],
      },
      {
        segment: "machines",
        label: "节点",
        icon: ServerIcon,
        description: "加入这张网络的设备与服务器",
        href: (routes) => routes.machines(),
        keywords: ["machines", "nodes", "devices", "jiedian", "设备"],
      },
      {
        segment: "dns",
        label: "DNS",
        icon: GlobeIcon,
        description: "节点 DNS 意图与节点名称",
        href: (routes) => routes.dns(),
        keywords: ["dns", "magicdns", "zone", "ttl", "名称解析"],
      },
    ],
  },
  {
    key: "policies",
    label: "访问控制",
    items: [
      {
        segment: "access",
        label: "访问授权",
        icon: ShieldCheckIcon,
        description: "哪些节点之间可以访问",
        href: (routes) => routes.access(),
        draft: "access",
        keywords: ["access", "acl", "grants", "authorization", "shouquan", "授权", "访问控制"],
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
        description: "跨节点检查监听与主动连接（当前实现）",
        href: (routes) => routes.connections,
        draft: "connections",
        badge: "高级",
        keywords: ["connections", "listeners", "dial", "transport", "lianjie", "高级连接"],
      },
    ],
  },
  {
    key: "diagnostics",
    label: "诊断",
    items: [
      {
        segment: "diagnostics",
        label: "诊断工作台",
        icon: ActivityIcon,
        description: "按问题选择检查入口",
        href: (routes) => routes.diagnostics(),
        keywords: ["diagnostics", "monitoring", "netcheck", "whois", "ping", "zhenduan", "诊断"],
      },
      {
        segment: "topology",
        label: "拓扑与流量检查",
        icon: WaypointsIcon,
        description: "已上报邻接与规划路径",
        href: (routes) => routes.topology(),
        keywords: ["topology", "map", "flow", "path", "tuopu"],
      },
    ],
  },
]

/** Pinned to the bottom of the sidebar, like Tailscale's settings. */
export const settingsItem: NavItem = {
  segment: "settings",
  label: "网络设置",
  icon: SettingsIcon,
  description: "名称、ID 与地址前缀",
  href: (routes) => routes.settings,
  keywords: ["settings", "network", "name", "prefix", "shezhi", "网络设置", "设置"],
}

export const navItems: NavItem[] = [...navGroups.flatMap((group) => group.items), settingsItem]

export function activeNavItem(pathname: string, base: string): { item: NavItem; group: NavGroup | null } | undefined {
  const segment = sectionSegment(pathname, base)
  if (segment === null) return undefined
  if (segment === settingsItem.segment) return { item: settingsItem, group: null }
  for (const group of navGroups) {
    const item = group.items.find((candidate) => candidate.segment === segment)
    if (item) return { item, group }
  }
  return undefined
}
