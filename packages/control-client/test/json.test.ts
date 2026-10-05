import assert from "node:assert/strict";
import { test } from "node:test";
import * as z from "zod";
import { parseJson, unwrapJsonNumber } from "../src/json.ts";

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
    '[1,-23000,null,true,false,"string",[],{}]',
    '"string root"',
  ]) assert.deepEqual(parseJson(text), JSON.parse(text));
  const value = parseJson('{"__proto__":{"polluted":true}}');
  assert.ok(value !== null && typeof value === "object");
  assert.equal(Object.getPrototypeOf(value), Object.prototype);
  assert.equal(Object.hasOwn(value, "__proto__"), true);
});

test("invalid JSON errors contain neither input values nor native parser excerpts", () => {
  for (const text of ['{"password":"private-secret",}', '["private-secret"', '{private-secret}', '/*private-secret*/{}', '01', '-01', '+1', '.1', '1.', '1e', 'NaN', 'Infinity', '[1 2]', '{"x":1e+}', 'true false', '"\\x20"']) {
    assert.throws(() => parseJson(text), error => error instanceof SyntaxError && error.message === "Invalid JSON response" && !String(error).includes("private-secret"));
  }
});

const number = z.preprocess(unwrapJsonNumber, z.number());

test("integer schemas accept only canonical safe integer tokens", () => {
  for (const token of ["0", "1", "-1", "9007199254740991", "-9007199254740991"]) {
    const value = parseJson(token);
    assert.equal(typeof value, "number");
    assert.equal(z.int().parse(value), Number(token));
  }
  for (const token of ["1.0000000000000001", "9007199254740991.1", "1e0", "1.0", "-0", "0.0", "-0e0", "1e-9999", "9007199254740992", "-9007199254740992", "9007199254740993"]) {
    assert.equal(z.int().safeParse(parseJson(token)).success, false, token);
  }
});

test("number schemas recover binary64 floats, exponents, signed zero and rounded values", () => {
  for (const token of ["1.0000000000000001", "9007199254740991.1", "1e0", "1.0", "-0", "-0.0", "-2.3e4", "5e-324", "1e-9999", "1.7976931348623157e308", "9007199254740992", "0"]) {
    assert.ok(Object.is(number.parse(parseJson(token)), Number(token)), token);
  }
  for (const token of ["1e309", "-1e309", "9".repeat(400)]) {
    assert.equal(number.safeParse(parseJson(token)).success, false);
  }
  assert.equal(z.preprocess(unwrapJsonNumber, z.number().min(-90).max(90)).safeParse(parseJson("90.5")).success, false);
});

test("non-integer markers cannot masquerade as JSON objects, arrays or other scalars", () => {
  const otherJson = z.union([z.strictObject({}), z.object({}), z.record(z.string(), z.string()), z.array(z.string()), z.int(), z.string(), z.boolean(), z.null(), z.literal(1)]);
  for (const token of ["1.0000000000000001", "9007199254740991.1", "1e0", "1.0", "-0"]) {
    const value = parseJson(token);
    assert.equal(otherJson.safeParse(value).success, false, token);
    assert.equal(z.union([otherJson, number]).parse(value), Number(token));
  }
  let calls = 0;
  const foreignFunction = () => { calls++; return 1; };
  const foreignObject = { valueOf() { calls++; return 1; } };
  for (const value of [foreignFunction, foreignObject, { value: 1 }, [], null, "1", 1]) assert.equal(unwrapJsonNumber(value), value);
  assert.equal(calls, 0);
});

test("source order preserves numeric-key maps, nested arrays and nullable numeric fields", () => {
  const schema = z.strictObject({
    integers: z.record(z.string(), z.int().nullable()),
    numbers: z.record(z.string(), z.array(number.nullable())),
  });
  const text = '{"numbers":{"10":[1.0,null],"2":[-0,1.0000000000000001],"1":[9007199254740991.1]},"integers":{"10":10,"2":2,"1":null}}';
  assert.deepEqual(schema.parse(parseJson(text)), JSON.parse(text));
  for (const bad of [
    '{"numbers":{},"integers":{"10":10,"2":1.0}}',
    '{"numbers":{"2":1.0},"integers":{}}',
    '{"numbers":1.0,"integers":{}}',
    '{"numbers":{},"integers":{"1":1,"\\u0031":1.0}}',
  ]) {
    assert.throws(() => schema.parse(parseJson(bad)));
  }
});

test("structural scan handles deep JSON without recursive parsing or reviver context", () => {
  const depth = 20_000;
  let value = parseJson("[".repeat(depth) + "1.0" + "]".repeat(depth));
  for (let index = 0; index < depth; index++) {
    assert.ok(Array.isArray(value));
    assert.equal(value.length, 1);
    value = value[0];
  }
  assert.equal(z.int().safeParse(value).success, false);
  assert.equal(number.parse(value), 1);
});
