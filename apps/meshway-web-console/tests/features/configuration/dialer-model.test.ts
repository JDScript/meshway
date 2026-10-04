import assert from "node:assert/strict"
import { test } from "vitest"

import {
  dialerMode,
  dialerProblems,
  dialerRows,
  effectiveDialers,
  priorityOf,
  withDialerMode,
  withEntryField,
  withProtocol,
} from "@/features/configuration/dialer-model"

test("rows are in the order tried: priority (default per protocol), then key", () => {
  const rows = dialerRows({
    vless: { priority: 50 },
    native_udp: {},
    native_tcp: { priority: 100 },
  })
  assert.deepEqual(
    rows.map((row) => [row.protocol, row.effectivePriority, row.priority]),
    [
      ["vless", 50, 50],
      ["native_tcp", 100, 100],
      ["native_udp", 100, null],
    ]
  )
})

test("adding a protocol writes {}, fields keep the rest of the entry", () => {
  let set = withProtocol({}, "native_tcp", true)
  assert.deepEqual(set, { native_tcp: {} })
  set = withEntryField(set, "native_tcp", "priority", 7)
  set = withEntryField(
    { native_tcp: { ...set.native_tcp, tcp: { noDelay: false } } },
    "native_tcp",
    "connectTimeout",
    "2s"
  )
  assert.deepEqual(set, {
    native_tcp: { priority: 7, tcp: { noDelay: false }, connectTimeout: "2s" },
  })
  set = withEntryField(set, "native_tcp", "priority", undefined)
  assert.equal("priority" in (set.native_tcp ?? {}), false)
  assert.deepEqual(withProtocol(set, "native_tcp", false), {})
  // A field on a protocol not in the set changes nothing.
  assert.deepEqual(withEntryField({}, "vless", "priority", 1), {})
})

test("priorities are digits only; the schema refuses bad timeouts", () => {
  assert.equal(priorityOf(""), undefined)
  assert.equal(priorityOf("1a2"), 12)
  assert.deepEqual(dialerProblems({ vless: { connectTimeout: "5s" } }), [])
  assert.deepEqual(
    dialerProblems({
      vless: { connectTimeout: "5 seconds" },
      native_udp: { connectTimeout: "500ms" },
    }),
    ["vless"]
  )
})

test("a node inherits (null), dials nothing ({}) or has its own set", () => {
  const defaults = { native_udp: {}, vless: {} }
  assert.equal(dialerMode(null), "inherit")
  assert.equal(dialerMode({}), "none")
  assert.equal(dialerMode({ vless: {} }), "own")
  assert.deepEqual(withDialerMode(null, "own", defaults), defaults)
  assert.deepEqual(withDialerMode({ vless: {} }, "own", defaults), {
    vless: {},
  })
  assert.deepEqual(withDialerMode({ vless: {} }, "none", defaults), {})
  assert.equal(withDialerMode({ vless: {} }, "inherit", defaults), null)
  assert.equal(effectiveDialers(null, defaults), defaults)
  assert.deepEqual(effectiveDialers({}, defaults), {})
})
