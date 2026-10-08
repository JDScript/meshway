import { Button } from "@meshway/ui/button"
import { XIcon } from "lucide-react"
import type { ComponentProps } from "react"

import { m } from "@/paraglide/messages.js"

/** The close control of a surface's header. Usable as a Base UI `render` target. */
export function CloseButton(props: ComponentProps<typeof Button>) {
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label={m.close()}
      className="-mr-1.5 rounded-full"
      {...props}
    >
      <XIcon />
    </Button>
  )
}
