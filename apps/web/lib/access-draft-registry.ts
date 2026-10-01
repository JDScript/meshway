import type { ConnectionNode, MeshAccessPolicy, MeshAccessSnapshot, TagSnapshot } from "@meshway/control-client"

import type { WorkingAccessRule } from "./access-rule-form.ts"
import { OwnedDraftRegistry } from "./draft-registry.ts"
import {
  hasUnsavedWork,
  restoreDocument,
  settleSaved,
  storeDocument,
  type DocumentBase,
  type RuntimeSchema,
  type StoredDocument,
} from "./policy-document.ts"

export type { Owner } from "./draft-registry.ts"

export type AccessView = "visual" | "jsonc"

/** The CAS base of an access snapshot, with the JSONC source the controller stored. */
export function accessBase(snapshot: MeshAccessSnapshot): DocumentBase<MeshAccessPolicy> {
  return { revision: snapshot.revision, value: snapshot.policy, source: snapshot.source_jsonc }
}

/**
 * Unsaved access work kept across in-app navigation: the shared JSONC
 * document (text, comments, CAS base), the inventory and tag snapshot read
 * with it, any unapplied rule-builder edits, and which editor was open.
 */
export type StoredAccessDraft = {
  base: MeshAccessSnapshot
  inventory: ConnectionNode[]
  tags: TagSnapshot
  document: StoredDocument<MeshAccessPolicy>
  builder: WorkingAccessRule | null
  /** Last builder session ID, so restored and new sessions never collide. */
  nextSession: number
  view: AccessView
}

export class AccessDraftRegistry extends OwnedDraftRegistry<StoredAccessDraft> {
  /**
   * A save finished after its editor unmounted, with no newer editor mounted.
   * Kept work that is exactly what was sent (same edit sequence) moves onto
   * the saved base and is dropped when nothing unsaved remains. Anything else
   * stays on its old base, and the next save meets the revision check.
   */
  adoptOrphanSave(expectedRevision: number, sentSeq: number, saved: MeshAccessSnapshot, schema: RuntimeSchema<MeshAccessPolicy>) {
    const current = this.entry
    if (!this.unowned() || !current || current.builder !== null) return
    if (current.base.revision !== expectedRevision || current.document.seq !== sentSeq) return
    const document = settleSaved(restoreDocument(current.document, schema), accessBase(saved), sentSeq, schema)
    if (!hasUnsavedWork(document)) {
      this.set(null)
      return
    }
    this.set({ ...current, base: saved, document: storeDocument(document) })
  }
}
