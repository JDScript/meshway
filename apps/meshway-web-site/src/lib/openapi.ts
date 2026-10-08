import type {
  Document,
  OperationObject,
  PathItemObject,
} from "fumadocs-openapi"
import { createOpenAPI } from "fumadocs-openapi/server"

const contract = createOpenAPI({
  input: {
    meshway: "../../packages/protocol/openapi.json",
  },
})

const methods = [
  "get",
  "post",
  "delete",
  "put",
  "patch",
  "head",
  "options",
] as const
const excludedTags = new Set(["execution", "node", "connectivity"])

export const apiTags: Record<string, { zh: string; en: string }> = {
  auth: { zh: "认证", en: "Authentication" },
  account: { zh: "账户", en: "Account" },
  networks: { zh: "网络", en: "Networks" },
  members: { zh: "成员与加入链接", en: "Members & join links" },
  configuration: { zh: "配置", en: "Configuration" },
  nodes: { zh: "节点", en: "Nodes" },
  enrollment: { zh: "设备接入", en: "Enrollment" },
  state: { zh: "状态", en: "State" },
  events: { zh: "事件流", en: "Event streams" },
  resources: { zh: "规则集", en: "Rule sets" },
  history: { zh: "操作历史", en: "Activity history" },
  observations: { zh: "实时观测", en: "Realtime observations" },
}

export const operationSlug = (id: string) =>
  id.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase()

export interface ReferenceOperation {
  path: string
  method: (typeof methods)[number]
  operation: OperationObject & { operationId: string }
  tag: string
  slug: string
}

/** The only public-reference filter. Generation and rendering share this view. */
async function loadReference() {
  const { bundled } = await contract.getSchema("meshway")
  const paths: NonNullable<Document["paths"]> = {}
  const operations: ReferenceOperation[] = []
  for (const [path, item] of Object.entries(bundled.paths ?? {})) {
    if (!path.startsWith("/api/v1/") && path !== "/api/v1") continue
    if (!item) continue
    const filtered: PathItemObject = { parameters: item.parameters }
    for (const method of methods) {
      const operation = item[method]
      if (!operation || operation.tags?.some((tag) => excludedTags.has(tag)))
        continue
      const id = operation.operationId
      const tag = operation.tags?.[0]
      if (!id || !tag || !apiTags[tag])
        throw new Error(`Unmapped API operation: ${path}`)
      filtered[method] = operation
      operations.push({
        path,
        method,
        operation: { ...operation, operationId: id },
        tag,
        slug: operationSlug(id),
      })
    }
    if (methods.some((method) => filtered[method])) paths[path] = filtered
  }
  const document: Document = {
    ...bundled,
    paths,
    tags: bundled.tags?.filter(
      (tag) => tag.name && !excludedTags.has(tag.name)
    ),
  }
  return { document, operations }
}

let reference: ReturnType<typeof loadReference> | undefined
export const getReference = () => (reference ??= loadReference())

/** Keep only reachable components in a page payload, including extension refs. */
export function operationDocument(
  document: Document,
  entry: ReferenceOperation
): Document {
  const refs = new Set<string>()
  function visit(value: unknown) {
    if (!value || typeof value !== "object") return
    if ("$ref" in value && typeof value.$ref === "string") {
      const name = value.$ref.replace("#/components/schemas/", "")
      if (!refs.has(name)) {
        refs.add(name)
        visit(document.components?.schemas?.[name])
      }
    }
    for (const child of Object.values(value)) visit(child)
  }
  const pathItem = {
    parameters: document.paths?.[entry.path]?.parameters,
    [entry.method]: entry.operation,
  }
  visit(pathItem)
  const result: Document = {
    openapi: document.openapi,
    info: document.info,
    paths: { [entry.path]: pathItem },
    components: {
      schemas: Object.fromEntries(
        Object.entries(document.components?.schemas ?? {}).filter(([name]) =>
          refs.has(name)
        )
      ),
      securitySchemes: document.components?.securitySchemes,
    },
    security: document.security,
  }
  return quietConstraints(isolateFieldAnnotations(result))
}

/** Scalar's shared ref proxies otherwise let one field's description overwrite another. */
function isolateFieldAnnotations(document: Document): Document {
  const result = structuredClone(document)
  const schemas = result.components?.schemas
  if (!schemas) return result
  const annotations = new Map<string, string>()
  function visit(value: unknown) {
    if (!value || typeof value !== "object") return
    if (
      "$ref" in value &&
      typeof value.$ref === "string" &&
      Object.keys(value).length > 1
    ) {
      const base = schemas?.[value.$ref.replace("#/components/schemas/", "")]
      if (base && typeof base === "object") {
        const key = JSON.stringify(value)
        let name = annotations.get(key)
        if (!name) {
          name = `ReferenceField${annotations.size + 1}`
          annotations.set(key, name)
          const siblings = Object.fromEntries(
            Object.entries(value).filter(([key]) => key !== "$ref")
          )
          const schema = Object.assign(
            {},
            structuredClone(base),
            { title: value.$ref.split("/").at(-1) },
            siblings
          )
          schemas[name] = schema
          visit(schema)
        }
        for (const key of Object.keys(value)) Reflect.deleteProperty(value, key)
        Object.assign(value, { $ref: `#/components/schemas/${name}` })
        return
      }
    }
    if (
      "properties" in value &&
      value.properties &&
      typeof value.properties === "object"
    ) {
      for (const [key, discriminator] of Object.entries(value.properties)) {
        if (
          ["type", "code", "kind"].includes(key) &&
          discriminator &&
          typeof discriminator === "object" &&
          "enum" in discriminator &&
          Array.isArray(discriminator.enum) &&
          discriminator.enum.length === 1 &&
          typeof discriminator.enum[0] === "string"
        ) {
          if (!("title" in value))
            Object.assign(value, { title: discriminator.enum[0] })
        }
      }
    }
    for (const child of Object.values(value)) visit(child)
  }
  visit(result.paths)
  for (const schema of Object.values(schemas)) visit(schema)
  return result
}

/** The contract's ID pattern: lowercase hyphenated UUIDs, never all zeros. */
const UUID_PATTERN =
  "^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"

/**
 * Constraints every ID and integer carries, said once on the overview page
 * instead of on each field: an ID's pattern and length (its format, uuid,
 * stays) and the JSON-safe integer maximum.
 */
function quietConstraints(document: Document): Document {
  function visit(value: unknown) {
    if (!value || typeof value !== "object") return
    if (
      "format" in value &&
      value.format === "uuid" &&
      "pattern" in value &&
      value.pattern === UUID_PATTERN
    ) {
      for (const key of ["pattern", "minLength", "maxLength"])
        Reflect.deleteProperty(value, key)
    }
    if ("maximum" in value && value.maximum === Number.MAX_SAFE_INTEGER)
      Reflect.deleteProperty(value, "maximum")
    for (const child of Object.values(value)) visit(child)
  }
  visit(document.paths)
  visit(document.components?.schemas)
  return document
}
