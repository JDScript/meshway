import { z } from "zod"
import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import { zIpGeo, type IpGeo } from "@/api/geojs"

// Looking up where nodes are (api/geojs.ts): whether this browser does it
// (on unless turned off in the account page; localStorage, shared by its
// tabs), and what it found, kept for a week so that a reload asks again only
// for addresses it has not seen. What it found is forgotten when the sign-in
// ends or changes, and when the lookups are turned off.

const MODE_KEY = "meshway.geo-lookup"
const FOUND_KEY = "meshway.geo"
/** An answer is asked again after this long. */
export const GEO_KEPT_MS = 7 * 24 * 60 * 60 * 1000
/** At most this many addresses are kept, the most recent. */
const MOST_KEPT = 500

const zFound = z.object({
  v: z.literal(1),
  /** By address: when geojs answered (Unix ms) and what it said. */
  answers: z.record(
    z.string(),
    z.object({ at: z.number().int().nonnegative(), geo: zIpGeo })
  ),
})
type Found = z.infer<typeof zFound>["answers"]

function storage(): Storage | null {
  try {
    return window.localStorage
  } catch {
    return null
  }
}

const modeStore = createStore<{ on: boolean }>(() => ({
  on: storage()?.getItem(MODE_KEY) !== "off",
}))

/** Whether this browser looks up where nodes are. */
export const useGeoLookupOn = (): boolean =>
  useStore(modeStore, (state) => state.on)

/** Forgets every answer kept in this browser. */
export function forgetGeo() {
  try {
    storage()?.removeItem(FOUND_KEY)
  } catch {
    // Kept answers only.
  }
}

/** Turns the lookups on or off in this browser; off forgets what they found. */
export function setGeoLookup(on: boolean) {
  try {
    if (on) storage()?.removeItem(MODE_KEY)
    else storage()?.setItem(MODE_KEY, "off")
  } catch {
    // A preference only.
  }
  if (!on) forgetGeo()
  modeStore.setState({ on })
}

let readRaw: string | null = null
let readFound: Found = {}

/** The answers kept, as last stored by any tab; read again only when they changed. */
function found(): Found {
  let raw: string | null
  try {
    raw = storage()?.getItem(FOUND_KEY) ?? null
  } catch {
    return {}
  }
  if (raw !== readRaw) {
    readRaw = raw
    try {
      const parsed = raw === null ? null : zFound.safeParse(JSON.parse(raw))
      readFound = parsed?.success ? parsed.data.answers : {}
    } catch {
      readFound = {}
    }
  }
  return readFound
}

/** What geojs said of `ip` within the last week, and when; undefined when nothing is kept. */
export function keptGeo(ip: string): { geo: IpGeo; at: number } | undefined {
  const kept = Object.hasOwn(found(), ip) ? found()[ip] : undefined
  return kept && Date.now() - kept.at < GEO_KEPT_MS ? kept : undefined
}

/** Keeps what geojs said of `ip`, dropping answers past a week and the oldest past the most kept. */
export function keepGeo(ip: string, geo: IpGeo) {
  const now = Date.now()
  const answers = Object.entries({ ...found(), [ip]: { at: now, geo } })
    .filter(([, kept]) => now - kept.at < GEO_KEPT_MS)
    .sort(([, a], [, b]) => b.at - a.at)
    .slice(0, MOST_KEPT)
  try {
    storage()?.setItem(
      FOUND_KEY,
      JSON.stringify({ v: 1, answers: Object.fromEntries(answers) })
    )
  } catch {
    // Kept answers only: the next load asks again.
  }
}

/** Keeps the mode in step with the other tabs. Called once, by `main.tsx`. */
export function watchGeoLookup() {
  window.addEventListener("storage", (event) => {
    if (event.storageArea !== window.localStorage) return
    if (event.key !== MODE_KEY && event.key !== null) return
    modeStore.setState({ on: event.newValue !== "off" })
  })
}
