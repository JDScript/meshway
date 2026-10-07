import type { GeoPoint } from "./astronomy"
import { TIME_ZONES } from "./time-zones.gen"

// Where the reader roughly is, without asking: the principal city of the
// browser's time zone. Only the sun theme and the sky use it; it never
// leaves the browser and never places a node.

let table: Map<string, GeoPoint> | null = null

function zones(): Map<string, GeoPoint> {
  table ??= new Map(
    TIME_ZONES.split(";").map((entry) => {
      const [zone = "", at = ""] = entry.split("=")
      const [lat = 0, lng = 0] = at.split(",").map(Number)
      return [zone, { lat, lng }]
    })
  )
  return table
}

/** The place of an IANA time zone; null for one the database places nowhere (`UTC`, `Etc/GMT+3`). */
export const placeOfZone = (zone: string): GeoPoint | null =>
  zones().get(zone) ?? null

/** The place of this browser's time zone; null when it has none. */
export function readerPlace(): GeoPoint | null {
  try {
    return placeOfZone(Intl.DateTimeFormat().resolvedOptions().timeZone)
  } catch {
    return null
  }
}
