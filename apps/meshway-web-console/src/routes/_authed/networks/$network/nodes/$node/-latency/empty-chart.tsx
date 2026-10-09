import { ActivityIcon } from "lucide-react"
import type { ReactNode } from "react"

/**
 * A chart with nothing measured: the plot's frame drawn faint with a flat
 * dashed line where values would be, what is missing over it, and what
 * would fill it.
 */
export function EmptyChart({
  title,
  children,
  action,
}: {
  title: string
  children?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="relative flex h-44 items-center justify-center overflow-hidden rounded-tile border border-divider">
      <svg
        aria-hidden="true"
        className="absolute inset-0 size-full text-divider"
        preserveAspectRatio="none"
        viewBox="0 0 600 176"
      >
        {[36, 72, 108, 144].map((y) => (
          <line
            key={y}
            x1="0"
            x2="600"
            y1={y}
            y2={y}
            stroke="currentColor"
            strokeWidth="1"
          />
        ))}
        <path
          d="M 0 118 C 60 112, 90 96, 150 104 S 250 124, 300 108 S 420 84, 470 98 S 560 114, 600 102"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeDasharray="5 6"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <div className="relative flex flex-col items-center gap-1.5 rounded-tile bg-background/80 px-5 py-3 text-center backdrop-blur-sm">
        <span className="mb-1 flex size-9 items-center justify-center rounded-full bg-fill-subtle text-muted-foreground">
          <ActivityIcon className="size-4.5" strokeWidth={1.75} />
        </span>
        <p className="text-row font-medium">{title}</p>
        {children ? (
          <p className="max-w-xs text-meta text-muted-foreground">{children}</p>
        ) : null}
        {action ? <div className="mt-1.5">{action}</div> : null}
      </div>
    </div>
  )
}
