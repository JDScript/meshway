import { cn } from "@/lib/utils"

/** The Meshway mark, matching the documentation site's logo. */
export function BrandMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 128 128"
      fill="none"
      aria-hidden="true"
      className={cn("size-8 shrink-0", className)}
    >
      <rect width="128" height="128" rx="30" fill="#102F38" />
      <path
        d="M27 86V42L64 72L101 42V86"
        stroke="#65D8C9"
        strokeWidth="9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M27 86L64 72L101 86" stroke="#91B3FF" strokeWidth="4" strokeLinecap="round" />
      <circle cx="27" cy="42" r="7" fill="#65D8C9" />
      <circle cx="64" cy="72" r="8" fill="#D3FFF8" />
      <circle cx="101" cy="42" r="7" fill="#65D8C9" />
      <circle cx="27" cy="86" r="6" fill="#91B3FF" />
      <circle cx="101" cy="86" r="6" fill="#91B3FF" />
    </svg>
  )
}
