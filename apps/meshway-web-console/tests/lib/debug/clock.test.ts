import assert from "node:assert/strict"
import { test } from "vitest"

import { clockOffset } from "@/lib/debug/clock"

test("no samples measure nothing", () => {
  assert.equal(clockOffset([]), null)
})

test("one response bounds the offset by its round trip and the header's second", () => {
  // Sent at 10.000 s, answered at 10.200 s, stamped 12 s: the controller's
  // clock read somewhere in [12.000, 13.000) while the browser's read
  // somewhere in [10.000, 10.200].
  assert.deepEqual(
    clockOffset([{ date: 12_000, sent: 10_000, received: 10_200 }]),
    { low: 1_800, high: 3_000, samples: 1 }
  )
})

test("samples together narrow it", () => {
  assert.deepEqual(
    clockOffset([
      { date: 12_000, sent: 10_000, received: 10_200 },
      { date: 13_000, sent: 10_700, received: 10_800 },
    ]),
    { low: 2_200, high: 3_000, samples: 2 }
  )
})

test("a sample that disagrees with the newer ones ends the narrowing", () => {
  // The browser's clock was set back between the two: the older sample
  // allows nothing the newer one does, and only the newer one counts.
  assert.deepEqual(
    clockOffset([
      { date: 12_000, sent: 10_000, received: 10_100 },
      { date: 12_000, sent: 20_000, received: 20_100 },
    ]),
    { low: -8_100, high: -7_000, samples: 1 }
  )
})
