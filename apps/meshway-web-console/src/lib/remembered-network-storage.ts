import {
  REMEMBERED_NETWORK,
  rememberedFor,
  rememberedValue,
} from "./remembered-network"

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
