import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "node:test"

import { completeAt, locationAt, schemasAt } from "./jsonc-complete.ts"

// The real generated OpenAPI document: components are JSON Schema 2020-12.
const openapi: unknown = JSON.parse(
  readFileSync(new URL("../../../packages/protocol/openapi.json", import.meta.url), "utf8")
)
const routing = { $ref: "#/components/schemas/RoutingPolicy" }

/** Splits `text|` at the cursor marker. */
function at(marked: string): [string, number] {
  const offset = marked.indexOf("|")
  return [marked.slice(0, offset) + marked.slice(offset + 1), offset]
}

test("locates keys and values in incomplete text", () => {
  assert.deepEqual(locationAt(...at(`{ "rules": [ { "action": { | `)), {
    kind: "key",
    path: ["rules", 0, "action"],
    present: [],
    from: 27,
    to: 27,
    prefix: "",
  })
  const value = locationAt(...at(`{ "default_action": "di|`))
  assert.equal(value.kind, "value")
  assert.ok(value.kind === "value")
  assert.deepEqual(value.path, ["default_action"])
  assert.equal(value.prefix, "di")
  const second = locationAt(...at(`{ "rules": [ {}, { "id": "x", | } ] }`))
  assert.ok(second.kind === "key")
  assert.deepEqual(second.path, ["rules", 1])
  assert.deepEqual(second.present, ["id"])
})

test("no completion inside comments or right after a finished value", () => {
  assert.equal(locationAt(...at(`{ // de|fault\n}`)).kind, "none")
  assert.equal(locationAt(...at(`{ /* a | */ }`)).kind, "none")
  assert.equal(locationAt(...at(`{ "a": 1 | }`)).kind, "none")
})

test("navigates $ref, items, and nullable unions in the generated schema", () => {
  const [exit] = schemasAt(openapi, routing, ["rules", 0, "action", "exit_node_id"])
  assert.deepEqual(exit?.type, ["string", "null"])
})

test("suggests missing property names with descriptions", () => {
  const completion = completeAt(...at(`{ "rules": [ { "id": "a", | } ] }`), openapi, routing)
  assert.ok(completion)
  const labels = completion.items.map((item) => item.label)
  assert.ok(labels.includes("source"))
  assert.ok(!labels.includes("id"), "already present")
  const source = completion.items.find((item) => item.label === "source")
  assert.equal(source?.insert, `"source": {}`)
  assert.match(source?.detail ?? "", /wildcards/)
})

test("suggests enum values filtered by the typed prefix and replaces the partial string", () => {
  const [text, offset] = at(`{ "default_action": "re|" }`)
  const completion = completeAt(text, offset, openapi, routing)
  assert.ok(completion)
  assert.deepEqual(
    completion.items.map((item) => item.insert),
    [`"reject"`]
  )
  assert.equal(text.slice(completion.from, completion.to), `"re"`)
})

test("suggests inventory references only where the schema marks them", () => {
  const schema = {
    type: "object",
    properties: { path: { type: "array", items: { type: "string", "x-meshway-ref": "node" } }, id: { type: "string" } },
  }
  const references = { node: [{ value: "a".repeat(64), label: "nyc", detail: "100.96.0.1" }] }
  const inPath = completeAt(...at(`{ "path": ["ny|`), schema, schema, references)
  assert.deepEqual(
    inPath?.items.map((item) => [item.label, item.insert]),
    [["nyc", JSON.stringify("a".repeat(64))]]
  )
  const inId = completeAt(...at(`{ "id": "|`), schema, schema, references)
  assert.deepEqual(inId?.items, [])
})
