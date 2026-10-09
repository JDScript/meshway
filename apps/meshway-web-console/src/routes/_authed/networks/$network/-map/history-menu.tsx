import { Button } from "@meshway/ui/button"
import { HistoryIcon } from "lucide-react"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  WINDOWS,
  type HistoryWindow,
  type WindowChoice,
} from "@/lib/history/window"
import { m } from "@/paraglide/messages.js"
import { chooseHistoryLayer } from "@/state/history-layer"

const windowLabel: Record<WindowChoice, () => string> = {
  "1h": () => m.history_window_1h(),
  "6h": () => m.history_window_6h(),
  "24h": () => m.history_window_24h(),
  "7d": () => m.history_window_7d(),
}

const OFF = "off"

/** The history layer's window, chosen in a small menu: off, or the last hour to week. */
export function HistoryMenu({
  networkId,
  window,
}: {
  networkId: string
  window: HistoryWindow | null
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="sm"
            className="-mr-2 gap-1.5 rounded-full text-meta text-muted-foreground"
            aria-label={m.overview_history()}
          >
            <HistoryIcon />
            {window ? windowLabel[window.choice]() : m.overview_history_off()}
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          value={window?.choice ?? OFF}
          onValueChange={(value) =>
            chooseHistoryLayer(
              networkId,
              WINDOWS.find((choice) => choice === value) ?? null
            )
          }
        >
          <DropdownMenuRadioItem value={OFF}>
            {m.overview_history_off()}
          </DropdownMenuRadioItem>
          {WINDOWS.map((choice) => (
            <DropdownMenuRadioItem key={choice} value={choice}>
              {windowLabel[choice]()}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
