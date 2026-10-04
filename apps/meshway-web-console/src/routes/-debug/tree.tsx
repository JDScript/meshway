import { ChevronRightIcon } from "lucide-react"
import { useState } from "react"

import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

// Decoded data as a tree that opens a level at a time: only what is open
// is drawn, so a whole snapshot stays cheap until someone looks into it.

/** The children a level draws before offering the rest. */
const SHOWN = 100

type Container = readonly unknown[] | Readonly<Record<string, unknown>>

const isContainer = (value: unknown): value is Container =>
  typeof value === "object" && value !== null

const entriesOf = (value: Container): [string, unknown][] =>
  Array.isArray(value)
    ? value.map((item, index) => [String(index), item])
    : Object.entries(value)

function Scalar({ value }: { value: unknown }) {
  if (value === null) return <span className="text-muted-foreground">null</span>
  return (
    <span className="break-all">
      {typeof value === "string" ? JSON.stringify(value) : String(value)}
    </span>
  )
}

function Entry({ name, value }: { name: string; value: unknown }) {
  const [open, setOpen] = useState(false)
  if (!isContainer(value))
    return (
      <div className="flex gap-2 py-0.5 pl-4">
        <span className="shrink-0 text-muted-foreground">{name}</span>
        <Scalar value={value} />
      </div>
    )
  const size = Array.isArray(value) ? value.length : Object.keys(value).length
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 rounded-sm py-0.5 text-left outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronRightIcon
          aria-hidden="true"
          className={cn(
            "size-3 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90"
          )}
        />
        <span className="text-muted-foreground">{name}</span>
        <span className="text-muted-foreground tabular-nums">
          {Array.isArray(value) ? `[${size}]` : `{${size}}`}
        </span>
      </button>
      {open ? <Children value={value} /> : null}
    </div>
  )
}

function Children({ value }: { value: Container }) {
  const [all, setAll] = useState(false)
  const entries = entriesOf(value)
  const shown = all ? entries : entries.slice(0, SHOWN)
  return (
    <div className="ml-1.5 border-l border-divider pl-2">
      {shown.map(([name, child]) => (
        <Entry key={name} name={name} value={child} />
      ))}
      {shown.length < entries.length ? (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="py-0.5 pl-4 text-muted-foreground hover:text-foreground"
        >
          {m.debug_tree_more({ count: entries.length - shown.length })}
        </button>
      ) : null}
    </div>
  )
}

/** `value` as an open tree, its first level shown. */
export function DataTree({ value }: { value: unknown }) {
  return (
    <div className="font-mono text-meta">
      {isContainer(value) ? (
        <Children value={value} />
      ) : (
        <Scalar value={value} />
      )}
    </div>
  )
}
