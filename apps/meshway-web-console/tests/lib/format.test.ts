import assert from "node:assert/strict"
import { test } from "vitest"

import { agoTime, relativeTime } from "@/lib/format"

const NOW = Date.parse("2026-10-04T09:00:00Z")

test("what happened is never said to be ahead of the clock", () => {
  const later = "2026-10-04T09:00:07Z"
  assert.notEqual(relativeTime(later, NOW), relativeTime(NOW, NOW))
  assert.equal(agoTime(later, NOW), relativeTime(NOW, NOW))
  const earlier = "2026-10-04T08:57:00Z"
  assert.equal(agoTime(earlier, NOW), relativeTime(earlier, NOW))
  assert.equal(agoTime("not a time", NOW), null)
})
