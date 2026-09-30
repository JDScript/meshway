const relative = new Intl.RelativeTimeFormat("zh-CN", { numeric: "auto" })

const steps: [Intl.RelativeTimeFormatUnit, number][] = [
  ["second", 60],
  ["minute", 60],
  ["hour", 24],
  ["day", 30],
  ["month", 12],
  ["year", Number.POSITIVE_INFINITY],
]

/** Relative distance from `reference`; null when `timestamp` is not parseable. */
export function formatRelative(timestamp: string, reference: number) {
  const parsed = Date.parse(timestamp)
  if (Number.isNaN(parsed)) return null
  let value = (parsed - reference) / 1000
  for (const [unit, size] of steps) {
    if (Math.abs(value) < size) return relative.format(Math.round(value), unit)
    value /= size
  }
  return null
}

export function formatClock(epochMs: number) {
  return new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(epochMs)
}

export function formatDateTime(timestamp: string) {
  const parsed = Date.parse(timestamp)
  if (Number.isNaN(parsed)) return timestamp
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(parsed)
}

export function byLastSeenDesc<T extends { last_seen: string }>(a: T, b: T) {
  return (Date.parse(b.last_seen) || 0) - (Date.parse(a.last_seen) || 0)
}
