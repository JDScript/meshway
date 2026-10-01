import assert from "node:assert/strict"
import { test } from "node:test"

import { nextPartStatus } from "./node-status.ts"

test("background read outcomes apply in read order, not arrival order", () => {
  const rendered = { at: 1_000, error: null }
  const failed = nextPartStatus(rendered, 3_000, "HTTP 502")
  assert.equal(failed.error, "HTTP 502")
  // A slower, older success arriving later does not hide the newer failure.
  assert.equal(nextPartStatus(failed, 2_000, null).error, "HTTP 502")
  // A newer success clears it; a late older failure cannot bring it back.
  const recovered = nextPartStatus(failed, 4_000, null)
  assert.equal(recovered.error, null)
  assert.equal(nextPartStatus(recovered, 3_500, "HTTP 502").error, null)
})
