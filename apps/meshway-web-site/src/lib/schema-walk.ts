// Following a JSON value through the contract's schema, to say what each
// field means. The value decides the way: a union takes the branch its
// tag (`type`, `kind`, `code`) names, else the one whose required fields
// it has; a nullable takes its non-null side. References are resolved as
// they are met, so recursive schemas are only followed as deep as the
// value goes.

export type Resolve = (node: unknown) => unknown

type Node = Record<string, unknown>

const isNode = (value: unknown): value is Node =>
  typeof value === "object" && value !== null && !Array.isArray(value)

const TAGS = ["type", "kind", "code"] as const

/** The schema node itself, references resolved (sibling keywords kept by `resolve`). */
function deref(schema: unknown, resolve: Resolve): Node | null {
  let node: unknown = schema
  for (let hops = 0; hops < 16 && isNode(node) && "$ref" in node; hops++)
    node = resolve(node)
  return isNode(node) ? node : null
}

const variants = (node: Node): unknown[] | null => {
  const list = node.oneOf ?? node.anyOf
  return Array.isArray(list) ? list : null
}

/** The tag value a branch requires (`"type": { "enum": ["x"] }` or `const`). */
function tagOf(branch: Node, tag: string, resolve: Resolve): unknown {
  const properties = branch.properties
  if (!isNode(properties)) return undefined
  const property = deref(properties[tag], resolve)
  if (!property) return undefined
  if ("const" in property) return property.const
  return Array.isArray(property.enum) && property.enum.length === 1
    ? property.enum[0]
    : undefined
}

/** The branch of a union `value` is in, or the node when it is not a union. */
function narrow(
  schema: unknown,
  value: unknown,
  resolve: Resolve
): Node | null {
  let node = deref(schema, resolve)
  for (let depth = 0; node && depth < 8; depth++) {
    const list = variants(node)
    if (!list) break
    const branches = list
      .map((branch) => deref(branch, resolve))
      .filter((branch): branch is Node => branch !== null)
      .filter((branch) => value === null || branch.type !== "null")
    let chosen: Node | undefined =
      branches.length === 1 ? branches[0] : undefined
    if (!chosen && isNode(value)) {
      for (const tag of TAGS) {
        if (!(tag in value)) continue
        chosen = branches.find(
          (branch) => tagOf(branch, tag, resolve) === value[tag]
        )
        if (chosen) break
      }
      chosen ??= branches.find(
        (branch) =>
          Array.isArray(branch.required) &&
          branch.required.every(
            (key) => typeof key === "string" && key in value
          )
      )
    }
    if (!chosen) return node
    // The union's own words describe the field more than a branch's.
    node =
      typeof node.description === "string" && !chosen.description
        ? { ...chosen, description: node.description }
        : chosen
  }
  return node
}

/** What the schema says a field holding `value` is: its description, else its branch's or type's. */
export function describe(
  schema: unknown,
  value: unknown,
  resolve: Resolve
): string | null {
  const own = isNode(schema) ? schema.description : undefined
  if (typeof own === "string" && own.trim()) return own.trim()
  const node = narrow(schema, value, resolve)
  return typeof node?.description === "string" && node.description.trim()
    ? node.description.trim()
    : null
}

/** The schema of `value[key]` (an object's field or an array's item). */
export function childSchema(
  schema: unknown,
  value: unknown,
  key: string | number,
  resolve: Resolve
): unknown {
  const node = narrow(schema, value, resolve)
  if (!node) return null
  if (typeof key === "number") return node.items ?? null
  const properties = node.properties
  if (isNode(properties) && key in properties) return properties[key]
  for (const part of Array.isArray(node.allOf) ? node.allOf : []) {
    const branch = deref(part, resolve)
    const inner = branch?.properties
    if (isNode(inner) && key in inner) return inner[key]
  }
  return isNode(node.additionalProperties) ? node.additionalProperties : null
}
