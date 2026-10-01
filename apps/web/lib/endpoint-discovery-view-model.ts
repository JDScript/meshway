import type { ControllerResult } from "./controller.ts"

export type DiscoveryClock = { time: number; monotonicTime: number | null }

/** Keep leases ageing even when the browser wall clock moves backwards. */
export function advanceDiscoveryClock(previous: DiscoveryClock, wallTime: number, monotonicTime: number): DiscoveryClock {
  const elapsed = previous.monotonicTime === null ? 0 : Math.max(0, monotonicTime - previous.monotonicTime)
  return {
    time: Math.max(previous.time + elapsed, wallTime),
    monotonicTime: Math.max(previous.monotonicTime ?? monotonicTime, monotonicTime),
  }
}

export type DiscoveryRead<T> = { initial: ControllerResult<T>; result: ControllerResult<T> }

/** Prop identity and request sequencing establish ordering, never server time. */
export function shownDiscoveryRead<T>(initial: ControllerResult<T>, read: DiscoveryRead<T>): ControllerResult<T> {
  return read.initial === initial ? read.result : initial
}
