import {
  ActivityIcon,
  KeyRoundIcon,
  CableIcon,
  LayoutDashboardIcon,
  NetworkIcon,
  RouteIcon,
  ServerIcon,
  ShieldCheckIcon,
  TagIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react"

export type NavItem = {
  href: string
  label: string
  icon: LucideIcon
  /** Short hint used by the command menu. */
  keywords: string[]
  /** A view whose data source is not implemented yet. */
  preview?: boolean
}

export type NavSection = { label: string; items: NavItem[] }

export const navSections: NavSection[] = [
  {
    label: "网络",
    items: [
      {
        href: "/",
        label: "概览",
        icon: LayoutDashboardIcon,
        keywords: ["dashboard", "overview", "gailan"],
      },
      {
        href: "/nodes",
        label: "节点",
        icon: ServerIcon,
        keywords: ["nodes", "inventory", "jiedian"],
      },
      {
        href: "/topology",
        label: "网络拓扑",
        icon: WaypointsIcon,
        keywords: ["map", "topology", "network", "adjacency", "path", "tuopu", "luoji"],
      },
    ],
  },
  {
    label: "策略",
    items: [
      {
        href: "/access",
        label: "访问授权",
        icon: ShieldCheckIcon,
        keywords: ["access", "acl", "authorization", "allow", "reject", "shouquan"],
      },
      {
        href: "/routing",
        label: "路由策略",
        icon: RouteIcon,
        keywords: ["routing", "policy", "rules", "draft", "luyou"],
      },
      {
        href: "/tags",
        label: "标签",
        icon: TagIcon,
        keywords: ["tags", "groups", "members", "biaoqian"],
      },
    ],
  },
  {
    label: "管理",
    items: [
      {
        href: "/network",
        label: "网络设置",
        icon: NetworkIcon,
        keywords: ["network", "settings", "dns", "zone", "ttl", "prefix", "网络名称"],
      },
      {
        href: "/enrollment",
        label: "注册令牌",
        icon: KeyRoundIcon,
        keywords: ["enrollment", "token", "zhuce"],
      },
      {
        href: "/connections",
        label: "高级连接",
        icon: CableIcon,
        keywords: ["connections", "连接配置", "listeners", "dial", "transport", "audit", "lianjie"],
      },
      {
        href: "/monitoring",
        label: "诊断",
        icon: ActivityIcon,
        keywords: ["monitoring", "diagnostics", "netcheck", "ping", "whois", "dns", "诊断", "jiankong"],
      },
    ],
  },
]

export const navItems: NavItem[] = navSections.flatMap((section) => section.items)

export function isActive(href: string, pathname: string): boolean {
  return href === "/" ? pathname === "/" : pathname === href || pathname.startsWith(`${href}/`)
}

export function currentItem(pathname: string): NavItem | undefined {
  return navItems.find((item) => isActive(item.href, pathname))
}

export function currentSection(pathname: string): NavSection | undefined {
  return navSections.find((section) => section.items.some((item) => isActive(item.href, pathname)))
}
