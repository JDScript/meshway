import assert from "node:assert/strict"
import { test } from "vitest"

import { fragmentLink, secretFromFragment } from "@/features/entry/fragment"

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
