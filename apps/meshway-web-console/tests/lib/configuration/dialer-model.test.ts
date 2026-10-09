import assert from "node:assert/strict"
import { test } from "vitest"

import {
  dialerMode,
  dialerProblems,
  dialerRows,
  withDialerMode,
  withEntryField,
  withProtocol,
} from "@/lib/configuration/dialer-model"

test("rows follow the platform carrier order regardless of insertion order", () => {
  const rows = dialerRows({
    vless: {},
    native_udp: {},
    native_tcp: {},
    trojan: {},
  })
  assert.deepEqual(
    rows.map((row) => row.protocol),
    ["native_udp", "native_tcp", "vless", "trojan"]
  )
})

test("adding a protocol writes {}, fields keep the rest of the entry", () => {
  let set = withProtocol({}, "native_tcp", true)
  assert.deepEqual(set, { native_tcp: {} })
  set = withEntryField(set, "native_tcp", "enabled", false)
  set = withEntryField(
    { native_tcp: { ...set.native_tcp, tcp: { noDelay: false } } },
    "native_tcp",
    "connectTimeout",
    "2s"
  )
  assert.deepEqual(set, {
    native_tcp: {
      enabled: false,
      tcp: { noDelay: false },
      connectTimeout: "2s",
    },
  })
  set = withEntryField(set, "native_tcp", "enabled", undefined)
  assert.equal("enabled" in (set.native_tcp ?? {}), false)
  assert.deepEqual(withProtocol(set, "native_tcp", false), {})
  // A field on a protocol not in the set changes nothing.
  assert.deepEqual(withEntryField({}, "vless", "enabled", false), {})
})

test("the schema refuses bad timeouts", () => {
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
})
