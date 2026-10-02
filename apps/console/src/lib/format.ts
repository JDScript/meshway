import { getLocale } from "@/paraglide/runtime.js"

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["day", 86_400],
  ["hour", 3_600],
  ["minute", 60],
  ["second", 1],
]

/** A time as the controller sends it (ISO 8601) or as the console keeps it (milliseconds since the epoch). */
type Instant = string | number

const toMs = (at: Instant): number | null => {
  const ms = typeof at === "number" ? at : Date.parse(at)
  return Number.isFinite(ms) ? ms : null
}

/** "3 minutes ago" in the current locale; null for an unparsable time. */
export function relativeTime(at: Instant, now: number): string | null {
  const ms = toMs(at)
  if (ms === null) return null
  const seconds = Math.round((ms - now) / 1000)
  const format = new Intl.RelativeTimeFormat(getLocale(), { numeric: "auto" })
  for (const [unit, size] of UNITS) {
    if (Math.abs(seconds) >= size || unit === "second")
      return format.format(Math.trunc(seconds / size), unit)
  }
  return null
}

/** The time of day in the current locale, to the second unless `seconds` is false. */
export function clockTime(
  at: number,
  { seconds = true }: { seconds?: boolean } = {}
): string {
  return new Intl.DateTimeFormat(getLocale(), {
    hour: "2-digit",
    minute: "2-digit",
    second: seconds ? "2-digit" : undefined,
  }).format(at)
}

/** A calendar date and time in the current locale; null for an unparsable time. */
export function dateTime(at: Instant): string | null {
  const ms = toMs(at)
  if (ms === null) return null
  return new Intl.DateTimeFormat(getLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(ms)
}

/** Unix seconds (the controller's account and invitation times) as a calendar date and time. */
export const unixDateTime = (seconds: number): string | null =>
  dateTime(seconds * 1000)
