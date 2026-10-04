import assert from "node:assert/strict"
import { test } from "node:test"

import {
  cidrFamily,
  isDomain,
  isProtocol,
  parsePort,
  selectorKind,
} from "./selectors.ts"

test("selectors as the reference lists them", () => {
  assert.equal(selectorKind("*", false), "all")
  assert.equal(selectorKind("alice@example.com", false), "user")
  assert.equal(selectorKind("group:dev", false), "group")
  assert.equal(selectorKind("tag:backend", false), "tag")
  assert.equal(
    selectorKind("node:0b6f4c4e-6f0e-4a9e-9b51-6b0c1d2e3f40", false),
    "node"
  )
  assert.equal(selectorKind("group:", false), null)
  assert.equal(selectorKind("node:abc", false), null)
  assert.equal(selectorKind("alice", false), null)
})

test("destinations also take the internet, domains and prefixes", () => {
  assert.equal(selectorKind("autogroup:internet", true), "internet")
  assert.equal(selectorKind("domain:example.com", true), "domain")
  assert.equal(selectorKind("10.0.0.0/8", true), "cidr")
  assert.equal(selectorKind("fd00::/8", true), "cidr")
  // Not for sources.
  assert.equal(selectorKind("autogroup:internet", false), null)
  assert.equal(selectorKind("10.0.0.0/8", false), null)
  assert.equal(selectorKind("domain:", true), null)
})

test("protocols: *, tcp/udp ports and ranges, icmp:*", () => {
  for (const value of ["*", "tcp:443", "tcp:8000-8100", "udp:*", "icmp:*"])
    assert.equal(isProtocol(value), true, value)
  for (const value of [
    "tcp",
    "tcp:0",
    "tcp:65536",
    "tcp:080",
    "tcp:9-8",
    "icmp:8",
    "sctp:*",
    "",
  ])
    assert.equal(isProtocol(value), false, value)
})

test("prefixes by family", () => {
  assert.equal(cidrFamily("100.64.0.0/10"), "ipv4")
  assert.equal(cidrFamily("fd42:7a65:8800::/48"), "ipv6")
  assert.equal(cidrFamily("100.64.0.0/33"), null)
  assert.equal(cidrFamily("fd42::/129"), null)
  assert.equal(cidrFamily("100.64.0.0"), null)
  assert.equal(cidrFamily("300.1.1.1/8"), null)
  assert.equal(cidrFamily("[fd42::]/48"), null)
})

test("domains and ports", () => {
  assert.equal(isDomain("corp.example"), true)
  assert.equal(isDomain("corp.example."), true)
  assert.equal(isDomain("-bad.example"), false)
  assert.equal(isDomain("1.2.3.4"), false)
  assert.equal(isDomain(""), false)
  assert.equal(parsePort("53"), 53)
  assert.equal(parsePort("0"), null)
  assert.equal(parsePort("65536"), null)
})
