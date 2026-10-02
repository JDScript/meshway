import { ControlApiError } from "@meshway/control-client"

import { currentSession, endSession, type Session } from "@/lib/session"

/** The controller serves this console, so the API is always same-origin. */
export const apiBase = () => window.location.origin

/** The controller rejected the credential itself: expired, revoked or logged out. */
export class SessionRejected extends Error {
  constructor() {
    super("session rejected")
    this.name = "SessionRejected"
  }
}

let onRejected: () => void = () => undefined

/** Where to go when the controller rejects this tab's session (set once by the router). */
export function handleSessionRejected(callback: () => void) {
  onRejected = callback
}

/**
 * Calls a generated operation with this tab's session. A 401 ends exactly
 * that session and hands over to the router; 403 and every other failure
 * propagate unchanged to the caller.
 */
export async function authed<T>(
  call: (base: string, token: string) => Promise<T>
): Promise<T> {
  const session = currentSession()
  if (!session) {
    onRejected()
    throw new SessionRejected()
  }
  try {
    return await call(apiBase(), session.token)
  } catch (error) {
    if (error instanceof ControlApiError && error.status === 401) {
      reject(session)
      throw new SessionRejected()
    }
    throw error
  }
}

function reject(session: Session) {
  endSession(session)
  onRejected()
}
