import { z } from "zod"

// The policy page in the address; values at their default are left out.
//
//   /networks/<network>/policy                           the layers
//   /networks/<network>/policy/<layer>?rule=&section=     one layer's workspace
//
// `rule` is the rule a link points at (by its ID), `section` the egress
// part shown.

export const POLICY_LAYERS = [
  "connections",
  "grants",
  "mesh",
  "egress",
  "source",
] as const
export type PolicyLayer = (typeof POLICY_LAYERS)[number]

export const isPolicyLayer = (value: string): value is PolicyLayer =>
  POLICY_LAYERS.some((layer) => layer === value)

export const EGRESS_SECTIONS = [
  "public",
  "profiles",
  "automatic",
  "assignments",
] as const
export type EgressSection = (typeof EGRESS_SECTIONS)[number]

export const zPolicyWorkspaceSearch = z.object({
  rule: z.string().min(1).max(64).optional().catch(undefined),
  section: z.enum(EGRESS_SECTIONS).optional().catch(undefined),
})

export type PolicyWorkspaceView = {
  rule: string | null
  section: EgressSection
}
