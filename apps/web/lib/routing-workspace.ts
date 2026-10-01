import type { PolicySnapshot, RoutingPolicy } from "@meshway/control-client"

import { jsonEqual } from "./jsonc.ts"
import {
  applyVisual,
  discardInvalidText,
  editText,
  isValid,
  openDocument,
  restoreDocument,
  revertToBase,
  semanticProblems,
  settleSaved,
  storeDocument,
  type DocumentBase,
  type DocumentProblem,
  type PolicyDocument,
  type RuntimeSchema,
  type StoredDocument,
} from "./policy-document.ts"
import {
  fromPolicy,
  ruleKey,
  toPolicy,
  toRule,
  type LocalIssue,
  type PolicyDraft,
  type RuleDraft,
} from "./policy-model.ts"

// The one routing draft shared by the visual editor and the JSONC editor.
// The JSONC document holds the text and the last valid policy; `keys` gives
// each of its rules a stable identity across reorders and text edits, so the
// visual editor can still tell new, modified and removed rules apart from the
// saved base (`saved-<index>` keys, as in `policy-model`).

export type RoutingWorkspace = {
  document: PolicyDocument<RoutingPolicy>
  /** One key per rule of `document.value`, in order. */
  keys: string[]
  /** Next suffix for `text-<n>` keys of rules that first appeared in the text. */
  nextKey: number
}

/**
 * The CAS base of a routing snapshot. The current contract stores no JSONC
 * source, so the canonical rendering is shown; once snapshots carry
 * `source_jsonc`, this is the one place that reads it.
 */
export function routingBase(snapshot: PolicySnapshot): DocumentBase<RoutingPolicy> {
  return { revision: snapshot.revision, value: snapshot.policy, source: null }
}

const savedKeys = (policy: RoutingPolicy) => policy.rules.map((_, index) => `saved-${index}`)

export function openWorkspace(
  base: DocumentBase<RoutingPolicy>,
  schema: RuntimeSchema<RoutingPolicy>,
  nextKey = 0
): RoutingWorkspace {
  return { document: openDocument(base, schema), keys: savedKeys(base.value), nextKey }
}

/** The visual editor's model of the current (last valid) policy. */
export function workspaceDraft(workspace: RoutingWorkspace): PolicyDraft {
  const draft = fromPolicy(workspace.document.value)
  return {
    ...draft,
    rules: draft.rules.map((rule, index) => {
      const key = workspace.keys[index] ?? rule.key
      return {
        ...rule,
        key,
        destinations: rule.destinations.map((selector, selectorIndex) => ({
          ...selector,
          key: `${key}-d${selectorIndex}`,
        })),
      }
    }),
  }
}

export type WorkspaceEdit =
  | { ok: true; workspace: RoutingWorkspace }
  | { ok: false; reason: "invalid_buffer" | "invalid_value"; problems: DocumentProblem[] }

/** A visual edit: the next draft replaces the policy, comments kept elsewhere. */
export function applyDraft(
  workspace: RoutingWorkspace,
  next: PolicyDraft,
  schema: RuntimeSchema<RoutingPolicy>
): WorkspaceEdit {
  const result = applyVisual(workspace.document, toPolicy(next), schema)
  if (!result.ok) return result
  return { ok: true, workspace: { ...workspace, document: result.document, keys: next.rules.map((rule) => rule.key) } }
}

/**
 * Keys for rules that came from text: a rule keeps the key of the previous
 * rule with the same ID (IDs are what an administrator recognises), else of
 * an identical previous rule, else it is new.
 */
function rekey(previous: readonly RuleDraft[], next: RoutingPolicy, nextKey: number) {
  const used = new Set<string>()
  let counter = nextKey
  const keys = next.rules.map((rule) => {
    const byId = previous.find((candidate) => !used.has(candidate.key) && candidate.id.trim() === rule.id)
    const same =
      byId ?? previous.find((candidate) => !used.has(candidate.key) && ruleKey(toRule(candidate)) === ruleKey(rule))
    if (same) {
      used.add(same.key)
      return same.key
    }
    return `text-${counter++}`
  })
  return { keys, nextKey: counter }
}

/** A text edit. The visual model follows only when the text is valid. */
export function editWorkspaceText(
  workspace: RoutingWorkspace,
  text: string,
  schema: RuntimeSchema<RoutingPolicy>
): RoutingWorkspace {
  const document = editText(workspace.document, text, schema)
  if (document === workspace.document) return workspace
  if (jsonEqual(document.value, workspace.document.value)) return { ...workspace, document }
  const { keys, nextKey } = rekey(workspaceDraft(workspace).rules, document.value, workspace.nextKey)
  return { document, keys, nextKey }
}

export function discardWorkspaceText(
  workspace: RoutingWorkspace,
  schema: RuntimeSchema<RoutingPolicy>
): RoutingWorkspace {
  return { ...workspace, document: discardInvalidText(workspace.document, schema) }
}

export function revertWorkspace(workspace: RoutingWorkspace, schema: RuntimeSchema<RoutingPolicy>): RoutingWorkspace {
  return { ...workspace, document: revertToBase(workspace.document, schema), keys: savedKeys(workspace.document.base.value) }
}

/**
 * A save of the work at `sentSeq` succeeded. When nothing was edited since,
 * the draft now is the saved policy and its rules are the base's rules.
 */
export function settleWorkspaceSave(
  workspace: RoutingWorkspace,
  saved: DocumentBase<RoutingPolicy>,
  sentSeq: number,
  schema: RuntimeSchema<RoutingPolicy>
): RoutingWorkspace {
  const document = settleSaved(workspace.document, saved, sentSeq, schema)
  if (workspace.document.seq !== sentSeq) return { ...workspace, document }
  return { ...workspace, document, keys: savedKeys(saved.value) }
}

/** Editor-side issues placed on the JSONC buffer (rule issues under `rules[i]`). */
export function localIssueProblems(workspace: RoutingWorkspace, issues: readonly LocalIssue[]): DocumentProblem[] {
  if (!isValid(workspace.document)) return []
  return semanticProblems(
    workspace.document,
    issues.map((issue) => {
      const index = issue.ruleKey === null ? -1 : workspace.keys.indexOf(issue.ruleKey)
      return { field: index >= 0 ? `rules[${index}].${issue.field}` : issue.field, message: issue.message }
    })
  )
}

export type StoredWorkspace = { document: StoredDocument<RoutingPolicy>; keys: string[]; nextKey: number }

export function storeWorkspace(workspace: RoutingWorkspace): StoredWorkspace {
  return { document: storeDocument(workspace.document), keys: workspace.keys, nextKey: workspace.nextKey }
}

export function restoreWorkspace(stored: StoredWorkspace, schema: RuntimeSchema<RoutingPolicy>): RoutingWorkspace {
  const document = restoreDocument(stored.document, schema)
  const keys =
    stored.keys.length === document.value.rules.length ? stored.keys : document.value.rules.map((_, index) => `restored-${index}`)
  return { document, keys, nextKey: stored.nextKey }
}
