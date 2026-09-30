"use client"

import type { ReactNode } from "react"

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

export type Option<T extends string> = {
  value: T
  label: string
  /** Optional secondary text rendered in the list only. */
  hint?: ReactNode
  disabled?: boolean
}

/**
 * Typed single-value Base UI Select over a fixed option list. The value type
 * flows from the options, so callers never parse strings back into unions.
 */
export function OptionSelect<T extends string>({
  id,
  value,
  options,
  onValueChange,
  placeholder,
  disabled,
  invalid,
  size = "default",
  className,
  name,
  "aria-label": ariaLabel,
  "aria-describedby": ariaDescribedBy,
  "data-testid": testId,
}: {
  id?: string
  value: T | null
  options: readonly Option<T>[]
  onValueChange: (value: T) => void
  placeholder?: string
  disabled?: boolean
  invalid?: boolean
  size?: "sm" | "default"
  className?: string
  name?: string
  "aria-label"?: string
  "aria-describedby"?: string
  "data-testid"?: string
}) {
  const items = options.map((option) => ({ value: option.value, label: option.label }))
  return (
    <Select<T>
      value={value}
      items={items}
      name={name}
      disabled={disabled}
      onValueChange={(next) => {
        const option = options.find((candidate) => candidate.value === next)
        if (option) onValueChange(option.value)
      }}
    >
      <SelectTrigger
        id={id}
        size={size}
        className={className}
        aria-label={ariaLabel}
        aria-describedby={ariaDescribedBy}
        aria-invalid={invalid || undefined}
        data-testid={testId}
      >
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent alignItemWithTrigger={false} align="start">
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
            {option.hint ? (
              <span className="text-muted-foreground">{option.hint}</span>
            ) : null}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
