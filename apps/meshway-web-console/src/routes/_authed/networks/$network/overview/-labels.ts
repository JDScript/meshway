import type { LocationSource } from "@meshway/control-client"

import type { LinkLook } from "@/globe/scene"
import type { MapNode } from "@/lib/network-map"
import { locationAbsentLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"

export const locationSourceLabel: Record<LocationSource, () => string> = {
  stun: () => m.overview_location_source_stun(),
  interface: () => m.overview_location_source_interface(),
}

/** The place a GeoIP result names, most specific last ("Seoul, KR"); null when it names none. */
export function placeText(node: MapNode): string | null {
  const data = node.location?.data
  if (!data) return null
  const parts = [data.city, data.region, data.country].filter(
    (part): part is string => typeof part === "string" && part !== ""
  )
  return parts.length ? [...new Set(parts)].join(", ") : null
}

/** Why a node is not on the globe, in words: the State's reason, or that GeoIP names only a region. */
export function unlocatedReason(node: MapNode): string {
  const reason = node.unlocated ?? "unknown"
  if (reason === "noPosition")
    return m.overview_unlocated_noPosition({ place: placeText(node) ?? "—" })
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
