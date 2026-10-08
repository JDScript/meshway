// Renders the generated OpenAPI contract as Markdown reference pages under
// docs/v2/reference. The contract is the only source: nothing here is written
// by hand, and the pages are regenerated on every docs build rather than
// committed. Operations are split by API root (Console /api/v1, Node
// /agent-api/v1); schemas are split into alphabetical pages so links stay
// stable while the contract grows.
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
const source = join(root, "packages/protocol/openapi.json");
const output = join(root, "docs/v2/reference");
const spec = JSON.parse(await readFile(source, "utf8"));
const schemas = spec.components?.schemas ?? {};

const buckets = [
  ["a-c", /^[A-Ca-c]/],
  ["d-l", /^[D-Ld-l]/],
  ["m-p", /^[M-Pm-p]/],
  ["q-z", /^[Q-Zq-z]/],
];
function bucketOf(name) {
  const bucket = buckets.find(([, pattern]) => pattern.test(name));
  if (!bucket) throw new Error(`No schema page for ${name}`);
  return bucket[0];
}
function anchor(text) {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}
function schemaLink(name) {
  if (!Object.hasOwn(schemas, name)) throw new Error(`Unknown schema ${name}`);
  return `[${name}](./schemas-${bucketOf(name)}#${anchor(name)})`;
}
function refName(ref) {
  const prefix = "#/components/schemas/";
  if (typeof ref !== "string" || !ref.startsWith(prefix)) throw new Error(`Unsupported $ref ${ref}`);
  return ref.slice(prefix.length);
}
function escapeCell(text) {
  return String(text).replace(/\|/g, "\\|").replace(/\r?\n\s*\r?\n/g, " ").replace(/\r?\n\s*/g, " ").trim();
}
function code(text) {
  return `\`${String(text).replace(/`/g, "\\`")}\``;
}

// A compact type expression with links to named schemas.
function typeOf(schema) {
  if (!schema || typeof schema !== "object") return "unknown";
  if (schema.$ref) return schemaLink(refName(schema.$ref));
  const variants = schema.oneOf ?? schema.anyOf;
  if (variants) {
    const parts = variants.map(typeOf);
    const nullIndex = parts.indexOf("null");
    if (nullIndex >= 0 && parts.length === 2) return `${parts[1 - nullIndex]} \\| null`;
    return parts.join(" \\| ");
  }
  if (schema.allOf) return schema.allOf.map(typeOf).join(" & ");
  const types = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : [];
  if (types.length === 1 && types[0] === "null") return "null";
  const nullable = types.includes("null");
  const base = types.filter(t => t !== "null")[0];
  let text;
  if (schema.enum) text = schema.enum.map(v => code(JSON.stringify(v))).join(" \\| ");
  else if (base === "array") text = `${typeOf(schema.items)}[]`;
  else if (base === "object") {
    if (schema.additionalProperties && typeof schema.additionalProperties === "object") text = `Record<string, ${typeOf(schema.additionalProperties)}>`;
    else if (schema.properties) text = "object";
    else text = "object";
  } else if (base === "string") text = schema.format ? `string (${schema.format})` : "string";
  else if (base === "integer") text = schema.format ? `integer (${schema.format})` : "integer";
  else if (base === "number") text = schema.format ? `number (${schema.format})` : "number";
  else if (base) text = base;
  else text = "any";
  return nullable ? `${text} \\| null` : text;
}

function constraints(schema) {
  const out = [];
  for (const key of ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "minLength", "maxLength", "minItems", "maxItems", "pattern"]) {
    if (schema[key] !== undefined) out.push(`${key} ${code(schema[key])}`);
  }
  return out.join(", ");
}

// utoipa places a nullable reference's description on the referenced branch
// of its `oneOf`; read it from there when the property itself has none.
function describe(schema) {
  if (schema.description) return schema.description;
  for (const variant of schema.oneOf ?? schema.anyOf ?? []) {
    if (variant.description) return variant.description;
  }
  return undefined;
}

function propertyRows(object) {
  const required = new Set(object.required ?? []);
  const rows = [];
  for (const [name, schema] of Object.entries(object.properties ?? {}).sort(([a], [b]) => a.localeCompare(b))) {
    const notes = [];
    const description = describe(schema);
    if (description) notes.push(escapeCell(description));
    const limits = constraints(schema);
    if (limits) notes.push(limits);
    if (schema.default !== undefined) notes.push(`default ${code(JSON.stringify(schema.default))}`);
    rows.push(`| ${code(name)} | ${typeOf(schema)} | ${required.has(name) ? "yes" : "no"} | ${notes.join("; ")} |`);
  }
  return rows;
}

function objectTable(object) {
  const rows = propertyRows(object);
  if (!rows.length) return object.additionalProperties === false ? "An object without properties.\n" : "An object with arbitrary properties.\n";
  const extra = object.additionalProperties === false ? "" : "\nAdditional properties are allowed.\n";
  return ["| Field | Type | Required | Notes |", "| --- | --- | --- | --- |", ...rows].join("\n") + "\n" + extra;
}

