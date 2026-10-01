import type { NodeSummary, PolicySnapshot } from "@meshway/control-client"

import { OwnedDraftRegistry } from "./draft-registry.ts"
import { policyKey, toPolicy, type PolicyDraft } from "./policy-model.ts"
import type { WorkingRule } from "./rule-form.ts"

export type { Owner } from "./draft-registry.ts"

/**
 * Unsaved routing work kept across in-app navigation: the outer draft, any
 * unapplied rule-builder edits, and the exact snapshot they are based on.
 * Restoring it restores the CAS base too, so a draft never silently moves
 * onto a newer revision. An entry exists while the draft differs from its
 * base or the builder holds unapplied edits.
 */
export type StoredRoutingDraft = {
  base: PolicySnapshot
  nodes: NodeSummary[]
  draft: PolicyDraft
  /** Unapplied rule-builder edits, or null. */
  builder: WorkingRule | null
  /** Next suffix for `new-<n>` rule keys, so restored keys never collide. */
  nextKey: number
  /** Last builder session ID, so restored and new sessions never collide. */
  nextSession: number
}

export class RoutingDraftRegistry extends OwnedDraftRegistry<StoredRoutingDraft> {
  /**
   * A save finished after its editor unmounted, with no newer editor mounted.
   * Kept work that is exactly what was saved, with no unapplied builder
   * edits, is no longer unsaved. Anything else stays on its old base: the
   * next save then meets the controller's revision check.
   */
  adoptOrphanSave(expectedRevision: number, sentKey: string) {
    const current = this.entry
    if (!this.unowned() || !current || current.builder !== null) return
    if (current.base.revision !== expectedRevision) return
    if (policyKey(toPolicy(current.draft)) === sentKey) this.set(null)
  }
}
