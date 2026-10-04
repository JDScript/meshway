// The network last opened in this browser, a preference only: `/` opens it
// when the account can still open it, else the first one listed. It is kept
// with the account it belongs to, so another account signing in on this
// browser starts from its own first network.

import { z } from "zod"

export const REMEMBERED_NETWORK = "meshway.network"

const zRemembered = z.object({
  v: z.literal(1),
  user: z.string(),
  network: z.string(),
})

/** The remembered network for `userId` in a stored value, or null. */
export function rememberedFor(
  stored: string | null,
  userId: string
): string | null {
  if (stored === null) return null
  let value: unknown
  try {
    value = JSON.parse(stored)
  } catch {
    return null
  }
  const parsed = zRemembered.safeParse(value)
  return parsed.success && parsed.data.user === userId
    ? parsed.data.network
    : null
}

/** The value to store for `userId` opening `networkId`. */
export function rememberedValue(userId: string, networkId: string): string {
  return JSON.stringify({
    v: 1,
    user: userId,
    network: networkId,
  } satisfies z.infer<typeof zRemembered>)
}

/**
 * Where `/` goes from the first page of the list: the remembered network
 * when it is listed there; when it may be on a later page, ask for it
 * (`check`) and fall back to the first; else the first, or nothing.
 */
export function landing(
  listed: readonly { id: string }[],
  more: boolean,
  remembered: string | null
): { open: string | null; check: string | null } {
  const first = listed[0]?.id ?? null
  if (remembered === null) return { open: first, check: null }
  if (listed.some((network) => network.id === remembered))
    return { open: remembered, check: null }
  return { open: first, check: more ? remembered : null }
}

/** The network `userId` last opened in this browser, or null. */
export function rememberedNetwork(userId: string): string | null {
  try {
    return rememberedFor(
      window.localStorage.getItem(REMEMBERED_NETWORK),
      userId
    )
  } catch {
    return null
  }
}

export function rememberNetwork(userId: string, networkId: string) {
  try {
    window.localStorage.setItem(
      REMEMBERED_NETWORK,
      rememberedValue(userId, networkId)
    )
  } catch {
    // A preference only.
  }
}

/** After leaving `networkId`: `/` no longer tries to open it for `userId`. */
export function forgetNetwork(userId: string, networkId: string) {
  if (rememberedNetwork(userId) !== networkId) return
  try {
    window.localStorage.removeItem(REMEMBERED_NETWORK)
  } catch {
    // A preference only.
  }
}