function taggedVariants(schema) {
  const variants = schema.oneOf;
  if (!Array.isArray(variants)) return null;
  const tag = ["type", "secretDelivery", "status", "kind", "phase"].find(candidate =>
    variants.every(v => v.properties?.[candidate]?.enum?.length === 1));
  return tag ? { tag, variants } : null;
}

function renderSchema(name, schema, usedBy) {
  const lines = [`### ${name}`, ""];
  if (schema.description) lines.push(schema.description.trim(), "");
  const tagged = taggedVariants(schema);
  if (tagged) {
    lines.push(`A closed union discriminated by ${code(tagged.tag)}.`, "");
    for (const variant of tagged.variants) {
      const value = variant.properties[tagged.tag].enum[0];
      lines.push(`#### ${name} · ${code(JSON.stringify(value))}`, "");
      if (variant.description) lines.push(variant.description.trim(), "");
      const rest = { ...variant, properties: { ...variant.properties } };
      delete rest.properties[tagged.tag];
      rest.required = (rest.required ?? []).filter(field => field !== tagged.tag);
      lines.push(objectTable(rest));
    }
  } else if (schema.oneOf || schema.anyOf) {
    lines.push(`One of: ${typeOf(schema)}`, "");
    for (const variant of schema.oneOf ?? schema.anyOf) {
      if (variant.properties) lines.push(objectTable(variant));
    }
  } else if (schema.enum) {
    lines.push(`A string with one of the values ${schema.enum.map(v => code(JSON.stringify(v))).join(", ")}.`, "");
  } else if (schema.properties || schema.type === "object") {
    lines.push(objectTable(schema));
  } else {
    const limits = constraints(schema);
    lines.push(`Type: ${typeOf(schema)}${limits ? ` (${limits})` : ""}`, "");
  }
  const users = usedBy.get(name);
  if (users?.size) lines.push(`Used by: ${[...users].sort().join(", ")}`, "");
  return lines.join("\n");
}

// Every operation that carries a schema in a parameter, body or response,
// recorded on the schema so a type page points back to its operations.
function collectUsage() {
  const usedBy = new Map();
  const note = (schema, label) => {
    if (!schema || typeof schema !== "object") return;
    if (schema.$ref) {
      const name = refName(schema.$ref);
      if (!usedBy.has(name)) usedBy.set(name, new Set());
      usedBy.get(name).add(label);
      return;
    }
    for (const key of ["oneOf", "anyOf", "allOf"]) for (const item of schema[key] ?? []) note(item, label);
    if (schema.items) note(schema.items, label);
  };
  for (const [path, item] of Object.entries(spec.paths)) {
    for (const [method, op] of Object.entries(item)) {
      const page = path.startsWith("/agent-api/") ? "agent" : "console";
      const label = `[${op.operationId}](./${page}#${anchor(op.operationId)})`;
      for (const parameter of op.parameters ?? []) note(parameter.schema, label);
      for (const content of Object.values(op.requestBody?.content ?? {})) note(content.schema, label);
      for (const response of Object.values(op.responses ?? {})) for (const content of Object.values(response.content ?? {})) note(content.schema, label);
      note(op["x-meshway-sse"]?.eventSchema, label);
      void method;
    }
  }
  return usedBy;
}

function renderOperation(path, method, op) {
  const lines = [`### ${op.operationId}`, "", `${code(method.toUpperCase())} ${code(path)}`, ""];
  if (op.summary) lines.push(op.summary, "");
  if (op.description) lines.push(op.description.trim(), "");
  const security = (op.security ?? spec.security ?? []).flatMap(entry => Object.keys(entry));
  lines.push(`Authentication: ${security.length ? security.map(code).join(", ") : "none"}`, "");
  const parameters = op.parameters ?? [];
  if (parameters.length) {
    lines.push("| Parameter | In | Type | Required | Notes |", "| --- | --- | --- | --- | --- |");
    for (const parameter of parameters) {
      lines.push(`| ${code(parameter.name)} | ${parameter.in} | ${typeOf(parameter.schema)} | ${parameter.required ? "yes" : "no"} | ${parameter.description ? escapeCell(parameter.description) : ""} |`);
    }
    lines.push("");
  }
  const body = op.requestBody;
  if (body) {
    for (const [mime, content] of Object.entries(body.content ?? {})) {
      lines.push(`Request body (${code(mime)}${body.required ? "" : ", optional"}): ${typeOf(content.schema)}`, "");
    }
  }
  const sse = op["x-meshway-sse"];
  if (sse) lines.push(`Server-sent events; each ${code("data")} line is one ${typeOf(sse.eventSchema)} (${code(sse.format)}).`, "");
  lines.push("| Status | Response | Problem codes |", "| --- | --- | --- |");
  for (const [status, response] of Object.entries(op.responses ?? {}).sort(([a], [b]) => Number(a) - Number(b))) {
    const contents = Object.entries(response.content ?? {});
    const isError = contents.some(([, content]) => content.schema?.$ref === "#/components/schemas/ErrorResponse");
    const text = contents.length
      ? contents.map(([mime, content]) => mime === "application/json" ? typeOf(content.schema) : `${code(mime)}`).join(", ")
      : response.description ? escapeCell(response.description) : "no body";
    // An error response's description is its list of problem codes.
    const codes = isError && response.description
      ? response.description.split(",").map(c => code(c.trim())).join(", ")
      : "";
    lines.push(`| ${status} | ${isError && response.description ? typeOf(contents[0][1].schema) : text} | ${codes} |`);
  }
  lines.push("");
  return lines.join("\n");
}

