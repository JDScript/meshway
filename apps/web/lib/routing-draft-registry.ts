import type { NodeSummary, PolicySnapshot, RoutingPolicy } from "@meshway/control-client"

import { OwnedDraftRegistry } from "./draft-registry.ts"
import { hasUnsavedWork, type DocumentBase, type RuntimeSchema } from "./policy-document.ts"
import type { WorkingRule } from "./rule-form.ts"
import {
  restoreWorkspace,
  settleWorkspaceSave,
  storeWorkspace,
  type StoredWorkspace,
} from "./routing-workspace.ts"

export type { Owner } from "./draft-registry.ts"

export type RoutingView = "visual" | "jsonc"

/**
 * Unsaved routing work kept across in-app navigation: the shared JSONC
 * document (text, comments and the CAS base it was loaded from), rule keys,
 * any unapplied rule-builder edits, and which editor was open. Restoring it
 * restores the CAS base too, so a draft never silently moves onto a newer
 * revision. An entry exists while there is unsaved text or value, or the
 * builder holds unapplied edits.
 */
export type StoredRoutingDraft = {
  base: PolicySnapshot
  nodes: NodeSummary[]
  workspace: StoredWorkspace
  /** Unapplied rule-builder edits, or null. */
  builder: WorkingRule | null
  /** Next suffix for visual-editor `new-<n>` rule keys, so restored keys never collide. */
  nextKey: number
  /** Last builder session ID, so restored and new sessions never collide. */
  nextSession: number
  view: RoutingView
}

export class RoutingDraftRegistry extends OwnedDraftRegistry<StoredRoutingDraft> {
  /**
   * A save finished after its editor unmounted, with no newer editor mounted.
   * Kept work that is exactly what was sent (same edit sequence) moves onto
   * the saved base; it is dropped only when nothing unsaved remains (for
   * example comments the controller did not store keep it). Anything else
   * stays on its old base, and the next save meets the revision check.
   */
  adoptOrphanSave(
    expectedRevision: number,
    sentSeq: number,
    saved: { snapshot: PolicySnapshot; base: DocumentBase<RoutingPolicy> },
    schema: RuntimeSchema<RoutingPolicy>
  ) {
    const current = this.entry
    if (!this.unowned() || !current || current.builder !== null) return
    if (current.base.revision !== expectedRevision || current.workspace.document.seq !== sentSeq) return
    const workspace = settleWorkspaceSave(restoreWorkspace(current.workspace, schema), saved.base, sentSeq, schema)
    if (!hasUnsavedWork(workspace.document)) {
      this.set(null)
      return
    }
    this.set({ ...current, base: saved.snapshot, workspace: storeWorkspace(workspace) })
  }
}
