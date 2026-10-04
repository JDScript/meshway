import { clockTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"
import type { Connection } from "@/state/live"

/** What a network's event stream is doing, and since when: the toolbar's dot and the debug panel. */
export function connectionWords(connection: Connection | null): {
  title: string
  detail: string | null
} {
  if (!connection) return { title: m.live_off(), detail: null }
  const time = clockTime(connection.since)
  switch (connection.phase) {
    case "live":
      return { title: m.live_live(), detail: m.live_live_since({ time }) }
    case "connecting":
      return connection.dropped
        ? {
            title: m.live_reconnecting(),
            detail: m.live_reconnecting_since({ time }),
          }
        : { title: m.live_connecting(), detail: m.live_connecting_detail() }
    case "paused":
      return { title: m.live_paused(), detail: m.live_paused_since({ time }) }
    case "stopped":
      return {
        title: m.live_stopped(),
        detail: m.live_stopped_detail({ time }),
      }
  }
}
