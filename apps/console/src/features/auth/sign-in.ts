import {
  ControlApiError,
  loginAccount,
  logoutAccount,
} from "@meshway/control-client"
import { zLoginRequest } from "@meshway/control-client/schemas"

import { apiBase } from "@/lib/api"
import { currentSession, endSession, startSession } from "@/lib/session"

export type SignInOutcome =
  "signed_in" | "missing" | "invalid" | "limited" | "unavailable" | "clock"

/**
 * Email and password go from this page to the controller only; neither is
 * kept. A previous session still held by this tab is revoked first.
 */
export async function signIn(
  email: string,
  password: string
): Promise<SignInOutcome> {
  const parsed = zLoginRequest.safeParse({
    email: email.trim().slice(0, 254),
    password,
  })
  if (!parsed.success || !parsed.data.email || !parsed.data.password)
    return "missing"
  let response
  try {
    response = await loginAccount(apiBase(), parsed.data)
  } catch (error) {
    if (error instanceof ControlApiError) {
      // 400/401/422 all mean "these credentials do not sign in"; which part
      // was wrong is deliberately not distinguished.
      if ([400, 401, 422].includes(error.status)) return "invalid"
      if (error.status === 429) return "limited"
    }
    return "unavailable"
  }
  const previous = currentSession()
  if (previous) {
    await logoutAccount(apiBase(), previous.token).catch(() => undefined)
    endSession(previous)
  }
  return startSession(response.session_token, response.expires_at)
    ? "signed_in"
    : "clock"
}

/** Revokes this tab's session on the controller, then forgets it here. */
export async function signOut(): Promise<"revoked" | "unconfirmed"> {
  const session = currentSession()
  if (!session) return "revoked"
  let outcome: "revoked" | "unconfirmed" = "revoked"
  try {
    await logoutAccount(apiBase(), session.token)
  } catch (error) {
    // 401: the controller no longer accepts it, which is the goal.
    if (!(error instanceof ControlApiError && error.status === 401))
      outcome = "unconfirmed"
  }
  endSession(session)
  return outcome
}
