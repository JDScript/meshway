import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { generatedFiles, validators, root } from "./generate-protocol.mjs";
import { operationModel } from "./codegen/operations.mjs";
import { compareArtifacts } from "./check-protocol.mjs";

const require = createRequire(join(root, "packages/control-client/package.json"));
const ref = name => ({ $ref: `#/components/schemas/${name}` });
const content = name => ({ "application/json": { schema: ref(name) } });
const closed = (properties, required = Object.keys(properties)) => ({ type: "object", properties, required, additionalProperties: false });
function fixture() {
  return {
    openapi: "3.1.0", info: { title: "Generator fixture", version: "0" },
    components: {
      securitySchemes: { FixtureBearer: { type: "http", scheme: "bearer" } },
      schemas: {
        FixtureInput: closed({ text: { type: "string", minLength: 1, maxLength: 3 } }),
        FixtureOutput: closed({
          id: { type: "string", format: "uuid", pattern: "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" },
          sequence: { type: "integer", minimum: 1, maximum: 9007199254740991 },
          time: { type: "string", format: "date-time" },
          requiredNullable: { type: ["string", "null"] },
          optionalNullable: { anyOf: [{ type: "string" }, { type: "null" }] },
        }, ["id", "sequence", "time", "requiredNullable"]),
        FixtureProblem: { oneOf: [
          closed({ code: { type: "string", const: "conflict" }, params: closed({ revision: { type: "integer", minimum: 1, maximum: 100 } }) }),
          closed({ code: { type: "string", enum: ["denied"] }, params: closed({}) }),
        ] },
        FixtureMap: { type: "object", additionalProperties: { type: "string" }, propertyNames: { type: "string", pattern: "^[a-z_]+$" }, maxProperties: 2 },
        FixtureAlias: { allOf: [ref("FixtureInput")], description: "One referenced schema" },
        FixtureInt32: { type: "integer", format: "int32" },
      },
    },
    paths: {
      "/fixture/value": { post: { operationId: "writeFixture", security: [{ FixtureBearer: [] }], requestBody: { required: true, content: content("FixtureInput") }, responses: { "201": { description: "Created", content: content("FixtureOutput") }, "409": { description: "Conflict", content: content("FixtureProblem") } } } },
      "/fixture/clear": { post: { operationId: "clearFixture", responses: { "204": { description: "Empty" } } } },
      "/fixture/read": { get: { operationId: "readFixture", responses: { "200": { description: "OK", content: content("FixtureOutput") } } } },
    },
  };
}
async function temporary(t) {
  const parent = join(tmpdir(), "opencode");
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, "meshway-codegen-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}
async function generatedFixture(t) {
  const directory = await temporary(t);
  const files = generatedFiles(fixture());
  for (const [path, text] of Object.entries(files)) {
    await mkdir(dirname(join(directory, path)), { recursive: true });
    await writeFile(join(directory, path), text);
  }
  const packageRoot = join(directory, "packages/control-client");
  await mkdir(join(packageRoot, "node_modules"), { recursive: true });
  await symlink(dirname(require.resolve("zod/package.json")), join(packageRoot, "node_modules/zod"));
  await copyFile(join(root, "packages/control-client/src/transport.ts"), join(packageRoot, "src/transport.ts"));
  await writeFile(join(packageRoot, "package.json"), '{"type":"module"}');
  return { directory, packageRoot, files };
}

test("generated schemas execute exact UUID, bounds, null, closed union and map constraints", async t => {
  const { packageRoot } = await generatedFixture(t);
  const schemas = await import(pathToFileURL(join(packageRoot, "src/generated/zod.gen.ts")));
  const valid = { id: "12345678-1234-0234-0234-123456789abc", sequence: 1, time: "2026-10-03T12:00:00+08:00", requiredNullable: null };
  assert.equal(schemas.zFixtureOutput.safeParse(valid).success, true); // UUID version/variant unrestricted by Rust pattern.
  for (const change of [{ id: "00000000-0000-0000-0000-000000000000" }, { id: valid.id.toUpperCase() }, { sequence: 0 }, { sequence: 2 ** 53 }, { sequence: 1.5 }, { time: "2026-02-30T12:00:00Z" }, { time: "2026-10-03" }, { requiredNullable: undefined }, { optionalNullable: undefined }, { extra: true }]) {
    assert.equal(schemas.zFixtureOutput.safeParse({ ...valid, ...change }).success, false, JSON.stringify(change));
  }
  assert.equal(schemas.zFixtureOutput.safeParse({ ...valid, optionalNullable: null }).success, true);
  assert.equal(schemas.zFixtureOutput.safeParse({ ...valid, time: "2016-12-31t23:59:60z" }).success, true);
  for (const time of ["2024-02-29T12:34:56.123456789+05:30", "2016-12-31T23:59:60Z", "2016-12-31t23:59:60z", "2026-10-03t12:00:00-08:00"]) {
    assert.equal(schemas.zFixtureOutput.safeParse({ ...valid, time }).success, true, time);
    for (const suffix of ["\n", "\r\n", "\r", "\u2028", "\u2029"]) assert.equal(schemas.zFixtureOutput.safeParse({ ...valid, time: time + suffix }).success, false, JSON.stringify(time + suffix));
  }
  for (const time of ["2026-02-29T00:00:00Z", "2026-04-31T00:00:00Z", "2026-01-01T24:00:00Z", "2026-01-01T00:00:00+24:00"]) assert.equal(schemas.zFixtureOutput.safeParse({ ...valid, time }).success, false, time);
  for (const value of [-2147483648, 2147483647]) assert.equal(schemas.zFixtureInt32.safeParse(value).success, true);
  for (const value of [-2147483649, 2147483648, 0.5]) assert.equal(schemas.zFixtureInt32.safeParse(value).success, false);
  assert.equal(schemas.zFixtureInput.safeParse({ text: "😀😀😀" }).success, true);
  assert.equal(schemas.zFixtureInput.safeParse({ text: "😀😀😀😀" }).success, false);
  assert.equal(schemas.zFixtureProblem.safeParse({ code: "conflict", params: { revision: 5 } }).success, true);
  for (const problem of [{ code: "denied", params: { revision: 5 } }, { code: "other", params: {} }, { code: "denied", params: {}, extra: true }, { code: "conflict", params: [5] }]) assert.equal(schemas.zFixtureProblem.safeParse(problem).success, false);
  assert.equal(schemas.zFixtureInput.safeParse(["text"]).success, false);
  assert.equal(schemas.zFixtureInput.safeParse(JSON.parse('{"text":"a","__proto__":"unexpected"}')).success, false);
  const map = JSON.parse('{"__proto__":"value"}');
  const decoded = schemas.zFixtureMap.parse(map);
  assert.equal(Object.hasOwn(decoded, "__proto__"), true);
  assert.equal(Object.getPrototypeOf(decoded), Object.prototype);
  assert.equal(schemas.zFixtureMap.safeParse({ a: "a", b: "b", c: "c" }).success, false);
  assert.equal(schemas.zFixtureMap.safeParse({ Upper: "x" }).success, false);
  assert.equal(schemas.zFixtureMap.safeParse(JSON.parse('{"__proto__":123}')).success, false);
});

test("integer schemas must prove a lossless JS-safe range before emission", () => {
  for (const integer of [
    { type: "integer" },
    { type: "integer", minimum: 0 },
    { type: "integer", maximum: 100 },
    { type: "integer", format: "int64" },
    { type: "integer", format: "int64", minimum: 0 },
    { type: "integer", format: "int64", maximum: Number.MAX_SAFE_INTEGER },
    { type: "integer", minimum: 0, maximum: 2 ** 53 },
    { type: "integer", minimum: -(2 ** 53), maximum: 0 },
    { type: "integer", minimum: 10, maximum: 1 },
  ]) assert.throws(() => validators({ $defs: { FixtureInteger: integer } }), /integer|Integer/);
  for (const integer of [
    { type: "integer", format: "int32" },
    { type: "integer", format: "int32", minimum: 0 },
    { type: "integer", minimum: Number.MIN_SAFE_INTEGER, maximum: Number.MAX_SAFE_INTEGER },
    { type: "integer", format: "int64", minimum: 0, maximum: Number.MAX_SAFE_INTEGER },
  ]) assert.doesNotThrow(() => validators({ $defs: { FixtureInteger: integer } }));
});

test("strict generated TypeScript and operation success/error result inference", async t => {
  const { packageRoot, files } = await generatedFixture(t);
  await writeFile(join(packageRoot, "consumer.ts"), `import { ControlClient } from "./src/transport.ts";
import { writeFixture, clearFixture } from "./src/generated/operations.gen.ts";
declare const client: ControlClient;
const result = await writeFixture(client, {text: "a"}, {token: "secret"});
if (result.ok) { const id: string = result.data.id; void id; }
else if (result.error.kind === "http" && result.error.problem?.code === "conflict") { const revision: number = result.error.problem.params.revision; void revision; }
const empty = await clearFixture(client);
if (empty.ok) { const value: undefined = empty.data; void value; }
// @ts-expect-error authenticated operation requires token
writeFixture(client, {text: "a"}, {});
// @ts-expect-error fields come from generated schema
writeFixture(client, {text: 42}, {token: "secret"});
`);
  const result = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "--noEmit", "--strict", "--exactOptionalPropertyTypes", "--noUncheckedIndexedAccess", "--skipLibCheck", "--target", "ES2022", "--module", "ESNext", "--moduleResolution", "Bundler", "--allowImportingTsExtensions", "consumer.ts"], { cwd: packageRoot, encoding: "utf8" });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(files["crates/control-client/src/generated/mod.rs"], /meshway_protocol::FixtureInput/);
  assert.doesNotMatch(files["crates/control-client/src/generated/mod.rs"], /pub struct|pub enum|derive\(Debug/);
  assert.deepEqual(generatedFiles(fixture()), files);
});

