"use client"

import Link from "next/link"
import { usePathname } from "next/navigation"

import { useDrafts } from "@/components/shell/console-shell"
import { diagnosticsNav, policyNav, sectionSegment, type SubNavItem } from "@/components/shell/nav"
import { useRoutes } from "@/components/shell/network-context"
import { cn } from "@/lib/utils"

const groups: Record<"policies" | "diagnostics", { label: string; items: SubNavItem[] }> = {
  policies: { label: "访问控制", items: policyNav },
  diagnostics: { label: "诊断", items: diagnosticsNav },
}

/** Second-level navigation for a tab that owns several pages. */
export function SectionTabs({ group }: { group: keyof typeof groups }) {
  const pathname = usePathname()
  const routes = useRoutes()
  const drafts = useDrafts()
  const segment = sectionSegment(pathname, routes.base)
  const { label, items } = groups[group]

  return (
    <nav aria-label={label} className="flex max-w-full">
      <ul className="flex max-w-full gap-1 overflow-x-auto rounded-lg border bg-card p-1 shadow-xs [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {items.map((item) => {
          const active = item.segment === segment
          const unsaved = item.draft ? drafts[item.draft] : false
          return (
            <li key={item.segment} className="shrink-0">
              <Link
                href={item.href(routes)}
                aria-current={active ? "page" : undefined}
                title={item.description}
                className={cn(
                  "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[0.8125rem] font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40",
                  active && "bg-secondary text-foreground shadow-xs"
                )}
              >
                <item.icon className="size-3.5" aria-hidden="true" />
                {item.label}
                {unsaved ? (
                  <span className="flex items-center gap-1 text-[0.6875rem] font-normal text-warning">
                    <span aria-hidden="true" className="size-1.5 rounded-full bg-warning" />
                    未保存
                  </span>
                ) : null}
              </Link>
            </li>
          )
        })}
      </ul>
    </nav>
  )
}
