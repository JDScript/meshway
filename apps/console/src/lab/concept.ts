import type { ComponentType } from "react"

import type { GlobeLook } from "@/features/globe/look"
import type { LinkSummary } from "@/features/overview/model"

import type { NodeFacts } from "./facts"

export type ConceptProps = {
  networkName: string
  nodeCount: number
  summary: LinkSummary
  selected: NodeFacts | null
  onSelect: (nodeId: string | null) => void
}

export type Concept = {
  id: "observatory" | "cartograph" | "aurora" | "schematic"
  name: () => string
  /** Forced theme, or null when the concept works in both. */
  theme: "dark" | "light" | null
  look: GlobeLook
  /** Width the selected-node panel takes on the right (desktop). */
  panelWidth: number
  Hud: ComponentType<ConceptProps>
}

export const known = (summary: LinkSummary) =>
  summary.kind === "known" ? summary : { total: 0, confirmed: 0, oneSided: 0, degraded: 0, unreported: 0 }
