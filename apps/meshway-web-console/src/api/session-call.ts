import type { ClientResult, ErrorResponse } from "@meshway/control-client"

import { currentSession, endSession } from "@/state/session"

import { ApiFailure, SessionRejected } from "./client"

/**
 * A call with this tab's session that keeps the page: a rejected session
 * ends here and the card asks for a sign-in again, so what the card holds
 * is not lost to the sign-in page.
 */
export async function withSession<T>(
  operation: (token: string) => Promise<ClientResult<T, ErrorResponse>>
): Promise<T> {
  const session = currentSession()
  if (!session) throw new SessionRejected()
  const settled = await operation(session.token)
  if (settled.ok) return settled.data
  if (settled.error.kind === "http" && settled.error.status === 401) {
    endSession(session)
    throw new SessionRejected()
  }
  throw new ApiFailure(settled.error)
}
