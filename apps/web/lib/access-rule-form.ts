import type { AccessAction, ConnectionNode, MeshAccessRule, TagSnapshot } from "@meshway/control-client"

import { suggestRuleId, validateAccess, type AccessIssue } from "./access-model.ts"

// Form state for the access rule builder. A rule ID is suggested from its
// action and endpoints until the administrator types one; it is never a
// random hex string.

export type AccessRuleForm = {
  id: string
  /** The administrator typed the ID; stop suggesting. */
  idEdited: boolean
  action: AccessAction
  source: string[]
  destination: string[]
  bidirectional: boolean
}

export type AccessRuleSession = {
  id: number
  mode: "new" | "edit"
  /** Index the rule is inserted at (new) or replaces (edit). */
  position: number
  initial: AccessRuleForm
  /** Unapplied contents to resume with (restored after navigating away). */
  working?: AccessRuleForm
}

/** An open builder holding edits that are not applied to the draft yet. */
export type WorkingAccessRule = { session: AccessRuleSession; form: AccessRuleForm }

export function newRuleForm(): AccessRuleForm {
  return { id: "", idEdited: false, action: "allow", source: [], destination: [], bidirectional: true }
}

export function toRuleForm(rule: MeshAccessRule): AccessRuleForm {
  return {
    id: rule.id,
    idEdited: true,
    action: rule.action,
    source: [...rule.source],
    destination: [...rule.destination],
    bidirectional: rule.bidirectional,
  }
}

export function toAccessRule(form: AccessRuleForm): MeshAccessRule {
  return {
    id: form.id.trim(),
    action: form.action,
    source: [...form.source],
    destination: [...form.destination],
    bidirectional: form.bidirectional,
  }
}

export function sameRuleForm(a: AccessRuleForm, b: AccessRuleForm): boolean {
  return JSON.stringify(toAccessRule(a)) === JSON.stringify(toAccessRule(b))
}

/** Applies a field change, keeping a suggested ID in step with the endpoints. */
export function updateRuleForm(
  form: AccessRuleForm,
  patch: Partial<Omit<AccessRuleForm, "idEdited">>,
  takenIds: readonly string[],
  inventory: readonly ConnectionNode[]
): AccessRuleForm {
  const next = { ...form, ...patch, idEdited: form.idEdited || patch.id !== undefined }
  if (next.idEdited) return next
  return { ...next, id: suggestRuleId(next, takenIds, inventory) }
}

/** Issues of the form against the other rules (IDs) and current references. */
export function validateRuleForm(
  form: AccessRuleForm,
  others: readonly MeshAccessRule[],
  inventory: readonly ConnectionNode[],
  tags: TagSnapshot
): AccessIssue[] {
  const rules = [...others, toAccessRule(form)]
  const index = rules.length - 1
  return validateAccess({ default_action: "reject", rules }, inventory, tags)
    .filter((issue) => issue.ruleIndex === index)
    .map((issue) => ({ ...issue, field: issue.field.replace(`rules[${index}].`, "") }))
}
