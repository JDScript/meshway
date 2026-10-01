import {
  ActivityIcon,
  KeyRoundIcon,
  CableIcon,
  LayoutDashboardIcon,
  RouteIcon,
  ServerIcon,
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
        href: "/connections",
        label: "连接配置",
        icon: CableIcon,
        keywords: ["connections", "listeners", "dial", "transport", "public key", "lianjie"],
      },
      {
        href: "/topology",
        label: "网络拓扑",
        icon: WaypointsIcon,
        keywords: ["map", "topology", "network", "adjacency", "path", "tuopu", "luoji"],
      },
      {
        href: "/routing",
        label: "路由策略",
        icon: RouteIcon,
        keywords: ["routing", "policy", "rules", "luyou"],
      },
    ],
  },
  {
    label: "运维",
    items: [
      {
        href: "/monitoring",
        label: "监控",
        icon: ActivityIcon,
        keywords: ["monitoring", "latency", "traffic", "jiankong"],
        preview: true,
      },
      {
        href: "/enrollment",
        label: "注册令牌",
        icon: KeyRoundIcon,
        keywords: ["enrollment", "token", "zhuce"],
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
