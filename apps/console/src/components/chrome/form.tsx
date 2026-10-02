import { cn } from "@/lib/utils"
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

const SIZE = {
  /** In a workspace or a dialog. */
  default: {
    control: "h-10 md:text-row",
    label: "text-meta text-muted-foreground",
  },
  /** On an entry card (sign-in, invitation, setup). */
  lg: { control: "h-11", label: "" },
} as const

/**
 * The text control on glass: the same fill and corners everywhere. Every
 * field shows a placeholder, an example of the value or its constraint
 * (never a stand-in for the label).
 */
export function FieldInput({
  className,
  size = "default",
  ...props
}: { placeholder: string; size?: keyof typeof SIZE } & Omit<
  ComponentProps<typeof Input>,
  "placeholder" | "size"
>) {
  return (
    <Input
      className={cn(
        "rounded-tile border-transparent bg-field px-3.5",
        SIZE[size].control,
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
  size = "default",
  ...props
}: {
  id: string
  label: string
  invalid?: boolean
  placeholder: string
  size?: keyof typeof SIZE
} & Omit<ComponentProps<typeof Input>, "id" | "placeholder" | "size">) {
  return (
    <Field data-invalid={invalid || undefined}>
      <FieldLabel htmlFor={id} className={SIZE[size].label}>
        {label}
      </FieldLabel>
      <FieldInput
        id={id}
        size={size}
        aria-invalid={invalid || undefined}
        {...props}
      />
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
