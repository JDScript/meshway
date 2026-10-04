// Why the controller will not take a join link. It names why only once the
// token matches a real link (used, revoked, expired, or its issuer may no
// longer invite); any other token is `invalid`, and says nothing more.

export type JoinRefusal =
  "invalid" | "used" | "revoked" | "expired" | "issuer_unavailable"

/** The refusal a problem code states, or null when the code is not about the link. */
export function joinRefusal(code: string | null): JoinRefusal | null {
  switch (code) {
    case "invalid_join_link":
      return "invalid"
    case "join_link_used":
      return "used"
    case "join_link_revoked":
      return "revoked"
    case "join_link_expired":
      return "expired"
    case "join_link_issuer_unavailable":
      return "issuer_unavailable"
    default:
      return null
  }
}
