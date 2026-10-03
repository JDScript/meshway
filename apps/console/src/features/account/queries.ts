import {
  changeAccountPassword,
  ControlApiError,
  getAccountSession,
  revokeAccountSessions,
} from "@meshway/control-client"

import { apiBase, authed, SessionRejected } from "@/lib/api"
import { currentSession } from "@/lib/session"

// The account's own changes. Reads (the session, the networks) are shared
// with the shell in `features/network/queries.ts`.

type PasswordChange =
  | "changed"
  | "wrong"
  /** This tab's session is gone; the router has taken over. */
  | "rejected"

/**
 * Replaces the password. The controller answers a wrong current password
 * with 401, as it does a dead session, so this call does not go through
 * `authed` (which would sign out): a 401 is told apart by asking for the
 * session. Success revokes every session of the account, this one too;
 * any other failure is thrown, for `failureOf`.
 */
export async function changePassword(
  current: string,
  next: string
): Promise<PasswordChange> {
  const session = currentSession()
  if (!session) {
    await authed(getAccountSession).catch(() => undefined)
    return "rejected"
  }
  try {
    await changeAccountPassword(apiBase(), session.token, {
      current_password: current,
      new_password: next,
    })
    return "changed"
  } catch (error) {
    if (!(error instanceof ControlApiError) || error.status !== 401) throw error
    try {
      await authed(getAccountSession)
      return "wrong"
    } catch (check) {
      if (check instanceof SessionRejected) return "rejected"
      throw check
    }
  }
}

/** Revokes every browser session of this account, this tab's included. */
export const revokeAllSessions = (userId: string) =>
  authed((base, token) =>
    revokeAccountSessions(base, token, { user_id: userId })
  )
