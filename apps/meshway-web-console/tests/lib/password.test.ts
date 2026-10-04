import assert from "node:assert/strict"
import { test } from "vitest"

import { codepoints, newPasswordProblem, utf8Bytes } from "@/lib/password"

const rules = {
  lengthUnit: "unicodeCodepoints" as const,
  minCodepoints: 12,
  maxCodepoints: 16,
  maxUtf8Bytes: 40,
}

test("lengths are Unicode code points and UTF-8 bytes, not UTF-16 units", () => {
  const astral = "😀".repeat(3)
  assert.equal(astral.length, 6)
  assert.equal(codepoints(astral), 3)
  assert.equal(utf8Bytes(astral), 12)
  assert.equal(utf8Bytes("é"), 2)
})

test("a new password is held to the published rules, then to its confirmation", () => {
  const ok = "correct horse"
  assert.equal(newPasswordProblem(rules, ok, ok), null)
  assert.equal(newPasswordProblem(rules, "short", "short"), "short")
  // Six emoji are 12 UTF-16 units but 6 code points: still short.
  const emoji = "😀".repeat(6)
  assert.equal(newPasswordProblem(rules, emoji, emoji), "short")
  const long = "a".repeat(17)
  assert.equal(newPasswordProblem(rules, long, long), "long")
  // 14 code points within the length, 56 bytes over the byte limit.
  const heavy = "😀".repeat(14)
  assert.equal(newPasswordProblem(rules, heavy, heavy), "too_many_bytes")
  assert.equal(newPasswordProblem(rules, ok, `${ok}!`), "mismatch")
})
