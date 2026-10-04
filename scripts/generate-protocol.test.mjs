import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink, copyFile, cp, statfs } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { createServer } from "node:http";
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
      "/fixture/remove": { delete: { operationId: "removeFixture", security: [{ FixtureBearer: [] }], requestBody: { required: true, content: content("FixtureInput") }, responses: { "204": { description: "Removed" } } } },
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
async function generatedFixture(t, document = fixture()) {
  const directory = await temporary(t);
  const files = generatedFiles(document);
  for (const [path, text] of Object.entries(files)) {
    await mkdir(dirname(join(directory, path)), { recursive: true });
    await writeFile(join(directory, path), text);
  }
  const packageRoot = join(directory, "packages/control-client");
  await mkdir(join(packageRoot, "node_modules"), { recursive: true });
  await symlink(dirname(require.resolve("zod/package.json")), join(packageRoot, "node_modules/zod"));
  await copyFile(join(root, "packages/control-client/src/transport.ts"), join(packageRoot, "src/transport.ts"));
  await copyFile(join(root, "packages/control-client/src/sse.ts"), join(packageRoot, "src/sse.ts"));
  await copyFile(join(root, "packages/control-client/src/json.ts"), join(packageRoot, "src/json.ts"));
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
    { type: "integer", format: "int32", minimum: -2147483649 },
    { type: "integer", format: "int32", maximum: 2147483648 },
    { type: "integer", format: "int32", minimum: 2147483648, maximum: 2147483649 },
    { type: "integer", format: null, minimum: 1, maximum: 100 },
  ]) assert.throws(() => validators({ $defs: { FixtureInteger: integer } }), /integer|Integer/);
  for (const integer of [
    { type: "integer", format: "int32" },
    { type: "integer", format: "int32", minimum: 0 },
    { type: "integer", minimum: Number.MIN_SAFE_INTEGER, maximum: Number.MAX_SAFE_INTEGER },
    { type: "integer", format: "int64", minimum: 0, maximum: Number.MAX_SAFE_INTEGER },
  ]) assert.doesNotThrow(() => validators({ $defs: { FixtureInteger: integer } }));
});

test("generated binary64 schemas retain measurement bounds and nullable absence", async t => {
  const document = fixture();
  document.components.schemas.FixtureLatitude = { type: "number", format: "double", minimum: -90, maximum: 90 };
  document.components.schemas.FixturePositiveMeasurement = { type: "number", exclusiveMinimum: 0, exclusiveMaximum: 10 };
  document.components.schemas.FixtureOptionalMeasurement = { type: ["number", "null"], format: "double", minimum: 0 };
  const { packageRoot } = await generatedFixture(t, document);
  const schemas = await import(pathToFileURL(join(packageRoot, "src/generated/zod.gen.ts")));
  for (const value of [-90, -40.7128, 0, 89.999999, 90]) assert.equal(schemas.zFixtureLatitude.safeParse(value).success, true);
  for (const value of [-90.0001, 90.0001, NaN, Infinity, -Infinity, "45", null]) assert.equal(schemas.zFixtureLatitude.safeParse(value).success, false);
  assert.equal(schemas.zFixturePositiveMeasurement.safeParse(0.125).success, true);
  for (const value of [0, 10]) assert.equal(schemas.zFixturePositiveMeasurement.safeParse(value).success, false);
  assert.equal(schemas.zFixtureOptionalMeasurement.parse(null), null);
  assert.equal(schemas.zFixtureOptionalMeasurement.parse(0), 0);
  assert.equal(schemas.zFixtureOptionalMeasurement.safeParse(undefined).success, false);
  for (const schema of [
    { type: "number", minimum: NaN },
    { type: "number", maximum: Infinity },
    { type: "number", exclusiveMinimum: true },
    { type: "number", minimum: 10, maximum: 1 },
    { type: "number", minimum: 1, exclusiveMaximum: 1 },
    { type: "number", format: "float" },
  ]) assert.throws(() => validators({ $defs: { FixtureNumber: schema } }));
});

