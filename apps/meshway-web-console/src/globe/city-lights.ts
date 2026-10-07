import { CITY_CORES, CITY_GLOW } from "./cities.gen"
import { landPoints } from "./land"

// The night's lights, from the generated glow (scripts/generate-cities.ts):
// land dots lit by the cities around them, and a bright core at each of the
// largest. Decorative only: they place no node.

export type CityLight = {
  lat: number
  lng: number
  /** 0–1: how bright. */
  glow: number
  /** In globe units, as a land dot's size. */
  size: number
  /** A city's core (whiter, drawn over the land's glow), not a lit land dot. */
  core: boolean
}

/** A land dot lit this little or less stays dark. */
const DARK = 2

export function cityLights(): CityLight[] {
  const lights: CityLight[] = []
  landPoints().forEach(({ lat, lng }, index) => {
    const level = Number.parseInt(CITY_GLOW[index] ?? "0", 16)
    if (!(level > DARK)) return
    const glow = level / 15
    lights.push({ lat, lng, glow, size: 1.5 + 1.6 * glow, core: false })
  })
  for (const entry of CITY_CORES.split(";")) {
    const [lat = 0, lng = 0, size = 1] = entry.split(",").map(Number)
    lights.push({ lat, lng, glow: 1, size: 2.2 + 0.75 * size, core: true })
  }
  return lights
}
