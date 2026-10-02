"use client"

import type { SignInKey } from "@/lib/action-scope"
import { discardSignInAction, signOutAction } from "@/lib/auth-actions"

/**
 * Signs out the sign-in `signIn` this page was rendered for. Step one revokes
 * it on the controller (and is refused if the browser already holds another
 * sign-in); step two is a fresh request that clears the cookie only if it is
 * still that sign-in. A full navigation then drops all client state.
 */
export async function signOutRendered(signIn: SignInKey, returnTo = "/"): Promise<void> {
  const { path } = await signOutAction(signIn, returnTo)
  await discardSignInAction(signIn)
  window.location.assign(path)
}
