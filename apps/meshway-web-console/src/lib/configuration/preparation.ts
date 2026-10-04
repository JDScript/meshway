import type {
  PreparationKind,
  PreparationRequirement,
} from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

import { pointerSegments } from "./jsonc"

// What the controller still has to prepare for a configuration, as its
// check reports it: material it will generate on apply (`willGenerate`),
// and what it needs before a value can be used (`requirements`).

const SIMPLE: Record<Exclude<PreparationKind, object>, () => string> = {
  addressPoolV4: () => m.configuration_preparation_address_pool_v4(),
  addressPoolV6: () => m.configuration_preparation_address_pool_v6(),
  addressV4: () => m.configuration_preparation_address_v4(),
  addressV6: () => m.configuration_preparation_address_v6(),
  identityProof: () => m.configuration_preparation_identity_proof(),
  ruleSetContent: () => m.configuration_preparation_rule_set_content(),
}

const MATERIAL: Record<
  Extract<PreparationKind, object>["material"],
  () => string
> = {
  uuid: () => m.configuration_preparation_uuid(),
  password: () => m.configuration_preparation_password(),
  certificate: () => m.configuration_preparation_certificate(),
  privateKey: () => m.configuration_preparation_private_key(),
  certificateValidation: () =>
    m.configuration_preparation_certificate_validation(),
}

export const preparationLabel = (kind: PreparationKind): string =>
  typeof kind === "string" ? SIMPLE[kind]() : MATERIAL[kind.material]()

/** A requirement the controller refused or reported, in words. */
export function requirementText(requirement: PreparationRequirement): string {
  if (requirement.kind === "ruleSetContent")
    return m.configuration_requirement_rule_set({
      key: pointerSegments(requirement.path)?.[1] ?? requirement.path,
    })
  return m.configuration_requirement({
    kind: preparationLabel(requirement.kind),
  })
}

/** The listener a preparation is for (`/nodes/<id>/listeners/<key>…`), else null. */
export function listenerOf(
  path: string
): { nodeId: string; key: string } | null {
  const [section, nodeId, part, key] = pointerSegments(path) ?? []
  return section === "nodes" &&
    nodeId !== undefined &&
    part === "listeners" &&
    key !== undefined
    ? { nodeId, key }
    : null
}
