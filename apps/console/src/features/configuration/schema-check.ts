// Hints from the controller's configuration JSON Schema (2020-12), checked
// as the text is typed. Only the keywords that schema uses are read
// ($ref into $defs, type, properties, required, additionalProperties,
// propertyNames, items, enum, oneOf discriminated by `type`, minimum,
// maximum, minLength, maxLength, pattern); anything else is left to the
// controller, whose validation is the one that counts.

export type SchemaIssue = {
  path: (string | number)[]
  code:
    | "type"
    | "required"
    | "unknown_key"
    | "enum"
    | "variant"
    | "range"
    | "length"
    | "pattern"
  /** The expected type, the missing or unknown key, or the allowed values. */
  detail: string
  /** About the key at `path` (an unknown or invalid name), not its value. */
  onKey?: true
}

type Schema = Readonly<Record<string, unknown>>

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

const isSchema = (value: unknown): value is Schema => isRecord(value)

const LIMIT = 200

/** A schema pattern as a Unicode regular expression; null when this engine cannot read it. */
function compile(pattern: string): RegExp | null {
  const known = patterns.get(pattern)
  if (known !== undefined) return known
  let compiled: RegExp | null
  try {
    compiled = new RegExp(pattern, "u")
  } catch {
    compiled = null
  }
  patterns.set(pattern, compiled)
  return compiled
}
const patterns = new Map<string, RegExp | null>()

/** Reads the controller's schema text; null when it is not a schema object. */
export function readSchema(text: string): Schema | null {
  try {
    const parsed: unknown = JSON.parse(text)
    return isSchema(parsed) ? parsed : null
  } catch {
    return null
  }
}

/** The issues of `value` against `root`, in document order, at most 200. */
export function checkSchema(root: Schema, value: unknown): SchemaIssue[] {
  const issues: SchemaIssue[] = []
  const defs = isRecord(root.$defs) ? root.$defs : {}

  const resolve = (schema: Schema, depth = 0): Schema => {
    const ref = schema.$ref
    if (typeof ref !== "string" || depth > 32) return schema
    const name = /^#\/\$defs\/(.+)$/.exec(ref)?.[1]
    const target = name === undefined ? undefined : defs[name]
    return isSchema(target) ? resolve(target, depth + 1) : {}
  }

  const add = (issue: SchemaIssue) => {
    if (issues.length < LIMIT) issues.push(issue)
  }

  const typeOf = (item: unknown): string => {
    if (item === null) return "null"
    if (Array.isArray(item)) return "array"
    if (typeof item === "number")
      return Number.isInteger(item) ? "integer" : "number"
    return typeof item
  }

  const typeMatches = (expected: unknown, actual: string): boolean => {
    const list = Array.isArray(expected) ? expected : [expected]
    return list.some(
      (type) =>
        type === actual ||
        (type === "number" && actual === "integer") ||
        typeof type !== "string"
    )
  }

  const visit = (input: Schema, item: unknown, path: (string | number)[]) => {
    if (issues.length >= LIMIT) return
    const schema = resolve(input)

    if (Array.isArray(schema.oneOf)) {
      const branches = schema.oneOf.filter(isSchema).map((b) => resolve(b))
      const tag = isRecord(item) ? item.type : undefined
      const discriminated = branches.filter((branch) => {
        const properties = isRecord(branch.properties) ? branch.properties : {}
        const type = properties.type
        return isSchema(type) && Array.isArray(type.enum)
      })
      if (discriminated.length === branches.length && branches.length > 0) {
        const chosen = branches.find((branch) => {
          const properties = isRecord(branch.properties)
            ? branch.properties
            : {}
          const type = properties.type
          return (
            isSchema(type) &&
            Array.isArray(type.enum) &&
            type.enum.includes(tag)
          )
        })
        if (chosen) visit(chosen, item, path)
        else
          add({
            path: isRecord(item) && "type" in item ? [...path, "type"] : path,
            code: "variant",
            detail: branches
              .flatMap((branch) => {
                const properties = isRecord(branch.properties)
                  ? branch.properties
                  : {}
                const type = properties.type
                return isSchema(type) && Array.isArray(type.enum)
                  ? type.enum.map(String)
                  : []
              })
              .join(", "),
          })
        return
      }
      // Undiscriminated: one branch matching is enough for a hint.
      for (const branch of branches) {
        if (
          checkSchema({ ...root, $ref: undefined, ...branch }, item).length ===
          0
        )
          return
      }
      add({ path, code: "variant", detail: "" })
      return
    }

    const actual = typeOf(item)
    if (schema.type !== undefined && !typeMatches(schema.type, actual)) {
      add({
        path,
        code: "type",
        detail: (Array.isArray(schema.type) ? schema.type : [schema.type])
          .map(String)
          .join(" | "),
      })
      return
    }

    if (Array.isArray(schema.enum) && !schema.enum.includes(item)) {
      add({ path, code: "enum", detail: schema.enum.map(String).join(", ") })
      return
    }

    if (typeof item === "number") {
      if (typeof schema.minimum === "number" && item < schema.minimum)
        add({ path, code: "range", detail: `≥ ${schema.minimum}` })
      else if (typeof schema.maximum === "number" && item > schema.maximum)
        add({ path, code: "range", detail: `≤ ${schema.maximum}` })
    }

    if (typeof item === "string") {
      const length = [...item].length
      if (typeof schema.minLength === "number" && length < schema.minLength)
        add({ path, code: "length", detail: `≥ ${schema.minLength}` })
      else if (
        typeof schema.maxLength === "number" &&
        length > schema.maxLength
      )
        add({ path, code: "length", detail: `≤ ${schema.maxLength}` })
      else if (typeof schema.pattern === "string") {
        const pattern = compile(schema.pattern)
        if (pattern && !pattern.test(item))
          add({ path, code: "pattern", detail: "" })
      }
    }

    if (Array.isArray(item) && isSchema(schema.items)) {
      const items = schema.items
      item.forEach((element, index) => visit(items, element, [...path, index]))
    }

    if (isRecord(item)) {
      const properties = isRecord(schema.properties) ? schema.properties : {}
      if (Array.isArray(schema.required))
        for (const key of schema.required)
          if (typeof key === "string" && !Object.hasOwn(item, key))
            add({ path, code: "required", detail: key })
      for (const [key, element] of Object.entries(item)) {
        const known = properties[key]
        if (isSchema(known)) {
          visit(known, element, [...path, key])
          continue
        }
        if (schema.additionalProperties === false) {
          add({
            path: [...path, key],
            code: "unknown_key",
            detail: key,
            onKey: true,
          })
          continue
        }
        if (isSchema(schema.propertyNames)) {
          const before = issues.length
          visit(schema.propertyNames, key, [...path, key])
          // A key that is not a valid name is said once, on the key.
          if (issues.length > before) {
            for (const issue of issues.slice(before)) issue.onKey = true
            continue
          }
        }
        if (isSchema(schema.additionalProperties))
          visit(schema.additionalProperties, element, [...path, key])
      }
    }
  }

  visit(root, value, [])
  return issues
}
