/** Whether two plain data values (what JSON can hold) are the same. */
export function sameData(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true
  if (
    typeof a !== "object" ||
    typeof b !== "object" ||
    a === null ||
    b === null
  )
    return false
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length)
      return false
    return a.every((item, index) => sameData(item, b[index]))
  }
  const left = Object.entries(a).filter(([, value]) => value !== undefined)
  const right = new Map(
    Object.entries(b).filter(([, value]) => value !== undefined)
  )
  return (
    left.length === right.size &&
    left.every(
      ([key, value]) => right.has(key) && sameData(value, right.get(key))
    )
  )
}
