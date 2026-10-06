import { spawnSync } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { operationModel, typescriptOperations, rustOperations } from "./codegen/operations.mjs";
import { integerBounds, parameterSchemas } from "./codegen/parameters.mjs";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
export const artifactPaths = [
  "packages/protocol/openapi.json",
  "crates/meshway-control-client/src/generated",
  "packages/control-client/src/generated",
];

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
  return result.stdout;
}

export async function generate(outputRoot = root) {
  const schema = join(outputRoot, artifactPaths[0]);
  // Always export actual routes; the checked-in artifact is never an input.
  const document = JSON.parse(run("cargo", ["run", "-p", "meshway-server", "--bin", "export-openapi", "--locked", "--quiet"]));
  const outputs = generatedFiles(document);
  await mkdir(dirname(schema), { recursive: true });
  await writeFile(schema, JSON.stringify(document, null, 2) + "\n");
  await clearGenerated(outputRoot);
  for (const [path, content] of Object.entries(outputs)) {
    await mkdir(dirname(join(outputRoot, path)), { recursive: true });
    await writeFile(join(outputRoot, path), content);
  }
  run("rustfmt", ["--edition", "2024", join(outputRoot, artifactPaths[1], "mod.rs")]);
}

// The successful actual-route export owns these directories, including removal
// of outputs that are no longer emitted by the current contract.
async function clearGenerated(outputRoot) {
  for (const directory of artifactPaths.slice(1)) await rm(join(outputRoot, directory), { recursive: true, force: true });
}

export function generatedFiles(document) {
  const model = operationModel(document);
  const schemas = parameterSchemas(model, document.components.schemas);
  const schema = validators({ ...document, components: { ...document.components, schemas } });
  const names = Object.keys(schemas).sort();
  return {
    [`${artifactPaths[1]}/mod.rs`]: rustOperations(model),
    [`${artifactPaths[2]}/zod.gen.ts`]: schema,
    [`${artifactPaths[2]}/types.gen.ts`]: `// Generated from Controller OpenAPI. Do not edit.\nimport type * as z from "zod";\nimport type * as schemas from "./zod.gen.ts";\n${names.map(name => `export type ${name} = z.output<typeof schemas.z${name}>;`).join("\n")}\n`,
    [`${artifactPaths[2]}/operations.gen.ts`]: typescriptOperations(model),
    [`${artifactPaths[2]}/json-schema.gen.ts`]: editorSchemas(document),
    [`${artifactPaths[2]}/index.ts`]: `// Generated from Controller OpenAPI. Do not edit.\nexport * from "./types.gen.ts";\nexport * from "./operations.gen.ts";\n`,
  };
}

// Preserve the authoritative annotations and bounds for local JSONC completion.
// JSON Pointer references retain their OpenAPI component location; no DTO or
// field-name inference is maintained by the editor.
function editorSchemas(document) {
  const schemaDocument = {
    $schema: "https://json-schema.org/draft/2020-12/schema",
    components: { schemas: document.components.schemas },
  };
  return `// Generated from the Server OpenAPI by scripts/protocol/generate.mjs. Do not edit.\n\nexport const schemaDocument = ${JSON.stringify(schemaDocument, null, 2)} as const;\n\nexport const componentSchemas = schemaDocument.components.schemas;\n\nexport function schemaFor(name: keyof typeof componentSchemas) {\n  return { ...schemaDocument, $ref: \`#/components/schemas/\${name}\` } as const;\n}\n`;
}

