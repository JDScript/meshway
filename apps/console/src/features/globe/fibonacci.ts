/** Samples on the whole sphere; land keeps roughly a third of them. */
export const LAND_SAMPLES = 36_000

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5))

/** Sample `index` of an `count`-point Fibonacci sphere, in degrees. */
export function fibonacciPoint(
  index: number,
  count: number
): { lat: number; lng: number } {
  const y = 1 - (2 * (index + 0.5)) / count
  const theta = GOLDEN_ANGLE * index
  const lng = (((((theta * 180) / Math.PI + 180) % 360) + 360) % 360) - 180
  return { lat: (Math.asin(y) * 180) / Math.PI, lng }
}
