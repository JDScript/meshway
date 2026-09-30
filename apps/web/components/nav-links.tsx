"use client"

import { ArrowUpRightIcon, LayoutDashboardIcon, ServerIcon } from "lucide-react"
import Link from "next/link"
import { usePathname } from "next/navigation"

import { docsUrl } from "@/lib/config"
import { cn } from "@/lib/utils"

const items = [
  { href: "/", label: "概览", icon: LayoutDashboardIcon },
  { href: "/nodes", label: "节点", icon: ServerIcon },
] as const

const itemClass =
  "flex h-8 items-center gap-2 rounded-md px-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/30 focus-visible:outline-none [&_svg]:size-4 [&_svg]:shrink-0"

export function NavLinks({ orientation }: { orientation: "vertical" | "horizontal" }) {
  const pathname = usePathname()

  return (
    <nav
      aria-label="主导航"
      className={cn(
        "flex gap-1",
        orientation === "vertical" ? "flex-col" : "flex-row items-center"
      )}
    >
      {items.map(({ href, label, icon: Icon }) => {
        const active = href === "/" ? pathname === "/" : pathname.startsWith(href)
        return (
          <Link
            key={href}
            href={href}
            aria-current={active ? "page" : undefined}
            className={cn(
              itemClass,
              active && "bg-muted font-medium text-foreground"
            )}
          >
            <Icon />
            {label}
          </Link>
        )
      })}
      <a href={docsUrl} target="_blank" rel="noreferrer" className={itemClass}>
        <ArrowUpRightIcon />
        文档
      </a>
    </nav>
  )
}
