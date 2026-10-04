// Where two readings of the same JSON data differ: what the cache holds
// against what a fresh read returned. Arrays are compared by position, as
// the controller orders them.

/** One place the two differ; `undefined` where a side has no value. */
export type Difference = { path: string; cached: unknown; read: unknown }

export type Differences = {
  /** The first `limit` differences, in document order. */
  found: Difference[]
  /** How many there are in all. */
  total: number
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

/** A JSON Pointer token (RFC 6901). */
const token = (key: string | number) =>
  String(key).replaceAll("~", "~0").replaceAll("/", "~1")

/** Where `cached` and `read` differ, as JSON Pointers, leaving out the `ignored` ones. */
export function differences(
  cached: unknown,
  read: unknown,
  {
    limit = 50,
    ignored = [],
  }: { limit?: number; ignored?: readonly string[] } = {}
): Differences {
  const found: Difference[] = []
  let total = 0
  const visit = (a: unknown, b: unknown, path: string) => {
    if (ignored.includes(path)) return
    if (Array.isArray(a) && Array.isArray(b)) {
      for (let index = 0; index < Math.max(a.length, b.length); index++)
        visit(a[index], b[index], `${path}/${index}`)
      return
    }
    if (isRecord(a) && isRecord(b)) {
      const keys = new Set([...Object.keys(a), ...Object.keys(b)])
      for (const key of [...keys].sort())
        visit(a[key], b[key], `${path}/${token(key)}`)
      return
    }
    if (Object.is(a, b)) return
    total++
    if (found.length < limit) found.push({ path, cached: a, read: b })
  }
  visit(cached, read, "")
  return { found, total }
}
