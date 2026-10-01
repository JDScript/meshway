import type { ConnectionNode, MeshAccessPolicy, MeshAccessSnapshot } from "@meshway/control-client"

import { accessKey, type WorkingGrant } from "./access-model.ts"
import { OwnedDraftRegistry } from "./draft-registry.ts"

export type { Owner } from "./draft-registry.ts"

/**
 * Unsaved access work kept across in-app navigation: the draft, any
 * unapplied grant-builder edits, and the exact snapshot they are based on
 * (the CAS base) together with the inventory read alongside it. An entry
 * exists while the draft differs from its base or the builder holds
 * unapplied edits.
 */
export type StoredAccessDraft = {
  base: MeshAccessSnapshot
  inventory: ConnectionNode[]
  draft: MeshAccessPolicy
  /** Unapplied grant-builder edits, or null. */
  builder: WorkingGrant | null
  /** Last builder session ID, so restored and new sessions never collide. */
  nextSession: number
}

export class AccessDraftRegistry extends OwnedDraftRegistry<StoredAccessDraft> {
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
    if (accessKey(current.draft) === sentKey) this.set(null)
  }
}
