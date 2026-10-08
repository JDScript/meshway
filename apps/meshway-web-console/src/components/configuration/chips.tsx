import { cn } from "@meshway/ui/cn"
import { CheckIcon, PlusIcon, XIcon, type LucideIcon } from "lucide-react"
import {
  useId,
  useMemo,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react"

import { FieldInput } from "@/components/chrome/form"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { m } from "@/paraglide/messages.js"

// Values as chips: a set (an access rule's sources, a group's members, a
// pool's prefixes) or one value (a topology rule's end). Each chip opens a
// picker to replace it, and a set's × removes one; the picker offers what
// the network has and takes a typed value in the form the field states.
// Chips keep one height, so nothing moves while editing.

export type Shown = {
  icon: LucideIcon | null
  label: string
  /** Shown in a monospace face: an address, a protocol, an ID. */
  mono: boolean
  /** Names something the network has (false: a warning edge). */
  known: boolean
}

export type ChipOption = {
  value: string
  label: string
  detail?: string
  /** The heading it is listed under. */
  group: string
}

const CHIP =
  "inline-flex h-7 max-w-full min-w-0 items-center gap-1.5 rounded-full border border-divider bg-fill-raised/60 px-2.5 text-row leading-none"

/** A chip that opens a picker: pointing at it and its open picker tint it; the keyboard rings it. */
const LIVE =
  "live-chip outline-none has-focus-visible:ring-2 has-focus-visible:ring-ring/50 focus-visible:ring-2 focus-visible:ring-ring/50"

const flagged = (known: boolean, invalid: boolean) =>
  cn(
    !known && "border-warning/50",
    invalid && "border-destructive/60 bg-destructive/8"
  )

function ChipContent({ shown }: { shown: Shown }) {
  const Icon = shown.icon
  return (
    <>
      {Icon ? (
        <Icon
          aria-hidden="true"
          className={cn(
            "size-3 shrink-0",
            shown.known ? "text-muted-foreground" : "text-warning"
          )}
        />
      ) : null}
      <span className={cn("truncate", shown.mono && "font-mono text-meta")}>
        {shown.label}
      </span>
    </>
  )
}

export type ChipKind = {
  options: readonly ChipOption[]
  /** A typed value is taken when it is in the field's form. */
  accepts: (value: string) => boolean
  show: (value: string) => Shown
  /** The typed form, as an example. */
  placeholder: string
}

/**
 * The picker: a field to search the offered values or type one, and the
 * offered values by group. Enter takes the highlighted value, or the typed
 * one when it is in the field's form; a typed value that is not is marked
 * on the field.
 */
function Picker({
  kind,
  chosen,
  label,
  disabled,
  onPick,
  className,
  children,
}: {
  kind: ChipKind
  chosen: readonly string[]
  label: string
  disabled?: boolean
  onPick: (value: string) => void
  className?: string
  children: ReactNode
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [active, setActive] = useState(0)
  /** The highlight was moved with the arrows: Enter takes it over the typed value. */
  const [moved, setMoved] = useState(false)
  const [refused, setRefused] = useState(false)
  const listId = useId()
  const typed = query.trim()
  const matches = useMemo(() => {
    const needle = typed.toLowerCase()
    return kind.options.filter(
      (option) =>
        needle === "" ||
        option.value.toLowerCase().includes(needle) ||
        option.label.toLowerCase().includes(needle) ||
        (option.detail?.toLowerCase().includes(needle) ?? false)
    )
  }, [kind.options, typed])
  const groups = useMemo(() => {
    const out = new Map<string, ChipOption[]>()
    for (const option of matches) {
      const list = out.get(option.group)
      if (list) list.push(option)
      else out.set(option.group, [option])
    }
    return [...out.entries()]
  }, [matches])
  const ordered = groups.flatMap(([, list]) => list)

  const close = () => {
    setOpen(false)
    setQuery("")
    setActive(0)
    setMoved(false)
    setRefused(false)
  }
  const pick = (value: string) => {
    if (!chosen.includes(value)) onPick(value)
    close()
  }
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault()
      if (ordered.length === 0) return
      const step = event.key === "ArrowDown" ? 1 : -1
      setActive((at) => (at + step + ordered.length) % ordered.length)
      setMoved(true)
    } else if (event.key === "Enter") {
      event.preventDefault()
      const highlighted = ordered[active]
      if (typed !== "" && kind.accepts(typed) && !moved) pick(typed)
      else if (highlighted) pick(highlighted.value)
      else setRefused(true)
    }
  }

  return (
    <Popover
      open={open}
      onOpenChange={(next) => (next ? setOpen(true) : close())}
    >
      <PopoverTrigger
        disabled={disabled}
        aria-label={label}
        className={cn(
          "inline-flex min-w-0 items-center disabled:pointer-events-none",
          className
        )}
      >
        {children}
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        collisionPadding={16}
        className="glass w-80 gap-1 rounded-row p-1.5 text-foreground ring-0"
      >
        <FieldInput
          autoFocus
          aria-label={label}
          aria-controls={listId}
          aria-invalid={refused || undefined}
          name="value"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          placeholder={kind.placeholder}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
            setActive(0)
            setMoved(false)
            setRefused(false)
          }}
          onKeyDown={onKeyDown}
          className="h-9 font-mono md:text-meta"
        />
        <ul
          id={listId}
          role="listbox"
          aria-label={label}
          className="flex max-h-72 flex-col overflow-y-auto overscroll-contain"
        >
          {ordered.length === 0 ? (
            <li className="flex h-9 items-center px-2.5 text-meta text-muted-foreground">
              {typed !== "" && kind.accepts(typed)
                ? m.configuration_picker_enter({ value: typed })
                : typed === "" && kind.options.length === 0
                  ? m.configuration_picker_type()
                  : m.configuration_picker_none()}
            </li>
          ) : null}
          {groups.map(([group, list]) => (
            <li key={group} role="presentation">
              <div className="px-2.5 pt-2 pb-1 text-meta text-muted-foreground">
                {group}
              </div>
              <ul role="presentation">
                {list.map((option) => {
                  const taken = chosen.includes(option.value)
                  const index = ordered.indexOf(option)
                  return (
                    <li
                      key={option.value}
                      role="option"
                      aria-selected={index === active}
                      aria-disabled={taken || undefined}
                      onPointerMove={() => setActive(index)}
                      onClick={() => {
                        if (!taken) pick(option.value)
                      }}
                      className={cn(
                        "flex h-9 cursor-pointer items-center gap-2.5 rounded-tile px-2.5 text-row",
                        index === active && "bg-fill-hover",
                        taken && "cursor-default text-muted-foreground"
                      )}
                    >
                      <span className="min-w-0 flex-1 truncate">
                        {option.label}
                      </span>
                      {option.detail ? (
                        <span className="max-w-40 shrink-0 truncate font-mono text-meta text-muted-foreground">
                          {option.detail}
                        </span>
                      ) : null}
                      <CheckIcon
                        aria-hidden="true"
                        className={cn(
                          "size-3.5 shrink-0",
                          taken ? "opacity-100" : "opacity-0"
                        )}
                      />
                    </li>
                  )
                })}
              </ul>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}

/** One value, chosen with the picker. */
export function ChipField({
  value,
  kind,
  label,
  editable,
  invalid = false,
  onChange,
}: {
  value: string
  kind: ChipKind
  label: string
  editable: boolean
  invalid?: boolean
  onChange: (value: string) => void
}) {
  const shown = kind.show(value)
  if (!editable)
    return (
      <span className={cn(CHIP, flagged(shown.known, invalid))} title={value}>
        <ChipContent shown={shown} />
      </span>
    )
  return (
    <Picker
      kind={kind}
      chosen={[value]}
      label={label}
      onPick={onChange}
      className={cn(CHIP, LIVE, flagged(shown.known, invalid))}
    >
      <ChipContent shown={shown} />
    </Picker>
  )
}

/** A set refused as a whole (a note on it or in it), and by index the values a note is on. */
export type SetRefusal = { set: boolean; values: ReadonlySet<number> }

/** A set of values: each chip replaces or removes itself; one more is added at the end. */
export function ChipSetField({
  values,
  kind,
  label,
  addLabel,
  editable,
  invalid,
  onChange,
}: {
  values: readonly string[]
  kind: ChipKind
  label: string
  addLabel: string
  editable: boolean
  /** How the set is refused: as a whole, and which of its values. */
  invalid?: SetRefusal
  onChange: (values: string[]) => void
}) {
  const whole = invalid?.set ?? false
  return (
    <span
      role="group"
      aria-label={label}
      className={cn(
        "inline-flex min-h-7 min-w-0 flex-wrap items-center gap-1 rounded-full",
        whole && values.length === 0 && "ring-1 ring-destructive/60"
      )}
    >
      {values.length === 0 && !editable ? (
        <span className="text-row text-muted-foreground">
          {m.configuration_chips_none()}
        </span>
      ) : null}
      {values.map((value, index) => {
        const shown = kind.show(value)
        // A value a note is on, or (the set refused) one of no kind.
        const bad =
          (invalid?.values.has(index) ?? false) ||
          (whole && !kind.accepts(value))
        if (!editable)
          return (
            <span
              key={`${value}-${index}`}
              title={value}
              className={cn(CHIP, flagged(shown.known, bad))}
            >
              <ChipContent shown={shown} />
            </span>
          )
        return (
          <span
            key={`${value}-${index}`}
            title={value}
            className={cn(CHIP, LIVE, "gap-0 px-0", flagged(shown.known, bad))}
          >
            <Picker
              kind={kind}
              chosen={values}
              label={label}
              onPick={(next) =>
                onChange(values.map((old, at) => (at === index ? next : old)))
              }
              className="h-full min-w-0 gap-1.5 rounded-l-full pr-1 pl-2.5 outline-none"
            >
              <ChipContent shown={shown} />
            </Picker>
            <button
              type="button"
              aria-label={m.configuration_chips_remove({ name: shown.label })}
              onClick={() => onChange(values.filter((_, at) => at !== index))}
              className="mr-1 inline-flex size-5 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors outline-none hover:bg-fill-selected hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <XIcon className="size-3" />
            </button>
          </span>
        )
      })}
      {editable ? (
        <Picker
          kind={kind}
          chosen={values}
          label={addLabel}
          onPick={(value) => onChange([...values, value])}
          className="size-7 justify-center rounded-full border border-dashed border-divider text-muted-foreground transition-colors outline-none hover:bg-fill-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-fill-selected data-popup-open:text-foreground"
        >
          <PlusIcon aria-hidden="true" className="size-3" />
        </Picker>
      ) : null}
    </span>
  )
}
