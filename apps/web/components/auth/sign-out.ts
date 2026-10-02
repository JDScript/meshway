"use client"

import type { SignInKey } from "@/lib/action-scope"
import { signOutAction } from "@/lib/auth-actions"

/**
 * Signs out the sign-in `signIn` this page was rendered for: revoked on the
 * controller and its own credential cookie deleted (refused if the browser
 * already holds another sign-in). A full navigation then drops client state.
 */
export async function signOutRendered(signIn: SignInKey, returnTo = "/"): Promise<void> {
  const { path } = await signOutAction(signIn, returnTo)
  window.location.assign(path)
}
