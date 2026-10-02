import { cn } from "cn"
import type { ComponentProps, ReactNode } from "react"

import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"

// Form pieces on glass. Fields are real inputs (named, labelled, Enter
// submits their form); a field that is not valid is marked in place (its
// ring), never by text that would push the layout: the commit bar or a
// toast says what is wrong.

/** A titled group of fields in a workspace. */
export function FormSection({
  title,
  children,
  className,
}: {
  title: string
  children: ReactNode
  className?: string
}) {
  return (
    <section className={cn("flex flex-col gap-3", className)}>
      <h3 className="text-row font-medium text-muted-foreground">{title}</h3>
      {children}
    </section>
  )
}

/** The text control on glass: the same fill, height and corners everywhere. */
export function FieldInput({
  className,
  ...props
}: ComponentProps<typeof Input>) {
  return (
    <Input
      className={cn(
        "h-10 rounded-tile border-transparent bg-field px-3.5 md:text-row",
        className
      )}
      {...props}
    />
  )
}

/** A labelled text field. */
export function TextField({
  id,
  label,
  invalid,
  ...props
}: { id: string; label: string; invalid?: boolean } & Omit<
  ComponentProps<typeof Input>,
  "id"
>) {
  return (
    <Field data-invalid={invalid || undefined}>
      <FieldLabel htmlFor={id} className="text-meta text-muted-foreground">
        {label}
      </FieldLabel>
      <FieldInput id={id} aria-invalid={invalid || undefined} {...props} />
    </Field>
  )
}

/** A labelled switch on one row, the size of a tile. */
export function SwitchRow({
  id,
  label,
  checked,
  disabled,
  onCheckedChange,
}: {
  id: string
  label: string
  checked: boolean
  disabled?: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <label
      htmlFor={id}
      className="flex h-12 items-center justify-between gap-3 rounded-tile bg-fill-subtle pr-3.5 pl-3.5 text-row font-medium has-disabled:opacity-60"
    >
      {label}
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={(next) => onCheckedChange(next)}
      />
    </label>
  )
}
