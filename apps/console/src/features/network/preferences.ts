// The network last opened in this browser: a preference only, read when the
// address names none.

const REMEMBERED_NETWORK = "meshway.network"

export function rememberedNetwork(): string | null {
  try {
    return window.localStorage.getItem(REMEMBERED_NETWORK)
  } catch {
    return null
  }
}

export function rememberNetwork(networkId: string) {
  try {
    window.localStorage.setItem(REMEMBERED_NETWORK, networkId)
  } catch {
    // A preference only.
  }
}