test("strict generated TypeScript and operation success/error result inference", async t => {
  const { packageRoot, files } = await generatedFixture(t);
  await writeFile(join(packageRoot, "consumer.ts"), `import { ControlClient } from "./src/transport.ts";
import { writeFixture, clearFixture, removeFixture } from "./src/generated/operations.gen.ts";
declare const client: ControlClient;
const result = await writeFixture(client, {text: "a"}, {token: "secret"});
if (result.ok) { const id: string = result.data.id; void id; }
else if (result.error.kind === "http" && result.error.problem?.code === "conflict") { const revision: number = result.error.problem.params.revision; void revision; }
const empty = await clearFixture(client);
await removeFixture(client, {text: "a"}, {token: "secret"});
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

function parameterFixture() {
  const document = fixture();
  document.components.schemas.FixtureNetworkId = { ...document.components.schemas.FixtureOutput.properties.id, minLength: 36, maxLength: 36 };
  document.components.schemas.FixtureRevision = { type: "integer", minimum: 1, maximum: Number.MAX_SAFE_INTEGER };
  const query = [
    { in: "query", name: "limit", schema: { type: "integer", format: "int32", minimum: 1, maximum: 100 } },
    { in: "query", name: "cursor", schema: { type: "string", maxLength: 100 } },
    { in: "query", name: "enabled", schema: { type: "boolean" } },
    { in: "query", name: "revision", schema: ref("FixtureRevision") },
  ];
  const responses = { "204": { description: "Empty" }, "409": { description: "Conflict", content: content("FixtureProblem") } };
  document.paths["/fixture/networks/{networkId}/members"] = {
    parameters: [{ in: "path", name: "networkId", required: true, schema: ref("FixtureNetworkId") }],
    get: { operationId: "listFixtureMembers", parameters: query, security: [{ FixtureBearer: [] }], responses },
  };
  document.paths["/fixture/value/{value}"] = {
    post: { operationId: "renameFixture", parameters: [{ in: "path", name: "value", required: true, schema: { type: "string", minLength: 1 } }, ...query], requestBody: { required: true, content: content("FixtureInput") }, security: [{ FixtureBearer: [] }], responses },
  };
  document.paths["/fixture/networks"] = { get: { operationId: "listFixtureNetworks", parameters: query, security: [{ FixtureBearer: [] }], responses } };
  document.paths["/fixture/networks/{networkId}"] = {
    parameters: [{ in: "path", name: "networkId", required: true, schema: ref("FixtureNetworkId") }],
    get: { operationId: "getFixtureNetwork", security: [{ FixtureBearer: [] }], responses },
  };
  document.paths["/fixture/scalars/{number}/{flag}"] = {
    get: { operationId: "scalarFixture", parameters: [
      { in: "path", name: "number", required: true, schema: { type: "integer", minimum: -100, maximum: 100 } },
      { in: "path", name: "flag", required: true, schema: { type: "boolean" } },
      { in: "query", name: "mode", schema: { type: "string", enum: ["\u0000\b\f\\back", "plain"] } },
    ], responses },
  };
  document.components.schemas.FixtureEvent = { oneOf: [
    closed({ type: { type: "string", const: "snapshot" }, text: { type: "string" } }),
    closed({ type: { type: "string", const: "ready" }, cursor: { type: "string" } }),
  ] };
  document.paths["/fixture/networks/{networkId}/events"] = {
    parameters: [{ in: "path", name: "networkId", required: true, schema: ref("FixtureNetworkId") }],
    get: { operationId: "watchFixture", security: [{ FixtureBearer: [] }],
      parameters: [{ in: "header", name: "Last-Event-ID", required: false, schema: { type: "string" } }],
      "x-meshway-sse": { format: "json-data", eventSchema: ref("FixtureEvent") },
      responses: { "200": { description: "Events", content: { "text/event-stream": { schema: { type: "string" } } } }, "409": { description: "Conflict", content: content("FixtureProblem") } },
    },
  };
  document.paths["/fixture/events"] = { get: {
    operationId: "watchPublicFixture",
    "x-meshway-sse": { format: "json-data", eventSchema: ref("FixtureEvent") },
    responses: { "200": { description: "Events", content: { "text/event-stream": { schema: { type: "string" } } } } },
  } };
  return document;
}

test("SSE declarations require explicit frame schemas and reject ambiguous HTTP semantics", () => {
  for (const mutate of [
    op => { delete op["x-meshway-sse"]; },
    op => { op["x-meshway-sse"].format = "raw"; },
    op => { op["x-meshway-sse"].eventSchema = ref("Absent"); },
    op => { op["x-meshway-sse"].eventSchema = closed({}); },
    op => { op["x-meshway-sse"].unknown = true; },
    op => { op.responses["200"].content["application/json"] = { schema: ref("FixtureEvent") }; },
    op => { op.responses["200"].content["text/event-stream"].schema = ref("FixtureEvent"); },
    op => { op.responses["204"] = op.responses["200"]; delete op.responses["200"]; },
    op => { op.requestBody = { required: true, content: content("FixtureInput") }; },
    op => { op.responses["409"].content = { "text/event-stream": { schema: { type: "string" } } }; },
    op => { op.operationId = "stream"; },
    op => { op.parameters[0].required = true; },
    op => { op.parameters[0].schema = { type: ["string", "null"] }; },
    op => { op.parameters[0].schema.maxLength = 1; },
    op => { op.parameters[0].name = "Authorization"; },
    op => { op.parameters.push({ ...op.parameters[0], name: "last-event-id" }); },
  ]) {
    const d = parameterFixture(); mutate(d.paths["/fixture/networks/{networkId}/events"].get);
    assert.throws(() => generatedFiles(d));
  }
  const d = parameterFixture();
  const route = d.paths["/fixture/networks/{networkId}/events"];
  route.post = route.get; delete route.get;
  assert.throws(() => generatedFiles(d));
});

test("generated SSE TS validates live HTTP frames and reconnects from the caller-applied cursor", async t => {
  const { packageRoot } = await generatedFixture(t, parameterFixture());
  const { watchFixture } = await import(pathToFileURL(join(packageRoot, "src/generated/operations.gen.ts")));
  const { ControlClient } = await import(pathToFileURL(join(packageRoot, "src/transport.ts")));
  const seen = [];
  const server = createServer((request, response) => {
    seen.push({ url: request.url, auth: request.headers.authorization, accept: request.headers.accept, cursor: request.headers["last-event-id"] });
    response.writeHead(200, { "content-type": "text/event-stream" });
    if (seen.length > 1) return response.end('data: {"type":"snapshot","text":123}\n\n');
    for (const byte of new TextEncoder().encode('\uFEFF: keepalive\r\ndata: {"type":"snapshot","text":"😀"}\r\n\r\nid: merely-decoded\ndata: {"type":"ready","cursor":"ready-cursor"}\n\n')) response.write(Uint8Array.of(byte));
    response.end();
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  const client = new ControlClient({ baseUrl: `http://127.0.0.1:${server.address().port}` });
  const networkId = "12345678-1234-0234-0234-123456789abc";
  const output = [];
  let applied;
  for await (const result of watchFixture(client, { networkId }, { token: "private-token", maxReconnects: 1, retryDelayMs: 100, resumeCursor: () => applied })) {
    output.push(result);
    if (result.ok && result.data.data.type === "ready") applied = result.data.data.cursor;
  }
  assert.deepEqual(output, [
    { ok: true, data: { data: { type: "snapshot", text: "😀" } } },
    { ok: true, data: { data: { type: "ready", cursor: "ready-cursor" }, id: "merely-decoded" } },
    { ok: false, error: { kind: "network", status: 200 } },
    { ok: false, error: { kind: "decode", status: 200 } },
  ]);
  assert.deepEqual(seen, [undefined, "ready-cursor"].map(cursor => ({ url: `/fixture/networks/${networkId}/events`, auth: "Bearer private-token", accept: "text/event-stream", cursor })));
  await writeFile(join(packageRoot, "consumer.ts"), `import { ControlClient } from "./src/transport.ts";
import { watchFixture } from "./src/generated/operations.gen.ts";
declare const client: ControlClient;
for await (const result of watchFixture(client, {networkId: "${networkId}"}, {token: "secret", resumeCursor: () => undefined})) {
  if (result.ok && result.data.data.type === "ready") { const cursor: string = result.data.data.cursor; void cursor; }
  else if (!result.ok && result.error.kind === "http" && result.error.problem?.code === "conflict") { const revision: number = result.error.problem.params.revision; void revision; }
}
// @ts-expect-error authenticated streams require a token
watchFixture(client, {networkId: "${networkId}"}, {maxReconnects: 0});
// @ts-expect-error resume cursor must be a string or undefined
watchFixture(client, {networkId: "${networkId}"}, {token: "secret", resumeCursor: () => 42});
`);
  const result = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "--noEmit", "--strict", "--exactOptionalPropertyTypes", "--noUncheckedIndexedAccess", "--skipLibCheck", "--target", "ES2022", "--module", "ESNext", "--moduleResolution", "Bundler", "--allowImportingTsExtensions", "consumer.ts"], { cwd: packageRoot, encoding: "utf8" });
  assert.equal(result.status, 0, result.stdout + result.stderr);
});

