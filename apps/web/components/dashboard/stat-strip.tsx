import Link from "next/link"
import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

export type Stat = {
  label: string
  value: ReactNode
  /** Denominator or unit shown next to the value. */
  suffix?: ReactNode
  detail: ReactNode
  href?: string
  meter?: { value: number; max: number }
}

/** A single row of key figures separated by hairlines, not a grid of cards. */
export function StatStrip({ stats }: { stats: Stat[] }) {
  return (
    <div className="grid overflow-hidden rounded-lg border bg-card shadow-xs sm:grid-cols-2 xl:grid-cols-4">
      {stats.map((stat, index) => {
        const body = (
          <>
            <span className="text-xs text-muted-foreground">{stat.label}</span>
            <span className="flex items-baseline gap-1.5">
              <span className="text-[1.625rem] leading-none font-semibold tracking-tight tabular-nums">{stat.value}</span>
              {stat.suffix ? (
                <span className="text-sm text-muted-foreground tabular-nums">{stat.suffix}</span>
              ) : null}
            </span>
            {stat.meter ? (
              <span
                aria-hidden="true"
                className="relative h-1 w-full overflow-hidden rounded-full bg-muted"
              >
                <span
                  className="absolute inset-y-0 left-0 rounded-full bg-brand"
                  style={{
                    width: `${stat.meter.max === 0 ? 0 : (stat.meter.value / stat.meter.max) * 100}%`,
                  }}
                />
              </span>
            ) : null}
            <span className="text-xs text-muted-foreground">{stat.detail}</span>
          </>
        )
        const className = cn(
          "flex flex-col gap-1.5 px-4 py-4 sm:px-5",
          index > 0 && "border-t sm:border-t-0",
          index % 2 === 1 && "sm:border-l",
          index >= 2 && "sm:border-t xl:border-t-0",
          index > 0 && "xl:border-l"
        )
        return stat.href ? (
          <Link
            key={stat.label}
            href={stat.href}
            className={cn(className, "transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none")}
          >
            {body}
          </Link>
        ) : (
          <div key={stat.label} className={className}>
            {body}
          </div>
        )
      })}
    </div>
  )
}
