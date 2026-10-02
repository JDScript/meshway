import { ControlApiError } from "@meshway/control-client"

import { apiBase, SessionRejected } from "@/lib/api"
import { currentSession, endSession } from "@/lib/session"

/**
 * `authed` for pages outside the shell that must come back after a fresh
 * sign-in (a device request, an invitation held in memory): a rejected
 * session ends exactly that session and hands over to `onRejected`, which
 * sends the person to the sign-in with this page as the way back.
 */
export async function withSession<T>(
  call: (base: string, token: string) => Promise<T>,
  onRejected: () => void
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
      endSession(session)
      onRejected()
      throw new SessionRejected()
    }
    throw error
  }
}
