// Maps the forms edit as rows (groups, tag owners, DNS servers and routes):
// renaming a row's key in place, and whether a name can be a key.

/** Renames a key; the row keeps its place in the map. */
export function renamed<V>(
  map: Readonly<Record<string, V>>,
  from: string,
  to: string
): Record<string, V> {
  return Object.fromEntries(
    Object.entries(map).map(([key, value]) => [key === from ? to : key, value])
  )
}

/** The map without `key`. */
export const without = <V>(
  map: Readonly<Record<string, V>>,
  key: string
): Record<string, V> =>
  Object.fromEntries(Object.entries(map).filter(([at]) => at !== key))

/** The map with `key` set to `value`, in place if it was there, else last. */
export const withEntry = <V>(
  map: Readonly<Record<string, V>>,
  key: string,
  value: V
): Record<string, V> => ({ ...map, [key]: value })

export type KeyProblem = "empty" | "space" | "taken"

/**
 * Why `prefix + name` cannot be a key of `map` (renaming `from`, or a new
 * row when null): no name, white space in it, or another row's key.
 */
export function keyProblem(
  map: Readonly<Record<string, unknown>>,
  prefix: string,
  name: string,
  from: string | null
): KeyProblem | null {
  if (name === "") return "empty"
  if (/\s/.test(name)) return "space"
  const key = prefix + name
  if (key !== from && Object.hasOwn(map, key)) return "taken"
  return null
}
