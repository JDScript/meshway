import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import type { LucideIcon } from "lucide-react"

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

/** An action as a round icon, named by its tooltip (and for assistive tech by the same label). */
export function IconAction({
  label,
  icon: Icon,
  onClick,
  disabled,
  className,
}: {
  label: string
  icon: LucideIcon
  onClick: () => void
  disabled?: boolean
  className?: string
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className={cn("rounded-full", className)}
            aria-label={label}
            disabled={disabled}
            onClick={onClick}
          >
            <Icon />
          </Button>
        }
      />
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
