// Generates src/globe/cities.gen.ts: how brightly each land dot glows on the
// globe's night side, and the bright cores of the largest cities, so the
// night looks like the Earth's at night. Decorative only: it places no node.
//
// A place's light is its population times how lit its country is (the
// square root of GDP per head against 20 000 USD, between 0.12 and 1.25:
// lit cities are not only big ones); it spreads around the place over a
// radius growing with the population. A land dot's glow is the light that
// reaches it, on a log scale, 0–15.
//
// Sources (public domain), Natural Earth v5.1.2:
//   https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_10m_populated_places_simple.geojson
//     sha256 fd3fa867a320cbd5c5b6bb5bc550afeec2939fb2cef688e508007282a55ac42f
//   https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_50m_admin_0_countries.geojson
//     sha256 3e458fc036ad0a66411f2c1e6cac49c5d7bfb81cb1123bc513b22511a2b7fdeb
//
// The glow follows src/globe/land.ts's land dots in order: regenerate it
// whenever they are.
//
// Usage: node apps/meshway-web-console/scripts/generate-cities.ts <places.geojson> <countries.geojson>
import { createHash } from "node:crypto"
import { readFileSync, writeFileSync } from "node:fs"

import { landPoints } from "../src/globe/land.ts"

const PLACES_SHA256 =
  "fd3fa867a320cbd5c5b6bb5bc550afeec2939fb2cef688e508007282a55ac42f"
const COUNTRIES_SHA256 =
  "3e458fc036ad0a66411f2c1e6cac49c5d7bfb81cb1123bc513b22511a2b7fdeb"

/** Places smaller than this give no light of their own. */
const SMALLEST = 50_000
/** Places this large also get a bright core. */
const CORE = 1_000_000
/** GDP per head (USD) at which a country is fully lit. */
const LIT_AT = 20_000
/** The light at which a dot is fully bright, and the light that one step is (log scale). */
const FULL = 3e7
const STEP = 2e4

function read(path: string | undefined, sha256: string): unknown {
  if (!path)
    throw new Error(
      "usage: generate-cities.ts <places.geojson> <countries.geojson>"
    )
  const raw = readFileSync(path)
  const digest = createHash("sha256").update(raw).digest("hex")
  if (digest !== sha256)
    throw new Error(`unexpected digest ${digest} for ${path}`)
  return JSON.parse(raw.toString("utf8"))
}

/** The `properties` of each feature of a GeoJSON collection. */
function properties(collection: unknown): Record<string, unknown>[] {
  if (
    typeof collection !== "object" ||
    collection === null ||
    !("features" in collection) ||
    !Array.isArray(collection.features)
  )
    throw new Error("not a feature collection")
  return collection.features.map((feature: unknown) => {
    if (
      typeof feature !== "object" ||
      feature === null ||
      !("properties" in feature) ||
      typeof feature.properties !== "object" ||
      feature.properties === null
    )
      throw new Error("a feature without properties")
    return Object.fromEntries(Object.entries(feature.properties))
  })
}

const number = (value: unknown) => (typeof value === "number" ? value : NaN)
const text = (value: unknown) => (typeof value === "string" ? value : "")

// How lit each country is.
const lit = new Map<string, number>()
for (const country of properties(read(process.argv[3], COUNTRIES_SHA256))) {
  const perHead = (number(country.GDP_MD) * 1e6) / number(country.POP_EST)
  if (!Number.isFinite(perHead) || perHead <= 0) continue
  lit.set(
    text(country.ADM0_A3),
    Math.min(1.25, Math.max(0.12, Math.sqrt(perHead / LIT_AT)))
  )
}

type Place = {
  lat: number
  lng: number
  people: number
  light: number
  reach: number
}
const places: Place[] = []
for (const place of properties(read(process.argv[2], PLACES_SHA256))) {
  const people = number(place.pop_max)
  if (!(people >= SMALLEST)) continue
  const light = people * (lit.get(text(place.adm0_a3)) ?? 0.5)
  places.push({
    lat: number(place.latitude),
    lng: number(place.longitude),
    people,
    light,
    // Degrees: 0.6° for a town of 100 000, 2.7° for a metropolis of 30 million.
    reach: 0.3 + 0.5 * (people / 1e6) ** 0.42,
  })
}

const RAD = Math.PI / 180
/** The angle (degrees) between two points. */
function apart(aLat: number, aLng: number, bLat: number, bLng: number) {
  const cos =
    Math.sin(aLat * RAD) * Math.sin(bLat * RAD) +
    Math.cos(aLat * RAD) * Math.cos(bLat * RAD) * Math.cos((aLng - bLng) * RAD)
  return Math.acos(Math.min(1, Math.max(-1, cos))) / RAD
}

const levels = landPoints().map(({ lat, lng }) => {
  let light = 0
  for (const place of places) {
    if (Math.abs(place.lat - lat) > place.reach * 3) continue
    const d = apart(lat, lng, place.lat, place.lng)
    if (d < place.reach * 3)
      light += place.light * Math.exp(-((d / place.reach) ** 2))
  }
  const bright = Math.log10(1 + light / STEP) / Math.log10(1 + FULL / STEP)
  return Math.round(Math.min(1, Math.max(0, bright)) * 15)
})

const round = (value: number) => Math.round(value * 10) / 10
const cores = places
  .filter((place) => place.people >= CORE)
  .sort((a, b) => b.light - a.light)
  .map((place) => {
    // 1–9 by light, on a log scale from a million to forty million.
    const size = Math.round(
      1 +
        8 *
          Math.min(
            1,
            Math.max(0, Math.log10(place.light / 1e6) / Math.log10(40))
          )
    )
    return `${round(place.lat)},${round(place.lng)},${size}`
  })

writeFileSync(
  new URL("../src/globe/cities.gen.ts", import.meta.url),
  `// Generated by scripts/generate-cities.ts from Natural Earth v5.1.2
// populated places and countries (public domain). Do not edit.
// The night's lights: decorative, they place no node.

/** Per land dot, in landPoints() order: its night glow, 0–15, one hex digit each. */
export const CITY_GLOW =
  ${JSON.stringify(levels.map((level) => level.toString(16)).join(""))}

/** The largest cities' bright cores: \`lat,lng,size\` (size 1–9), separated by \`;\`. */
export const CITY_CORES = ${JSON.stringify(cores.join(";"))}
`
)
const histogram = Array.from(
  { length: 16 },
  (_, level) => levels.filter((value) => value === level).length
)
console.log(
  `${places.length} places, ${cores.length} cores, glow levels`,
  histogram.join(" ")
)
