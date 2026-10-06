// Narrow parameter subset shared by the operation emitters. Unsupported schema
// semantics are rejected, never erased to a string or an arbitrary JSON value.
export function integerBounds(schema) {
  if (schema.format !== undefined && !["int32", "int64"].includes(schema.format)) throw new Error(`Unsupported integer format ${schema.format}`);
  for (const key of ["minimum", "maximum"]) {
    if (schema[key] !== undefined && !Number.isSafeInteger(schema[key])) throw new Error(`Unsafe integer ${key}`);
    if (schema.format === "int32" && schema[key] !== undefined && (schema[key] < -2147483648 || schema[key] > 2147483647)) throw new Error(`Integer ${key} contradicts int32 range`);
  }
  const minimum = schema.minimum ?? (schema.format === "int32" ? -2147483648 : undefined);
  const maximum = schema.maximum ?? (schema.format === "int32" ? 2147483647 : undefined);
  if (!Number.isSafeInteger(minimum) || !Number.isSafeInteger(maximum)) throw new Error("Integer requires safe minimum and maximum or int32 format");
  if (minimum > maximum) throw new Error("Empty integer range");
  return { minimum, maximum };
}

const uuidPattern = "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$";
const annotations = ["description", "title", "x-meshway-ref"];
function keys(value, allowed) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Expected parameter object");
  for (const key of Object.keys(value)) if (!allowed.includes(key)) throw new Error(`Unsupported parameter keyword: ${key}`);
}

function scalar(schema, schemas, seen = new Set()) {
  if (schema?.$ref) {
    keys(schema, ["$ref", ...annotations]);
    const match = /^#\/components\/schemas\/([A-Za-z][A-Za-z0-9]*)$/.exec(schema.$ref);
    const name = match?.[1];
    if (!name || !Object.hasOwn(schemas, name) || seen.has(name)) throw new Error("Invalid parameter component reference");
    return { ...scalar(schemas[name], schemas, new Set([...seen, name])), component: name };
  }
  if (schema?.allOf) {
    keys(schema, ["allOf", ...annotations]);
    if (!Array.isArray(schema.allOf) || schema.allOf.length !== 1) throw new Error("Unsupported parameter allOf");
    return scalar(schema.allOf[0], schemas, seen);
  }
  if (schema?.type === "string") {
    keys(schema, ["type", "format", "minLength", "maxLength", "pattern", "enum", "const", ...annotations]);
    if (schema.format !== undefined && schema.format !== "uuid") throw new Error("Unsupported parameter string format");
    if (schema.pattern !== undefined && schema.pattern !== uuidPattern) throw new Error("Unsupported parameter pattern; Rust needs an equivalent validator");
    if (schema.format === "uuid" && schema.pattern !== uuidPattern) throw new Error("UUID parameter requires canonical pattern");
    if (schema.pattern === uuidPattern && (schema.minLength !== 36 || schema.maxLength !== 36)) throw new Error("Canonical UUID parameter requires exact length bounds");
    for (const key of ["minLength", "maxLength"]) if (schema[key] !== undefined && (!Number.isSafeInteger(schema[key]) || schema[key] < 0)) throw new Error(`Invalid parameter ${key}`);
    if ((schema.minLength ?? 0) > (schema.maxLength ?? Infinity)) throw new Error("Empty parameter string range");
    if (schema.const !== undefined && (typeof schema.const !== "string" || !schema.const.isWellFormed())) throw new Error("Invalid parameter string constant");
    if (schema.enum !== undefined && (!Array.isArray(schema.enum) || !schema.enum.length || schema.enum.some(value => typeof value !== "string" || !value.isWellFormed()))) throw new Error("Invalid parameter string enum");
    return { schema, kind: "string", rustScalar: "String", uuid: schema.pattern === uuidPattern };
  }
  if (schema?.type === "integer") {
    keys(schema, ["type", "format", "minimum", "maximum", ...annotations]);
    return { schema, kind: "integer", rustScalar: schema.format === "int32" ? "i32" : "i64", ...integerBounds(schema) };
  }
  if (schema?.type === "boolean") {
    keys(schema, ["type", ...annotations]);
    return { schema, kind: "boolean", rustScalar: "bool" };
  }
  throw new Error("Only non-null scalar path/query parameters are supported");
}

