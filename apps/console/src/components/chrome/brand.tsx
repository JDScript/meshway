import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

export function BrandMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      className={cn("size-6", className)}
    >
      <circle
        cx="12"
        cy="12"
        r="10.25"
        fill="none"
        stroke="currentColor"
        strokeOpacity="0.35"
        strokeWidth="1.5"
      />
      <path
        d="M6.5 15.5 12 6.5l5.5 9H6.5Z"
        fill="none"
        stroke="var(--brand)"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <circle cx="12" cy="6.5" r="1.9" fill="currentColor" />
      <circle cx="6.5" cy="15.5" r="1.9" fill="currentColor" />
      <circle cx="17.5" cy="15.5" r="1.9" fill="var(--brand)" />
    </svg>
  )
}

export function Brand({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-2 text-lead font-semibold tracking-tight",
        className
      )}
    >
      <BrandMark />
      {m.app_name()}
    </span>
  )
}
