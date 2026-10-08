"use client"

import {
  ControlClient,
  exchangeDelegation,
  logout,
} from "@meshway/control-client"
import { useSyncExternalStore } from "react"
import { z } from "zod"

import { localized, type Lang } from "./i18n"
import { consoleUrl } from "./site"

// Trying the API from the reference: a delegated session from the console
// (authorization code with PKCE). The site sends people to the console,
// which asks for consent and returns a one-time code here; the code and the
// verifier kept for the round trip become a one-hour session. The session's
// token is held in memory only: a reload or a change of language drops it
// and authorizing again is one click while the console is signed in.

/** The client the server knows this site by (`--delegation-client docs=<origin>`). */
const CLIENT = "docs"

/** The verifier, state and page of a sign-in on its way through the console; for that trip only. */
const PENDING = "meshway.try-it.pending"

const zPending = z.object({
  verifier: z.string().regex(/^[A-Za-z0-9_-]{43}$/),
  state: z.string().regex(/^[A-Za-z0-9_-]{22}$/),
  returnTo: z.string().startsWith("/"),
  redirectUri: z.string(),
})

export type TrySession = {
  token: string
  email: string
  /** Epoch milliseconds. */
  expiresAt: number
}

let session: TrySession | null = null
const listeners = new Set<() => void>()

function set(next: TrySession | null) {
  session = next
  for (const listener of listeners) listener()
}

/** The delegated session, while it lasts. */
export function trySession(now = Date.now()): TrySession | null {
  return session && session.expiresAt > now ? session : null
}

export function useTrySession(): TrySession | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => session,
    () => null
  )
}

/** The server's API, where the console is served (the console's own origin). */
export const apiClient = () => new ControlClient({ baseUrl: `${consoleUrl}/` })

const base64url = (bytes: Uint8Array) =>
  btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "")

const random = (length: number) =>
  base64url(crypto.getRandomValues(new Uint8Array(length)))

/** Sends the reader to the console to authorize this site, returning to this page. */
export async function startSignIn(lang: Lang) {
  const verifier = random(32)
  const state = random(16)
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(verifier)
  )
  const redirectUri = new URL(
    localized(lang, "/auth/callback"),
    window.location.origin
  ).href
  sessionStorage.setItem(
    PENDING,
    JSON.stringify({
      verifier,
      state,
      returnTo: `${window.location.pathname}${window.location.hash}`,
      redirectUri,
    })
  )
  const target = new URL("/authorize-site", consoleUrl)
  target.searchParams.set("client", CLIENT)
  target.searchParams.set("redirect_uri", redirectUri)
  target.searchParams.set("code_challenge", base64url(new Uint8Array(digest)))
  target.searchParams.set("code_challenge_method", "S256")
  target.searchParams.set("state", state)
  window.location.assign(target.href)
}

export type CallbackOutcome =
  | { kind: "signed-in"; returnTo: string }
  | { kind: "failed"; reason: "state" | "exchange" | "network" }

/**
 * Finishes a sign-in returning from the console: the code is exchanged
 * with the kept verifier, once. The address loses its code first, so it is
 * neither reused nor left in the history.
 */
export async function finishSignIn(): Promise<CallbackOutcome> {
  const params = new URLSearchParams(window.location.search)
  window.history.replaceState(null, "", window.location.pathname)
  const raw = sessionStorage.getItem(PENDING)
  sessionStorage.removeItem(PENDING)
  let pending: z.output<typeof zPending>
  try {
    pending = zPending.parse(JSON.parse(raw ?? "null"))
  } catch {
    return { kind: "failed", reason: "state" }
  }
  const code = params.get("code")
  if (!code || params.get("state") !== pending.state)
    return { kind: "failed", reason: "state" }
  const result = await exchangeDelegation(apiClient(), {
    client: CLIENT,
    code,
    codeVerifier: pending.verifier,
    redirectUri: pending.redirectUri,
  })
  if (!result.ok)
    return {
      kind: "failed",
      reason: result.error.kind === "http" ? "exchange" : "network",
    }
  const { token, user, session: issued } = result.data
  set({
    token,
    email: user.email,
    expiresAt: Date.parse(issued.expiresAt),
  })
  return { kind: "signed-in", returnTo: pending.returnTo }
}

/** Ends the delegated session at the server, then forgets it. */
export async function signOutOfTry() {
  const current = session
  set(null)
  if (current) await logout(apiClient(), { token: current.token })
}

/** Forgets a session the server no longer takes (a 401). */
export const dropTrySession = () => set(null)
