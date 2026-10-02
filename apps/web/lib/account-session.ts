import "server-only"

import { getAccountSession, listAccountPasswordResets } from "@meshway/control-client"
import { redirect } from "next/navigation"
import { connection } from "next/server"
import { cache } from "react"

import type { ConsoleIdentity } from "@/lib/account-model"
import { controllerUrl, describeError, isCredentialRejected } from "@/lib/controller"
import { EXPIRED_LOGIN_PATH, requireConsoleCredential, type ConsoleCredential } from "@/lib/session"

/**
 * Proves a console credential with the controller, according to its kind: a
 * user session through the user-session endpoint, the administrator token
 * through a bootstrap-only endpoint (so a user session can never pass as an
 * administrator). Throws the controller's error; 401 means unusable.
 */
export async function verifyCredential(credential: Pick<ConsoleCredential, "kind" | "token">): Promise<void> {
  if (credential.kind === "user") await getAccountSession(controllerUrl(), credential.token)
  else await listAccountPasswordResets(controllerUrl(), credential.token)
}

/**
 * The signed-in identity for one render, read live from the controller. A
 * rejected credential redirects to login; any other failure is reported, and
 * the console never falls back to claiming administrator authority.
 */
export const fetchConsoleIdentity = cache(async (): Promise<ConsoleIdentity> => {
  await connection()
  const credential = await requireConsoleCredential()
  if (credential.kind === "bootstrap") return { kind: "bootstrap" }
  try {
    const session = await getAccountSession(controllerUrl(), credential.token)
    return {
      kind: "user",
      email: session.user.email,
      memberships: session.memberships.map((membership) => ({ networkId: membership.network_id, role: membership.role })),
    }
  } catch (error) {
    if (!isCredentialRejected(error)) return { kind: "user-unverified", error: describeError(error, credential.token) }
  }
  redirect(EXPIRED_LOGIN_PATH)
})
