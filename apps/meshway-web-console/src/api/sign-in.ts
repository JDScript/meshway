import {
  login,
  logout,
  register,
  type SessionIssuedResponse,
} from "@meshway/control-client"

import { currentSession, endSession, startSession } from "@/state/session"

import { call, client } from "./client"

// Email and password go from the page to the controller only; neither is
// kept. The controller normalises the email and applies its own rules.

/** How a sign-in or registration ended; failures are thrown as `ApiFailure`. */
type Issued = "signed_in" | "clock"

/** Starts the issued session, after revoking one this tab still holds. */
async function adopt(issued: SessionIssuedResponse): Promise<Issued> {
  const previous = currentSession()
  if (previous) {
    await logout(client, { token: previous.token })
    endSession(previous)
  }
  return startSession({
    id: issued.session.id,
    token: issued.token,
    expiresAt: issued.session.expiresAt,
  })
    ? "signed_in"
    : "clock"
}

export async function signIn(email: string, password: string): Promise<Issued> {
  return adopt(await call(login(client, { email, password })))
}

export async function signUp(email: string, password: string): Promise<Issued> {
  return adopt(await call(register(client, { email, password })))
}

/** Revokes this tab's session on the controller, then forgets it here. */
export async function signOut(): Promise<"revoked" | "unconfirmed"> {
  const session = currentSession()
  if (!session) return "revoked"
  const result = await logout(client, { token: session.token })
  endSession(session)
  // A 401 means the controller no longer accepts it, which is the goal.
  const rejected =
    !result.ok && result.error.kind === "http" && result.error.status === 401
  return result.ok || rejected ? "revoked" : "unconfirmed"
}
