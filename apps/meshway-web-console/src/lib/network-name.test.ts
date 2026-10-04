import assert from "node:assert/strict"
import { test } from "node:test"

import { checkNetworkName, trimWhiteSpace } from "./network-name.ts"

const rules = {
  minCodepoints: 1,
  maxCodepoints: 128,
  maxInputUtf8Bytes: 1024,
  trim: "unicodeWhiteSpace",
  forbidControlCharacters: true,
} as const

test("trims exactly Unicode White_Space, not U+FEFF", () => {
  assert.equal(trimWhiteSpace("\u3000\u00a0家\u2028"), "家")
  assert.equal(trimWhiteSpace("\u2000a b\u205f"), "a b")
  assert.equal(trimWhiteSpace("\ufeffa\ufeff"), "\ufeffa\ufeff")
  assert.equal(trimWhiteSpace("\u200ba"), "\u200ba")
  assert.equal(trimWhiteSpace("   "), "")
})

test("a name is trimmed and counted in code points", () => {
  assert.deepEqual(checkNetworkName("  家庭网络 ", rules), {
    ok: true,
    name: "家庭网络",
  })
  assert.deepEqual(checkNetworkName("😀".repeat(128), rules), {
    ok: true,
    name: "😀".repeat(128),
  })
  assert.deepEqual(checkNetworkName("😀".repeat(129), rules), {
    ok: false,
    problem: "length",
  })
  assert.deepEqual(checkNetworkName("\u3000 ", rules), {
    ok: false,
    problem: "empty",
  })
  assert.deepEqual(checkNetworkName("\ufeff", rules), {
    ok: true,
    name: "\ufeff",
  })
})

test("control characters are refused in the raw input, edges included", () => {
  assert.deepEqual(checkNetworkName("a\tb", rules), {
    ok: false,
    problem: "control",
  })
  assert.deepEqual(checkNetworkName("name\n", rules), {
    ok: false,
    problem: "control",
  })
  assert.deepEqual(checkNetworkName("\u0085a", rules), {
    ok: false,
    problem: "control",
  })
  assert.deepEqual(checkNetworkName("a\u007f", rules), {
    ok: false,
    problem: "control",
  })
})

test("the byte limit applies to the raw input before trimming", () => {
  const padded = `${" ".repeat(1000)}name${" ".repeat(30)}`
  assert.deepEqual(checkNetworkName(padded, rules), {
    ok: false,
    problem: "bytes",
  })
  assert.deepEqual(checkNetworkName(`${" ".repeat(1000)}name`, rules), {
    ok: true,
    name: "name",
  })
  // 342 × 3 bytes is over 1024, though only 342 code points.
  assert.deepEqual(checkNetworkName("家".repeat(342), rules), {
    ok: false,
    problem: "bytes",
  })
})

test("follows the numbers the controller publishes", () => {
  const strict = { ...rules, minCodepoints: 3, maxCodepoints: 4 }
  assert.deepEqual(checkNetworkName("ab", strict), {
    ok: false,
    problem: "length",
  })
  assert.deepEqual(checkNetworkName("abcde", strict), {
    ok: false,
    problem: "length",
  })
  assert.deepEqual(
    checkNetworkName("a\tb", { ...rules, forbidControlCharacters: false }),
    {
      ok: true,
      name: "a\tb",
    }
  )
})