test("parameter declarations, scalar semantics and template binding fail closed", () => {
  const route = "/fixture/networks/{networkId}/members";
  for (const mutate of [
    d => { d.paths[route].parameters = []; },
    d => { d.paths[route].get.parameters = null; },
    d => { d.paths[route].parameters[0].name = "unknown"; },
    d => { d.paths[route].parameters[0].required = false; },
    d => { d.paths[route].get.parameters.push(d.paths[route].parameters[0]); },
    d => { d.paths[route].get.parameters.push(d.paths[route].get.parameters[0]); },
    d => { d.paths[route].parameters[0].style = "label"; },
    d => { d.paths[route].parameters[0].explode = true; },
    d => { d.paths[route].get.parameters[0].explode = false; },
    d => { d.paths[route].get.parameters[0].style = "spaceDelimited"; },
    d => { d.paths[route].get.parameters[0].allowReserved = true; },
    d => { d.paths[route].get.parameters[0].in = "header"; },
    d => { d.paths[route].get.parameters[0].required = true; },
    d => { d.paths[route].get.parameters[0].schema = { type: "array", items: { type: "string" } }; },
    d => { d.paths[route].get.parameters[0].schema = closed({}); },
    d => { d.paths[route].get.parameters[0].schema = { type: ["string", "null"] }; },
    d => { d.paths[route].get.parameters[0].schema = { type: "integer", format: "int64" }; },
    d => { d.paths[route].get.parameters[0].schema = { type: "string", pattern: "^.*$" }; },
    d => { d.paths[route].get.parameters[0].schema = { type: "string", format: "date-time" }; },
    d => { d.paths[route].get.parameters[0].schema = { type: "string", unknown: true }; },
    d => { d.paths["/fixture/{networkId}/{networkId}"] = d.paths[route]; delete d.paths[route]; },
    d => { d.paths["//evil/{networkId}"] = d.paths[route]; delete d.paths[route]; },
    d => { d.paths["/fixture/{networkId}?secret=x"] = d.paths[route]; delete d.paths[route]; },
    d => { d.components.schemas.ListFixtureMembersQuery = closed({}); },
    d => { delete d.components.schemas.FixtureNetworkId.maxLength; },
  ]) { const d = parameterFixture(); mutate(d); assert.throws(() => generatedFiles(d)); }
  const d = parameterFixture();
  d.paths[route].parameters[0].style = "simple";
  d.paths[route].parameters[0].explode = false;
  d.paths[route].get.parameters[0].style = "form";
  d.paths[route].get.parameters[0].explode = true;
  assert.deepEqual(generatedFiles(d), generatedFiles(parameterFixture()));
});

