import assert from "node:assert/strict"
import test from "node:test"
import { encodeSessionCookie, isUserSessionToken, parseSessionCookie, signInKeyOf, userSessionMaxAge } from "./session-cookie.ts"

const user = `mws_${"a".repeat(64)}`
const sessionId = "0123456789abcdef0123456789abcdef"

test("cookies roundtrip with their kind; kinds cannot be swapped", () => {
  const value = encodeSessionCookie({ kind: "user", token: user, expiresAt: 2000, sessionId })
  assert.deepEqual(parseSessionCookie(value, 1000), { kind: "active", credential: { kind: "user", token: user, expiresAt: 2000, sessionId } })
  const admin = encodeSessionCookie({ kind: "bootstrap", token: "admin.token-value", expiresAt: 2000, sessionId })
  const parsed = parseSessionCookie(admin, 1000)
  assert.equal(parsed.kind === "active" && parsed.credential.token, "admin.token-value")
  // A user session never passes as the administrator token and vice versa.
  assert.throws(() => encodeSessionCookie({ kind: "bootstrap", token: user, expiresAt: 2000, sessionId }))
  assert.throws(() => encodeSessionCookie({ kind: "user", token: "admin-token", expiresAt: 2000, sessionId }))
  assert.equal(parseSessionCookie(`v2.bootstrap.2000.${sessionId}.${user}`, 1000).kind, "expired")
  assert.equal(parseSessionCookie(`v2.user.2000.${sessionId}.admin-token`, 1000).kind, "expired")
})

test("expiry is absolute and malformed or legacy cookies are unusable", () => {
  const value = encodeSessionCookie({ kind: "user", token: user, expiresAt: 2000, sessionId })
  assert.equal(parseSessionCookie(value, 2000).kind, "expired")
  for (const bad of ["", `v1.2000.${user}`, `v2.admin.2000.${sessionId}.${user}`, `v2.user.x.${sessionId}.${user}`, `v2.user.2000.short.${user}`]) {
    assert.equal(parseSessionCookie(bad, 1000).kind, "expired")
  }
})

test("user cookie lifetime never exceeds the controller's expiry", () => {
  assert.equal(userSessionMaxAge(1600, 1000), 600)
  assert.equal(userSessionMaxAge(1000, 1000), null)
  assert.equal(userSessionMaxAge(999, 1000), null)
  assert.ok(isUserSessionToken(user))
  assert.ok(!isUserSessionToken(`mws_${"A".repeat(64)}`))
})

test("the sign-in fence key commits to the credential, not only to the editable cookie fields", () => {
  const other = `mws_${"c".repeat(64)}`
  const key = signInKeyOf({ kind: "user", token: user, expiresAt: 2000, sessionId })
  assert.match(key, /^user:[0-9a-f]{32}$/)
  assert.ok(!key.includes(user.slice(4, 36)))
  // Same session ID (as in a forged cookie), different credential: a different key.
  assert.notEqual(signInKeyOf({ kind: "user", token: other, expiresAt: 2000, sessionId }), key)
  // Same credential, new sign-in ID: a different key.
  assert.notEqual(signInKeyOf({ kind: "user", token: user, expiresAt: 2000, sessionId: "f".repeat(32) }), key)
  // Expiry metadata is not part of the identity.
  assert.equal(signInKeyOf({ kind: "user", token: user, expiresAt: 9999, sessionId }), key)
})
