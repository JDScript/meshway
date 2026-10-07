import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import { isDay, nextTurn, sunAt, type GeoPoint } from "@/lib/sky/astronomy"
import { readerPlace } from "@/lib/sky/place"
import { skyOf } from "@/lib/sky/sky"

// The reader's sun, kept current: whether it is day where they roughly are
// (their time zone's city), and the sky that goes with it. The sun theme
// follows `day`; the backdrop reads the sky from `<html>`'s
// --sky-daylight, --sky-glow and --sky-rising. Without a place (a zone placed nowhere) there is neither.

type SkyState = {
  place: GeoPoint | null
  /** Whether the sun is up at the place; null without one. */
  day: boolean | null
}

const skyStore = createStore<SkyState>(() => {
  const place = readerPlace()
  return { place, day: place ? isDay(Date.now(), place) : null }
})

export const skyState = () => skyStore.getState()

export const subscribeSky = (listener: (state: SkyState) => void) =>
  skyStore.subscribe(listener)

/** Whether the reader has a place for the sun theme to follow. */
export const useSkyPlaced = (): boolean =>
  useStore(skyStore, (state) => state.place !== null)

/** How often the sky's colours move on; sunrise and sunset are also timed to the minute. */
const TICK_MS = 60_000

/** Keeps the sun and the sky current; called once, by `main.tsx`. */
export function watchSky() {
  const root = document.documentElement
  let timer: number | undefined
  const update = () => {
    window.clearTimeout(timer)
    const { place } = skyStore.getState()
    const now = Date.now()
    if (!place) {
      root.style.removeProperty("--sky-daylight")
      root.style.removeProperty("--sky-glow")
      root.style.removeProperty("--sky-rising")
      return
    }
    const sky = skyOf(sunAt(now, place))
    root.style.setProperty("--sky-daylight", sky.daylight.toFixed(3))
    root.style.setProperty("--sky-glow", sky.glow.toFixed(3))
    root.style.setProperty("--sky-rising", sky.dawn ? "1" : "0")
    const day = isDay(now, place)
    if (day !== skyStore.getState().day) skyStore.setState({ day })
    const turn = nextTurn(now, place)
    timer = window.setTimeout(
      update,
      Math.max(1_000, Math.min(TICK_MS, (turn ?? Infinity) - now))
    )
  }
  update()
  // Timers sleep with a hidden tab: catch up when it is seen again.
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") update()
  })
}
