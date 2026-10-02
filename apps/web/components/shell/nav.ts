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
  UserRoundIcon,
  UsersIcon,
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
  /** One line, shown in the command menu. */
  description: string
  href: (routes: ConsoleRoutes) => string
  draft?: DraftKind
  /** Small qualifier after the label, e.g. an advanced entry. */
  badge?: string
  /** Pages reached from inside a main section (its tabs or context); the sidebar highlights that section. */
  parent?: string
  keywords: string[]
}

/** The sidebar, in order: the console's six places. ⌘K also lists the pages under them. */
export const navMain: NavItem[] = [
  {
    segment: "",
    label: "概览",
    icon: LayoutDashboardIcon,
    description: "网络地图与需要处理的事项",
    href: (routes) => routes.overview,
    keywords: ["overview", "dashboard", "map", "gailan", "地图"],
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
    segment: "members",
    label: "成员",
    icon: UsersIcon,
    description: "成员、角色、邀请与设备归属",
    href: (routes) => routes.members,
    keywords: ["members", "users", "invite", "roles", "chengyuan", "邀请", "用户", "角色"],
  },
  {
    segment: "access",
    label: "访问控制",
    icon: ShieldCheckIcon,
    description: "哪些节点之间可以访问",
    href: (routes) => routes.access(),
    draft: "access",
    keywords: ["access", "acl", "grants", "authorization", "shouquan", "授权", "访问授权"],
  },
  {
    segment: "routing",
    label: "路由",
    icon: RouteIcon,
    description: "流量经过哪些节点、从哪里出去",
    href: (routes) => routes.routing(),
    draft: "routing",
    keywords: ["routing", "policy", "via", "exit", "luyou", "路由策略", "出口"],
  },
  {
    segment: "settings",
    label: "设置",
    icon: SettingsIcon,
    description: "网络名称、地址与 DNS",
    href: (routes) => routes.settings,
    keywords: ["settings", "network", "name", "prefix", "shezhi", "网络设置"],
  },
]

/** Pages inside a main section: its tabs, or reached from a node, an edge or a path. */
export const navContext: NavItem[] = [
  {
    segment: "tags",
    label: "标签",
    icon: TagIcon,
    description: "管理员维护的节点分组",
    href: (routes) => routes.tags(),
    parent: "machines",
    keywords: ["tags", "groups", "biaoqian"],
  },
  {
    segment: "connections",
    label: "手工连接",
    icon: CableIcon,
    description: "跨节点检查监听与主动连接（高级）",
    href: (routes) => routes.connections,
    draft: "connections",
    badge: "高级",
    parent: "machines",
    keywords: ["connections", "listeners", "dial", "transport", "lianjie", "高级连接"],
  },
  {
    segment: "diagnostics",
    label: "诊断",
    icon: ActivityIcon,
    description: "按问题选择检查入口",
    href: (routes) => routes.diagnostics(),
    parent: "machines",
    keywords: ["diagnostics", "monitoring", "netcheck", "whois", "ping", "zhenduan", "诊断"],
  },
  {
    segment: "topology",
    label: "拓扑与流量检查",
    icon: WaypointsIcon,
    description: "已上报邻接与规划路径",
    href: (routes) => routes.topology(),
    parent: "machines",
    keywords: ["topology", "flow", "path", "tuopu"],
  },
  {
    segment: "dns",
    label: "DNS",
    icon: GlobeIcon,
    description: "节点 DNS 意图与节点名称",
    href: (routes) => routes.dns(),
    parent: "settings",
    keywords: ["dns", "magicdns", "zone", "ttl", "名称解析"],
  },
  {
    segment: "account",
    label: "我的账户",
    icon: UserRoundIcon,
    description: "密码、登录会话与我的设备",
    href: (routes) => routes.account,
    keywords: ["account", "password", "profile", "zhanghu", "密码", "退出"],
  },
]

export const navItems: NavItem[] = [...navMain, ...navContext]

/** The page for `pathname`, and the main section it belongs to (itself for a main page). */
export function activeNavItem(pathname: string, base: string): { item: NavItem; main: NavItem | null } | undefined {
  const segment = sectionSegment(pathname, base)
  if (segment === null) return undefined
  const item = navItems.find((candidate) => candidate.segment === segment)
  if (!item) return undefined
  const main = item.parent === undefined ? (navMain.includes(item) ? item : null) : (navMain.find((candidate) => candidate.segment === item.parent) ?? null)
  return { item, main }
}

/** Draft kinds that count toward a main section's unsaved marker. */
export function sectionDrafts(main: NavItem): DraftKind[] {
  return navItems.flatMap((item) => (item.draft && (item === main || item.parent === main.segment) ? [item.draft] : []))
}
