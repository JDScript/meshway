import { HORIZON } from "./astronomy"

// The sky behind the globe at the reader's sun: how much of the day's light
// there is, and how strong a dawn or dusk glow sits on the horizon.

export type Sky = {
  /** 0 at night (the sun 6° and more below the horizon) to 1 by day (10° up). */
  daylight: number
  /** The twilight glow, strongest with the sun just below the horizon; 0 by full day and night. */
  glow: number
  /** The glow is a dawn's (the sun rising) rather than a dusk's. */
  dawn: boolean
}

const smoothstep = (from: number, to: number, x: number) => {
  const t = Math.min(1, Math.max(0, (x - from) / (to - from)))
  return t * t * (3 - 2 * t)
}

/** The sky for the sun at `altitude` (degrees), rising or setting. */
export function skyOf({
  altitude,
  rising,
}: {
  altitude: number
  rising: boolean
}): Sky {
  // The glow peaks a little below the horizon and is gone 10° either side.
  const glow = Math.max(0, 1 - Math.abs(altitude - (HORIZON - 1.5)) / 10)
  return {
    daylight: smoothstep(-6, 10, altitude),
    glow: glow * glow * (3 - 2 * glow),
    dawn: rising,
  }
}
