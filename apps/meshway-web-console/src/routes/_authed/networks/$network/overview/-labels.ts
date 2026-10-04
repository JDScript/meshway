import type { LinkLook } from "@/globe/scene"
import { placeText } from "@/lib/geo"
import type { MapNode } from "@/lib/network-map"
import { locationAbsentLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

/** Why a node is not on the globe, in words: the State's reason, or that GeoIP names only a region. */
export function unlocatedReason(node: MapNode): string {
  const reason = node.unlocated ?? "unknown"
  if (reason === "noPosition")
    return m.overview_unlocated_noPosition({
      place: (node.location && placeText(node.location.data)) ?? "—",
    })
  if (reason === "unknown") return m.overview_unlocated_unknown()
  return locationAbsentLabel[reason]()
}

/** The colour a link's look takes on the globe, for its swatch in a list. */
export const lookColor: Record<LinkLook, string> = {
  matched: "var(--link-confirmed)",
  one_sided: "var(--link-one-sided)",
  conflicting: "var(--link-degraded)",
  failed: "var(--link-degraded)",
  preparing: "var(--link-configured)",
  expired: "var(--link-configured)",
  closed: "var(--link-configured)",
  history: "var(--globe-land)",
}
