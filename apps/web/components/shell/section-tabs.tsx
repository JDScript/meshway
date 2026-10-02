"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"

import { useNetwork } from "@/components/shell/network-context"
import { sectionSegment } from "@/lib/routes"
import type { ConsoleRoutes } from "@/lib/routes"
import { cn } from "@/lib/utils"

type Tab = { segment: string; label: string; href: (routes: ConsoleRoutes) => string; badge?: string }

const sections: Record<"nodes" | "settings", Tab[]> = {
  nodes: [
    { segment: "machines", label: "全部节点", href: (routes) => routes.machines() },
    { segment: "tags", label: "标签", href: (routes) => routes.tags() },
    { segment: "connections", label: "手工连接", href: (routes) => routes.connections, badge: "高级" },
  ],
  settings: [
    { segment: "settings", label: "网络", href: (routes) => routes.settings },
    { segment: "dns", label: "DNS", href: (routes) => routes.dns() },
  ],
}

/** The pages of one sidebar section, as links under its title. */
export function SectionTabs({ section }: { section: keyof typeof sections }) {
  const { routes } = useNetwork()
  const current = sectionSegment(usePathname(), routes.base)
  return (
    <nav aria-label="分区" className="-mt-2 flex gap-1 border-b">
      {sections[section].map((tab) => {
        const active = tab.segment === current
        return (
          <Link
            key={tab.segment}
            href={tab.href(routes)}
            aria-current={active ? "page" : undefined}
            className={cn(
              "relative -mb-px inline-flex h-9 items-center gap-1.5 border-b-2 px-2.5 text-[0.8125rem] font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring/40",
              active ? "border-foreground text-foreground" : "border-transparent text-muted-foreground hover:text-foreground"
            )}
          >
            {tab.label}
            {tab.badge ? <span className="rounded border px-1 text-[0.625rem] leading-4 font-normal text-muted-foreground">{tab.badge}</span> : null}
          </Link>
        )
      })}
    </nav>
  )
}
