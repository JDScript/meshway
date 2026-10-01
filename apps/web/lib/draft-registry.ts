/** Handle of one mounted editor. Only the current owner may write or clear. */
export type Owner = { readonly token: number }

/**
 * In-memory store for one editor's unsaved work, with explicit ownership.
 * Each mounted editor claims the store; asynchronous completions from an
 * editor that has since unmounted (or been replaced by a newer mount) can
 * therefore never overwrite or clear work that belongs to the newer editor.
 * Listeners are notified only when the store flips between empty and kept.
 */
export class OwnedDraftRegistry<T> {
  protected entry: T | null = null
  private owner: number | null = null
  private tokens = 0
  private listeners = new Set<() => void>()

  read(): T | null {
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

  /** No editor is mounted (a completion can only be an orphan). */
  protected unowned(): boolean {
    return this.owner === null
  }

  /** Owner-only write of the editor's current state. */
  write(owner: Owner | null, value: T) {
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

  protected set(value: T | null) {
    const changed = (this.entry === null) !== (value === null)
    this.entry = value
    if (changed) for (const listener of this.listeners) listener()
  }
}
