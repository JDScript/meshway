import { cn } from "@/lib/utils"

const SIZE = {
  /** In a list row's lead slot. */
  sm: "size-4 text-meta",
  /** The account button in the top bar. */
  md: "size-8 text-meta",
} as const

const TONE = {
  raised: "bg-fill-raised text-foreground ring-1 ring-divider",
  inverse: "bg-foreground text-background",
} as const

/** An account's mark: the initial of its email. */
export function InitialAvatar({
  email,
  size = "sm",
  tone = "raised",
}: {
  email: string | null
  size?: keyof typeof SIZE
  tone?: keyof typeof TONE
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex shrink-0 items-center justify-center rounded-full font-semibold uppercase",
        SIZE[size],
        TONE[tone]
      )}
    >
      {email?.[0] ?? "·"}
    </span>
  )
}
