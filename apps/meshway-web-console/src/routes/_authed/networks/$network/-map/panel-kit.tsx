import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { ChevronLeftIcon } from "lucide-react"
import type { ReactNode } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import type { LinkLook } from "@/globe/scene"
import { m } from "@/paraglide/messages.js"

import { lookColor } from "./labels"

// The parts every level of the map panel is made of, so the network, a
// node and a link read as one panel: the same head, section titles, fact
// rows and link swatch.

/** A level's head: back to the level it was opened from, its name, close. */
export function LevelHead({
  title,
  titleId,
  meta,
  onBack,
  onClose,
  action,
}: {
  title: ReactNode
  titleId?: string
  meta?: ReactNode
  onBack?: () => void
  onClose: () => void
  action?: ReactNode
}) {
  return (
    <header className="flex shrink-0 flex-col gap-1 px-4 pt-3 pb-3">
      <div className="flex h-8 min-w-0 items-center gap-1">
        {onBack ? (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={m.map_back()}
            className="-ml-1.5 rounded-full"
            onClick={onBack}
          >
            <ChevronLeftIcon />
          </Button>
        ) : null}
        <h2
          id={titleId}
          className="min-w-0 flex-1 truncate text-lead font-semibold tracking-tight"
        >
          {title}
        </h2>
        {action}
        <CloseButton onClick={onClose} />
      </div>
      {meta ? (
        <div className="flex h-5 min-w-0 items-center gap-2 text-meta text-muted-foreground">
          {meta}
        </div>
      ) : null}
    </header>
  )
}

/** A titled part of a level. */
export function PanelSection({
  title,
  aside,
  children,
}: {
  title: string
  aside?: ReactNode
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-1">
      <div className="flex h-7 items-center gap-2 px-4">
        <h3 className="flex-1 text-meta font-medium text-muted-foreground">
          {title}
        </h3>
        {aside ? (
          <span className="text-meta text-muted-foreground tabular-nums">
            {aside}
          </span>
        ) : null}
      </div>
      {children}
    </section>
  )
}

/** Label and value rows, one label column. */
export function Facts({ children }: { children: ReactNode }) {
  return <dl className="flex flex-col px-4">{children}</dl>
}

export function Fact({
  label,
  children,
  detail,
  stacked = false,
}: {
  label: string
  children: ReactNode
  detail?: ReactNode
  /** A long label (two nodes' names) sits over its value instead of beside it. */
  stacked?: boolean
}) {
  if (stacked)
    return (
      <div className="flex min-h-9 flex-col gap-0.5 py-2 text-row">
        <dt className="truncate text-meta text-muted-foreground">{label}</dt>
        <dd className="flex min-w-0 items-baseline justify-between gap-3">
          <span className="min-w-0 break-words">{children}</span>
          {detail ? (
            <span className="shrink-0 text-meta text-muted-foreground">
              {detail}
            </span>
          ) : null}
        </dd>
      </div>
    )
  return (
    <div className="grid min-h-9 grid-cols-[5.5rem_minmax(0,1fr)] items-baseline gap-3 py-2 text-row">
      <dt className="truncate text-meta text-muted-foreground">{label}</dt>
      <dd className="min-w-0">
        <div className="break-words">{children}</div>
        {detail ? (
          <div className="mt-0.5 text-meta text-muted-foreground">{detail}</div>
        ) : null}
      </dd>
    </div>
  )
}

/** A link's colour, as its line is drawn. */
export function LinkSwatch({
  look,
  className,
}: {
  look: LinkLook
  className?: string
}) {
  return (
    <span
      aria-hidden="true"
      data-look={look}
      className={cn("link-swatch", className)}
      style={{ color: lookColor[look] }}
    />
  )
}

/** The scrolling body under a level's head. */
export function LevelBody({ children }: { children: ReactNode }) {
  return (
    <div className="scroll-edge flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pb-3">
      {children}
    </div>
  )
}
