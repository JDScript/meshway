import assert from "node:assert/strict"
import test from "node:test"
import { invitationPath, invitationTokenFromHash, passwordProblem, roleIn } from "./account-model.ts"

const token = `mwi_${"b".repeat(64)}`

test("invitation secret stays in the fragment and roundtrips", () => {
  const path = invitationPath(token)
  assert.ok(path.startsWith("/invite#"))
  assert.equal(new URL(path, "http://web.test").search, "")
  assert.equal(invitationTokenFromHash(new URL(path, "http://web.test").hash), token)
  assert.equal(invitationTokenFromHash("#token=mws_" + "b".repeat(64)), null)
  assert.equal(invitationTokenFromHash(""), null)
  assert.throws(() => invitationPath("mwi_short"))
})

test("roles come only from live memberships; the administrator token has none", () => {
  const identity = { kind: "user" as const, email: "a@b.test", memberships: [{ networkId: "n1", role: "member" as const }] }
  assert.equal(roleIn(identity, "n1"), "member")
  assert.equal(roleIn(identity, "n2"), null)
  assert.equal(roleIn({ kind: "bootstrap" }, "n1"), null)
})

test("password length is counted in UTF-8 bytes", () => {
  assert.ok(passwordProblem("short"))
  assert.equal(passwordProblem("密码密码"), null)
  assert.ok(passwordProblem("x".repeat(1025)))
})
