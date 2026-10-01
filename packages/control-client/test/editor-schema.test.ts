import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { componentSchemas, schemaFor } from "../src/generated/json-schema.gen.ts";

test("JSONC schemas preserve the complete Rust OpenAPI components and reference targets", async () => {
  const document: unknown = JSON.parse(await readFile(new URL("../../protocol/openapi.json", import.meta.url), "utf8"));
  assert(document && typeof document === "object" && "components" in document);
  const components = document.components;
  assert(components && typeof components === "object" && "schemas" in components);
  assert.deepEqual(componentSchemas, components.schemas);
  assert.equal(schemaFor("RoutingPolicy").$ref, "#/components/schemas/RoutingPolicy");
  assert.equal(schemaFor("RoutingPolicy").components.schemas.RoutingPolicy, componentSchemas.RoutingPolicy);
  assert.equal(componentSchemas.MeshAccessGrant.properties.path.maxItems, 8);
  assert.equal(componentSchemas.MeshAccessGrant.properties.id.maxLength, 64);
});
