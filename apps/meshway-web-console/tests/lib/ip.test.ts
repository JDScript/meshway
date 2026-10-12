import assert from "node:assert/strict"
import { test } from "vitest"

import { isIp, isIpv4, isIpv6, parsePort, rangeCidr } from "@/lib/ip"

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

test("a range that is exactly one prefix is written as it", () => {
  assert.equal(rangeCidr("10.0.0.0", "10.255.255.255"), "10.0.0.0/8")
  assert.equal(rangeCidr("203.0.113.7", "203.0.113.7"), "203.0.113.7/32")
  assert.equal(rangeCidr("10.0.0.0", "10.0.0.2"), null)
  assert.equal(rangeCidr("10.0.0.1", "10.0.0.2"), null)
  assert.equal(rangeCidr("0.0.0.0", "255.255.255.255"), "0.0.0.0/0")
  assert.equal(
    rangeCidr("2001:db8::", "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff"),
    "2001:db8::/32"
  )
  assert.equal(
    rangeCidr("::ffff:1.2.3.0", "::ffff:1.2.3.255"),
    "::ffff:1.2.3.0/120"
  )
  assert.equal(rangeCidr("2001:db8::1", "2001:db8::2"), null)
  assert.equal(rangeCidr("10.0.0.0", "2001:db8::"), null)
})
