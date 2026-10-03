import assert from "node:assert/strict"
import { test } from "node:test"

import { newPasswordProblem } from "../../lib/password.ts"
import { fragmentLink, heldSecret, secretFromFragment } from "./fragment.ts"

const invitation = `mwi_${"0".repeat(64)}`
const reset = `mwr_${"f".repeat(64)}`

test("a fragment yields only a secret of its own kind", () => {
  assert.equal(
    secretFromFragment(`#token=${invitation}`, "invitation"),
    invitation
  )
  assert.equal(
    secretFromFragment(`token=${invitation}`, "invitation"),
    invitation
  )
  assert.equal(secretFromFragment(`#token=${reset}`, "reset"), reset)
  assert.equal(secretFromFragment(`#token=${reset}`, "invitation"), null)
  assert.equal(secretFromFragment(`#token=${invitation}`, "reset"), null)
  assert.equal(secretFromFragment("", "invitation"), null)
  assert.equal(
    secretFromFragment(`#token=${invitation.toUpperCase()}`, "invitation"),
    null
  )
  assert.equal(secretFromFragment(`#token=${invitation}x`, "invitation"), null)
  assert.equal(
    secretFromFragment(
      `#token=${invitation}&token=${invitation}`,
      "invitation"
    ),
    null
  )
})

test("links round-trip through their fragment", () => {
  const link = new URL(fragmentLink("https://console.test", "reset", reset))
  assert.equal(link.pathname, "/reset-password")
  assert.equal(link.search, "")
  assert.equal(secretFromFragment(link.hash, "reset"), reset)
  const invite = new URL(
    fragmentLink("https://console.test", "invitation", invitation)
  )
  assert.equal(invite.pathname, "/invite")
  assert.equal(secretFromFragment(invite.hash, "invitation"), invitation)
})

test("a held secret is read once and dropped", () => {
  const held = heldSecret("invitation")
  assert.equal(held.current(), undefined)
  held.take(`#token=${invitation}`)
  assert.equal(held.current(), invitation)
  held.take("#token=nope")
  assert.equal(held.current(), null)
  held.drop()
  assert.equal(held.current(), undefined)
})

test("new passwords follow the controller's byte bounds", () => {
  assert.equal(newPasswordProblem("short", "short"), "short")
  // Bytes, not characters: three CJK characters are nine bytes.
  assert.equal(newPasswordProblem("密码密", "密码密"), "short")
  assert.equal(newPasswordProblem("密码密码", "密码密码"), null)
  assert.equal(newPasswordProblem("x".repeat(1025), "x".repeat(1025)), "long")
  assert.equal(newPasswordProblem("x".repeat(12), "x".repeat(13)), "mismatch")
  assert.equal(newPasswordProblem("x".repeat(1024), "x".repeat(1024)), null)
})
