import type { ManagedListenersSnapshot } from "@meshway/control-client"

import { nextPartStatus, type PartStatus } from "./node-status.ts"

/**
 * What the page knows about managed listeners: the snapshot it shows and
 * when that was read, plus the outcome of the latest read attempt. They are
 * tracked separately so a failed read keeps the last snapshot on screen as
 * a dated cache instead of silently passing it off as current.
 */
export type ManagedReadState = {
  snapshot: ManagedListenersSnapshot | null
  /** Read time of `snapshot` (0 when none). */
  snapshotAt: number
  /** Latest read attempt by read time; older completions never override it. */
  status: PartStatus
}

export type ManagedFreshness = "current" | "cached" | "failed"

export function initialManagedRead(
  read: { ok: true; data: ManagedListenersSnapshot; fetchedAt: number } | { ok: false; error: string; fetchedAt: number }
): ManagedReadState {
  return read.ok
    ? { snapshot: read.data, snapshotAt: read.fetchedAt, status: { at: read.fetchedAt, error: null } }
    : { snapshot: null, snapshotAt: 0, status: { at: read.fetchedAt, error: read.error } }
}

/**
 * A successful read (page, poll, save response or explicit reread). The
 * snapshot moves forward only: a newer revision, or the same revision read
 * later. The status moves only if this read is newer than the last attempt.
 */
export function recordManagedSuccess(state: ManagedReadState, snapshot: ManagedListenersSnapshot, at: number): ManagedReadState {
  const adopt =
    state.snapshot === null ||
    snapshot.revision > state.snapshot.revision ||
    (snapshot.revision === state.snapshot.revision && at >= state.snapshotAt)
  return {
    snapshot: adopt ? snapshot : state.snapshot,
    snapshotAt: adopt ? at : state.snapshotAt,
    status: nextPartStatus(state.status, at, null),
  }
}

export function recordManagedFailure(state: ManagedReadState, error: string, at: number): ManagedReadState {
  return { ...state, status: nextPartStatus(state.status, at, error) }
}

export function managedFreshness(state: ManagedReadState): ManagedFreshness {
  if (state.status.error === null && state.snapshot !== null) return "current"
  return state.snapshot === null ? "failed" : "cached"
}
