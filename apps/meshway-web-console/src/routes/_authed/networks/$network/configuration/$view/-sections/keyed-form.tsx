import { useState, type ReactNode } from "react"
import { toast } from "sonner"

import { FieldInput } from "@/components/chrome/form"
import { NoneLine } from "@/components/configuration/form-kit"
import { Button } from "@/components/ui/button"
import { keyProblem, type KeyProblem } from "@/lib/configuration/keyed"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

const TOAST = "configuration-key"

// A map edited as rows (groups, tag owners, DNS servers and routes): each
// row's key is renamed in place and taken when the field is left or Enter
// is pressed (a key half typed is not yet a key: the map cannot hold two
// rows under one, or one under none); a refused key goes back to what it
// was, and a toast says why. New rows are named first, then filled.

export type KeyRule = {
  /** What the key is written with before the name (`group:`), shown beside the field. */
  prefix: string
  placeholder: string
}

const PROBLEM: Record<KeyProblem, () => string> = {
  empty: () => m.configuration_key_empty(),
  space: () => m.configuration_key_space(),
  taken: () => m.configuration_key_taken(),
}

const keyName = (key: string, prefix: string) =>
  key.startsWith(prefix) ? key.slice(prefix.length) : key

/** The field a key is renamed in. */
export function KeyInput({
  map,
  value,
  rule,
  label,
  editable,
  invalid,
  onRename,
  className,
}: {
  /** The map the key is in. */
  map: Readonly<Record<string, unknown>>
  value: string
  rule: KeyRule
  label: string
  editable: boolean
  invalid?: boolean
  onRename: (key: string) => void
  className?: string
}) {
  const name = keyName(value, rule.prefix)
  const [typed, setTyped] = useState<string | null>(null)
  const commit = () => {
    if (typed === null) return
    setTyped(null)
    const next = typed.trim()
    if (next === name) return
    const problem = keyProblem(map, rule.prefix, next, value)
    if (problem) toast.error(PROBLEM[problem](), { id: TOAST })
    else onRename(rule.prefix + next)
  }
  if (!editable)
    return (
      <span
        className={cn("truncate font-mono text-meta leading-7", className)}
        title={value}
      >
        {value}
      </span>
    )
  return (
    <span className={cn("inline-flex min-w-0 items-center", className)}>
      {rule.prefix ? (
        <span className="shrink-0 pl-2.5 font-mono text-meta text-muted-foreground">
          {rule.prefix}
        </span>
      ) : null}
      <FieldInput
        aria-label={label}
        name="key"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder={rule.placeholder}
        aria-invalid={invalid || undefined}
        value={typed ?? name}
        onChange={(event) => setTyped(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault()
            commit()
          } else if (event.key === "Escape" && typed !== null) {
            event.stopPropagation()
            setTyped(null)
          }
        }}
        className={cn(
          "h-7 min-w-0 flex-1 rounded-full bg-transparent font-mono text-ellipsis hover:bg-fill-hover focus-visible:bg-field md:text-meta dark:bg-transparent dark:hover:bg-fill-hover dark:focus-visible:bg-field",
          rule.prefix ? "pl-0.5" : "px-2.5"
        )}
      />
    </span>
  )
}

/** Names a new row: typed, then added with Enter or the button. */
export function AddKey({
  map,
  rule,
  label,
  addLabel,
  onAdd,
}: {
  map: Readonly<Record<string, unknown>>
  rule: KeyRule
  label: string
  addLabel: string
  onAdd: (key: string) => void
}) {
  const [name, setName] = useState("")
  const [refused, setRefused] = useState(false)
  const add = () => {
    const next = name.trim()
    const problem = keyProblem(map, rule.prefix, next, null)
    if (problem) {
      setRefused(true)
      toast.error(PROBLEM[problem](), { id: TOAST })
      return
    }
    setName("")
    onAdd(rule.prefix + next)
  }
  return (
    <form
      className="flex h-11 items-center gap-2 rounded-surface border border-dashed border-divider pr-1.5 pl-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        add()
      }}
    >
      {rule.prefix ? (
        <span className="shrink-0 pl-2 font-mono text-meta text-muted-foreground">
          {rule.prefix}
        </span>
      ) : null}
      <FieldInput
        aria-label={label}
        name="new-key"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        placeholder={rule.placeholder}
        aria-invalid={refused || undefined}
        value={name}
        onChange={(event) => {
          setName(event.target.value)
          setRefused(false)
        }}
        className={cn(
          "h-8 min-w-0 flex-1 bg-transparent font-mono md:text-meta dark:bg-transparent",
          rule.prefix ? "pl-0.5" : "px-2.5"
        )}
      />
      <Button
        type="submit"
        size="sm"
        variant="secondary"
        className="h-8 min-w-16 shrink-0 rounded-full px-3.5"
        disabled={name.trim() === ""}
      >
        {addLabel}
      </Button>
    </form>
  )
}

/** The rows of a map, or the line saying it has none. */
export function KeyedRows({
  count,
  none,
  label,
  children,
}: {
  count: number
  none: string
  label: string
  children: ReactNode
}) {
  return (
    <>
      {count === 0 ? <NoneLine>{none}</NoneLine> : null}
      <ul aria-label={label} className="flex flex-col gap-2">
        {children}
      </ul>
    </>
  )
}
