import type { ManagedListenerSettings } from "@meshway/control-client"

import type { ManagedListenerForm } from "./managed-listener-model.ts"

/**
 * One unsaved managed-listener form. `baseRevision` is the network CAS the
 * form was opened on; it only moves on an explicit rebase after a conflict.
 * `original` is the saved settings being edited (null when creating).
 */
export type ManagedListenerDraft = {
  key: string
  kind: "create" | "edit" | "enroll"
  /** Owning node; null for the add-node flow, where enrollment binds it. */
  nodeId: string | null
  baseRevision: number | null
  original: ManagedListenerSettings | null
  /** What the form held when opened; the draft is kept while `form` differs. */
  initial: ManagedListenerForm
  form: ManagedListenerForm
  /** The ID still follows the preset's suggestion (create / enroll only). */
  idFollows: boolean
  /** The node page reopens its sheet on return. */
  open: boolean
}

export function createKey(nodeId: string): string {
  return `node:${nodeId}:create`
}

export function editKey(nodeId: string, listenerId: string): string {
  return `node:${nodeId}:edit:${listenerId}`
}

export const ENROLL_KEY = "enroll"

/**
 * In-memory, per-network store of unsaved managed-listener forms, keyed by
 * where they were opened. Only dirty forms are kept. Completions clear a
 * form only if it is still the exact object they sent, so a slow save never
 * drops edits made after it. Nothing is persisted; logout resets it.
 */
export class ManagedListenerDraftRegistry {
  private entries = new Map<string, ManagedListenerDraft>()
  private listeners = new Set<() => void>()
  private version = 0

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  /** Changes on every write, for useSyncExternalStore. */
  snapshot(): number {
    return this.version
  }

  get(key: string): ManagedListenerDraft | undefined {
    return this.entries.get(key)
  }

  forNode(nodeId: string): ManagedListenerDraft[] {
    return [...this.entries.values()].filter((draft) => draft.nodeId === nodeId)
  }

  hasDraft(): boolean {
    return this.entries.size > 0
  }

  put(draft: ManagedListenerDraft) {
    this.entries.set(draft.key, draft)
    this.notify()
  }

  remove(key: string) {
    if (this.entries.delete(key)) this.notify()
  }

  /** Clears only if the kept draft is still `sent` (identity), e.g. after a successful save. */
  settle(key: string, sent: ManagedListenerDraft) {
    if (this.entries.get(key) === sent) this.remove(key)
  }

  reset() {
    if (this.entries.size === 0) return
    this.entries.clear()
    this.notify()
  }

  private notify() {
    this.version += 1
    for (const listener of this.listeners) listener()
  }
}