test("generated parameter TS compiles and sends encoded path/query through a real HTTP server", async t => {
  const { packageRoot, files } = await generatedFixture(t, parameterFixture());
  const { listFixtureMembers, renameFixture, listFixtureNetworks, getFixtureNetwork, scalarFixture } = await import(pathToFileURL(join(packageRoot, "src/generated/operations.gen.ts")));
  const { ControlClient } = await import(pathToFileURL(join(packageRoot, "src/transport.ts")));
  const received = [];
  const server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += chunk;
    received.push({ url: request.url, auth: request.headers.authorization, body, method: request.method });
    response.writeHead(204).end();
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  const client = new ControlClient({ baseUrl: `http://127.0.0.1:${server.address().port}` });
  const networkId = "12345678-1234-0234-0234-123456789abc";
  assert.equal((await listFixtureMembers(client, { networkId }, { limit: 3, cursor: "", enabled: false, revision: Number.MAX_SAFE_INTEGER }, { token: "private-token" })).ok, true);
  assert.deepEqual(received[0], { method: "GET", url: `/fixture/networks/${networkId}/members?cursor=&enabled=false&limit=3&revision=9007199254740991`, auth: "Bearer private-token", body: "" });
  const raw = "a/\\?#%2F%2e Unicode😀 !'()*";
  const encoded = "a%2F%5C%3F%23%252F%252e%20Unicode%F0%9F%98%80%20%21%27%28%29%2A";
  for (let i = 0; i < 2; i++) assert.equal((await renameFixture(client, { value: raw }, { cursor: raw }, { text: "x" }, { token: "private-token" })).ok, true);
  assert.deepEqual(received[1], { method: "POST", url: `/fixture/value/${encoded}?cursor=${encoded}`, auth: "Bearer private-token", body: '{"text":"x"}' });
  assert.deepEqual(received[1], received[2]); // no mutation/pre-encoding on repeated use
  assert.equal((await listFixtureMembers(client, { networkId }, {}, { token: "private-token" })).ok, true);
  assert.equal(received[3].url, `/fixture/networks/${networkId}/members`);
  const count = received.length;
  for (const path of [{}, { networkId: "bad" }, { networkId: networkId.toUpperCase() }, { networkId: networkId + "\n" }, { networkId, extra: "x" }, { networkId: null }]) {
    assert.deepEqual(await listFixtureMembers(client, path, {}, { token: "private-token" }), { ok: false, error: { kind: "request" } });
  }
  for (const query of [{ limit: 0 }, { limit: 101 }, { limit: 1.5 }, { cursor: null }, { cursor: undefined }, { token: "query-secret" }, { enabled: "true" }, { revision: 0 }, { revision: 2 ** 53 }]) {
    assert.deepEqual(await listFixtureMembers(client, { networkId }, query, { token: "private-token" }), { ok: false, error: { kind: "request" } });
  }
  for (const value of ["", ".", "..", "\ud800"]) assert.deepEqual(await renameFixture(client, { value }, {}, { text: "x" }, { token: "private-token" }), { ok: false, error: { kind: "request" } });
  assert.equal(received.length, count);
  assert.equal((await listFixtureNetworks(client, { limit: 5 }, { token: "private-token" })).ok, true);
  assert.equal(received.at(-1).url, "/fixture/networks?limit=5");
  assert.equal((await getFixtureNetwork(client, { networkId }, { token: "private-token" })).ok, true);
  assert.equal(received.at(-1).url, `/fixture/networks/${networkId}`);
  assert.equal((await scalarFixture(client, { number: -3, flag: false }, { mode: "\u0000\b\f\\back" })).ok, true);
  assert.equal(received.at(-1).url, "/fixture/scalars/-3/false?mode=%00%08%0C%5Cback");
  assert.deepEqual(await scalarFixture(client, { number: 101, flag: false }, {}), { ok: false, error: { kind: "request" } });
  assert.deepEqual(await scalarFixture(client, { number: 1, flag: false }, { mode: "invalid" }), { ok: false, error: { kind: "request" } });
  let ipv6;
  const ipv6Client = new ControlClient({ baseUrl: "http://[::1]:9876", fetch: async url => { ipv6 = String(url); return new Response(null, { status: 204 }); } });
  assert.equal((await renameFixture(ipv6Client, { value: "https://evil.test/#x" }, {}, { text: "x" }, { token: "private-token" })).ok, true);
  assert.equal(ipv6, "http://[::1]:9876/fixture/value/https%3A%2F%2Fevil.test%2F%23x");
  await writeFile(join(packageRoot, "consumer.ts"), `import { ControlClient } from "./src/transport.ts";
import { listFixtureMembers, renameFixture } from "./src/generated/operations.gen.ts";
import type { ListFixtureMembersPath, ListFixtureMembersQuery } from "./src/generated/types.gen.ts";
declare const client: ControlClient;
const path: ListFixtureMembersPath = { networkId: "${networkId}" };
const query: ListFixtureMembersQuery = { limit: 5, cursor: "", enabled: false, revision: 1 };
await listFixtureMembers(client, path, query, {token: "secret"});
await renameFixture(client, {value: "a"}, {}, {text: "x"}, {token: "secret"});
// @ts-expect-error missing required path
listFixtureMembers(client, {}, {}, {token: "secret"});
// @ts-expect-error boolean query is typed
listFixtureMembers(client, path, {enabled: "true"}, {token: "secret"});
// @ts-expect-error optional is not nullable
listFixtureMembers(client, path, {cursor: null}, {token: "secret"});
// @ts-expect-error explicit undefined is not omission
listFixtureMembers(client, path, {cursor: undefined}, {token: "secret"});
// @ts-expect-error token cannot be a query parameter
listFixtureMembers(client, path, {token: "secret"}, {token: "secret"});
`);
  const result = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "--noEmit", "--strict", "--exactOptionalPropertyTypes", "--noUncheckedIndexedAccess", "--skipLibCheck", "--target", "ES2022", "--module", "ESNext", "--moduleResolution", "Bundler", "--allowImportingTsExtensions", "consumer.ts"], { cwd: packageRoot, encoding: "utf8" });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.deepEqual(generatedFiles(parameterFixture()), files);
});

