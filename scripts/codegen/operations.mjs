// Both clients consume this deliberately small, fail-closed operation model.
const identifier = /^[A-Za-z][A-Za-z0-9]*$/;
const reserved = new Set("as async await break case catch class const continue crate debugger default delete do dyn else enum export extends extern false final finally fn for function if impl import in instanceof let loop match mod move mut new null override package priv protected pub public ref return self Self static struct super switch this throw trait true try type typeof union unsafe unsized use var virtual void volatile where while with yield constructor request clone".split(" "));
function keys(value, allowed, context) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Expected ${context} object`);
  for (const key of Object.keys(value)) if (!allowed.includes(key)) throw new Error(`Unsupported ${context} keyword: ${key}`);
}
export function operationModel(document) {
  keys(document, ["openapi", "info", "paths", "components", "security", "tags", "externalDocs"], "document");
  if (document.openapi !== "3.1.0") throw new Error("Expected native OpenAPI 3.1.0");
  keys(document.components, ["schemas", "securitySchemes"], "components");
  const names = new Set();
  const rustNames = new Set();
  const output = [];
  function component(content) {
    keys(content, ["application/json"], "JSON content (streams require a separate client)");
    if (!content["application/json"]) throw new Error("Missing JSON content");
    keys(content["application/json"], ["schema"], "media");
    const schema = content["application/json"].schema;
    keys(schema, ["$ref"], "operation schema");
    const name = schema.$ref?.replace(/^#\/components\/schemas\//, "");
    if (!name || schema.$ref === name || !identifier.test(name) || !Object.hasOwn(document.components.schemas, name)) throw new Error("Operations must reference named local component schemas");
    return name;
  }
  for (const [path, item] of Object.entries(document.paths)) {
    // Parameters need a separate schema-aware encoding extension.
    if (!/^\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+$/.test(path)) throw new Error(`Unsupported operation path: ${path}`);
    keys(item, ["get", "post", "summary", "description"], "path");
    for (const method of ["get", "post"]) {
      if (!Object.hasOwn(item, method)) continue;
      const op = item[method];
      keys(op, ["operationId", "summary", "description", "tags", "security", "requestBody", "responses", "deprecated"], "operation");
      const name = op.operationId;
      const rustName = typeof name === "string" ? name.replace(/[A-Z]/g, letter => `_${letter.toLowerCase()}`) : "";
      if (typeof name !== "string" || !/^[a-z][A-Za-z0-9]*$/.test(name) || names.has(name) || rustNames.has(rustName) || reserved.has(name) || reserved.has(rustName)) throw new Error(`Invalid or duplicate operationId: ${name}`);
      names.add(name);
      rustNames.add(rustName);
      const security = op.security ?? document.security ?? [];
      if (!Array.isArray(security)) throw new Error("Invalid security declaration");
      let authenticated = false;
      if (security.length) {
        if (security.length !== 1 || Object.keys(security[0]).length !== 1) throw new Error("Only single required bearer authentication is supported");
        const [schemeName, scopes] = Object.entries(security[0])[0];
        const scheme = document.components.securitySchemes?.[schemeName];
        keys(scheme, ["type", "scheme", "bearerFormat", "description"], "security scheme");
        if (!Array.isArray(scopes) || scopes.length || scheme.type !== "http" || scheme.scheme !== "bearer") throw new Error("Unsupported security scheme");
        authenticated = true;
      }
      let body;
      if (op.requestBody !== undefined) {
        keys(op.requestBody, ["required", "content", "description"], "request body");
        if (method !== "post" || op.requestBody.required !== true) throw new Error("Only required POST JSON bodies are supported");
        body = component(op.requestBody.content);
      }
      const responses = Object.entries(op.responses).map(([status, response]) => {
        if (!/^[245]\d\d$/.test(status)) throw new Error(`Unsupported response status: ${status}`);
        keys(response, ["description", "content"], "response");
        if (status === "204") {
          if (response.content !== undefined) throw new Error("204 cannot declare content");
          return { status: Number(status), type: undefined };
        }
        return { status: Number(status), type: component(response.content) };
      }).sort((a, b) => a.status - b.status);
      const successes = responses.filter(response => response.status < 300);
      if (successes.length !== 1) throw new Error("Exactly one success response is supported");
      const errors = responses.filter(response => response.status >= 400);
      if (new Set(errors.map(response => response.type)).size > 1) throw new Error("Multiple error envelopes require a generator extension");
      output.push({ name, rustName, method: method.toUpperCase(), path, authenticated, body, success: successes[0], errors, errorType: errors[0]?.type });
    }
  }
  if (!output.length) throw new Error("No implemented operations to generate");
  return output.sort((a, b) => a.name.localeCompare(b.name, "en"));
}

const banner = "// Generated from Controller OpenAPI. Do not edit.\n";
export function typescriptOperations(model) {
  const schemas = new Set();
  const functions = model.map(op => {
    for (const name of [op.body, op.success.type, op.errorType]) if (name) schemas.add(name);
    const options = op.authenticated ? "AuthenticatedRequestOptions" : "RequestOptions";
    return `export function ${op.name}(client: ControlClient, ${op.body ? `body: z.output<typeof z${op.body}>, ` : ""}options: ${options}${op.authenticated ? "" : " = {}"}) {\n  return client.request({ method: ${JSON.stringify(op.method)}, path: ${JSON.stringify(op.path)}, authenticated: ${op.authenticated}, successStatus: ${op.success.status}, success: ${op.success.type ? `z${op.success.type}` : "undefined"}, errors: [${op.errors.map(error => error.status).join(", ")}], problem: ${op.errorType ? `z${op.errorType}` : "undefined"}${op.body ? `, body: { schema: z${op.body}, value: body }` : ""} }, options);\n}`;
  });
  return `${banner}\nimport type * as z from "zod";\nimport { ControlClient, type AuthenticatedRequestOptions, type RequestOptions } from "../transport.ts";\nimport { ${[...schemas].sort().map(name => `z${name}`).join(", ")} } from "./zod.gen.ts";\n\n${functions.join("\n\n")}\n`;
}

export function rustOperations(model) {
  const functions = model.map(op => {
    const success = op.success.type ? `meshway_protocol::${op.success.type}` : "()";
    const error = op.errorType ? `meshway_protocol::${op.errorType}` : "std::convert::Infallible";
    return `    pub async fn ${op.rustName}(&self${op.body ? `, body: &meshway_protocol::${op.body}` : ""}${op.authenticated ? ", token: &str" : ""}) -> Result<${success}, crate::ClientError<${error}>> {\n        let response = self.send(reqwest::Method::${op.method}, ${JSON.stringify(op.path)}, ${op.authenticated ? "Some(token)" : "None"}, ${op.body ? "Some(body)" : "None::<&()>"}).await?;\n        match response.status {\n            ${op.success.status} => ${op.success.type ? "response.json()" : "response.empty()"},\n            ${op.errors.length ? op.errors.map(error => error.status).join(" | ") + " => Err(response.problem()),\n            " : ""}_ => Err(response.unexpected()),\n        }\n    }`;
  });
  return `${banner}\nimpl crate::Client {\n${functions.join("\n\n")}\n}\n`;
}
