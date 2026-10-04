import { fibonacciPoint, LAND_SAMPLES } from "./fibonacci"
import { LAND_BITS } from "./land-dots.gen"

/** Land dots of the decorative base layer, rebuilt from the generated bitset. */
export function landPoints(): { lat: number; lng: number }[] {
  const bytes = Uint8Array.from(atob(LAND_BITS), (char) => char.charCodeAt(0))
  const points: { lat: number; lng: number }[] = []
  for (let index = 0; index < LAND_SAMPLES; index += 1) {
    if (((bytes[index >> 3] ?? 0) >> (index & 7)) & 1)
      points.push(fibonacciPoint(index, LAND_SAMPLES))
  }
  return points
}
