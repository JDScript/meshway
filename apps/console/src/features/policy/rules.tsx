import { cn } from "cn"
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CircleAlertIcon,
  InfoIcon,
  PlusIcon,
  Trash2Icon,
} from "lucide-react"
import { motion, useReducedMotion } from "motion/react"
import type { KeyboardEvent, ReactNode } from "react"

import { FieldInput } from "@/components/chrome/form"
import { Button } from "@/components/ui/button"
import { SPRING_LAYOUT } from "@/lib/ease"
import { m } from "@/paraglide/messages.js"

import type { PlacedDiagnostic } from "./document"
import { withNodeNames, type SelectorNames } from "./selectors"

// The pieces every layer is drawn with. Ordered layers (Mesh routes, egress
// rules) are numbered lists that read top to bottom and end in what happens
// when nothing matched; unordered layers (connections, grants) are a grid
// of cards with no numbers, since their order means nothing.

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
 * One rule: a summary that opens it in place, and when open its fields.
 * Open is marked by its fill; hover only tints a closed rule.
 */
export function RuleItem({
  open,
  onToggle,
  summary,
  aside,
  actions,
  children,
  label,
  onKeyDown,
  className,
}: {
  open: boolean
  onToggle: () => void
  summary: ReactNode
  /** At the summary's end (counts, the action). */
  aside?: ReactNode
  /** Beside the summary button (reorder). */
  actions?: ReactNode
  children: ReactNode
  label: string
  onKeyDown?: (event: KeyboardEvent<HTMLButtonElement>) => void
  className?: string
}) {
  return (
    <div
      data-open={open || undefined}
      className={cn(
        "rounded-row transition-colors duration-200",
        open ? "bg-fill-selected" : "bg-fill-subtle hover:bg-fill-hover",
        className
      )}
    >
      <div className="flex min-w-0 items-start">
        <button
          type="button"
          aria-expanded={open}
          aria-label={label}
          onClick={onToggle}
          onKeyDown={onKeyDown}
          className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-row py-2.5 pr-2 pl-3.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
        >
          <span className="flex min-w-0 flex-1 flex-col gap-1.5">
            {summary}
          </span>
          {aside}
        </button>
        {actions ? (
          <span className="flex shrink-0 items-center gap-0.5 self-center pr-2">
            {actions}
          </span>
        ) : null}
      </div>
      {open ? (
        <div className="swap-in flex flex-col gap-3 border-t border-divider px-3.5 pt-3 pb-3.5">
          {children}
        </div>
      ) : null}
    </div>
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
  return (event: KeyboardEvent<HTMLButtonElement>) => {
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
}: {
  items: readonly { key: string; node: ReactNode }[]
  end: ReactNode
  label: string
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
        <span className="relative flex justify-center pt-4">
          <span
            aria-hidden="true"
            className="size-2 rounded-full border border-dashed border-muted-foreground/60"
          />
        </span>
        <div className="flex min-h-10 items-center rounded-row border border-dashed border-divider px-3.5 text-row text-muted-foreground">
          {end}
        </div>
      </li>
    </ol>
  )
}

/** The number on the rail, joined to the next by a hairline. */
function Rail({ index }: { index: number }) {
  return (
    <span className="relative flex justify-center pt-4">
      <span
        aria-hidden="true"
        className="absolute top-10 -bottom-4 left-1/2 w-px -translate-x-1/2 bg-linear-to-b from-divider to-divider/40"
      />
      <span className="relative flex size-6 items-center justify-center rounded-full border border-divider bg-fill-raised font-mono text-meta font-medium tabular-nums shadow-xs">
        {index + 1}
      </span>
    </span>
  )
}

/** Unordered rules: cards in a grid; an open card takes the whole row. */
export function RuleGrid({
  children,
  label,
}: {
  children: ReactNode
  label: string
}) {
  return (
    <ul
      aria-label={label}
      className="grid grid-cols-1 gap-2 @2xl/workspace:grid-cols-2"
    >
      {children}
    </ul>
  )
}

/** A labelled control inside an open rule. */
export function RuleField({
  label,
  htmlFor,
  children,
}: {
  label: string
  htmlFor?: string
  children: ReactNode
}) {
  return (
    <div className="grid grid-cols-1 items-start gap-1.5 @md/workspace:grid-cols-[7rem_minmax(0,1fr)] @md/workspace:gap-3">
      {htmlFor ? (
        <label
          htmlFor={htmlFor}
          className="pt-1.5 text-meta text-muted-foreground"
        >
          {label}
        </label>
      ) : (
        <span className="pt-1.5 text-meta text-muted-foreground">{label}</span>
      )}
      <div className="flex min-h-7 min-w-0 flex-wrap items-center gap-1.5">
        {children}
      </div>
    </div>
  )
}

/** A rule's ID, edited in place. */
export function RuleIdField({
  id,
  value,
  invalid,
  disabled,
  onChange,
}: {
  id: string
  value: string
  invalid: boolean
  disabled: boolean
  onChange: (value: string) => void
}) {
  return (
    <RuleField label={m.policy_rule_id()} htmlFor={id}>
      <FieldInput
        id={id}
        name={id}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        maxLength={64}
        className="h-8 max-w-72 font-mono"
        value={value}
        aria-invalid={invalid || undefined}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      />
    </RuleField>
  )
}

/** The end of an open rule: remove it. */
export function RuleFooter({
  onRemove,
  children,
}: {
  onRemove?: () => void
  children?: ReactNode
}) {
  return (
    <div className="flex min-h-7 items-center justify-between gap-2">
      <span className="flex min-w-0 flex-wrap items-center gap-1">
        {children}
      </span>
      {onRemove ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="rounded-full text-destructive hover:text-destructive"
          onClick={onRemove}
        >
          <Trash2Icon />
          {m.policy_rule_remove()}
        </Button>
      ) : null}
    </div>
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
      className="flex h-11 w-full items-center justify-center gap-1.5 rounded-row border border-dashed border-divider text-row text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
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
