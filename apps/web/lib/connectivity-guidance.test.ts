import assert from "node:assert/strict"
import test from "node:test"

import { diagnosticsHref, peerDiagnosticCommands } from "./connectivity-guidance.ts"

test("peer commands reject self, stale input, and shell syntax in identity", () => {
  const source = "a".repeat(64)
  const target = "b".repeat(64)
  assert.equal(peerDiagnosticCommands(source, source), null)
  assert.equal(peerDiagnosticCommands(source, undefined), null)
  assert.equal(peerDiagnosticCommands(source, `${target};echo injected`), null)
  assert.equal(peerDiagnosticCommands("$(echo source)", target), null)
  const commands = peerDiagnosticCommands(source, target)
  assert.ok(commands)
  assert.ok(commands.ping.startsWith(`meshway-agent ping ${target} `))
  assert.ok(commands.whois.startsWith(`meshway-agent whois ${target} `))
  assert.ok(!commands.ping.includes(source))
})

test("node diagnostic navigation encodes inputs as query values", () => {
  const value = "node&target=other#netcheck"
  const url = new URL(diagnosticsHref(value), "http://localhost")
  assert.equal(url.pathname, "/monitoring")
  assert.equal(url.searchParams.get("source"), value)
  assert.equal(url.searchParams.get("target"), null)
  assert.equal(url.hash, "")
})
