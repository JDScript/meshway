import { spawnSync } from "node:child_process";
import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const artifactPaths = [
  "packages/protocol/openapi.json",
  "apps/agent/src/generated",
  "packages/control-client/src/generated",
];

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
}

export async function generate(outputRoot = root) {
  const schema = join(outputRoot, artifactPaths[0]);
  const rust = join(outputRoot, artifactPaths[1]);
  const typescript = join(outputRoot, artifactPaths[2]);
  await mkdir(dirname(schema), { recursive: true });
  run("cargo", ["run", "--locked", "--quiet", "-p", "meshway-controller", "--", "export-openapi", "--output", schema]);
  run("cargo", ["run", "--locked", "--quiet", "-p", "meshway-controller", "--features", "codegen", "--bin", "generate-rust-client", "--", "--input", schema, "--output", rust]);
  const rustFiles = (await readdir(rust)).filter((file) => file.endsWith(".rs"));
  run("rustfmt", ["--edition", "2024", ...rustFiles.map((file) => join(rust, file))]);

  const require = createRequire(join(root, "packages/control-client/package.json"));
  const generatorPackage = dirname(require.resolve("@hey-api/openapi-ts/package.json"));
  const { createClient } = await import(pathToFileURL(join(generatorPackage, "dist/index.mjs")).href);
  await rm(typescript, { recursive: true, force: true });
  await createClient({
    configFile: false,
    input: schema,
    output: { path: typescript },
    plugins: ["@hey-api/typescript"],
  });
  const document = JSON.parse(await readFile(schema, "utf8"));
  await writeFile(join(typescript, "zod.gen.ts"), validators(document));
  await writeFile(join(typescript, "operations.gen.ts"), operations(document));
  await writeFile(join(typescript, "json-schema.gen.ts"), editorSchemas(document));
}

// Preserve the authoritative annotations and bounds for local JSONC completion.
// JSON Pointer references retain their OpenAPI component location; no DTO or
// field-name inference is maintained by the editor.
function editorSchemas(document) {
  const schemaDocument = {
    $schema: "https://json-schema.org/draft/2020-12/schema",
    components: { schemas: document.components.schemas },
  };
  return `// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.\n\nexport const schemaDocument = ${JSON.stringify(schemaDocument, null, 2)} as const;\n\nexport const componentSchemas = schemaDocument.components.schemas;\n\nexport function schemaFor(name: keyof typeof componentSchemas) {\n  return { ...schemaDocument, $ref: \`#/components/schemas/\${name}\` } as const;\n}\n`;
}

