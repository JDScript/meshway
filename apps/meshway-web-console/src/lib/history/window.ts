// A history window the person chose: a fixed span ending when it was
// chosen (or refreshed). The controller binds a cursor to the exact window,
// so a window never rolls on by itself; refreshing reads a new one.

export const WINDOWS = ["1h", "6h", "24h", "7d"] as const
export type WindowChoice = (typeof WINDOWS)[number]

const SPAN_SECONDS: Record<WindowChoice, number> = {
  "1h": 3_600,
  "6h": 6 * 3_600,
  "24h": 24 * 3_600,
  // The controller's longest window.
  "7d": 7 * 86_400,
}

/** Buckets small enough to show change, at most 360 per series. */
const BUCKET_SECONDS: Record<WindowChoice, number> = {
  "1h": 60,
  "6h": 300,
  "24h": 900,
  "7d": 3_600,
}

export type HistoryWindow = {
  choice: WindowChoice
  fromMs: number
  toMs: number
  bucketSeconds: number
}

/** The window ending at `endMs`, to the second. */
export function historyWindow(
  choice: WindowChoice,
  endMs: number
): HistoryWindow {
  const toMs = Math.floor(endMs / 1000) * 1000
  return {
    choice,
    fromMs: toMs - SPAN_SECONDS[choice] * 1000,
    toMs,
    bucketSeconds: BUCKET_SECONDS[choice],
  }
}

export const iso = (ms: number) => new Date(ms).toISOString()
