// The controller's clock against the browser's, from the `Date` header of
// its responses: whole seconds, stamped at some moment between sending the
// request and receiving the response. Each response bounds the offset; the
// samples taken together narrow it.

/** One response: its `Date` header and the browser's clock around it, in milliseconds. */
export type ClockSample = { date: number; sent: number; received: number }

/** The controller's clock minus the browser's, in milliseconds: somewhere in [low, high]. */
export type ClockOffset = { low: number; high: number; samples: number }

/** The offset one response allows. */
const boundsOf = ({ date, sent, received }: ClockSample) => ({
  low: date - received,
  high: date + 1_000 - sent,
})

/**
 * What the samples (oldest first) allow together, from the newest back to
 * the first one that disagrees with those after it (a clock that was set
 * meanwhile); null without samples.
 */
export function clockOffset(
  samples: readonly ClockSample[]
): ClockOffset | null {
  let offset: ClockOffset | null = null
  for (let index = samples.length - 1; index >= 0; index--) {
    const sample = samples[index]
    if (!sample) continue
    const { low, high } = boundsOf(sample)
    if (!offset) {
      offset = { low, high, samples: 1 }
      continue
    }
    const narrowed: ClockOffset = {
      low: Math.max(offset.low, low),
      high: Math.min(offset.high, high),
      samples: offset.samples + 1,
    }
    if (narrowed.low > narrowed.high) break
    offset = narrowed
  }
  return offset
}
