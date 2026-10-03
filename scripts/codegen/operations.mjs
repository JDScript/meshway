// Both clients consume this deliberately small, fail-closed operation model.
import { parameters, rustParameterStruct, rustParameterValue } from "./parameters.mjs";
const identifier = /^[A-Za-z][A-Za-z0-9]*$/;
const reserved = new Set("as async await break case catch class const continue crate debugger default delete do dyn else enum export extends extern false final finally fn for function if impl import in instanceof let loop match mod move mut new null override package priv protected pub public ref return self Self static struct super switch this throw trait true try type typeof union unsafe unsized use var virtual void volatile where while with yield constructor request stream clone".split(" "));
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
  function reference(schema) {
    keys(schema, ["$ref"], "operation schema");
    const name = schema.$ref?.replace(/^#\/components\/schemas\//, "");
    if (!name || schema.$ref === name || !identifier.test(name) || !Object.hasOwn(document.components.schemas, name)) throw new Error("Operations must reference named local component schemas");
    return name;
  }
  function component(content) {
    keys(content, ["application/json"], "JSON content");
    if (!content["application/json"]) throw new Error("Missing JSON content");
    keys(content["application/json"], ["schema"], "media");
    return reference(content["application/json"].schema);
  }
  for (const [path, item] of Object.entries(document.paths)) {
    keys(item, ["get", "post", "delete", "summary", "description", "parameters"], "path");
    for (const method of ["get", "post", "delete"]) {
      if (!Object.hasOwn(item, method)) continue;
      const op = item[method];
      keys(op, ["operationId", "summary", "description", "tags", "security", "requestBody", "responses", "deprecated", "parameters", "x-meshway-sse"], "operation");
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
      let eventType;
      if (Object.hasOwn(op, "x-meshway-sse")) {
        const stream = op["x-meshway-sse"];
        keys(stream, ["format", "eventSchema"], "SSE declaration");
        if (method !== "get" || op.requestBody !== undefined || stream.format !== "json-data") throw new Error("Only GET JSON-data SSE is supported");
        eventType = reference(stream.eventSchema);
      }
      if (op.requestBody !== undefined) {
        keys(op.requestBody, ["required", "content", "description"], "request body");
        if (!["post", "delete"].includes(method) || op.requestBody.required !== true) throw new Error("Only required mutation JSON bodies are supported");
        body = component(op.requestBody.content);
      }
      const responses = Object.entries(op.responses).map(([status, response]) => {
        if (!/^[245]\d\d$/.test(status)) throw new Error(`Unsupported response status: ${status}`);
        keys(response, ["description", "content"], "response");
        if (eventType && Number(status) < 300) {
          if (status !== "200") throw new Error("SSE requires a 200 response");
          keys(response.content, ["text/event-stream"], "SSE content");
          const media = response.content["text/event-stream"];
          keys(media, ["schema"], "SSE media");
          keys(media.schema, ["type"], "SSE byte stream schema");
          if (media.schema.type !== "string") throw new Error("SSE response must describe string bytes, not its event object");
          return { status: 200, type: eventType };
        }
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
      output.push({ name, rustName, typeName: name[0].toUpperCase() + name.slice(1), ...parameters(path, item, op, document.components.schemas), method: method.toUpperCase(), path, authenticated, body, stream: eventType !== undefined, success: successes[0], errors, errorType: errors[0]?.type });
    }
  }
  if (!output.length) throw new Error("No implemented operations to generate");
  return output.sort((a, b) => a.name.localeCompare(b.name, "en"));
}

const banner = "// Generated from Controller OpenAPI. Do not edit.\n";
export function typescriptOperations(model) {
  const schemas = new Set();
  const optionTypes = new Set();
  const functions = model.map(op => {
    for (const name of [op.body, op.success.type, op.errorType]) if (name) schemas.add(name);
    const options = `${op.authenticated ? "Authenticated" : ""}${op.stream ? "Stream" : "Request"}Options`;
    optionTypes.add(options);
    const args = [], values = [];
    for (const [suffix, list, name] of [["Path", op.pathParameters, "path"], ["Query", op.queryParameters, "query"]]) if (list.length) {
      const schema = op.typeName + suffix;
      schemas.add(schema);
      args.push(`${name}: z.output<typeof z${schema}>, `);
      values.push(`, ${name}Parameters: { schema: z${schema}, value: ${name} }`);
    }
    return `export function ${op.name}(client: ControlClient, ${args.join("")}${op.body ? `body: z.output<typeof z${op.body}>, ` : ""}options: ${options}${op.authenticated ? "" : " = {}"}) {\n  return client.${op.stream ? "stream" : "request"}({ method: ${JSON.stringify(op.method)}, path: ${JSON.stringify(op.path)}, authenticated: ${op.authenticated}, successStatus: ${op.success.status}, success: ${op.success.type ? `z${op.success.type}` : "undefined"}, errors: [${op.errors.map(error => error.status).join(", ")}], problem: ${op.errorType ? `z${op.errorType}` : "undefined"}${values.join("")}${op.body ? `, body: { schema: z${op.body}, value: body }` : ""} }, options);\n}`;
  });
  return `${banner}\nimport type * as z from "zod";\nimport { ControlClient, ${[...optionTypes].sort().map(name => `type ${name}`).join(", ")} } from "../transport.ts";\nimport { ${[...schemas].sort().map(name => `z${name}`).join(", ")} } from "./zod.gen.ts";\n\n${functions.join("\n\n")}\n`;
}

export function rustOperations(model) {
  const functions = model.map(op => {
    const success = op.success.type ? `meshway_protocol::${op.success.type}` : "()";
    const error = op.errorType ? `meshway_protocol::${op.errorType}` : "std::convert::Infallible";
    const hasParameters = op.pathParameters.length || op.queryParameters.length;
    const args = `${op.pathParameters.length ? `, path: &${op.typeName}Path` : ""}${op.queryParameters.length ? `, query: &${op.typeName}Query` : ""}`;
    const setup = hasParameters ? `        let path_values = [${op.pathParameters.map(p => `(${JSON.stringify(p.name)}, ${rustParameterValue(p, `&path.r#${p.rustName}`)})`).join(", ")}];\n        let query_values = [${op.queryParameters.map(p => `(${JSON.stringify(p.name)}, query.r#${p.rustName}.as_ref().map(|value| Ok::<_, crate::ClientError<${error}>>(${rustParameterValue(p, "value")})).transpose()?)`).join(", ")}];\n        let url = self.parameter_url(${JSON.stringify(op.path)}, &path_values, &query_values)?;\n` : "";
    if (op.stream) {
      const contract = op.errorType ? `crate::StreamResponse::json(${op.success.status}, &[${op.errors.map(error => error.status).join(", ")}])` : `crate::StreamResponse::untyped(${op.success.status})`;
      return `    pub async fn ${op.rustName}(&self${args}${op.authenticated ? ", token: &str" : ""}, options: &crate::StreamOptions<'_>) -> Result<crate::EventStream<${success}, ${error}>, crate::ClientError<${error}>> {\n${setup || `        let url = self.parameter_url(${JSON.stringify(op.path)}, &[], &[])?;\n`}        self.stream(url, ${op.authenticated ? "Some(token)" : "None"}, options, ${contract}).await\n    }`;
    }
    return `    pub async fn ${op.rustName}(&self${args}${op.body ? `, body: &meshway_protocol::${op.body}` : ""}${op.authenticated ? ", token: &str" : ""}) -> Result<${success}, crate::ClientError<${error}>> {\n${setup}        let response = self.${hasParameters ? "send_url" : "send"}(reqwest::Method::${op.method}, ${hasParameters ? "url" : JSON.stringify(op.path)}, ${op.authenticated ? "Some(token)" : "None"}, ${op.body ? "Some(body)" : "None::<&()>"}).await?;\n        match response.status {\n            ${op.success.status} => ${op.success.type ? "response.json()" : "response.empty()"},\n            ${op.errors.length ? op.errors.map(error => error.status).join(" | ") + " => Err(response.problem()),\n            " : ""}_ => Err(response.unexpected()),\n        }\n    }`;
  });
  const structs = model.map(op => rustParameterStruct(op, "Path", op.pathParameters) + rustParameterStruct(op, "Query", op.queryParameters)).filter(Boolean);
  return `${banner}\n${structs.length ? structs.join("\n") + "\n" : ""}impl crate::Client {\n${functions.join("\n\n")}\n}\n`;
}
