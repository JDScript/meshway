import assert from "node:assert/strict"
import { beforeEach, test, vi } from "vitest"

import {
  forgetNetwork,
  landing,
  rememberedNetwork,
  rememberNetwork,
} from "@/lib/remembered-network"

const stored = new Map<string, string>()
vi.stubGlobal("window", {
  localStorage: {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => void stored.set(key, value),
    removeItem: (key: string) => void stored.delete(key),
  },
})
beforeEach(() => stored.clear())

test("a remembered network belongs to the account that opened it", () => {
  rememberNetwork("user-a", "net-1")
  assert.equal(rememberedNetwork("user-a"), "net-1")
  assert.equal(rememberedNetwork("user-b"), null)
  forgetNetwork("user-b", "net-1")
  assert.equal(rememberedNetwork("user-a"), "net-1")
  forgetNetwork("user-a", "net-1")
  assert.equal(rememberedNetwork("user-a"), null)
})

test("anything else stored is no preference", () => {
  assert.equal(rememberedNetwork("user-a"), null)
  for (const value of ["net-1", '{"v":2,"user":"user-a"}', "{"]) {
    stored.set("meshway.network", value)
    assert.equal(rememberedNetwork("user-a"), null)
  }
})

test("`/` opens the remembered network when it is listed", () => {
  const listed = [{ id: "net-1" }, { id: "net-2" }]
  assert.deepEqual(landing(listed, false, "net-2"), {
    open: "net-2",
    check: null,
  })
  assert.deepEqual(landing(listed, false, null), { open: "net-1", check: null })
})

test("a remembered network no longer listed falls back to the first", () => {
  const listed = [{ id: "net-1" }]
  assert.deepEqual(landing(listed, false, "gone"), {
    open: "net-1",
    check: null,
  })
  // It may be on a later page: ask for it first.
  assert.deepEqual(landing(listed, true, "later"), {
    open: "net-1",
    check: "later",
  })
  assert.deepEqual(landing([], false, "gone"), { open: null, check: null })
})
