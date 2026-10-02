import { getLocale } from "@/paraglide/runtime.js"

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["day", 86_400],
  ["hour", 3_600],
  ["minute", 60],
  ["second", 1],
]

/** "3 minutes ago" in the current locale; null for an unparsable time. */
export function relativeTime(iso: string, now: number): string | null {
  const at = Date.parse(iso)
  if (Number.isNaN(at)) return null
  const seconds = Math.round((at - now) / 1000)
  const format = new Intl.RelativeTimeFormat(getLocale(), { numeric: "auto" })
  for (const [unit, size] of UNITS) {
    if (Math.abs(seconds) >= size || unit === "second") return format.format(Math.trunc(seconds / size), unit)
  }
  return null
}

export function clockTime(at: number): string {
  return new Intl.DateTimeFormat(getLocale(), { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(at)
}

/** A calendar date and time in the current locale; null for an unparsable time. */
export function dateTime(iso: string): string | null {
  const at = Date.parse(iso)
  if (Number.isNaN(at)) return null
  return new Intl.DateTimeFormat(getLocale(), { dateStyle: "medium", timeStyle: "short" }).format(at)
}
