import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import * as schemas from "../src/generated/zod.gen.ts";

const root = fileURLToPath(new URL("../../..", import.meta.url));
const document = JSON.parse(await readFile(join(root, "packages/protocol/openapi.json"), "utf8"));

type Media = { schema: { $ref: string }; example?: unknown };
type Operation = {
  operationId: string;
  requestBody?: { content: { "application/json"?: Media } };
  responses: Record<string, { content?: { "application/json"?: Media } }>;
};
const operations = new Map<string, Operation>();
for (const item of Object.values(document.paths) as Record<string, Operation>[])
  for (const method of ["get", "post", "delete"]) if (item[method]) operations.set(item[method].operationId, item[method]);

function validate(media: Media, value: unknown, label: string, errors: string[]) {
  const name = media.schema.$ref.replace(/^#\/components\/schemas\//, "");
  const schema = (schemas as Record<string, unknown>)[`z${name}`] as { safeParse(value: unknown): { success: boolean; error?: { issues: { path: PropertyKey[]; code: string; message: string }[] } } } | undefined;
  if (!schema) { errors.push(`${label}: no generated schema z${name}`); return; }
  const result = schema.safeParse(value);
  if (!result.success) errors.push(`${label} (${name}): ${result.error?.issues.map(issue => `${issue.path.join(".") || "<root>"}: ${issue.message}`).join("; ")}`);
}

type Fixture = { request?: unknown; responses?: Record<string, unknown> };
async function fixtures(): Promise<[string, string, Fixture][]> {
  const directory = join(root, "packages/protocol/examples");
  const entries: [string, string, Fixture][] = [];
  for (const file of (await readdir(directory)).sort()) {
    if (!file.endsWith(".json")) continue;
    const parsed = JSON.parse(await readFile(join(directory, file), "utf8")) as Record<string, Fixture>;
    for (const [id, entry] of Object.entries(parsed)) entries.push([file, id, entry]);
  }
  return entries;
}

test("every fixture example parses with the operation's generated schema", async () => {
  const errors: string[] = [];
  let count = 0;
  {
    for (const [file, id, entry] of await fixtures()) {
      const operation = operations.get(id);
      if (!operation) { errors.push(`${file}/${id}: unknown operation`); continue; }
      if (entry.request !== undefined) {
        count++;
        const media = operation.requestBody?.content["application/json"];
        if (!media) errors.push(`${file}/${id}: request example without a JSON request body`);
        else validate(media, entry.request, `${file}/${id} request`, errors);
      }
      for (const [status, example] of Object.entries(entry.responses ?? {})) {
        count++;
        const media = operation.responses[status]?.content?.["application/json"];
        if (!media) errors.push(`${file}/${id} ${status}: no JSON response declared`);
        else validate(media, example, `${file}/${id} ${status}`, errors);
      }
    }
  }
  assert.deepEqual(errors, []);
  assert.ok(count > 0);
});

test("fixtures cover every JSON request body and success response", async () => {
  const covered = new Set<string>();
  for (const [, id, entry] of await fixtures()) {
    if (entry.request !== undefined) covered.add(`${id} request`);
    for (const status of Object.keys(entry.responses ?? {})) covered.add(`${id} ${status}`);
  }
  const missing: string[] = [];
  for (const operation of operations.values()) {
    if (operation.requestBody?.content["application/json"] && !covered.has(`${operation.operationId} request`)) missing.push(`${operation.operationId} request`);
    for (const [status, response] of Object.entries(operation.responses))
      if (status.startsWith("2") && response.content?.["application/json"] && !covered.has(`${operation.operationId} ${status}`)) missing.push(`${operation.operationId} ${status}`);
  }
  assert.deepEqual(missing, []);
});

test("every example in the exported contract parses with its schema", () => {
  const errors: string[] = [];
  let count = 0;
  for (const operation of operations.values()) {
    const request = operation.requestBody?.content["application/json"];
    if (request?.example !== undefined) { count++; validate(request, request.example, `${operation.operationId} request`, errors); }
    for (const [status, response] of Object.entries(operation.responses)) {
      const media = response.content?.["application/json"];
      if (media?.example !== undefined) { count++; validate(media, media.example, `${operation.operationId} ${status}`, errors); }
    }
  }
  assert.deepEqual(errors, []);
  assert.ok(count > 0);
});
