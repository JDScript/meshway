import {
  REMEMBERED_NETWORK,
  rememberedFor,
  rememberedValue,
} from "./remembered"

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
