import type { Document, ReferenceObject } from "fumadocs-openapi"

import type { ReferenceOperation } from "./openapi"

function isReference(value: unknown): value is ReferenceObject {
  return (
    typeof value === "object" &&
    value !== null &&
    "$ref" in value &&
    typeof value.$ref === "string"
  )
}

export function codeSamples(document: Document, entry: ReferenceOperation) {
  const { operation, path, method } = entry
  const parameters = [
    ...(document.paths?.[path]?.parameters ?? []),
    ...(operation.parameters ?? []),
  ]
  const resolved = parameters.map((parameter) => {
    if (isReference(parameter))
      throw new Error(
        `Referenced parameter needs SDK mapping: ${parameter.$ref}`
      )
    return parameter
  })
  const pathParameters = resolved.filter((parameter) => parameter.in === "path")
  const queryParameters = resolved.filter(
    (parameter) => parameter.in === "query"
  )
  const authenticated = (operation.security ?? document.security ?? []).some(
    (item) => Object.keys(item).length > 0
  )
  const body = operation.requestBody
  if (body && isReference(body))
    throw new Error(`Referenced request body: ${body.$ref}`)
  const media = body?.content?.["application/json"]
  if (isReference(media))
    throw new Error(`Referenced request media: ${media.$ref}`)
  const bodySchema = media?.schema
  if (bodySchema && !isReference(bodySchema))
    throw new Error(`Inline body needs SDK mapping: ${operation.operationId}`)
  const schemaName = isReference(bodySchema)
    ? bodySchema.$ref.split("/").at(-1)
    : undefined
  const stream = "x-meshway-sse" in operation
  const args = ["client"]
  const declarations: string[] = []
  const sample = (parameter: (typeof resolved)[number]) => {
    const schema = isReference(parameter.schema)
      ? document.components?.schemas?.[
          parameter.schema.$ref.replace("#/components/schemas/", "")
        ]
      : parameter.schema
    return schema &&
      typeof schema === "object" &&
      !isReference(schema) &&
      schema.enum?.[0] !== undefined
      ? JSON.stringify(schema.enum[0])
      : schema &&
          typeof schema === "object" &&
          !isReference(schema) &&
          schema.type === "integer"
        ? String(schema.minimum ?? 1)
        : JSON.stringify(`<${parameter.name}>`)
  }
  if (pathParameters.length) {
    for (const parameter of pathParameters) {
      declarations.push(`const ${parameter.name} = ${sample(parameter)}`)
    }
    args.push(
      `{ ${pathParameters.map((parameter) => parameter.name).join(", ")} }`
    )
  }
  const required = queryParameters.filter((parameter) => parameter.required)
  if (queryParameters.length) {
    const fields = required
      .map((parameter) => `${parameter.name}: ${sample(parameter)}`)
      .join(", ")
    const optional =
      required.length < queryParameters.length
        ? " // Optional filters, limit and cursor: see parameters below."
        : ""
    declarations.push(
      `const query = {${fields ? ` ${fields} ` : ""}}${optional}`
    )
    args.push("query")
  }
  const search = required.length
    ? `?${required
        .map(
          (parameter) =>
            `${parameter.name}=${JSON.parse(sample(parameter)) as string | number}`
        )
        .join("&")}`
    : ""
  if (body) {
    if (!schemaName)
      throw new Error(`Missing SDK body schema: ${operation.operationId}`)
    declarations.push(
      `const body = z${schemaName}.parse(\n  JSON.parse(await readFile("request.json", "utf8")),\n)`
    )
    args.push("body")
  }
  if (authenticated) args.push("{ token }")
  const imports = [
    ...(body ? ['import { readFile } from "node:fs/promises"'] : []),
    `import { ControlClient, ${operation.operationId} } from "@meshway/control-client"`,
    ...(body
      ? [`import { z${schemaName} } from "@meshway/control-client/schemas"`]
      : []),
  ]
  const setup = [
    "const server = process.env.MESHWAY_SERVER",
    ...(authenticated ? ["const token = process.env.MESHWAY_TOKEN"] : []),
    `if (!server${authenticated ? " || !token" : ""}) throw new Error("Set MESHWAY_SERVER${authenticated ? " and MESHWAY_TOKEN" : ""}")`,
    "const client = new ControlClient({ baseUrl: server })",
  ]
  const call = `${operation.operationId}(${args.join(", ")})`
  const result = stream
    ? `for await (const result of ${call}) {\n  if (!result.ok) throw new Error(result.error.kind)\n  // Apply result.data.data as part of a complete batch/event.\n  // Only then acknowledge its cursor for reconnection.\n}`
    : `const result = await ${call}\nif (!result.ok) throw new Error(result.error.kind)\n// result.data is the validated response for your application.`
  const typescript = [
    imports.join("\n"),
    setup.join("\n"),
    declarations.join("\n"),
    result,
  ]
    .filter(Boolean)
    .join("\n\n")
  const curl = [
    `curl${stream ? " --no-buffer" : ""} --request ${method.toUpperCase()} \\\n  "$MESHWAY_SERVER${path.replace(/\{([^}]+)\}/g, "<$1>")}${search}"`,
    ...(authenticated
      ? ['  --header "Authorization: Bearer $MESHWAY_TOKEN"']
      : []),
    `  --header "Accept: ${stream ? "text/event-stream" : "application/json"}"`,
    ...(body
      ? [
          '  --header "Content-Type: application/json"',
          "  --data-binary @request.json",
        ]
      : []),
  ].join(" \\\n")
  return { curl, typescript }
}
