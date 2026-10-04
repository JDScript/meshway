import { readFile, readdir } from "node:fs/promises";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import * as schemas from "../../packages/control-client/src/generated/zod.gen.ts";

const root = fileURLToPath(new URL("../..", import.meta.url));
async function walk(path) {
  const entries = await readdir(path, { withFileTypes: true });
  return (await Promise.all(entries.map(entry => entry.isDirectory()
    ? walk(join(path, entry.name)) : [join(path, entry.name)]))).flat();
}
let count = 0;
const errors = [];
for (const path of await walk(join(root, "docs/v2"))) {
  if (!path.endsWith(".md")) continue;
  const text = await readFile(path, "utf8");
  for (const match of text.matchAll(/<!-- schema: ([A-Za-z][A-Za-z0-9_]*) -->\s*```json\n([\s\S]*?)\n```/g)) {
    count++;
    const schema = schemas[`z${match[1]}`];
    const label = `${relative(root, path)} (${match[1]})`;
    if (!schema || typeof schema.safeParse !== "function") { errors.push(`${label}: unknown generated schema`); continue; }
    let value;
    try { value = JSON.parse(match[2]); } catch { errors.push(`${label}: invalid JSON`); continue; }
    const result = schema.safeParse(value);
    if (!result.success) errors.push(`${label}: ${result.error.issues.map(issue => `${issue.path.join(".")}: ${issue.code}`).join("; ")}`);
  }
}
if (!count) errors.push("No schema-bound documentation examples found.");
for (const error of errors) console.error(error);
console.log(`Checked ${count} schema-bound JSON examples.`);
if (errors.length) process.exitCode = 1;
