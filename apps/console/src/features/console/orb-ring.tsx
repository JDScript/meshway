import type { LinkLook } from "@/features/globe/scene"
import { lookColor } from "@/features/overview/labels"
import type { LinkSummary } from "@/features/overview/model"

const SIZE = 60
const RADIUS = 28
const GAP = 2.2

/**
 * The link-state bar of the network chip, bent into a ring around the orb:
 * the same proportions, so the two read as one indicator.
 */
export function OrbRing({ links }: { links: LinkSummary | null }) {
  const known = links?.kind === "known" ? links : null
  const all: { look: LinkLook; value: number }[] = known
    ? [
        { look: "confirmed", value: known.confirmed },
        { look: "one_sided", value: known.oneSided },
        { look: "degraded", value: known.degraded },
        { look: "configured", value: known.unreported },
      ]
    : []
  const rows = all.filter((row) => row.value > 0)
  const total = rows.reduce((sum, row) => sum + row.value, 0)
  const circumference = 2 * Math.PI * RADIUS
  const usable = circumference - (rows.length > 1 ? rows.length * GAP : 0)
  let offset = 0
  return (
    <svg viewBox={`0 0 ${SIZE} ${SIZE}`} className="pointer-events-none absolute inset-0 size-full -rotate-90" aria-hidden="true">
      <circle cx={SIZE / 2} cy={SIZE / 2} r={RADIUS} fill="none" stroke="currentColor" strokeOpacity="0.1" strokeWidth="1.5" />
      {rows.map((row) => {
        const length = (row.value / total) * usable
        const dash = `${length} ${circumference - length}`
        const element = (
          <circle
            key={row.look}
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={RADIUS}
            fill="none"
            stroke={lookColor[row.look]}
            strokeWidth="2"
            strokeLinecap="round"
            strokeDasharray={dash}
            strokeDashoffset={-offset}
            className="transition-[stroke-dasharray,stroke-dashoffset] duration-700"
          />
        )
        offset += length + (rows.length > 1 ? GAP : 0)
        return element
      })}
    </svg>
  )
}