test("generated Rust parameter methods compile and execute against HTTP with shared typed aliases", async t => {
  const disk = await statfs(root);
  assert.ok(disk.bavail * disk.bsize >= 50 * 1024 ** 3, "Rust fixture requires 50 GiB free; retire inactive owned caches first");
  const { directory, files } = await generatedFixture(t, parameterFixture());
  const project = join(directory, "rust-fixture");
  await mkdir(join(project, "src"), { recursive: true });
  await copyFile(join(root, "crates/control-client/src/runtime.rs"), join(project, "src/runtime.rs"));
  await cp(join(root, "crates/control-client/src/sse"), join(project, "src/sse"), { recursive: true });
  await writeFile(join(project, "src/generated.rs"), files["crates/control-client/src/generated/mod.rs"]);
  await writeFile(join(project, "Cargo.toml"), `[workspace]
[package]
name = "meshway-codegen-parameter-fixture"
version = "0.0.0"
edition = "2024"
[dependencies]
bytes = "1"
meshway-model = { path = ${JSON.stringify(join(root, "crates/model"))} }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.149"
futures-util = "0.3"
reqwest = { version = "0.13", default-features = false, features = ["rustls", "stream"] }
reqwest-middleware = "0.5"
axum = "0.8.9"
tokio = { version = "1.49.0", features = ["macros", "rt-multi-thread", "net", "sync", "time"] }
`);
  await writeFile(join(project, "src/lib.rs"), `#![forbid(unsafe_code)]
extern crate self as meshway_protocol;
mod runtime;
mod generated;
mod sse;
pub use runtime::{Client, ClientError};
pub use sse::{EventStream, EventDelivery, StreamOptions, StreamResponse};
pub use generated::*;
pub use meshway_model::{NetworkId as FixtureNetworkId, Revision as FixtureRevision};
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FixtureInput { pub text: String }
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FixtureOutput { pub id: FixtureNetworkId, pub sequence: u64, pub time: String, pub required_nullable: Option<String> }
#[derive(serde::Deserialize)]
pub struct FixtureProblem { pub code: String }
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum FixtureEvent { Snapshot { text: String }, Ready { cursor: String } }
#[cfg(test)]
mod parameter_tests {
    use super::*;
    #[tokio::test]
    async fn actual_generated_event_request() {
        let router = axum::Router::new().route("/fixture/networks/{networkId}/events", axum::routing::get(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["accept"], "text/event-stream");
            assert_eq!(headers["authorization"], "Bearer private-token");
            assert_eq!(headers["last-event-id"], "caller-applied");
            axum::http::Response::builder().header("content-type", "text/event-stream")
                .body(axum::body::Body::from("data: {\\"type\\":\\"snapshot\\",\\"text\\":\\"😀\\"}\\n\\nid: decoded\\ndata: {\\"type\\":\\"ready\\",\\"cursor\\":\\"ready\\"}\\n\\n")).unwrap()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::new(&format!("http://{}", listener.local_addr().unwrap()), reqwest::Client::builder(), 1024).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        let mut events = client.watch_fixture(&WatchFixturePath { network_id: "12345678-1234-0234-0234-123456789abc".parse().unwrap() }, "private-token", &StreamOptions { applied_cursor: Some("caller-applied"), ..Default::default() }).await.unwrap();
        let first = events.next().await.unwrap().unwrap();
        assert!(first.id.is_none());
        assert!(matches!(first.data, FixtureEvent::Snapshot { text } if text == "😀"));
        let ready = events.next().await.unwrap().unwrap();
        assert_eq!(ready.id.as_deref(), Some("decoded"));
        assert!(matches!(ready.data, FixtureEvent::Ready { cursor } if cursor == "ready"));
        assert!(events.next().await.is_none());
        server.abort();
    }
    #[tokio::test]
    async fn actual_parameter_requests() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = seen.clone();
        let router = axum::Router::new().fallback(move |uri: axum::http::Uri, headers: axum::http::HeaderMap, body: String| {
            let received = received.clone();
            async move {
                received.lock().unwrap().push((uri.to_string(), headers.get("authorization").map(|value| value.to_str().unwrap()).unwrap_or("").to_owned(), body));
                axum::http::StatusCode::NO_CONTENT
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::new(&format!("http://{}", listener.local_addr().unwrap()), reqwest::Client::builder(), 1024).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        let path = ListFixtureMembersPath { network_id: "12345678-1234-0234-0234-123456789abc".parse().unwrap() };
        let query = ListFixtureMembersQuery { cursor: Some(String::new()), enabled: Some(false), limit: Some(3), revision: Some(FixtureRevision::MAX) };
        client.list_fixture_members(&path, &query, "private-token").await.unwrap();
        client.list_fixture_members(&path, &ListFixtureMembersQuery::default(), "private-token").await.unwrap();
        let value = "a/\\\\?#%2F%2e Unicode😀 !'()*";
        let rename_path = RenameFixturePath { value: value.to_owned() };
        let rename_query = RenameFixtureQuery { cursor: Some(value.to_owned()), ..Default::default() };
        for _ in 0..2 { client.rename_fixture(&rename_path, &rename_query, &FixtureInput { text: "x".into() }, "private-token").await.unwrap(); }
        for limit in [0, 101] {
            assert!(matches!(client.list_fixture_members(&path, &ListFixtureMembersQuery { limit: Some(limit), ..Default::default() }, "private-token").await, Err(ClientError::Request)));
        }
        for value in ["", ".", ".."] {
            assert!(matches!(client.rename_fixture(&RenameFixturePath { value: value.into() }, &RenameFixtureQuery::default(), &FixtureInput { text: "x".into() }, "private-token").await, Err(ClientError::Request)));
        }
        client.list_fixture_networks(&ListFixtureNetworksQuery { limit: Some(5), ..Default::default() }, "private-token").await.unwrap();
        client.get_fixture_network(&GetFixtureNetworkPath { network_id: path.network_id }, "private-token").await.unwrap();
        client.scalar_fixture(&ScalarFixturePath { number: -3, flag: false }, &ScalarFixtureQuery { mode: Some("\\0\\u{8}\\u{c}\\\\back".into()) }).await.unwrap();
        assert!(matches!(client.scalar_fixture(&ScalarFixturePath { number: 101, flag: false }, &ScalarFixtureQuery::default()).await, Err(ClientError::Request)));
        assert!(matches!(client.scalar_fixture(&ScalarFixturePath { number: 1, flag: false }, &ScalarFixtureQuery { mode: Some("invalid".into()) }).await, Err(ClientError::Request)));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 7);
        assert_eq!(seen[0].0, "/fixture/networks/12345678-1234-0234-0234-123456789abc/members?cursor=&enabled=false&limit=3&revision=9007199254740991");
        assert_eq!(seen[1].0, "/fixture/networks/12345678-1234-0234-0234-123456789abc/members");
        assert_eq!(seen[2].0, "/fixture/value/a%2F%5C%3F%23%252F%252e%20Unicode%F0%9F%98%80%20%21%27%28%29%2A?cursor=a%2F%5C%3F%23%252F%252e%20Unicode%F0%9F%98%80%20%21%27%28%29%2A");
        assert_eq!(seen[2], seen[3]);
        assert!(seen[..6].iter().all(|request| request.1 == "Bearer private-token"));
        assert_eq!(seen[2].2, r#"{"text":"x"}"#);
        assert_eq!(seen[4].0, "/fixture/networks?limit=5");
        assert_eq!(seen[5].0, "/fixture/networks/12345678-1234-0234-0234-123456789abc");
        assert_eq!(seen[6].0, "/fixture/scalars/-3/false?mode=%00%08%0C%5Cback");
        server.abort();
    }
}
`);
  const result = spawnSync("cargo", ["test", "--offline", "--quiet"], { cwd: project, encoding: "utf8", env: { ...process.env, CARGO_TARGET_DIR: join(root, "target"), CARGO_INCREMENTAL: "0", CARGO_BUILD_JOBS: "4" }, timeout: 240_000 });
  assert.equal(result.status, 0, result.error?.message ?? result.stdout + result.stderr);
  const clippy = spawnSync("cargo", ["clippy", "--offline", "--all-targets", "--", "-D", "warnings"], { cwd: project, encoding: "utf8", env: { ...process.env, CARGO_TARGET_DIR: join(root, "target"), CARGO_INCREMENTAL: "0", CARGO_BUILD_JOBS: "4" }, timeout: 240_000 });
  assert.equal(clippy.status, 0, clippy.stdout + clippy.stderr);
});
