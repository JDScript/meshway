import assert from "node:assert/strict"
import { test } from "node:test"

import { deviceCallback, devicePath, parseDeviceRequest } from "./request.ts"

// The request exactly as the App builds it (apps/apple/core/src/signin.rs:
// `begin_authorization`, `url::form_urlencoded` pairs in this order).
const app = {
  redirect_uri: "meshway://enroll/callback",
  state: "a".repeat(64),
  code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
  name: "Mac & 中文",
  agent_version: "v+1",
}
const query = (params: Record<string, string>) =>
  new URLSearchParams(params).toString()

test("the App's request is read as sent", () => {
  const value = parseDeviceRequest(`?${query(app)}`)
  assert.ok(value)
  assert.equal(value.state, app.state)
  assert.equal(value.request.name, "Mac & 中文")
  assert.equal(value.request.agent_version, "v+1")
  assert.equal(value.request.redirect_uri, app.redirect_uri)
})

test("values the router would turn into JSON stay strings", () => {
  const value = parseDeviceRequest(
    query({ ...app, agent_version: "1.0", name: "007", state: "true" })
  )
  assert.ok(value)
  assert.equal(value.request.agent_version, "1.0")
  assert.equal(value.request.name, "007")
  assert.equal(value.state, "true")
})

test("requests the App does not make are refused", () => {
  for (const redirect_uri of [
    "https://evil.test",
    "meshway://enroll/callback?x=1",
    "meshway://enroll/callback/",
    "MESHWAY://enroll/callback",
  ])
    assert.equal(parseDeviceRequest(query({ ...app, redirect_uri })), null)
  for (const override of [
    { state: "" },
    { state: "x".repeat(257) },
    { state: "a\n" },
    { state: "a\u0085" },
    { name: " " },
    { name: "\u0085" },
    { agent_version: "x".repeat(65) },
    { code_challenge: "x".repeat(43) },
    { code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM=" },
  ])
    assert.equal(parseDeviceRequest(query({ ...app, ...override })), null)
  assert.equal(parseDeviceRequest(`${query(app)}&state=second`), null)
  const missing = new URLSearchParams(app)
  missing.delete("state")
  assert.equal(parseDeviceRequest(missing.toString()), null)
})

test("unknown keys are ignored and left out of the canonical path", () => {
  const value = parseDeviceRequest(`${query(app)}&next=https://evil.test`)
  assert.ok(value)
  assert.ok(!devicePath(value).includes("next"))
})

test("the canonical path round-trips, up to the longest request", () => {
  const value = parseDeviceRequest(query(app))
  assert.ok(value)
  const path = devicePath(value)
  assert.ok(path.startsWith("/authorize-device?redirect_uri="))
  assert.deepEqual(parseDeviceRequest(path.split("?")[1] ?? ""), value)
  const longest = parseDeviceRequest(
    query({
      ...app,
      name: "😀".repeat(128),
      agent_version: "😀".repeat(64),
      state: "😀".repeat(256),
    })
  )
  assert.ok(longest)
  const long = devicePath(longest)
  assert.ok(long.length <= 8192, `${long.length}`)
  assert.deepEqual(parseDeviceRequest(long.split("?")[1] ?? ""), longest)
})

test("the callback carries the code or the denial, and the state", () => {
  const value = parseDeviceRequest(query({ ...app, state: "opaque + & ✓" }))
  assert.ok(value)
  const approved = new URL(deviceCallback(value, { code: "secret" }))
  assert.equal(
    `${approved.protocol}//${approved.host}${approved.pathname}`,
    app.redirect_uri
  )
  assert.deepEqual([...approved.searchParams.keys()], ["code", "state"])
  assert.equal(approved.searchParams.get("code"), "secret")
  assert.equal(approved.searchParams.get("state"), "opaque + & ✓")
  const denied = new URL(deviceCallback(value, { error: "access_denied" }))
  assert.deepEqual([...denied.searchParams.keys()], ["error", "state"])
  assert.equal(denied.searchParams.get("error"), "access_denied")
})
