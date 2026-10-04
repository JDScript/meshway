import assert from "node:assert/strict"
import { test } from "vitest"

import { activityTarget, endsThisTab, pagedCount, shortId } from "@/lib/account"

const mine = "11111111-2222-4333-8444-555555555555"
const other = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"

test("a revocation ends this tab only when it covers this session", () => {
  assert.equal(endsThisTab({ kind: "one", sessionId: mine }, mine), true)
  assert.equal(endsThisTab({ kind: "one", sessionId: other }, mine), false)
  assert.equal(endsThisTab({ kind: "others" }, mine), false)
  assert.equal(endsThisTab({ kind: "all" }, mine), true)
})

test("a session target is this sign-in or another one by its short ID", () => {
  assert.deepEqual(activityTarget({ type: "session", sessionId: mine }, mine), {
    kind: "this_session",
  })
  assert.deepEqual(
    activityTarget({ type: "session", sessionId: other }, mine),
    { kind: "session", short: "aaaaaaaa" }
  )
  assert.deepEqual(activityTarget({ type: "session", sessionId: mine }, null), {
    kind: "session",
    short: "11111111",
  })
  assert.deepEqual(activityTarget({ type: "account", userId: mine }, mine), {
    kind: "account",
  })
  assert.deepEqual(
    activityTarget({ type: "configuration", revision: 7 }, mine),
    { kind: "configuration", revision: 7 }
  )
})

test("short IDs and paged counts", () => {
  assert.equal(shortId(other), "aaaaaaaa")
  assert.equal(pagedCount(3, false), 3)
  assert.equal(pagedCount(100, true), "100+")
})
