import type {
  Freshness,
  LinkEvidence,
  LinkPhase,
  LocationSource,
} from "@meshway/control-client"

import type { LinkLook } from "@/globe/scene"
import type { MapNode, Unlocated } from "@/lib/network-map"
import { m } from "@/paraglide/messages.js"

export const freshnessLabel: Record<Freshness, () => string> = {
  current: () => m.overview_freshness_current(),
  awaiting_report: () => m.overview_freshness_awaiting_report(),
  expired: () => m.overview_freshness_expired(),
  config_mismatch: () => m.overview_freshness_config_mismatch(),
  superseded: () => m.overview_freshness_superseded(),
  revoked: () => m.overview_freshness_revoked(),
}

/** A freshness's dot colour: only a current report is lit. */
export const freshnessColor = (freshness: Freshness) =>
  freshness === "current"
    ? "var(--vitality-live)"
    : freshness === "revoked" || freshness === "config_mismatch"
      ? "var(--destructive)"
      : "var(--vitality-quiet)"

export const phaseLabel: Record<LinkPhase, () => string> = {
  authenticated: () => m.overview_phase_authenticated(),
  preparing: () => m.overview_phase_preparing(),
  failed: () => m.overview_phase_failed(),
  closed: () => m.overview_phase_closed(),
}

export const evidenceLabel: Record<LinkEvidence, () => string> = {
  matched: () => m.overview_evidence_matched(),
  oneSided: () => m.overview_evidence_oneSided(),
  conflicting: () => m.overview_evidence_conflicting(),
  expired: () => m.overview_evidence_expired(),
}

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

const unlocatedText: Record<Exclude<Unlocated, "noPosition">, () => string> = {
  noObservation: () => m.overview_unlocated_noObservation(),
  observationExpired: () => m.overview_unlocated_observationExpired(),
  datasetUnavailable: () => m.overview_unlocated_datasetUnavailable(),
  notInDataset: () => m.overview_unlocated_notInDataset(),
  lookupFailed: () => m.overview_unlocated_lookupFailed(),
  ambiguousObservation: () => m.overview_unlocated_ambiguousObservation(),
  unknown: () => m.overview_unlocated_unknown(),
}

/** Why a node is not on the globe, in words. */
export function unlocatedReason(node: MapNode): string {
  if (node.unlocated === "noPosition")
    return m.overview_unlocated_noPosition({ place: placeText(node) ?? "—" })
  return unlocatedText[node.unlocated ?? "unknown"]()
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
