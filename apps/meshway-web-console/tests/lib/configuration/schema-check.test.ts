import assert from "node:assert/strict"
import { test } from "vitest"

import { checkSchema, readSchema } from "@/lib/configuration/schema-check"

// A small schema in the controller's own shape (a $ref root into $defs).
const SCHEMA = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  $ref: "#/$defs/Root",
  $defs: {
    Root: {
      type: "object",
      additionalProperties: false,
      required: ["dns"],
      properties: {
        dns: { $ref: "#/$defs/Dns" },
        groups: {
          type: "object",
          propertyNames: { type: "string", pattern: "^group:" },
          additionalProperties: { type: "array", items: { type: "string" } },
        },
        rules: { type: "array", items: { $ref: "#/$defs/Action" } },
        port: { type: "integer", minimum: 1, maximum: 65535 },
        name: { type: ["string", "null"], maxLength: 3 },
      },
    },
    Dns: {
      type: "object",
      additionalProperties: false,
      required: ["domain"],
      properties: {
        domain: { type: "string" },
        magicDNS: { type: "boolean" },
      },
    },
    Action: {
      oneOf: [
        {
          type: "object",
          additionalProperties: false,
          required: ["type", "nodes"],
          properties: {
            type: { type: "string", enum: ["exit"] },
            nodes: { type: "array", items: { type: "string" } },
          },
        },
        {
          type: "object",
          additionalProperties: false,
          required: ["type"],
          properties: { type: { type: "string", enum: ["direct"] } },
        },
      ],
    },
  },
}

const check = (value: unknown) => checkSchema(SCHEMA, value)

test("a valid document has no issues", () => {
  assert.deepEqual(
    check({
      dns: { domain: "x", magicDNS: true },
      groups: { "group:a": ["b"] },
      rules: [{ type: "exit", nodes: ["n"] }, { type: "direct" }],
      port: 443,
      name: null,
    }),
    []
  )
})

test("types, required and unknown keys are placed by path", () => {
  const issues = check({ dns: { magicDNS: "yes" }, extra: 1 })
  assert.deepEqual(
    issues.map(({ path, code, detail, onKey }) => ({
      path,
      code,
      detail,
      onKey,
    })),
    [
      { path: ["dns"], code: "required", detail: "domain", onKey: undefined },
      {
        path: ["dns", "magicDNS"],
        code: "type",
        detail: "boolean",
        onKey: undefined,
      },
      { path: ["extra"], code: "unknown_key", detail: "extra", onKey: true },
    ]
  )
})

test("a discriminated variant is checked against its own branch", () => {
  const issues = check({
    dns: { domain: "x" },
    rules: [{ type: "exit" }, { type: "direct", nodes: [] }, { type: "drop" }],
  })
  assert.deepEqual(
    issues.map(({ path, code, detail }) => ({ path, code, detail })),
    [
      { path: ["rules", 0], code: "required", detail: "nodes" },
      { path: ["rules", 1, "nodes"], code: "unknown_key", detail: "nodes" },
      { path: ["rules", 2, "type"], code: "variant", detail: "exit, direct" },
    ]
  )
})

test("ranges, lengths, patterns and key names", () => {
  const issues = check({
    dns: { domain: "x" },
    port: 0,
    name: "four",
    groups: { team: ["a"] },
  })
  assert.deepEqual(
    issues.map(({ path, code, onKey }) => ({ path, code, onKey })),
    [
      { path: ["port"], code: "range", onKey: undefined },
      { path: ["name"], code: "length", onKey: undefined },
      { path: ["groups", "team"], code: "pattern", onKey: true },
    ]
  )
  // Lengths count code points, not UTF-16 units.
  assert.deepEqual(check({ dns: { domain: "x" }, name: "😀😀😀" }), [])
})

test("integers are not numbers with fractions", () => {
  const issues = check({ dns: { domain: "x" }, port: 1.5 })
  assert.deepEqual(issues[0]?.code, "type")
})

test("the schema text must be an object", () => {
  assert.equal(readSchema("[]"), null)
  assert.equal(readSchema("{"), null)
  assert.ok(readSchema(JSON.stringify(SCHEMA)))
})
