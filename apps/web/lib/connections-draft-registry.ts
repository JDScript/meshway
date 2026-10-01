import type { ConnectionsSnapshot } from "@meshway/control-client"

import {
  draftKey,
  fromPlan,
  type ConnectionsDraft,
  type WorkingSheet,
} from "./connections-model.ts"
import { OwnedDraftRegistry } from "./draft-registry.ts"

export type { Owner } from "./draft-registry.ts"

/**
 * Unsaved connections work kept across in-app navigation: the outer draft,
 * any unapplied sheet edits, and the exact snapshot they are based on (the
 * CAS base). An entry exists while either the draft differs from its base or
 * a sheet holds unapplied edits.
 */
export type StoredConnectionsDraft = {
  base: ConnectionsSnapshot
  draft: ConnectionsDraft
  /** Unapplied listener / candidate sheet edits, or null. */
  sheet: WorkingSheet | null
  /** Next suffix for `new-<n>` item keys, so restored keys never collide. */
  nextKey: number
  /** Last sheet session ID, so restored and new sessions never collide. */
  nextSession: number
}

/** One draft for the whole plan, shared by the global and per-node editors. */
export class ConnectionsDraftRegistry extends OwnedDraftRegistry<StoredConnectionsDraft> {
  /**
   * A save finished after its editor unmounted, with no newer editor mounted.
   * If the kept work is still based on what was saved, move it onto the saved
   * snapshot: a pristine draft disappears, unapplied sheet edits survive on
   * the new base. Any other kept work is left untouched.
   */
  adoptOrphanSave(expectedRevision: number, sentKey: string, snapshot: ConnectionsSnapshot) {
    const current = this.entry
    if (!this.unowned() || !current) return
    if (current.base.revision !== expectedRevision || draftKey(current.draft) !== sentKey) return
    if (current.sheet === null) this.set(null)
    else this.set({ ...current, base: snapshot, draft: fromPlan(snapshot.configuration) })
  }
}
