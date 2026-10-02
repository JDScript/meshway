import { StatusDot, type DotShape } from "@/components/chrome/status-dot"
import type { LinkLook, Vitality } from "@/features/globe/scene"

import { lookColor, vitalityColor } from "./labels"

const vitalityShape: Record<Vitality, DotShape> = {
  live: "live",
  contact: "solid",
  quiet: "hollow",
}

/** A node's vitality: lit only while its runtime report is fresh. */
export function VitalityDot({
  vitality,
  size,
}: {
  vitality: Vitality
  size?: "sm" | "md"
}) {
  return (
    <StatusDot
      color={vitalityColor[vitality]}
      shape={vitalityShape[vitality]}
      size={size}
    />
  )
}

/** A link's evidence: only both-sides-confirmed links are lit. */
export function LookDot({
  look,
  size,
}: {
  look: LinkLook
  size?: "sm" | "md"
}) {
  return (
    <StatusDot
      color={lookColor[look]}
      shape={look === "confirmed" ? "live" : "solid"}
      size={size}
    />
  )
}
