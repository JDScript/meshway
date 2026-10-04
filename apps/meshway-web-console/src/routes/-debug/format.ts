import { shortId } from "@/lib/format"
import { getLocale } from "@/paraglide/runtime.js"

/** The time of day to the millisecond: entries a few milliseconds apart stay apart. */
export const traceTime = (at: number) =>
  new Intl.DateTimeFormat(getLocale(), {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
    hourCycle: "h23",
  }).format(at)

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi

/** `text` with each UUID cut to its first eight characters. */
export const shortIds = (text: string) => text.replace(UUID, shortId)

/** A query key on one line: its parts in order, IDs short. */
export const keyLabel = (key: readonly unknown[]) =>
  key
    .map((part) =>
      typeof part === "string" ? shortIds(part) : JSON.stringify(part)
    )
    .join(" / ")

/** A size in bytes, as B, kB or MB. */
export function byteSize(size: number) {
  const [value, unit] =
    size < 1_000
      ? [size, "byte"]
      : size < 1_000_000
        ? [size / 1_000, "kilobyte"]
        : [size / 1_000_000, "megabyte"]
  return new Intl.NumberFormat(getLocale(), {
    style: "unit",
    unit,
    unitDisplay: "short",
    maximumFractionDigits: 1,
  }).format(value)
}

/** Milliseconds, as the locale writes them. */
export const milliseconds = (ms: number) =>
  new Intl.NumberFormat(getLocale(), {
    style: "unit",
    unit: "millisecond",
    unitDisplay: "short",
    maximumFractionDigits: 0,
  }).format(ms)

/** A value on one line, cut at `max` characters. */
export function brief(value: unknown, max = 80) {
  const text = value === undefined ? "—" : JSON.stringify(value)
  return text.length > max ? `${text.slice(0, max - 1)}…` : text
}
