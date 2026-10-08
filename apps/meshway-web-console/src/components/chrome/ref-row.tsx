import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { ArrowUpRightIcon } from "lucide-react"
import type { ReactNode } from "react"

// A row inside a workspace that names something with a page of its own (a
// node, a network): a 16px lead mark, the name (and a line under it), a
// detail, its state in a fixed column, and the way there (↗) at its end.
// The fill reaches the section's edges, as a tile's does, and its text
// starts where a tile's does.

const ROW =
  "flex min-h-12 w-full items-center gap-3 rounded-tile py-1.5 pr-2 pl-3.5 text-left"
// On phones the state takes only its width (right edges still line up) and the detail yields to the name.
const STATE =
  "max-w-28 shrink-0 truncate text-right text-meta text-muted-foreground sm:w-28"
const END = "flex size-8 shrink-0 items-center justify-center"

type Content = {
  lead: ReactNode
  title: ReactNode
  /** A line under the name. */
  subtitle?: ReactNode
  /** Beside the state, in mono (an address). */
  detail?: ReactNode
  state?: ReactNode
}

function Body({
  lead,
  title,
  subtitle,
  detail,
  state,
  end,
}: Content & { end: ReactNode }) {
  return (
    <>
      <span className="flex size-4 shrink-0 items-center justify-center">
        {lead}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-row font-medium">{title}</span>
        {subtitle !== undefined ? (
          <span className="block truncate text-meta text-muted-foreground">
            {subtitle}
          </span>
        ) : null}
      </span>
      {detail !== undefined ? (
        <span className="hidden shrink-0 font-mono text-meta text-muted-foreground tabular-nums sm:inline">
          {detail}
        </span>
      ) : null}
      <span className={STATE}>{state}</span>
      {end}
    </>
  )
}

/**
 * One reference row. With `onOpen` alone the whole row goes there; with
 * `onSelect` too the row chooses it here (`current`) and ↗ goes there; with
 * `action` (an `IconAction`, the size of ↗) the end holds that instead.
 */
export function RefRow({
  onOpen,
  openLabel,
  onSelect,
  current = false,
  action,
  ...content
}: Content & {
  onOpen?: () => void
  /** Names ↗ when it is its own control. */
  openLabel?: string
  onSelect?: () => void
  current?: boolean
  action?: ReactNode
}) {
  if (action !== undefined || (!onOpen && !onSelect))
    return (
      <li className={ROW}>
        <Body {...content} end={<span className={END}>{action}</span>} />
      </li>
    )
  if (!onSelect)
    return (
      <li>
        <button type="button" onClick={onOpen} className={cn("list-row", ROW)}>
          <Body
            {...content}
            end={
              <span className={END}>
                <ArrowUpRightIcon className="size-4 text-muted-foreground" />
              </span>
            }
          />
        </button>
      </li>
    )
  return (
    <li className="relative">
      <button
        type="button"
        aria-current={current ? "true" : undefined}
        onClick={onSelect}
        className={cn("list-row", ROW)}
      >
        <Body {...content} end={onOpen ? <span className={END} /> : null} />
      </button>
      {onOpen ? (
        <Button
          variant="ghost"
          size="icon"
          className="absolute top-1/2 right-2 -translate-y-1/2 rounded-full"
          aria-label={openLabel}
          onClick={onOpen}
        >
          <ArrowUpRightIcon />
        </Button>
      ) : null}
    </li>
  )
}

/** The reference rows of a section, 2px apart. */
export function RefList({ children }: { children: ReactNode }) {
  return <ul className="flex flex-col gap-0.5">{children}</ul>
}

/** A heading over the rows' state column (a section's action), aligned with it. */
export function RefStateHeading({
  children,
  withEnd = true,
}: {
  children: ReactNode
  /** The rows carry ↗ (or an action) after the state. */
  withEnd?: boolean
}) {
  return (
    <span className={cn(STATE, withEnd ? "mr-13" : "mr-2")}>{children}</span>
  )
}
