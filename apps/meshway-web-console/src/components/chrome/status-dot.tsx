import { cn } from "@meshway/ui/cn"

export type DotShape = "live" | "solid" | "hollow"

/** A state dot in a token colour; see `.status-dot` for the shapes. */
export function StatusDot({
  color,
  shape = "solid",
  size = "md",
}: {
  color: string
  shape?: DotShape
  size?: "sm" | "md"
}) {
  return (
    <span
      aria-hidden="true"
      data-shape={shape}
      className={cn("status-dot", size === "sm" ? "size-1.5" : "size-2")}
      style={{ color }}
    />
  )
}
