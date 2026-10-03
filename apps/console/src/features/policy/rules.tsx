import { cn } from "@/lib/utils"
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CircleAlertIcon,
  InfoIcon,
  PlusIcon,
  Trash2Icon,
} from "lucide-react"
import { motion, useReducedMotion } from "motion/react"
import { useEffect, useRef, type KeyboardEvent, type ReactNode } from "react"

import { FieldInput } from "@/components/chrome/form"
import { Button } from "@/components/ui/button"
import { SPRING_LAYOUT } from "@/lib/ease"
import { m } from "@/paraglide/messages.js"

import type { PlacedDiagnostic } from "./document"
import { takeRuleFocus } from "./navigation"
import { withNodeNames, type SelectorNames } from "./selectors"

// The pieces every layer is drawn with. A rule is one row that is its own
// editor: its selectors open pickers, its action toggles in place, its ID
// is edited inline and it is removed at its end; nothing expands, so
// nothing moves. Ordered layers (Mesh routes, egress rules) number their
// rows on a rail that reads top to bottom and ends in what happens when
// nothing matched; unordered layers (connections, grants) are a plain list.

/** A diagnostic that reports a rule shadowed by earlier ones (inventory-dependent), not an error. */
export const isLint = (diagnostic: PlacedDiagnostic) =>
  /^mesh\.[^[]/.test(diagnostic.field) && diagnostic.placed.kind === "field"

/** The controller's notes on one rule. */
export function RuleNotes({
  notes,
  names,
  className,
}: {
  notes: readonly PlacedDiagnostic[]
  names: SelectorNames
  className?: string
}) {
  if (notes.length === 0) return null
  return (
    <ul className={cn("flex flex-col gap-1", className)}>
      {notes.map((note, index) => {
        const lint = isLint(note)
        const Icon = lint ? InfoIcon : CircleAlertIcon
        return (
          <li
            key={`${note.field}-${index}`}
            className="flex items-start gap-2 text-meta"
          >
            <Icon
              aria-hidden="true"
              className={cn(
                "mt-0.5 size-3 shrink-0",
                lint ? "text-warning" : "text-destructive"
              )}
            />
            <span className="min-w-0 break-words text-muted-foreground">
              {withNodeNames(note.message, names)}
            </span>
          </li>
        )
      })}
    </ul>
  )
}

/** Counts for a row: problems and lints, as small marks that keep their slot. */
export function NoteCounts({ notes }: { notes: readonly PlacedDiagnostic[] }) {
  const lints = notes.filter(isLint).length
  const errors = notes.length - lints
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-meta tabular-nums">
      {errors > 0 ? (
        <span
          className="inline-flex items-center gap-1 text-destructive"
          aria-label={m.policy_count_problems({ count: errors })}
        >
          <CircleAlertIcon aria-hidden="true" className="size-3" />
          {errors}
        </span>
      ) : null}
      {lints > 0 ? (
        <span
          className="inline-flex items-center gap-1 text-warning"
          aria-label={m.policy_count_lints({ count: lints })}
        >
          <InfoIcon aria-hidden="true" className="size-3" />
          {lints}
        </span>
      ) : null}
    </span>
  )
}

/**
 * One rule's row. A pill while it fits one line (concentric with its 28px
 * chips, 8px in), a rounded block when it wraps. `current` is the rule the
 * address points at, marked by its fill; hover only tints the others.
 */
