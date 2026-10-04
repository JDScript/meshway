import assert from "node:assert/strict"
import { test } from "vitest"

import {
  dropLinkSecret,
  linkSecret,
  takeLinkSecret,
} from "@/state/link-secrets"

test("a link's secret is held until dropped; a fragment without one is null", () => {
  assert.equal(linkSecret("join"), undefined)
  takeLinkSecret("join", "#token=abc")
  assert.equal(linkSecret("join"), "abc")
  assert.equal(linkSecret("reset"), undefined)
  dropLinkSecret("join")
  assert.equal(linkSecret("join"), undefined)
  takeLinkSecret("join", "#nothing")
  assert.equal(linkSecret("join"), null)
})
