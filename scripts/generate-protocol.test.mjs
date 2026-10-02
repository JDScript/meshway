import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { operations, root, validators } from "./generate-protocol.mjs";

const require = createRequire(join(root, "packages/control-client/package.json"));
const zodUrl = pathToFileURL(require.resolve("zod")).href;
const temp = await mkdtemp(join(root, "node_modules/.codegen-test-"));
let sequence = 0;
async function runtime(document) {
  const file = join(temp, `schema-${sequence++}.mjs`);
  await writeFile(file, validators(document).replace('from "zod"', `from ${JSON.stringify(zodUrl)}`));
  return import(pathToFileURL(file).href);
}
const ref = name => ({ $ref: `#/components/schemas/${name}` });
const object = properties => ({ type: "object", additionalProperties: false, properties, required: Object.keys(properties) });
const variant = (kind, extra = {}) => object({ kind: { type: "string", const: kind }, ...extra });
const document = schemas => ({ openapi: "3.1.0", components: { schemas }, paths: {} });

try {
  await test("tagged oneOf/anyOf validate unique variants, fields and required nullable values", async () => {
    for (const keyword of ["oneOf", "anyOf"]) {
      const doc = document({
        Native: variant("native", { port: { type: "integer", minimum: 1, maximum: 65535 } }),
        Vless: variant("vless", { endpoint: { type: "string" } }),
        Carrier: { [keyword]: [ref("Native"), ref("Vless")] },
        Snapshot: object({ carrier: { anyOf: [ref("Carrier"), { type: "null" }] } }),
        Optional: { ...object({ label: { type: "string" }, carrier: { anyOf: [ref("Carrier"), { type: "null" }] } }), required: [] },
      });
      const { zCarrier, zSnapshot, zOptional } = await runtime(doc);
      for (const valid of [{ kind: "native", port: 443 }, { kind: "vless", endpoint: "wss://example.test" }]) {
        assert.deepEqual(zCarrier.parse(valid), valid);
        assert.equal(zSnapshot.safeParse({ carrier: valid }).success, true);
      }
      for (const invalid of [{ kind: "unknown" }, { kind: "native" }, { kind: "native", port: 0 }, { kind: "native", port: 443, endpoint: "x" }, { kind: "vless", endpoint: 3 }, null]) {
        assert.equal(zCarrier.safeParse(invalid).success, false);
      }
      assert.equal(zSnapshot.safeParse({ carrier: null }).success, true);
      assert.equal(zSnapshot.safeParse({}).success, false);
      assert.equal(zOptional.safeParse({}).success, true);
      assert.equal(zOptional.safeParse({ carrier: null }).success, true);
      assert.equal(zOptional.safeParse({ label: undefined }).success, false);
      assert.equal(zOptional.safeParse({ label: null }).success, false);
      assert.throws(() => validators(document({ Carrier: { [keyword]: [variant("native"), variant("native")] } })), /Conflicting discriminator/);
      assert.throws(() => validators(document({ Carrier: { [keyword]: [object({ value: { type: "string" } }), variant("native")] } })), /required string discriminator/);
      assert.throws(() => validators(document({ Carrier: { [keyword]: [{ ...variant("native"), additionalProperties: true }, variant("vless")] } })), /closed objects/);
    }
  });

  await test("typed maps check keys, nested values, nullability and property budgets", async () => {
    const { zMap, zScalar } = await runtime(document({
      Map: { type: "object", propertyNames: { type: "string", pattern: "^[A-Z]{2}$" }, additionalProperties: object({ target: { type: ["string", "null"] } }), maxProperties: 2 },
      Scalar: { anyOf: [{ type: "string" }, { type: "null" }] },
    }));
    assert.equal(zMap.safeParse({ US: { target: null }, JP: { target: "mesh-only" } }).success, true);
    for (const value of [{ us: { target: "x" } }, { US: { target: 1 } }, { US: {} }, { US: { target: "x", extra: true } }, { US: { target: "x" }, JP: { target: "x" }, KR: { target: "x" } }]) {
      assert.equal(zMap.safeParse(value).success, false);
    }
    assert.equal(zScalar.safeParse(null).success, true);
    assert.equal(zScalar.safeParse("x").success, true);
    assert.equal(zScalar.safeParse(1).success, false);
    for (const schema of [{ type: "object", additionalProperties: true }, { allOf: [{ type: "string" }] }, { type: "object", maxProperties: 2, properties: { kind: { type: "string" } }, required: [] }]) {
      assert.throws(() => validators(document({ Unsupported: schema })));
    }
  });

  await test("actual Rust policy schema: bounded variants/maps work and unclosed ExitMatch blocks publication", async () => {
    const result = spawnSync("cargo", ["run", "--locked", "--quiet", "-p", "meshway-policy", "--example", "policy_v3_schema"], {
      cwd: root, encoding: "utf8", env: { ...process.env, CARGO_BUILD_JOBS: "2", CARGO_TARGET_DIR: join(root, "target/p3a-codegen") },
    });
    assert.equal(result.status, 0, result.stderr);
    const schema = JSON.parse(result.stdout);
    assert.throws(() => validators(schema), /Component ExitMatch: Only closed objects/);
    const { zProfileFallback, zAutomaticRouting } = await runtime({ $defs: {
      ProfileFallback: schema.$defs.ProfileFallback,
      AutomaticRouting: schema.$defs.AutomaticRouting,
    } });
    for (const fallback of [{ kind: "none" }, { kind: "direct" }, { kind: "nodes", candidates: ["node:x"], via: [] }]) {
      assert.deepEqual(zProfileFallback.parse(fallback), fallback);
      assert.equal(zProfileFallback.safeParse({ ...fallback, unknown: true }).success, false);
    }
    assert.equal(zAutomaticRouting.safeParse({ regions: { JP: "mesh-only" }, fallback: "mesh-only" }).success, true);
    assert.equal(zAutomaticRouting.safeParse({ regions: { JP: 1 }, fallback: "mesh-only" }).success, false);

    // Explicit fixture for the source-owner handoff patch: do not conceal or
    // rewrite the authoritative schema in the production generator.
    for (const item of schema.$defs.ExitMatch.oneOf) item.maxProperties = 2;
    const { zNetworkPolicy } = await runtime(schema);
    const policy = {
      connections: [{ id: "default", a: "*", b: "*", action: "allow" }],
      grants: [{ id: "members", src: ["*"], dst: ["*"], bidirectional: false }],
      mesh: [], public_rules: [], assignments: [],
      profiles: [{ id: "custom", rules: [{ id: "r", destinations: [{ kind: "cidr", value: "10.0.0.0/8" }], target: "direct", candidates: [], via: [] }], fallback: { kind: "none" } }],
      automatic: { regions: { JP: "custom" }, fallback: "mesh-only" },
    };
    assert.deepEqual(zNetworkPolicy.parse(policy), policy);
    for (const mutate of [p => { p.automatic.regions.JP = 42; }, p => { p.profiles[0].fallback.unknown = true; }, p => { p.profiles[0].rules[0].destinations[0].unknown = true; }, p => { p.profiles[0].rules[0].destinations[0].kind = "unknown"; }, p => { delete p.grants; }]) {
      const invalid = structuredClone(policy);
      mutate(invalid);
      assert.equal(zNetworkPolicy.safeParse(invalid).success, false);
    }
    // Use the same Hey API plugin as production, then compile generated
    // operations against both inferred Zod output and generated DTOs.
    const schemas = JSON.parse(JSON.stringify(schema.$defs).replaceAll("#/$defs/", "#/components/schemas/"));
    const doc = document(schemas);
    const content = { "application/json": { schema: ref("NetworkPolicy") } };
    doc.paths["/fixture/policy"] = { post: {
      operationId: "saveFixturePolicy", requestBody: { required: true, content },
      responses: { 200: { description: "Fixture", content } },
    } };
    const generated = join(temp, "generated");
    await mkdir(generated);
    const input = join(temp, "fixture.json");
    await writeFile(input, JSON.stringify(doc));
    const { createClient } = await import(pathToFileURL(join(require.resolve("@hey-api/openapi-ts/package.json"), "../dist/index.mjs")).href);
    await createClient({ configFile: false, input, output: { path: generated }, plugins: ["@hey-api/typescript"] });
    await writeFile(join(generated, "zod.gen.ts"), validators(doc));
    await writeFile(join(generated, "operations.gen.ts"), operations(doc));
    await writeFile(join(temp, "transport.ts"), await readFile(join(root, "packages/control-client/src/transport.ts"), "utf8"));
    // Dependencies resolve through the actual SDK rather than adding a package.
    await writeFile(join(temp, "tsconfig.json"), JSON.stringify({ compilerOptions: {
      strict: true, noUncheckedIndexedAccess: true, exactOptionalPropertyTypes: true, noEmit: true, target: "ES2023", module: "ESNext", moduleResolution: "Bundler", allowImportingTsExtensions: true,
      paths: { zod: [join(dirname(require.resolve("zod/package.json")), "index.d.ts")] },
    }, include: ["**/*.ts"] }));
    const checked = spawnSync("pnpm", ["exec", "tsc", "-p", join(temp, "tsconfig.json")], { cwd: root, encoding: "utf8" });
    assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  });

  await test("existing OpenAPI validators and operations remain byte-compatible", async () => {
    const doc = JSON.parse(await readFile(join(root, "packages/protocol/openapi.json"), "utf8"));
    for (const [file, emit] of [["zod.gen.ts", validators], ["operations.gen.ts", operations]]) {
      assert.equal(emit(doc), await readFile(join(root, "packages/control-client/src/generated", file), "utf8"));
    }
  });
} finally {
  await rm(temp, { recursive: true, force: true });
}
