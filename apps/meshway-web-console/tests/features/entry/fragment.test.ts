import assert from "node:assert/strict"
import { test } from "vitest"

import {
  fragmentLink,
  heldSecret,
  secretFromFragment,
} from "@/features/entry/fragment"

const secret = `mwj_${"0".repeat(64)}`

test("a fragment yields exactly one bounded, printable token", () => {
  assert.equal(secretFromFragment(`#token=${secret}`), secret)
  assert.equal(secretFromFragment(`token=${secret}`), secret)
  assert.equal(secretFromFragment(""), null)
  assert.equal(secretFromFragment("#token="), null)
  assert.equal(secretFromFragment(`#token=${secret}&token=${secret}`), null)
  assert.equal(secretFromFragment("#token=a%20b"), null)
  assert.equal(secretFromFragment(`#token=${"a".repeat(257)}`), null)
})

test("a link round-trips through its fragment", () => {
  const link = fragmentLink("https://console.example", "join", secret)
  assert.equal(link, `https://console.example/join#token=${secret}`)
  assert.equal(secretFromFragment(new URL(link).hash), secret)
  assert.ok(
    fragmentLink("https://c.example", "reset", "x").startsWith(
      "https://c.example/reset-password#"
    )
  )
})

test("a held secret is read once and dropped", () => {
  const held = heldSecret()
  assert.equal(held.current(), undefined)
  held.take("#token=abc")
  assert.equal(held.current(), "abc")
  held.drop()
  assert.equal(held.current(), undefined)
  held.take("#nothing")
  assert.equal(held.current(), null)
})

test("a second fragment is announced, and counted", () => {
  const held = heldSecret()
  let heard = 0
  const stop = held.subscribe(() => (heard += 1))
  held.take("#token=one")
  held.take("#token=two")
  assert.equal(held.taken(), 2)
  assert.equal(heard, 2)
  assert.equal(held.current(), "two")
  stop()
  held.take("#token=three")
  assert.equal(heard, 2)
})
