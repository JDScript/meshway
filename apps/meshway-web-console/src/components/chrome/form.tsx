import { cn } from "@meshway/ui/cn"
import type { ComponentProps } from "react"

import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"

// Form pieces on glass. Fields are real inputs (named, labelled, Enter
// submits their form); a field that is not valid is marked in place (its
// ring), never by text that would push the layout: the commit bar or a
// toast says what is wrong.

export const FIELD_SIZE = {
  /** In a workspace or a dialog. */
  default: {
    control: "h-10 md:text-row",
    label: "text-meta text-muted-foreground",
  },
  /** On an entry card (sign-in, invitation, setup). */
  lg: { control: "h-11", label: "" },
} as const

export type FieldSize = keyof typeof FIELD_SIZE

/**
 * The text control on glass: the same fill and corners everywhere. Every
 * field shows a placeholder, an example of the value or its constraint
 * (never a stand-in for the label).
 */
export function FieldInput({
  className,
  size = "default",
  ...props
}: { placeholder: string; size?: FieldSize } & Omit<
  ComponentProps<typeof Input>,
  "placeholder" | "size"
>) {
  return (
    <Input
      className={cn(
        // The field fill on both themes, enabled or not: shadcn's own dark and disabled fills would replace it.
        "rounded-tile border-transparent bg-field px-3.5 disabled:bg-field dark:bg-field dark:disabled:bg-field",
        FIELD_SIZE[size].control,
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
  size?: FieldSize
} & Omit<ComponentProps<typeof Input>, "id" | "placeholder" | "size">) {
  return (
    <Field data-invalid={invalid || undefined}>
      <FieldLabel htmlFor={id} className={FIELD_SIZE[size].label}>
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
        className="data-unchecked:bg-foreground/20"
        checked={checked}
        disabled={disabled}
        onCheckedChange={(next) => onCheckedChange(next)}
      />
    </label>
  )
}
