import type { ConnectionsSnapshot } from "@meshway/control-client"

import {
  draftKey,
  fromPlan,
  type ConnectionsDraft,
  type WorkingSheet,
} from "./connections-model.ts"

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

/** Handle of one mounted editor. Only the current owner may write or clear. */
export type Owner = { readonly token: number }

/**
 * In-memory store with explicit ownership. Each mounted editor claims the
 * store; asynchronous completions from an editor that has since unmounted
 * (or been replaced by a newer mount) can therefore never overwrite or clear
 * work that belongs to the newer editor.
 */
export class ConnectionsDraftRegistry {
  private entry: StoredConnectionsDraft | null = null
  private owner: number | null = null
  private tokens = 0
  private listeners = new Set<() => void>()

  read(): StoredConnectionsDraft | null {
    return this.entry
  }

  hasDraft(): boolean {
    return this.entry !== null
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  claim(): Owner {
    this.tokens += 1
    this.owner = this.tokens
    return { token: this.tokens }
  }

  release(owner: Owner) {
    if (this.owner === owner.token) this.owner = null
  }

  owns(owner: Owner | null): boolean {
    return owner !== null && this.owner === owner.token
  }

  /** Owner-only write of the editor's current state. */
  write(owner: Owner | null, value: StoredConnectionsDraft) {
    if (!this.owns(owner)) return
    this.set(value)
  }

  /** Owner-only clear (nothing unsaved, or an explicit discard / reload). */
  clear(owner: Owner | null) {
    if (!this.owns(owner)) return
    this.set(null)
  }

  /** Logout: drops everything regardless of owner. */
  reset() {
    this.set(null)
  }

  /**
   * A save finished after its editor unmounted, with no newer editor mounted.
   * If the kept work is still based on what was saved, move it onto the saved
   * snapshot: a pristine draft disappears, unapplied sheet edits survive on
   * the new base. Any other kept work is left untouched.
   */
  adoptOrphanSave(expectedRevision: number, sentKey: string, snapshot: ConnectionsSnapshot) {
    const current = this.entry
    if (this.owner !== null || !current) return
    if (current.base.revision !== expectedRevision || draftKey(current.draft) !== sentKey) return
    if (current.sheet === null) this.set(null)
    else this.set({ ...current, base: snapshot, draft: fromPlan(snapshot.configuration) })
  }

  private set(value: StoredConnectionsDraft | null) {
    const changed = (this.entry === null) !== (value === null)
    this.entry = value
    if (changed) for (const listener of this.listeners) listener()
  }
}
