import assert from "node:assert/strict";
import { test } from "node:test";
import { parseJson } from "../src/json.ts";

test("JSON decoding rejects duplicate decoded keys at every nesting level", () => {
  for (const text of [
    '{"email":"first","email":"second"}',
    '{"user":{"email":"first","\\u0065mail":"second"}}',
    '[{"ok":1},{"nested":[{"same":0,"same":1}]}]',
    '{"__proto__":0,"__proto__":1}',
    '{"\\\\":0,"\\u005c":1}',
    '{"😀":0,"\\ud83d\\ude00":1}',
  ]) assert.throws(() => parseJson(text), { name: "SyntaxError", message: "Invalid JSON response" });
});

test("JSON key checks preserve values, separate objects and escaped string boundaries", () => {
  for (const text of [
    '{"a":1,"b":2}',
    '{"a":{"a":1},"b":[{"a":2},{"a":3}]}',
    '{"a":["a","a",{"b":"a"}],"b":null}',
    String.raw`{"":"empty","\"":false,"__proto__":{"value":2}}`,
    JSON.stringify({ text: '"key": "value", {"key": 2}, \\"', nested: [{ text: "same key" }] }),
    '[1,-2.3e4,null,true,false,"string",[],{}]',
    '"string root"',
  ]) assert.deepEqual(parseJson(text), JSON.parse(text));
  const value = parseJson('{"__proto__":{"polluted":true}}');
  assert.ok(value !== null && typeof value === "object");
  assert.equal(Object.getPrototypeOf(value), Object.prototype);
  assert.equal(Object.hasOwn(value, "__proto__"), true);
});

test("invalid JSON errors contain neither input values nor native parser excerpts", () => {
  for (const text of ['{"password":"private-secret",}', '["private-secret"', '{private-secret}', '/*private-secret*/{}']) {
    assert.throws(() => parseJson(text), error => error instanceof SyntaxError && error.message === "Invalid JSON response" && !String(error).includes("private-secret"));
  }
});
