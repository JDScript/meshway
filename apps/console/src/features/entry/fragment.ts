import {
  zAcceptInvitationRequest,
  zRedeemPasswordResetRequest,
} from "@meshway/control-client/schemas"

// Invitation and reset links carry their one-time secret in the fragment
// (`/invite#token=mwi_…`, `/reset-password#token=mwr_…`), which browsers
// never send to a server or put in a Referer. A page reads it once into
// memory and replaces the address without it; it then travels only in the
// body of the request that uses it.

export type FragmentKind = "invitation" | "reset"

const schemas = {
  invitation: zAcceptInvitationRequest.shape.invitation_token,
  reset: zRedeemPasswordResetRequest.shape.reset_token,
}

/** The secret in a fragment (with or without `#`), or null when it holds none of this kind. */
export function secretFromFragment(
  hash: string,
  kind: FragmentKind
): string | null {
  const token = new URLSearchParams(hash.replace(/^#/, "")).getAll("token")
  if (token.length !== 1) return null
  const parsed = schemas[kind].safeParse(token[0])
  return parsed.success ? parsed.data : null
}

/** The link a secret is handed over as; the inverse of `secretFromFragment`. */
export function fragmentLink(
  origin: string,
  kind: FragmentKind,
  secret: string
): string {
  const path = kind === "invitation" ? "/invite" : "/reset-password"
  return `${origin}${path}#${new URLSearchParams({ token: secret })}`
}

/**
 * One page's secret, held in memory only. `take` reads a fragment (the
 * route then replaces the address without it); `current` is what was read:
 * `undefined` before any fragment, `null` for one without a valid secret.
 */
export function heldSecret(kind: FragmentKind) {
  let held: string | null | undefined
  return {
    take(hash: string) {
      held = secretFromFragment(hash, kind)
    },
    current: () => held,
    /** After it has been used, or the person leaves it behind. */
    drop() {
      held = undefined
    },
  }
}