// Types are inferred from the same validators used at runtime. The narrow
// emitter keeps the transport free of unchecked generic casts. It fails closed
// when the API grows beyond the supported JSON/SSE operations and component schemas.
// JSON Schema lengths count Unicode code points, unlike Zod's native min/max
// string checks (UTF-16 code units). Generate explicit scalar-length refinements.
export function validators(document) {
  const emitted = new Set();
  const visiting = new Set();
  const definitions = [];
  let usesTypedMap = false;
  let usesDateTime = false;
  let usesTimestampMillis = false;
  let usesSocketAddress = false;
  let usesNumber = false;
  const schemas = document.components?.schemas ?? document.$defs;
  const referencePrefix = document.components?.schemas ? "#/components/schemas/" : "#/$defs/";
  if (!schemas) throw new Error("Missing schema definitions");
  function keys(schema, allowed) {
    if (schema["x-meshway-ref"] !== undefined && !["node", "tag", "selector"].includes(schema["x-meshway-ref"])) {
      throw new Error("Unsupported Meshway schema reference annotation");
    }
    for (const key of Object.keys(schema)) {
      if (![...allowed, "description", "title", "x-meshway-ref"].includes(key)) throw new Error(`Unsupported schema keyword: ${key}`);
    }
  }
  function component(name) {
    if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(name) || !schemas[name]) throw new Error(`Invalid component: ${name}`);
    if (visiting.has(name)) throw new Error("Recursive schemas require an explicit generator extension");
    if (emitted.has(name)) return;
    visiting.add(name);
    let code;
    try {
      code = emit(schemas[name]);
    } catch (error) {
      throw new Error(`Component ${name}: ${error.message}`, { cause: error });
    }
    definitions.push(`export const z${name} = ${code};`);
    emitted.add(name);
    visiting.delete(name);
  }
  function emit(schema) {
    if (!schema || typeof schema !== "object" || Array.isArray(schema)) throw new Error("Expected a typed schema object");
    if (schema.allOf) {
      keys(schema, ["allOf"]);
      if (!Array.isArray(schema.allOf) || schema.allOf.length !== 1) throw new Error("Only single-schema allOf wrappers are supported");
      return emit(schema.allOf[0]);
    }
    if (schema.anyOf || schema.oneOf) {
      keys(schema, ["anyOf", "oneOf"]);
      if (schema.anyOf && schema.oneOf) throw new Error("Mixed union keywords are unsupported");
      const variants = schema.anyOf ?? schema.oneOf;
      if (!Array.isArray(variants) || variants.length < 2) throw new Error("Union requires at least two variants");
      const empty = variants.find(item => item.type === "null" && Object.keys(item).length === 1);
      if (empty) {
        const values = variants.filter(item => item !== empty);
        if (values.some(item => item.type === "null")) throw new Error("Duplicate null variant");
        if (schema.oneOf && !values.every(item => excludesNull(item))) throw new Error("Nullable oneOf requires non-null remaining branches");
        return `${values.length === 1 ? emit(values[0]) : emit({ [schema.oneOf ? "oneOf" : "anyOf"]: values })}.nullable()`;
      }
      // Prove every pair disjoint before using z.union. This also covers external
      // enums (literal strings vs object payloads) and multi-field literal tags.
      const objects = variants.map(item => {
        if (!item.$ref) return item;
        keys(item, ["$ref"]);
        return schemas[referenceName(item.$ref)];
      });
      const literals = property => property?.type === "string"
        ? (typeof property.const === "string" ? [property.const] : property.enum)
        : undefined;
      const distinct = (a, b) => Array.isArray(a) && Array.isArray(b) && a.length > 0 && b.length > 0 && a.every(value => !b.includes(value));
      const disjoint = (a, b) => {
        if (typeof a?.type !== "string" || typeof b?.type !== "string") return false;
        if (a.type !== b.type) return ![a.type, b.type].every(type => ["integer", "number"].includes(type));
        if (a.type === "string") return distinct(literals(a), literals(b));
        if (a.type !== "object") return false;
        return (a.required ?? []).some(name => b.required?.includes(name) && distinct(literals(a.properties?.[name]), literals(b.properties?.[name])));
      };
      if (objects.some((a, i) => objects.slice(i + 1).some(b => !disjoint(a, b)))) throw new Error("Union branches must be provably disjoint");
      return `z.union([${variants.map(emit).join(", ")}])`;
    }
    if (schema.$ref) {
      keys(schema, ["$ref"]);
      const name = referenceName(schema.$ref);
      component(name);
      return `z${name}`;
    }
    if (Array.isArray(schema.type)) {
      if (schema.type.length !== 2 || schema.type[1] !== "null" || typeof schema.type[0] !== "string") throw new Error("Only T|null unions are supported");
      if (Object.hasOwn(schema, "const")) throw new Error("Nullable type with const is unsupported; use a plain string constant");
      return `${emit({ ...schema, type: schema.type[0] })}.nullable()`;
    }
    switch (schema.type) {
      case "object": {
        keys(schema, ["type", "properties", "required", "additionalProperties", "maxProperties", "minProperties", "propertyNames"]);
        const properties = schema.properties ?? {};
        const required = schema.required ?? [];
        if (!Array.isArray(required) || new Set(required).size !== required.length || required.some(name => !Object.hasOwn(properties, name))) throw new Error("Invalid required properties");
        const bounds = {};
        for (const key of ["minProperties", "maxProperties"]) {
          if (schema[key] !== undefined) {
            if (!Number.isSafeInteger(schema[key]) || schema[key] < 0) throw new Error(`Invalid ${key}`);
            bounds[key] = schema[key];
          }
        }
        const isMap = schema.additionalProperties && typeof schema.additionalProperties === "object" && !Object.keys(properties).length && !required.length;
        let code;
        if (isMap) {
          const hasNames = Object.hasOwn(schema, "propertyNames");
          if (hasNames && schema.propertyNames?.type !== "string") throw new Error("Typed map propertyNames must be a string schema");
          usesTypedMap = true;
          code = `typedMap(${hasNames ? emit(schema.propertyNames) : "z.string()"}, ${emit(schema.additionalProperties)}, ${JSON.stringify(bounds)})`;
        } else {
          if (Object.hasOwn(schema, "propertyNames")) throw new Error("propertyNames is only supported on typed maps");
          // utoipa tagged variants omit additionalProperties=false. Their
          // required full property set + matching maxProperties is equally closed.
          const boundedClosed = schema.additionalProperties === undefined && schema.maxProperties === Object.keys(properties).length && required.length === Object.keys(properties).length;
          if (schema.additionalProperties !== false && !boundedClosed) throw new Error("Only closed objects or typed maps are supported");
          code = `z.strictObject({\n${Object.entries(properties).map(([name, property]) => {
            const value = emit(property);
            // JSON omission is not an explicit undefined property.
             return `  [${JSON.stringify(name)}]: ${value}${required.includes(name) ? "" : ".exactOptional()"}`;
          }).join(",\n")}\n})`;
        }
        for (const [key, operator] of [["minProperties", ">="], ["maxProperties", "<="]]) {
          if (!isMap && schema[key] !== undefined) {
            code += `.refine(value => Object.keys(value).length ${operator} ${schema[key]}, ${JSON.stringify(`${key} ${schema[key]}`)})`;
          }
        }
        return code;
      }
      case "string": {
        keys(schema, ["type", "format", "minLength", "maxLength", "pattern", "enum", "const"]);
        if (schema.const !== undefined) {
          if (typeof schema.const !== "string" || Object.keys(schema).some(key => !["type", "const", "description", "title", "x-meshway-ref"].includes(key))) throw new Error("Only plain string constants are supported");
          return `z.literal(${JSON.stringify(schema.const)})`;
        }
        if (schema.enum !== undefined) {
          if (!Array.isArray(schema.enum) || !schema.enum.length || schema.enum.some(value => typeof value !== "string") || Object.keys(schema).some(key => !["type", "enum", "description", "title", "x-meshway-ref"].includes(key))) throw new Error("Only plain string enums are supported");
          return `z.enum(${JSON.stringify(schema.enum)})`;
        }
        if (schema.format && !["date-time", "meshway-timestamp-ms", "meshway-socket-address", "password", "uuid", "ipv4", "ipv6"].includes(schema.format)) throw new Error(`Unsupported string format ${schema.format}`);
        if (schema.format === "uuid" && !schema.pattern) throw new Error("UUID format requires the authoritative canonical UUID pattern");
        if (schema.format === "date-time") usesDateTime = true;
        if (schema.format === "meshway-timestamp-ms") usesTimestampMillis = usesDateTime = true;
        if (schema.format === "meshway-socket-address") usesSocketAddress = true;
        let code = "z.string()";
        if (schema.format === "meshway-socket-address") code += '.refine(socketAddress, "Expected numeric IP:port without an IPv6 scope")';
        else if (schema.format === "meshway-timestamp-ms") code += '.refine(timestampMillis, "Expected nonnegative RFC 3339 exact milliseconds")';
        else if (schema.format === "date-time") code += '.refine(rfc3339, "Expected RFC 3339 date-time")';
        else if (["ipv4", "ipv6"].includes(schema.format)) code = `z.${schema.format}()`;
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
        // z.int() itself imposes the JS-safe range. Prove the source schema is
        // within it first rather than silently narrowing unbounded/int64 data.
        const { minimum, maximum } = integerBounds(schema);
        return `z.int().gte(${minimum}).lte(${maximum})`;
      }
      case "number": {
        usesNumber = true;
        keys(schema, ["type", "format", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"]);
        if (schema.format !== undefined && schema.format !== "double") throw new Error("Only binary64 number schemas are supported");
        for (const key of ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"]) {
          if (schema[key] !== undefined && !Number.isFinite(schema[key])) throw new Error(`Invalid number ${key}`);
        }
        const lower = Math.max(schema.minimum ?? -Infinity, schema.exclusiveMinimum ?? -Infinity);
        const upper = Math.min(schema.maximum ?? Infinity, schema.exclusiveMaximum ?? Infinity);
        if (lower > upper || (lower === upper && (schema.exclusiveMinimum === lower || schema.exclusiveMaximum === upper))) throw new Error("Empty number range");
        // Zod numbers reject NaN/Infinity. JSON and Rust f64 use the same finite
        // binary64 representation; integer schemas retain their separate safe bounds.
        let code = "z.number()";
        for (const [key, operation] of [["minimum", "gte"], ["maximum", "lte"], ["exclusiveMinimum", "gt"], ["exclusiveMaximum", "lt"]]) {
          if (schema[key] !== undefined) code += `.${operation}(${schema[key]})`;
        }
        return `z.preprocess(unwrapJsonNumber, ${code})`;
      }
      case "boolean":
        keys(schema, ["type"]);
        return "z.boolean()";
      case "null":
        keys(schema, ["type"]);
        return "z.null()";
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
  function referenceName(reference) {
    if (typeof reference !== "string" || !reference.startsWith(referencePrefix)) throw new Error("Only local definition references are supported");
    const name = reference.slice(referencePrefix.length);
    if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(name) || !Object.hasOwn(schemas, name)) throw new Error(`Invalid component: ${name}`);
    return name;
  }
  function excludesNull(schema, seen = new Set()) {
    if (schema.$ref) {
      const name = referenceName(schema.$ref);
      if (seen.has(name)) return false;
      return excludesNull(schemas[name], new Set([...seen, name]));
    }
    if (schema.anyOf || schema.oneOf) return (schema.anyOf ?? schema.oneOf).every(item => excludesNull(item, seen));
    return typeof schema.type === "string" && schema.type !== "null";
  }
  for (const name of Object.keys(schemas).sort()) component(name);
  return `// Generated from the Server OpenAPI by scripts/protocol/generate.mjs. Do not edit.\n\nimport * as z from "zod";\n${usesNumber ? 'import { unwrapJsonNumber } from "../json.ts";\n' : ""}\n${usesTypedMap ? typedMapHelper + "\n\n" : ""}${usesDateTime ? dateTimeHelper + "\n\n" : ""}${usesTimestampMillis ? timestampMillisHelper + "\n\n" : ""}${usesSocketAddress ? socketAddressHelper + "\n\n" : ""}${definitions.join("\n\n")}\n`;
}

// RFC 3339 permits lowercase t/z and leap seconds. Zod's ISO validator is
// narrower, and Date.parse normalizes invalid calendar dates in some engines.
const dateTimeHelper = `function rfc3339(value: string): boolean {
  const parts = /^(\\d{4})-(\\d{2})-(\\d{2})[Tt](\\d{2}):(\\d{2}):(\\d{2})(?:\\.\\d+)?(?:[Zz]|[+-](\\d{2}):(\\d{2}))$/.exec(value);
  if (!parts || parts[0].length !== value.length) return false;
  const year = Number(parts[1]), month = Number(parts[2]), day = Number(parts[3]);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
  return days !== undefined && day >= 1 && day <= days && Number(parts[4]) <= 23 &&
    Number(parts[5]) <= 59 && Number(parts[6]) <= 60 &&
    (parts[7] === undefined || (Number(parts[7]) <= 23 && Number(parts[8]) <= 59));
}`;

// The execution/signature scalar is narrower than ordinary observation times.
// Inspect every fractional digit before conversion; Date.parse would truncate.
const timestampMillisHelper = `function timestampMillis(value: string): boolean {
  if (!rfc3339(value)) return false;
  const parts = /^(\\d{4})-(\\d{2})-(\\d{2})[Tt](\\d{2}):(\\d{2}):(\\d{2})(?:\\.(\\d+))?(?:[Zz]|([+-])(\\d{2}):(\\d{2}))$/.exec(value);
  if (!parts) return false;
  const fraction = parts[7] ?? "";
  if (/[^0]/.test(fraction.slice(3))) return false;
  const date = new Date(0);
  date.setUTCFullYear(Number(parts[1]), Number(parts[2]) - 1, Number(parts[3]));
  const second = Number(parts[6]);
  if (second === 60) return false;
  date.setUTCHours(Number(parts[4]), Number(parts[5]), Math.min(second, 59), Number(fraction.slice(0, 3).padEnd(3, "0")));
  const offset = parts[8] === undefined ? 0 : (Number(parts[9]) * 60 + Number(parts[10])) * (parts[8] === "+" ? 1 : -1);
  const millis = date.getTime() - offset * 60000;
  return millis >= 0 && millis <= 253402300799999;
}`;

const socketAddressHelper = `const socketIpv4 = z.ipv4();
const socketIpv6 = z.ipv6();
function socketAddress(value: string): boolean {
  const parts = /^(?:\\[([^\\]]+)\\]|([^:]+)):([0-9]+)$/.exec(value);
  if (!parts || parts[0].length !== value.length) return false;
  const port = Number(parts[3]);
  if (port < 1 || port > 65535) return false;
  const host = parts[1] ?? parts[2];
  if (host === undefined || /[^0-9a-fA-F:.%]/.test(host)) return false;
  if (parts[1] === undefined) return socketIpv4.safeParse(parts[2]).success;
  const separator = parts[1].lastIndexOf("%");
  if (separator < 0) return socketIpv6.safeParse(parts[1]).success;
  const scope = parts[1].slice(separator + 1);
  return scope.length > 0 && !/[^0]/.test(scope) && socketIpv6.safeParse(parts[1].slice(0, separator)).success;
}`;

// Zod 4 record skips __proto__ before validation and makes enum keys exhaustive.
// JSON maps require validation of every present own key, not either behavior.
const typedMapHelper = `function typedMap<Value extends z.ZodType>(
  keySchema: z.ZodType<string>,
  valueSchema: Value,
  bounds: { minProperties?: number; maxProperties?: number },
) {
  return z.custom<Record<string, unknown>>((input: unknown) =>
    input !== null && typeof input === "object" && !Array.isArray(input) &&
    (Object.getPrototypeOf(input) === Object.prototype || Object.getPrototypeOf(input) === null),
    "Expected a JSON object",
  ).transform((input, ctx) => {
    const entries = Object.entries(input);
    let invalid = false;
    for (const [limit, violated] of [
      ["minProperties", bounds.minProperties !== undefined && entries.length < bounds.minProperties],
      ["maxProperties", bounds.maxProperties !== undefined && entries.length > bounds.maxProperties],
    ] as const) {
      if (violated) {
        ctx.addIssue({ code: "custom", message: limit + " exceeded" });
        invalid = true;
      }
    }
    const output: Record<string, z.output<Value>> = {};
    for (const [name, value] of entries) {
      const key = keySchema.safeParse(name);
      const parsed = valueSchema.safeParse(value);
      for (const result of [key, parsed]) {
        if (!result.success) {
          invalid = true;
          for (const issue of result.error.issues) ctx.addIssue({ ...issue, path: [name, ...issue.path] });
        }
      }
      if (key.success && parsed.success) {
        // Defining an own data property preserves __proto__ without invoking
        // Object.prototype's setter or changing the output prototype.
        Object.defineProperty(output, name, { value: parsed.data, enumerable: true, writable: true, configurable: true });
      }
    }
    return invalid ? z.NEVER : output;
  });
}`;

export function operations(document) { return typescriptOperations(operationModel(document)); }

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await generate();
}
