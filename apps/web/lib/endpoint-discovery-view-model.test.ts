import assert from "node:assert/strict"
import { test } from "node:test"

import type { ControllerResult } from "./controller.ts"
import { advanceDiscoveryClock, shownDiscoveryRead } from "./endpoint-discovery-view-model.ts"

test("expired leases stay expired after wall-clock rollback and continue ageing on monotonic time", () => {
  const expiresAt = 30_000
  let clock = advanceDiscoveryClock({ time: 29_000, monotonicTime: null }, 29_000, 100)
  clock = advanceDiscoveryClock(clock, 30_001, 1_101)
  assert.ok(clock.time >= expiresAt)
  clock = advanceDiscoveryClock(clock, 20_000, 2_101)
  assert.ok(clock.time >= expiresAt)
  assert.equal(clock.time, 31_001)
  // Visibility / refresh samples at an older wall time cannot resurrect it.
  clock = advanceDiscoveryClock(clock, 20_000, 2_101)
  assert.equal(clock.time, 31_001)
  // A genuinely newer lease can still be valid at the same UI clock.
  assert.ok(clock.time < 60_000)
})

test("a failed refresh replaces initial data even if the server timestamp moved backwards", () => {
  const initial: ControllerResult<string> = { ok: true, data: "old observation", fetchedAt: 30_000 }
  const failed: ControllerResult<string> = { ok: false, error: "offline", fetchedAt: 20_000 }
  assert.equal(shownDiscoveryRead(initial, { initial, result: failed }), failed)
})

test("new initial prop identity supersedes local results and late responses to an old prop", () => {
  const initial: ControllerResult<string> = { ok: true, data: "old observation", fetchedAt: 30_000 }
  const replacement: ControllerResult<string> = { ok: true, data: "new observation", fetchedAt: 20_000 }
  const pendingOld: ControllerResult<string> = { ok: true, data: "obsolete response", fetchedAt: 40_000 }
  assert.equal(shownDiscoveryRead(replacement, { initial, result: pendingOld }), replacement)
  const failed: ControllerResult<string> = { ok: false, error: "offline", fetchedAt: 10_000 }
  assert.equal(shownDiscoveryRead(replacement, { initial: replacement, result: failed }), failed)
})
