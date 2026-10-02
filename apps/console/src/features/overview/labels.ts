import { m } from "@/paraglide/messages.js"

import type { LinkLook, Vitality } from "../globe/scene"
import type { RuntimeFreshness } from "../topology/runtime"

export const vitalityLabel: Record<Vitality, () => string> = {
  live: () => m.vitality_live(),
  contact: () => m.vitality_contact(),
  quiet: () => m.vitality_quiet(),
}

export const lookLabel: Record<LinkLook, () => string> = {
  confirmed: () => m.link_confirmed(),
  one_sided: () => m.link_one_sided(),
  degraded: () => m.link_degraded(),
  configured: () => m.link_configured(),
}

export const freshnessLabel: Record<RuntimeFreshness, () => string> = {
  unreported: () => m.report_unreported(),
  awaiting: () => m.report_awaiting(),
  withdrawn: () => m.report_withdrawn(),
  outdated: () => m.report_outdated(),
  expired: () => m.report_expired(),
  fresh: () => m.report_fresh(),
}

/** Token colour of each link look, matching the globe. */
export const lookColor: Record<LinkLook, string> = {
  confirmed: "var(--link-confirmed)",
  one_sided: "var(--link-one-sided)",
  degraded: "var(--link-degraded)",
  configured: "var(--link-configured)",
}

export const vitalityColor: Record<Vitality, string> = {
  live: "var(--brand)",
  contact: "var(--foreground)",
  quiet: "var(--muted-foreground)",
}

export function rttText(ms: number | null): string | null {
  if (ms === null) return null
  return ms === 0 ? m.node_rtt_sub() : m.node_rtt({ ms: String(ms) })
}