test("unknown semantics and ambiguous unions fail generation", () => {
  for (const mutate of [
    d => { d.paths["/fixture/read"].get.responses["200"].content = { "text/event-stream": { schema: ref("FixtureOutput") } }; },
    d => { d.paths["/fixture/read"].get.parameters = [{ in: "query", name: "q" }]; },
    d => { d.paths["/fixture/read"].get.responses["200"].content["application/json"].schema = closed({}); },
    d => { d.paths["/fixture/read"].get.callbacks = {}; },
    d => { d.paths["/fixture/read"].get.operationId = "clearFixture"; },
    d => { d.paths["/fixture/clear"].post.responses["204"].content = content("FixtureOutput"); },
    d => { d.components.schemas.FixtureInput.properties.text.unknownConstraint = true; },
    d => { d.components.schemas.FixtureInput.additionalProperties = true; },
    d => { d.components.schemas.FixtureOutput.properties.id.pattern = "["; },
    d => { d.components.schemas.FixtureOutput.properties.sequence.maximum = 2 ** 53; },
    d => { d.components.schemas.FixtureProblem.oneOf[1].properties.code.enum = ["conflict"]; },
    d => { d.components.schemas.FixtureAlias.allOf.push(ref("FixtureOutput")); },
  ]) {
    const document = fixture(); mutate(document); assert.throws(() => generatedFiles(document));
  }
  assert.throws(() => operationModel({ ...fixture(), openapi: "3.0.3" }));
  assert.throws(() => validators({ $defs: { Recursive: ref("Recursive") } }));
});

test("drift detects edited, missing and obsolete files without modifying actual output", async t => {
  const dir = await temporary(t);
  for (const side of ["expected", "actual"]) await mkdir(join(dir, side, "generated"), { recursive: true });
  await writeFile(join(dir, "expected/generated/one"), "expected");
  await writeFile(join(dir, "actual/generated/one"), "user edit");
  await writeFile(join(dir, "expected/generated/missing"), "new");
  await writeFile(join(dir, "actual/generated/obsolete"), "old");
  assert.deepEqual(await compareArtifacts(join(dir, "expected"), join(dir, "actual"), ["generated"]), ["generated/missing", "generated/obsolete", "generated/one"]);
  assert.equal(await readFile(join(dir, "actual/generated/one"), "utf8"), "user edit");
});