function renderOperationsPage(title, intro, prefix) {
  const tagDescriptions = new Map((spec.tags ?? []).map(tag => [tag.name, tag.description ?? ""]));
  const byTag = new Map();
  for (const [path, item] of Object.entries(spec.paths)) {
    if (!path.startsWith(prefix)) continue;
    for (const [method, op] of Object.entries(item)) {
      const tag = op.tags?.[0] ?? "other";
      if (!byTag.has(tag)) byTag.set(tag, []);
      byTag.get(tag).push({ path, method, op });
    }
  }
  const order = (spec.tags ?? []).map(tag => tag.name);
  const tags = [...byTag.keys()].sort((a, b) => (order.indexOf(a) - order.indexOf(b)) || a.localeCompare(b));
  const lines = [header(title), intro, "", "| Operation | Method | Path |", "| --- | --- | --- |"];
  for (const tag of tags) {
    for (const { path, method, op } of byTag.get(tag)) {
      lines.push(`| [${op.operationId}](#${anchor(op.operationId)}) | ${code(method.toUpperCase())} | ${code(path)} |`);
    }
  }
  lines.push("");
  for (const tag of tags) {
    lines.push(`## ${tag}`, "");
    if (tagDescriptions.get(tag)) lines.push(tagDescriptions.get(tag), "");
    for (const { path, method, op } of byTag.get(tag)) lines.push(renderOperation(path, method, op));
  }
  return lines.join("\n");
}

function header(title) {
  return `---\ntitle: ${title}\noutline: [2, 3]\n---\n\n# ${title}\n\n<!-- Generated from packages/protocol/openapi.json by docs/scripts/generate-reference.mjs. Do not edit. -->\n`;
}

const usedBy = collectUsage();
await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });

const operationCount = Object.values(spec.paths).reduce((n, item) => n + Object.keys(item).length, 0);
const schemaNames = Object.keys(schemas).sort((a, b) => a.localeCompare(b));
const index = [
  header("API 参考"),
  `本节由 \`packages/protocol/openapi.json\`（Server ${code(spec.info.version)}，由实际路由生成）在每次文档构建时渲染，不是手写文档，也不单独维护。字段语义、行为与限制见各主题页面；本节回答"这个操作／类型到底有哪些字段"。原始契约：[openapi.json](/v2/contracts/openapi.json)。`,
  "",
  `| 页面 | 内容 |`,
  `| --- | --- |`,
  `| [Console API](./console) | \`/api/v1\` 下 ${Object.entries(spec.paths).filter(([p]) => p.startsWith("/api/")).reduce((n, [, item]) => n + Object.keys(item).length, 0)} 个操作，用户会话 |`,
  `| [Node API](./agent) | \`/agent-api/v1\` 下 ${Object.entries(spec.paths).filter(([p]) => p.startsWith("/agent-api/")).reduce((n, [, item]) => n + Object.keys(item).length, 0)} 个操作，Node 会话 |`,
  ...buckets.map(([bucket]) => {
    const names = schemaNames.filter(name => bucketOf(name) === bucket);
    return `| [Schemas ${bucket.toUpperCase()}](./schemas-${bucket}) | ${names.length} 个类型：${names[0]} … ${names[names.length - 1]} |`;
  }),
  "",
  `共 ${operationCount} 个操作、${schemaNames.length} 个类型。表格中 Required 指字段必须出现；可为 null 的字段标为 ${code("T | null")}，省略与 null 含义不同，见 [HTTP 与生成客户端](/v2/api)。`,
  "",
].join("\n");
await writeFile(join(output, "index.md"), index);
await writeFile(join(output, "console.md"), renderOperationsPage(
  "Console API",
  "`/api/v1` 下的操作，使用用户会话（`Authorization: Bearer`）。错误响应统一为 [ErrorResponse](./schemas-d-l#errorresponse)，code 见 [Actions 与 Problem](/v2/api/problems)。",
  "/api/",
));
await writeFile(join(output, "agent.md"), renderOperationsPage(
  "Node API",
  "`/agent-api/v1` 下的操作，使用 Node 会话；接入前的操作以设备签名证明身份。错误响应统一为 [ErrorResponse](./schemas-d-l#errorresponse)。",
  "/agent-api/",
));
for (const [bucket] of buckets) {
  const names = schemaNames.filter(name => bucketOf(name) === bucket);
  const lines = [header(`Schemas ${bucket.toUpperCase()}`), `${names.length} 个类型。`, ""];
  for (const name of names) lines.push(renderSchema(name, schemas[name], usedBy));
  await writeFile(join(output, `schemas-${bucket}.md`), lines.join("\n"));
}
console.log(`Rendered ${operationCount} operations and ${schemaNames.length} schemas into docs/v2/reference.`);
