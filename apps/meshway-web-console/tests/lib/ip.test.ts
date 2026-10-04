import assert from "node:assert/strict"
import { test } from "vitest"

import { isIp, isIpv4, isIpv6, parsePort } from "@/lib/ip"

test("addresses, without zones or brackets", () => {
  assert.equal(isIpv4("100.64.0.1"), true)
  assert.equal(isIpv4("256.0.0.1"), false)
  assert.equal(isIpv6("fd42::1"), true)
  assert.equal(isIpv6("::ffff:1.2.3.4"), true)
  assert.equal(isIpv6("fe80::1%eth0"), false)
  assert.equal(isIpv6("[fd42::1]"), false)
  assert.equal(isIp("example.com"), false)
})

test("ports as written", () => {
  assert.equal(parsePort("53"), 53)
  assert.equal(parsePort("65535"), 65535)
  assert.equal(parsePort("0"), null)
  assert.equal(parsePort("65536"), null)
  assert.equal(parsePort("080"), null)
  assert.equal(parsePort(" 80"), null)
})
