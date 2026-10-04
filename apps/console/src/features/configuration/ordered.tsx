import { ArrowDownIcon, ArrowUpIcon } from "lucide-react"
import { motion, useReducedMotion } from "motion/react"
import { useState, type KeyboardEvent, type ReactNode } from "react"

import { Button } from "@/components/ui/button"
import { SPRING_LAYOUT } from "@/lib/ease"
import { m } from "@/paraglide/messages.js"

import { moved } from "./routing"

// Ordered rules (Mesh routes, egress rules, the automatic profile's
// countries): read top to bottom, the first that matches decides. Each is
// numbered on a rail that ends in what happens when none matched; a rule
// moves with its arrows or Alt+↑/↓, and glides to its place.

/**
 * Keys that follow rows as they move, so a moved row keeps its element
 * (and its focus) and glides. Rows have no IDs: a list changed from outside
 * (discarded, read again) just takes keys by position.
 */
export function useRowKeys(count: number) {
  const [rows, setRows] = useState(() => ({
    next: count,
    keys: Array.from({ length: count }, (_, at) => at),
  }))
  let current = rows
  if (rows.keys.length !== count) {
    const keys = rows.keys.slice(0, count)
    let next = rows.next
    while (keys.length < count) keys.push(next++)
    current = { next, keys }
    setRows(current)
  }
  return {
    key: (index: number) => current.keys[index] ?? -1 - index,
    moved(index: number, delta: number) {
      setRows({ ...current, keys: moved(current.keys, index, delta) })
    },
    removed(index: number) {
      setRows({
        ...current,
        keys: current.keys.filter((_, at) => at !== index),
      })
    },
  }
}

/** Up and down for a rule of an ordered list. */
export function MoveButtons({
  index,
  count,
  label,
  onMove,
}: {
  index: number
  count: number
  label: string
  onMove: (delta: -1 | 1) => void
}) {
  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="icon-xs"
        className="rounded-full text-muted-foreground"
        disabled={index === 0}
        aria-label={m.configuration_move_up({ name: label })}
        onClick={() => onMove(-1)}
      >
        <ArrowUpIcon />
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-xs"
        className="rounded-full text-muted-foreground"
        disabled={index === count - 1}
        aria-label={m.configuration_move_down({ name: label })}
        onClick={() => onMove(1)}
      >
        <ArrowDownIcon />
      </Button>
    </>
  )
}

/** Alt+↑ / Alt+↓ inside a rule moves it. */
export function moveKeys(
  index: number,
  count: number,
  onMove: (delta: -1 | 1) => void
) {
  return (event: KeyboardEvent<HTMLElement>) => {
    if (!event.altKey) return
    if (event.key === "ArrowUp" && index > 0) {
      event.preventDefault()
      onMove(-1)
    } else if (event.key === "ArrowDown" && index < count - 1) {
      event.preventDefault()
      onMove(1)
    }
  }
}

/** An ordered list: each rule numbered on a rail, ending in what happens when none matched (`end`). */
export function OrderedRules({
  items,
  end,
  label,
  add,
}: {
  items: readonly { key: number; node: ReactNode }[]
  end: ReactNode
  label: string
  add?: ReactNode
}) {
  const reduce = useReducedMotion()
  return (
    <ol aria-label={label} className="relative flex flex-col gap-2">
      {items.map((item, index) => (
        <motion.li
          key={item.key}
          layout="position"
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="relative grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2"
        >
          <Rail index={index} />
          {item.node}
        </motion.li>
      ))}
      <li className="relative grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2">
        <span className="relative flex justify-center pt-4.5">
          <span
            aria-hidden="true"
            className="size-2 rounded-full border border-dashed border-muted-foreground/60"
          />
        </span>
        <div className="flex min-h-11 flex-wrap items-center gap-x-3 gap-y-2 rounded-surface border border-dashed border-divider px-4 py-2 text-row text-muted-foreground">
          {end}
        </div>
      </li>
      {add ? (
        <li className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2">
          <span aria-hidden="true" />
          {add}
        </li>
      ) : null}
    </ol>
  )
}

/** The number on the rail, joined to the next by a hairline. */
function Rail({ index }: { index: number }) {
  return (
    // Centred on the row's first line: 8 + 28 / 2 = 22, half the 44px row.
    <span className="relative flex justify-center pt-2.5">
      <span
        aria-hidden="true"
        className="absolute top-8.5 -bottom-4.5 left-1/2 w-px -translate-x-1/2 bg-linear-to-b from-divider to-divider/40"
      />
      <span className="relative flex size-6 items-center justify-center rounded-full border border-divider bg-fill-raised font-mono text-meta font-medium tabular-nums shadow-xs">
        {index + 1}
      </span>
    </span>
  )
}