export function parameters(path, item, op, schemas) {
  const parts = path.split("/").slice(1);
  if (!path.startsWith("/") || parts.some(part => !/^(?:[A-Za-z0-9_-]+|\{[A-Za-z][A-Za-z0-9]*\})$/.test(part))) throw new Error(`Unsupported operation path: ${path}`);
  const placeholders = parts.filter(part => part.startsWith("{")).map(part => part.slice(1, -1));
  if (new Set(placeholders).size !== placeholders.length) throw new Error("Duplicate path placeholder");
  const output = [];
  let hasResumeHeader = false;
  for (const list of [item.parameters === undefined ? [] : item.parameters, op.parameters === undefined ? [] : op.parameters]) {
    if (!Array.isArray(list)) throw new Error("Invalid parameters declaration");
    for (const parameter of list) {
      keys(parameter, ["name", "in", "required", "description", "deprecated", "style", "explode", "schema"]);
      const { name, in: location } = parameter;
      if (location === "header" && typeof name === "string" && name.toLowerCase() === "last-event-id" && op["x-meshway-sse"] !== undefined) {
        if (hasResumeHeader) throw new Error("Duplicate SSE cursor header");
        hasResumeHeader = true;
        keys(parameter, ["name", "in", "required", "description", "deprecated", "schema"]);
        if (parameter.required !== undefined && parameter.required !== false) throw new Error("SSE cursor header must be optional");
        keys(parameter.schema, ["type", "description", "title"]);
        if (parameter.schema.type !== "string") throw new Error("SSE cursor header must be a non-null string");
        // This header is controlled by StreamOptions, never a second generated
        // argument that could bypass caller-applied cursor handling on retries.
        continue;
      }
      if (typeof name !== "string" || !/^[a-z][A-Za-z0-9]*$/.test(name) || ["constructor", "prototype", "self", "super", "crate"].includes(name)) throw new Error("Invalid parameter name");
      if (!["path", "query"].includes(location)) throw new Error("Only path/query parameters supported");
      if (output.some(value => value.name === name && value.location === location)) throw new Error("Duplicate parameter declaration");
      if (parameter.required !== undefined && typeof parameter.required !== "boolean") throw new Error("Invalid parameter required");
      if (location === "path" && (parameter.required !== true || !placeholders.includes(name))) throw new Error("Unknown or optional path parameter");
      if (parameter.style !== undefined && parameter.style !== (location === "path" ? "simple" : "form")) throw new Error("Unsupported parameter style");
      if (parameter.explode !== undefined && parameter.explode !== (location === "query")) throw new Error("Unsupported parameter explode");
      output.push({ name, rustName: name.replace(/[A-Z]/g, letter => `_${letter.toLowerCase()}`), location, required: parameter.required === true, wireSchema: parameter.schema, ...scalar(parameter.schema, schemas) });
    }
  }
  if (placeholders.some(name => !output.some(value => value.location === "path" && value.name === name))) throw new Error("Missing path parameter declaration");
  return {
    parts,
    pathParameters: placeholders.map(name => output.find(value => value.location === "path" && value.name === name)),
    queryParameters: output.filter(value => value.location === "query").sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0),
  };
}

export function parameterSchemas(model, schemas) {
  const output = { ...schemas };
  for (const op of model) for (const [suffix, list] of [["Path", op.pathParameters], ["Query", op.queryParameters]]) {
    if (!list.length) continue;
    const name = op.typeName + suffix;
    if (Object.hasOwn(output, name)) throw new Error(`Parameter schema collision: ${name}`);
    output[name] = { type: "object", properties: Object.fromEntries(list.map(p => [p.name, p.wireSchema])), required: list.filter(p => p.required).map(p => p.name), additionalProperties: false };
  }
  return output;
}

export function rustParameterStruct(op, suffix, list) {
  if (!list.length) return "";
  return `#[derive(Clone${list.every(p => !p.required) ? ", Default" : ""})]\npub struct ${op.typeName}${suffix} {\n${list.map(p => `    pub r#${p.rustName}: ${p.required ? "" : "Option<"}${p.component ? `meshway_protocol::${p.component}` : p.rustScalar}${p.required ? "" : ">"},`).join("\n")}\n}\n`;
}

export function rustParameterValue(p, expression) {
  // Serialize shared newtypes to an explicitly typed scalar, not a JSON value.
  const value = p.component ? `serde_json::from_slice::<${p.rustScalar}>(&serde_json::to_vec(${expression}).map_err(|_| crate::ClientError::Request)?).map_err(|_| crate::ClientError::Request)?` : `{ let input = ${expression}; input.to_owned() }`;
  const checks = [];
  if (p.kind === "integer") checks.push(`!(${p.minimum}..=${p.maximum}).contains(&value)`);
  if (p.kind === "string") {
    if (p.uuid) checks.push('!(value.len() == 36 && value != "00000000-0000-0000-0000-000000000000" && value.bytes().enumerate().all(|(index, byte)| if matches!(index, 8 | 13 | 18 | 23) { byte == b\'-\' } else { byte.is_ascii_digit() || (b\'a\'..=b\'f\').contains(&byte) }))');
    if (p.schema.minLength > 0) checks.push(`value.chars().count() < ${p.schema.minLength}`);
    if (p.schema.maxLength !== undefined) checks.push(`value.chars().count() > ${p.schema.maxLength}`);
    if (p.schema.const !== undefined) checks.push(`value != ${rustString(p.schema.const)}`);
    if (p.schema.enum !== undefined) checks.push(`![${p.schema.enum.map(rustString).join(", ")}].contains(&value.as_str())`);
  }
  return `{ let value = ${value}; ${checks.length ? `if ${checks.join(" || ")} { return Err(crate::ClientError::Request); } ` : ""}value.to_string() }`;
}

function rustString(value) {
  return '"' + [...value].map(character => {
    if (character === '"' || character === "\\") return "\\" + character;
    const code = character.codePointAt(0);
    return code < 32 || code === 127 ? `\\u{${code.toString(16)}}` : character;
  }).join("") + '"';
}
