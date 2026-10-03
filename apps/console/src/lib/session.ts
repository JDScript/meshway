import { z } from "zod"

// The browser credential is the controller's opaque `mws_` user session,
// kept in localStorage and sent as a bearer header (docs/decisions.md). A
// stored session of another shape (an earlier console's) is ignored.
//
// Each tab copies the session into memory once, when it loads, and every
// request uses that copy. If another tab replaces or removes it, this tab
// reloads instead of silently continuing with a different sign-in.

const STORAGE_KEY = "meshway.session"

const zStoredSession = z.object({
  v: z.literal(2),
  /** The controller's session ID; keys tab state such as caches. Not a credential. */
  id: z.string().min(1).max(64),
  token: z.string().min(1).max(512),
  /** Unix milliseconds, the controller's absolute expiry. */
  expiresAt: z.number().int().positive(),
})

export type Session = z.infer<typeof zStoredSession>

function read(): Session | null {
  let raw: string | null
  try {
    raw = window.localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
  if (raw === null) return null
  try {
    const parsed = zStoredSession.safeParse(JSON.parse(raw))
    return parsed.success ? parsed.data : null
  } catch {
    return null
  }
}

let snapshot: Session | null = read()

const listeners = new Set<() => void>()

/**
 * Calls `listener` whenever this tab's sign-in starts or ends (what is kept
 * per sign-in, such as drafts, drops then). Returns the unsubscribe.
 */
export function onSessionChange(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function changed() {
  for (const listener of listeners) listener()
}

window.addEventListener("storage", (event) => {
  if (event.storageArea !== window.localStorage) return
  if (event.key !== STORAGE_KEY && event.key !== null) return
  if (read()?.id !== snapshot?.id) window.location.reload()
})

/** This tab's sign-in, or null when signed out or past its absolute expiry. */
export function currentSession(): Session | null {
  if (snapshot && snapshot.expiresAt <= Date.now()) return null
  return snapshot
}

/** A session as the controller issued it; `expiresAt` is RFC 3339. */
type IssuedSession = { id: string; token: string; expiresAt: string }

/** False when the controller's expiry has already passed (clock skew). */
export function startSession(issued: IssuedSession): boolean {
  const expiresAt = Date.parse(issued.expiresAt)
  if (!Number.isFinite(expiresAt) || expiresAt <= Date.now()) return false
  const next: Session = { v: 2, id: issued.id, token: issued.token, expiresAt }
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
  snapshot = next
  changed()
  return true
}

/**
 * Forgets `session` in this tab, and in storage only while storage still
 * holds that same sign-in: a late call never removes a newer one.
 */
export function endSession(session: Session) {
  if (read()?.id === session.id) window.localStorage.removeItem(STORAGE_KEY)
  if (snapshot?.id === session.id) {
    snapshot = null
    changed()
  }
}
