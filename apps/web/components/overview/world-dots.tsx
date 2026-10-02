import { memo } from "react"

import { WORLD_DOTS } from "@/lib/world-dots.gen"

/** One path of round zero-length segments: a few thousand dots, one DOM node. */
const DOT_PATH = (() => {
  const { step, rows } = WORLD_DOTS
  const parts: string[] = []
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x += 1) {
      if (row[x] === "1") parts.push(`M${(x * step + step / 2).toFixed(2)} ${(y * step + step / 2).toFixed(2)}h0`)
    }
  })
  return parts.join("")
})()

export const WORLD_WIDTH = 360
export const WORLD_HEIGHT = WORLD_DOTS.rows.length * WORLD_DOTS.step

/**
 * Decorative dotted land backdrop (Natural Earth 1:110m, public domain).
 * Nodes are only drawn on it with a sourced geographic placement.
 */
export const WorldDots = memo(function WorldDots({ children }: { children?: React.ReactNode }) {
  return (
    <svg
      aria-hidden={children ? undefined : true}
      viewBox={`0 0 ${WORLD_WIDTH} ${WORLD_HEIGHT}`}
      preserveAspectRatio="xMidYMid meet"
      className="pointer-events-none absolute inset-0 size-full text-[var(--map-dot)]"
    >
      <path d={DOT_PATH} stroke="currentColor" strokeWidth={0.62} strokeLinecap="round" fill="none" />
      {children}
    </svg>
  )
})