export function RuleRow({
  current,
  added = false,
  label,
  children,
  end,
  notes,
  names,
  onKeyDown,
}: {
  current: boolean
  /** Just added: it comes into view, as its first picker opens. */
  added?: boolean
  /** Names the row (the rule's ID). */
  label: string
  /** The rule itself: its selectors and what joins them. */
  children: ReactNode
  /** At the row's end: the action, the ID, moving, removing. */
  end: ReactNode
  notes: readonly PlacedDiagnostic[]
  names: SelectorNames
  onKeyDown?: (event: KeyboardEvent<HTMLDivElement>) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  // The rule a link points at, or one just added, comes into view; a jump to it also focuses it, once.
  useEffect(() => {
    if (current || added) ref.current?.scrollIntoView({ block: "nearest" })
    if (current && takeRuleFocus(label))
      ref.current?.focus({ preventScroll: true })
  }, [current, added, label])
  return (
    <div
      ref={ref}
      tabIndex={-1}
      role="group"
      aria-label={label}
      aria-current={current ? "true" : undefined}
      onKeyDown={onKeyDown}
      className="rule-row group/rule flex scroll-my-4 flex-col gap-1.5 rounded-surface p-2 outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <div className="flex min-h-7 flex-wrap items-center gap-x-3 gap-y-2">
        {/* The rule keeps at least this much of the line; short of it, the end moves to its own line. */}
        <div className="flex min-w-0 flex-[1_1_18rem] flex-wrap items-center gap-1.5">
          {children}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-1">{end}</div>
      </div>
      <RuleNotes notes={notes} names={names} className="px-1.5 pb-0.5" />
    </div>
  )
}

/** Unordered rules: a plain list, no numbers, since their order means nothing. */
export function RuleList({
  children,
  label,
}: {
  children: ReactNode
  label: string
}) {
  return (
    <ul aria-label={label} className="flex flex-col gap-2">
      {children}
    </ul>
  )
}

const ID_SLOT = "w-36 shrink-0 font-mono text-meta text-muted-foreground"

/** A rule's ID at its row's end: quiet text until pointed at, edited in place. */
export function RuleIdInput({
  value,
  invalid,
  editable,
  onChange,
}: {
  value: string
  invalid: boolean
  editable: boolean
  onChange: (value: string) => void
}) {
  // One fixed slot, so the controls before it line up from row to row; a longer ID ends in an ellipsis until edited.
  if (!editable)
    return (
      <span className={cn(ID_SLOT, "truncate px-2.5 leading-7")}>{value}</span>
    )
  return (
    <FieldInput
      aria-label={m.policy_rule_id()}
      name="rule-id"
      placeholder={m.policy_rule_id_placeholder()}
      autoComplete="off"
      autoCapitalize="none"
      spellCheck={false}
      maxLength={64}
      className={cn(
        ID_SLOT,
        "h-7 rounded-full bg-transparent px-2.5 text-ellipsis hover:bg-fill-hover focus-visible:bg-field focus-visible:text-foreground md:text-meta dark:bg-transparent dark:hover:bg-fill-hover dark:focus-visible:bg-field"
      )}
      value={value}
      aria-invalid={invalid || undefined}
      onChange={(event) => onChange(event.target.value)}
    />
  )
}

/** Removes a rule; quiet until its row is pointed at (always shown on touch). */
export function RemoveRule({
  label,
  onRemove,
}: {
  label: string
  onRemove: () => void
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      aria-label={m.policy_rule_remove_named({ name: label })}
      onClick={onRemove}
      className="rounded-full text-muted-foreground opacity-0 transition-opacity group-focus-within/rule:opacity-100 group-hover/rule:opacity-100 hover:text-destructive focus-visible:opacity-100 pointer-coarse:opacity-100"
    >
      <Trash2Icon />
    </Button>
  )
}

/** Up and down for a rule of an ordered list (also Alt+↑/↓ on the rule). */
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
        aria-label={m.policy_move_up({ name: label })}
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
        aria-label={m.policy_move_down({ name: label })}
        onClick={() => onMove(1)}
      >
        <ArrowDownIcon />
      </Button>
    </>
  )
}

/** Alt+↑ / Alt+↓ on a focused rule moves it; focus stays on it. */
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

/**
 * An ordered list read top to bottom: each rule numbered on a rail, ending
 * in what happens when none matched (`end`).
 */
export function OrderedRules({
  items,
  end,
  label,
  add,
}: {
  items: readonly { key: string; node: ReactNode }[]
  end: ReactNode
  label: string
  /** Adding a rule, in the rules' column. */
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
          className="relative grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2.5"
        >
          <Rail index={index} />
          {item.node}
        </motion.li>
      ))}
      <li className="relative grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2.5">
        <span className="relative flex justify-center pt-4.5">
          <span
            aria-hidden="true"
            className="size-2 rounded-full border border-dashed border-muted-foreground/60"
          />
        </span>
        <div className="flex min-h-11 items-center rounded-surface border border-dashed border-divider px-4 text-row text-muted-foreground">
          {end}
        </div>
      </li>
      {add ? (
        <li className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2.5">
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
    // Centred on the row's first line: 10 + 24 / 2 = 22, half the 44px row.
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

/** Adds a rule to a layer. */
export function AddRule({
  label,
  onAdd,
}: {
  label: string
  onAdd: () => void
}) {
  return (
    <button
      type="button"
      onClick={onAdd}
      className="flex h-11 w-full items-center justify-center gap-1.5 rounded-surface border border-dashed border-divider text-row text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
    >
      <PlusIcon className="size-3.5" />
      {label}
    </button>
  )
}

/** A layer's count and facts, above its rules. */
export function LayerFacts({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-6 flex-wrap items-center gap-x-3 gap-y-1 text-meta text-muted-foreground">
      {children}
    </div>
  )
}

/** One fact in `LayerFacts`. */
export function Fact({
  icon,
  children,
}: {
  icon?: ReactNode
  children: ReactNode
}) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-divider px-2 py-0.5">
      {icon}
      {children}
    </span>
  )
}
