import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

/**
 * Status vocabulary shared by every page. Tones carry meaning only:
 * success = confirmed/current, warning = needs attention or pending,
 * danger = failed/revoked/invalid, info = in progress or informational,
 * neutral = unknown, never measured or not applicable.
 */
export type Tone = "success" | "warning" | "danger" | "info" | "neutral"

const toneClass: Record<Tone, string> = {
  success: "border-success/25 bg-success/10 text-success",
  warning: "border-warning/30 bg-warning/12 text-[color-mix(in_oklch,var(--warning),var(--foreground)_35%)] dark:text-warning",
  danger: "border-destructive/25 bg-destructive/10 text-destructive",
  info: "border-info/25 bg-info/10 text-info",
  neutral: "border-border bg-muted text-muted-foreground",
}

const dotClass: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-destructive",
  info: "bg-info",
  neutral: "bg-neutral",
}

export function StatusDot({ tone, className }: { tone: Tone; className?: string }) {
  return <span aria-hidden="true" className={cn("inline-block size-1.5 shrink-0 rounded-full", dotClass[tone], className)} />
}

export function StatusBadge({
  tone,
  children,
  dot = false,
  title,
  className,
}: {
  tone: Tone
  children: ReactNode
  dot?: boolean
  title?: string
  className?: string
}) {
  return (
    <span
      title={title}
      data-tone={tone}
      className={cn(
        "inline-flex h-5 w-fit shrink-0 items-center gap-1.5 rounded-full border px-2 text-[0.6875rem] font-medium whitespace-nowrap",
        toneClass[tone],
        className
      )}
    >
      {dot ? <StatusDot tone={tone} /> : null}
      {children}
    </span>
  )
}
