// The rendered sign-in fence, shared by server and client (no secrets).
//
// Every page and form is rendered for one console sign-in, identified by a
// non-secret key "<kind>:<marker>". The server derives the marker from the
// cookie's credential and Web sign-in ID (lib/session.ts), so it cannot be
// reversed into the credential, and editing the cookie's unsigned fields
// cannot make one credential pass as another sign-in. Actions with
// effects carry the key they were rendered with; the server compares it with
// the sign-in of the cookie the request actually carries and refuses on any
// difference, so work rendered for account A is never carried out with
// account B's cookie after B signed in elsewhere in the same browser. It is
// an ownership fence, not a credential: the controller still authenticates
// the cookie's session and authorizes every call.

export type SignInKey = string

/** First argument of every network action: the network and sign-in it was rendered for. */
export type ActionScope = { readonly networkId: string; readonly signIn: SignInKey }

/** Form field carrying the rendered sign-in on standalone pages. */
export const SIGN_IN_FIELD = "sign_in"

const SIGN_IN_KEY = /^(user|bootstrap):[0-9a-f]{32}$/

export function isSignInKey(value: unknown): value is SignInKey {
  return typeof value === "string" && SIGN_IN_KEY.test(value)
}

/** Where a refused, stale action sends the browser. */
export const SESSION_CHANGED_PATH = "/session-changed"
