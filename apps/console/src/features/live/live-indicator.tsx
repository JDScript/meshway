import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { clockTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { useConnection, type Connection } from "./connection"

function words(connection: Connection | null): {
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
    case "stopped":
      return {
        title: m.live_stopped(),
        detail: m.live_stopped_detail({ time }),
      }
  }
}

const dot = (connection: Connection | null) => {
  if (connection?.phase === "live") return "bg-success"
  if (connection?.phase === "connecting")
    return connection.dropped
      ? "bg-warning"
      : "border border-muted-foreground motion-safe:animate-pulse"
  return "border border-muted-foreground"
}

/**
 * The event stream of the open network, as a dot that keeps its place in
 * the toolbar; opening it states what the stream is doing and since when.
 */
export function LiveIndicator({ networkId }: { networkId: string }) {
  const connection = useConnection(networkId)
  const { title, detail } = words(connection)
  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="bar-button"
            aria-label={`${m.live_status()}: ${title}`}
            data-live={connection?.phase ?? "off"}
          >
            <span
              aria-hidden="true"
              className={cn("size-2 rounded-full", dot(connection))}
            />
          </Button>
        }
      />
      <PopoverContent align="end" className="w-64">
        <p className="font-medium">{title}</p>
        {detail ? (
          <p className="text-meta text-muted-foreground">{detail}</p>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}
