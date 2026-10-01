import { ArrowLeftIcon } from "lucide-react"
import Link from "next/link"
import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

/**
 * Page title row: optional back link, title with inline status, one-line
 * purpose, and the page's primary actions on the right. `meta` carries the
 * read time and similar provenance in small type.
 */
export function PageHeader({
  title,
  description,
  actions,
  back,
  status,
  meta,
}: {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  back?: { href: string; label: string }
  status?: ReactNode
  meta?: ReactNode
}) {
  return (
    <header data-slot="page-header" className="flex flex-col gap-3">
      {back ? (
        <Link
          href={back.href}
          className="inline-flex w-fit items-center gap-1 rounded text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/40"
        >
          <ArrowLeftIcon className="size-3.5" aria-hidden="true" />
          {back.label}
        </Link>
      ) : null}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h1 className="min-w-0 truncate text-[1.375rem] leading-tight font-semibold tracking-tight">{title}</h1>
            {status}
          </div>
          {description ? <p className="max-w-3xl text-[0.8125rem] leading-relaxed text-muted-foreground">{description}</p> : null}
          {meta ? <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground tabular-nums">{meta}</div> : null}
        </div>
        {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
      </div>
    </header>
  )
}

/** Shared page width and vertical rhythm for console pages. */
export function PageBody({ children, className }: { children: ReactNode; className?: string; width?: "default" | "wide" }) {
  return (
    <div data-slot="page-body" className={cn("console-page flex min-w-0 flex-col gap-6", className)}>
      {children}
    </div>
  )
}

/** Heading for a group of panels inside a page. */
export function SectionTitle({ title, description, actions, id }: {
  title: string
  description?: ReactNode
  actions?: ReactNode
  id?: string
}) {
  return (
    <div className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
      <div className="flex min-w-0 flex-col gap-0.5">
        <h2 id={id} className="text-[0.9375rem] font-semibold tracking-tight">{title}</h2>
        {description ? <p className="text-xs leading-relaxed text-muted-foreground">{description}</p> : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  )
}
