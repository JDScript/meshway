import { cn } from "cn"

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { m } from "@/paraglide/messages.js"

import type { LinkSummary } from "./model"
import { lookColor } from "./labels"

const DASH = "—"

/**
 * The corner summary. Every slot is always rendered, so loading, empty and
 * failing states never change its size.
 */
export function StatsPanel({
  nodes,
  links,
  stale,
}: {
  /** null while unread. */
  nodes: number | null
  links: LinkSummary | null
  /** Time of the last good runtime read when the latest refresh failed. */
  stale: string | null
}) {
  const known = links?.kind === "known" ? links : null
  const total = known?.total ?? 0
  const segments = known
    ? [
        { key: "confirmed", value: known.confirmed, color: lookColor.confirmed },
        { key: "one_sided", value: known.oneSided, color: lookColor.one_sided },
        { key: "degraded", value: known.degraded, color: lookColor.degraded },
        { key: "unreported", value: known.unreported, color: lookColor.configured },
      ]
    : []
  const linkValue = known ? `${known.confirmed}/${known.total}` : links?.kind === "none" ? "0" : DASH

  return (
    <div className="glass pointer-events-auto w-[300px] rounded-3xl px-5 pt-4 pb-4" aria-live="polite">
      <div className="flex items-start gap-8">
        <Stat value={nodes === null ? DASH : String(nodes)} label={m.stat_nodes({ count: nodes ?? 0 })} />
        <Stat value={linkValue} label={m.stat_links({ count: total })} />
        <Tooltip>
          <TooltipTrigger
            render={
              <span
                aria-label={stale ? m.overview_refresh_failed({ time: stale }) : undefined}
                className={cn(
                  "mt-2 ml-auto size-1.5 rounded-full transition-colors duration-500",
                  stale ? "bg-warning" : "bg-transparent"
                )}
              />
            }
          />
          {stale ? <TooltipContent>{m.overview_refresh_failed({ time: stale })}</TooltipContent> : null}
        </Tooltip>
      </div>

      <div className="mt-4 flex h-1 w-full gap-0.5 overflow-hidden rounded-full bg-foreground/8">
        {segments.map((segment) =>
          segment.value > 0 ? (
            <span
              key={segment.key}
              className="h-full rounded-full transition-[flex-grow] duration-700"
              style={{ flexGrow: segment.value, background: segment.color }}
            />
          ) : null
        )}
      </div>

      <div className="mt-3 flex justify-between text-[11.5px] text-muted-foreground tabular-nums">
        <Legend color={lookColor.one_sided} active={(known?.oneSided ?? 0) > 0} text={m.stat_one_sided({ count: String(known?.oneSided ?? 0) })} />
        <Legend color={lookColor.degraded} active={(known?.degraded ?? 0) > 0} text={m.stat_degraded({ count: String(known?.degraded ?? 0) })} />
        <Legend color={lookColor.configured} active={(known?.unreported ?? 0) > 0} text={m.stat_unreported({ count: String(known?.unreported ?? 0) })} />
      </div>
    </div>
  )
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="min-w-14">
      <div className="text-[28px] leading-none font-semibold tracking-tight tabular-nums">{value}</div>
      <div className="mt-1.5 text-xs text-muted-foreground">{label}</div>
    </div>
  )
}

function Legend({ color, active, text }: { color: string; active: boolean; text: string }) {
  return (
    <span className={cn("flex items-center gap-1.5 transition-opacity", active ? "opacity-100" : "opacity-45")}>
      <span aria-hidden="true" className="size-1.5 rounded-full" style={{ background: color }} />
      {text}
    </span>
  )
}
