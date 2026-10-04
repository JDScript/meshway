import { getLocale } from "@/paraglide/runtime.js"

/** A duration in microseconds as milliseconds: one decimal below 10 ms. */
export function millis(us: number): string {
  const ms = us / 1000
  return `${new Intl.NumberFormat(getLocale(), {
    maximumFractionDigits: ms < 10 ? 1 : 0,
  }).format(ms)} ms`
}

export function percent(share: number): string {
  return new Intl.NumberFormat(getLocale(), {
    style: "percent",
    maximumFractionDigits: 1,
  }).format(share)
}

/** A bucket's length ("5 分钟", "1 hour"). */
export function span(seconds: number): string {
  const [unit, size] =
    seconds % 3600 === 0
      ? (["hour", 3600] as const)
      : seconds % 60 === 0
        ? (["minute", 60] as const)
        : (["second", 1] as const)
  return new Intl.NumberFormat(getLocale(), {
    style: "unit",
    unit,
    unitDisplay: "long",
  }).format(seconds / size)
}

const sameDay = (a: number, b: number) =>
  new Date(a).toDateString() === new Date(b).toDateString()

/** A span's two ends, the date said once when both fall on the same day. */
export function rangeEnds(fromMs: number, toMs: number): [string, string] {
  const date = new Intl.DateTimeFormat(getLocale(), {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })
  const time = new Intl.DateTimeFormat(getLocale(), {
    hour: "2-digit",
    minute: "2-digit",
  })
  return [
    date.format(fromMs),
    (sameDay(fromMs, toMs) ? time : date).format(toMs),
  ]
}

export const rangeText = (fromMs: number, toMs: number) =>
  rangeEnds(fromMs, toMs).join(" – ")