// Hey API emits DTOs. These deliberately narrow schema and operation
// emitter keeps the transport free of unchecked generic casts. It fails closed
// when the API grows beyond JSON GET/POST operations with component schemas.
// JSON Schema lengths count Unicode code points, unlike Zod's native min/max
// string checks (UTF-16 code units). Generate explicit scalar-length refinements.
function validators(document) {
  const emitted = new Set();
  const visiting = new Set();
  const definitions = [];
  const schemas = document.components.schemas;
  function keys(schema, allowed) {
    if (schema["x-meshway-ref"] !== undefined && !["node", "tag", "selector"].includes(schema["x-meshway-ref"])) {
      throw new Error("Unsupported Meshway schema reference annotation");
    }
    for (const key of Object.keys(schema)) {
      if (![...allowed, "description", "title", "x-meshway-ref"].includes(key)) throw new Error(`Unsupported schema keyword: ${key}`);
    }
  }
  function component(name) {
    if (!/^[A-Za-z][A-Za-z0-9]*$/.test(name) || !schemas[name]) throw new Error(`Invalid component: ${name}`);
    if (visiting.has(name)) throw new Error("Recursive schemas require an explicit generator extension");
    if (emitted.has(name)) return;
    visiting.add(name);
    const code = emit(schemas[name]);
    definitions.push(`export const z${name} = ${code};`);
    emitted.add(name);
    visiting.delete(name);
  }
  function emit(schema) {
    if (schema.$ref) {
      keys(schema, ["$ref"]);
      const match = /^#\/components\/schemas\/([A-Za-z][A-Za-z0-9]*)$/.exec(schema.$ref);
      if (!match) throw new Error("Only component references are supported");
      component(match[1]);
      return `z${match[1]}`;
    }
    if (Array.isArray(schema.type)) {
      if (schema.type.length !== 2 || schema.type[1] !== "null" || typeof schema.type[0] !== "string") throw new Error("Only T|null unions are supported");
      return `${emit({ ...schema, type: schema.type[0] })}.nullable()`;
    }
    switch (schema.type) {
      case "object": {
        keys(schema, ["type", "properties", "required", "additionalProperties"]);
        if (schema.additionalProperties !== false || !schema.properties || !Array.isArray(schema.required)) throw new Error("Only closed component objects are supported");
        if (schema.required.some((name) => !Object.hasOwn(schema.properties, name))) throw new Error("Required property is missing");
        return `z.strictObject({\n${Object.entries(schema.properties).map(([name, property]) => `  ${JSON.stringify(name)}: ${emit(property)}${schema.required.includes(name) ? "" : ".optional()"}`).join(",\n")}\n})`;
      }
      case "string": {
        keys(schema, ["type", "format", "minLength", "maxLength", "pattern", "enum"]);
        if (schema.enum !== undefined) {
          if (!Array.isArray(schema.enum) || !schema.enum.length || schema.enum.some(value => typeof value !== "string") || Object.keys(schema).some(key => !["type", "enum", "description", "title", "x-meshway-ref"].includes(key))) throw new Error("Only plain string enums are supported");
          return `z.enum(${JSON.stringify(schema.enum)})`;
        }
        if (schema.format && schema.format !== "date-time") throw new Error(`Unsupported string format ${schema.format}`);
        let code = schema.format ? "z.iso.datetime({ offset: true })" : "z.string()";
        for (const [key, operator] of [["minLength", ">="], ["maxLength", "<="]]) {
          if (schema[key] !== undefined) {
            if (!Number.isSafeInteger(schema[key]) || schema[key] < 0) throw new Error(`Invalid ${key}`);
            code += `.refine(value => [...value].length ${operator} ${schema[key]}, ${JSON.stringify(`${key} ${schema[key]} Unicode code points`)})`;
          }
        }
        if (schema.pattern !== undefined) {
          new RegExp(schema.pattern, "u");
          code += `.regex(new RegExp(${JSON.stringify(schema.pattern)}, "u"))`;
        }
        return code;
      }
      case "integer": {
        keys(schema, ["type", "format", "minimum", "maximum"]);
        if (schema.format && !["int32", "int64"].includes(schema.format)) throw new Error(`Unsupported integer format ${schema.format}`);
        let code = "z.int()";
        for (const [key, operation] of [["minimum", "gte"], ["maximum", "lte"]]) {
          if (schema[key] !== undefined) {
            if (!Number.isSafeInteger(schema[key])) throw new Error(`Unsafe integer ${key}`);
            code += `.${operation}(${schema[key]})`;
          }
        }
        return code;
      }
      case "boolean":
        keys(schema, ["type"]);
        return "z.boolean()";
      case "array": {
        keys(schema, ["type", "items", "minItems", "maxItems"]);
        let code = `z.array(${emit(schema.items)})`;
        for (const [key, operation] of [["minItems", "min"], ["maxItems", "max"]]) {
          if (schema[key] !== undefined) {
            if (!Number.isSafeInteger(schema[key]) || schema[key] < 0) throw new Error(`Invalid ${key}`);
            code += `.${operation}(${schema[key]})`;
          }
        }
        return code;
      }
      default: throw new Error(`Unsupported schema type ${schema.type}`);
    }
  }
  for (const name of Object.keys(schemas).sort()) component(name);
  return `// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.\n\nimport * as z from "zod";\n\n${definitions.join("\n\n")}\n`;
}

