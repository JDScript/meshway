import assert from "node:assert/strict"
import { test } from "node:test"

import { canonicalNodeDnsName } from "./dns-guidance.ts"

test("canonical DNS preserves the full durable identity and saved IDNA zone", () => {
  const first = "0123456789abcdef".repeat(2)
  const last = "fedcba9876543210".repeat(2)
  assert.equal(canonicalNodeDnsName(first + last, "xn--bcher-kva.test"), `id.${first}.${last}.xn--bcher-kva.test`)
  assert.equal(canonicalNodeDnsName("0".repeat(64), "meshway.test"), `id.${"0".repeat(32)}.${"0".repeat(32)}.meshway.test`)
})

test("invalid identities and malformed or non-ASCII zones cannot be copied as query names", () => {
  const id = "a".repeat(64)
  for (const invalid of ["a".repeat(63), "a".repeat(65), "A".repeat(64), "g".repeat(64), ` ${id}`, `${id}\n`]) {
    assert.equal(canonicalNodeDnsName(invalid, "meshway.test"), null)
  }
  for (const zone of ["", "test", "BÜCHER.TEST.", "meshway.test.", "meshway.TEST", "meshway.test\n", "-bad.test", "bad_.test", "a".repeat(64) + ".test", "a.".repeat(64) + "test", "test; echo injected"]) {
    assert.equal(canonicalNodeDnsName(id, zone), null)
  }
})
