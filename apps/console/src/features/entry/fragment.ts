// Join and reset links carry their one-time secret in the fragment
// (`/join#token=…`, `/reset-password#token=…`), which browsers never send to
// a server or put in a Referer. A page reads it once into memory and
// replaces the address without it; it then travels only in the body of the
// request that uses it. Its format is the controller's to check.

type FragmentKind = "join" | "reset"

const PATHS: Record<FragmentKind, string> = {
  join: "/join",
  reset: "/reset-password",
}

/** Long enough for any one-time token the controller issues; bounds what is kept. */
const MAX_TOKEN = 256

/** The secret in a fragment (with or without `#`), or null when it holds none. */
export function secretFromFragment(hash: string): string | null {
  const tokens = new URLSearchParams(hash.replace(/^#/, "")).getAll("token")
  if (tokens.length !== 1) return null
  const token = tokens[0] ?? ""
  return token.length > 0 &&
    token.length <= MAX_TOKEN &&
    /^[\x21-\x7e]+$/.test(token)
    ? token
    : null
}

/** The link a secret is handed over as; the inverse of `secretFromFragment`. */
export function fragmentLink(
  origin: string,
  kind: FragmentKind,
  secret: string
): string {
  return `${origin}${PATHS[kind]}#${new URLSearchParams({ token: secret })}`
}

/**
 * One page's secret, held in memory only. `take` reads a fragment (the
 * route then replaces the address without it); `current` is what was read:
 * `undefined` before any fragment, `null` for one without a secret.
 * `taken` counts the fragments read, so a page already showing can start
 * over when a second link changes only the fragment; `subscribe` hears it.
 *
 * Make one per page in a module the route and its component both import:
 * a route file's own module state is not shared with its split component.
 */
export function heldSecret() {
  let held: string | null | undefined
  let taken = 0
  const listeners = new Set<() => void>()
  return {
    take(hash: string) {
      held = secretFromFragment(hash)
      taken += 1
      for (const listener of listeners) listener()
    },
    current: () => held,
    taken: () => taken,
    subscribe(listener: () => void) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    /** After it has been used, or the person leaves it behind. */
    drop() {
      held = undefined
    },
  }
}
