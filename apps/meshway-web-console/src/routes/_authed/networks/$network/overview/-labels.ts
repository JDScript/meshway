import type { LinkLook } from "@/globe/scene"
import { placeText } from "@/lib/geo"
import type { MapNode } from "@/lib/network-map"
import { locationAbsentLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

/** Why a node is not on the globe, in words: the server's reason, the lookup's, or that geojs names only a region. */
export function unlocatedReason(node: MapNode): string {
  const reason = node.unlocated ?? "unknown"
  switch (reason) {
    case "noPosition":
      return m.overview_unlocated_noPosition({
        place:
          (node.geo?.kind === "found" ? placeText(node.geo.geo) : null) ?? "—",
      })
    case "lookupOff":
      return m.geo_off()
    case "lookupPending":
      return m.geo_pending()
    case "lookupFailed":
      return m.geo_failed()
    case "unknown":
      return m.overview_unlocated_unknown()
    default:
      return locationAbsentLabel[reason]()
  }
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
