import assert from "node:assert/strict"
import test from "node:test"
import { authorizationPath, deviceCallback, parseDeviceAuthorization, safeLoginReturn } from "./device-login.ts"

const params = { redirect_uri: "meshway://enroll/callback", state: "opaque + & ✓", code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM", name: "Mac", agent_version: "1.0" }

test("authorization validation rejects arbitrary redirects, duplicate and oversized values", () => {
  assert.ok(parseDeviceAuthorization(params))
  for (const redirect_uri of ["https://evil.test", "meshway://enroll/callback?x=1", "meshway://enroll/callback/", "MESHWAY://enroll/callback"]) {
    assert.equal(parseDeviceAuthorization({ ...params, redirect_uri }), null)
  }
  for (const override of [{ state: "" }, { state: "x".repeat(257) }, { state: "a\n" }, { name: " " }, { name: "\u0085" }, { agent_version: "x".repeat(65) }, { code_challenge: "x".repeat(43) }, { state: ["a", "b"] }]) {
    assert.equal(parseDeviceAuthorization({ ...params, ...override }), null)
  }
})

test("login return is canonical and callback roundtrips state", () => {
  const value = parseDeviceAuthorization(params)
  assert.ok(value)
  const path = authorizationPath(value)
  assert.equal(safeLoginReturn(path), path)
  const maximumUnicode = parseDeviceAuthorization({ ...params, name: "😀".repeat(128), agent_version: "😀".repeat(64), state: "😀".repeat(256) })
  assert.ok(maximumUnicode)
  assert.equal(safeLoginReturn(authorizationPath(maximumUnicode)), authorizationPath(maximumUnicode))
  for (const bad of ["//evil.test", "https://evil.test", "/authorize-device/../evil?x=1", `${path}&state=second`, `${path}&next=https://evil.test`]) assert.equal(safeLoginReturn(bad), "/")
  const callback = new URL(deviceCallback(value, { code: "secret" }))
  assert.equal(callback.searchParams.get("state"), params.state)
  assert.equal(callback.searchParams.get("code"), "secret")
  assert.equal(new URL(deviceCallback(value, { error: "access_denied" })).searchParams.get("error"), "access_denied")
})
