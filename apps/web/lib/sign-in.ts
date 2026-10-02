import "server-only"

import { logoutAccount } from "@meshway/control-client"

import { controllerUrl } from "@/lib/controller-config"
import { readSession } from "@/lib/session"

/** One string field submitted exactly once; anything else is empty. */
export function formField(formData: FormData, name: string): string {
  const raw = formData.getAll(name)
  return raw.length === 1 && typeof raw[0] === "string" ? raw[0] : ""
}

/**
 * Before a new sign-in replaces the cookie, revoke a still-present user
 * session so switching accounts never leaves the previous one live. Best
 * effort: the new sign-in proceeds either way.
 */
export async function revokePreviousUserSession() {
  const session = await readSession()
  if (session.kind !== "active" || session.credential.kind !== "user") return
  try {
    await logoutAccount(controllerUrl(), session.credential.token)
  } catch {
    // Already invalid, or the controller is unreachable; it expires on its own.
  }
}