function operations(document) {
  if (document.openapi !== "3.1.0") throw new Error("Expected native OpenAPI 3.1.0");
  const types = new Set();
  const validators = new Set();
  const names = new Set();
  const functions = [];
  function component(content) {
    if (Object.keys(content).join() !== "application/json") throw new Error("Only JSON media is supported by the TypeScript operation emitter");
    const schema = content["application/json"].schema;
    const match = /^#\/components\/schemas\/([A-Za-z][A-Za-z0-9]*)$/.exec(schema.$ref);
    if (!match || Object.keys(schema).length !== 1) throw new Error("Operations must reference component schemas");
    const name = match[1];
    if (!document.components.schemas[name]) throw new Error(`Missing component ${name}`);
    types.add(name);
    validators.add(`z${name}`);
    return name;
  }
  for (const [path, item] of Object.entries(document.paths)) {
    if (!/^\/[a-zA-Z0-9/{}_\-]+$/.test(path)) throw new Error(`Unsupported operation path: ${path}`);
    for (const [method, operation] of Object.entries(item)) {
      if (!["get", "post"].includes(method) || operation.servers || document.servers?.length) {
        throw new Error(`Unsupported operation shape: ${method} ${path}; extend the emitter first`);
      }
      const parameters = operation.parameters ?? [];
      const placeholders = [...path.matchAll(/\{([a-z_]+)\}/g)].map(match => match[1]);
      if (parameters.length !== placeholders.length || parameters.some(parameter =>
        parameter.in !== "path" || parameter.required !== true || parameter.schema?.type !== "string" ||
        !placeholders.includes(parameter.name) || !/^[a-z_]+$/.test(parameter.name))) {
        throw new Error(`Only required string path parameters are supported: ${path}`);
      }
      const pathExpression = placeholders.length
        ? "`" + path.replace(/\{([a-z_]+)\}/g, (_, name) => `\${encodeURIComponent(${name})}`) + "`"
        : JSON.stringify(path);
      const security = operation.security ?? document.security ?? [];
      let authenticated = false;
      if (security.length) {
        if (security.length !== 1 || Object.keys(security[0]).length !== 1) throw new Error("Only single required bearer authentication is supported");
        const [name, scopes] = Object.entries(security[0])[0];
        const scheme = document.components.securitySchemes?.[name];
        if (!Array.isArray(scopes) || scopes.length || scheme?.type !== "http" || scheme.scheme !== "bearer") throw new Error(`Unsupported security scheme: ${name}`);
        authenticated = true;
      }
      const name = operation.operationId;
      if (!/^[a-z][A-Za-z0-9]+$/.test(name) || names.has(name)) throw new Error(`Invalid or duplicate operationId: ${name}`);
      names.add(name);
      const successes = Object.keys(operation.responses).filter((status) => /^2\d\d$/.test(status));
      if (successes.length !== 1 || successes[0] !== "200") throw new Error("Exactly one HTTP 200 response is currently supported");
      const response = component(operation.responses["200"].content);
      let body;
      if (operation.requestBody) {
        if (!operation.requestBody.required || method !== "post") throw new Error("Only required POST request bodies are currently supported");
        body = component(operation.requestBody.content);
      }
      functions.push(`export async function ${name}Operation(baseUrl: string${authenticated ? ", token: string" : ""}${parameters.map(parameter => `, ${parameter.name}: string`).join("")}${body ? `, body: ${body}` : ""}): Promise<${response}> {\n  return request(baseUrl, { method: ${JSON.stringify(method.toUpperCase())}, path: ${pathExpression}${authenticated ? ", token" : ""}${body ? `, body: z${body}.parse(body)` : ""} }, z${response});\n}`);
    }
  }
  return `// Generated from the controller OpenAPI by scripts/generate-protocol.mjs. Do not edit.\n\nimport { request } from "../transport.ts";\nimport type { ${[...types].sort().join(", ")} } from "./types.gen.ts";\nimport { ${[...validators].sort().join(", ")} } from "./zod.gen.ts";\n\n${functions.join("\n\n")}\n`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await generate();
}
