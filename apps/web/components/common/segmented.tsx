"use client"

import type { ReactNode } from "react"

import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { cn } from "@/lib/utils"

/**
 * Single-choice segmented control over a typed option list. Re-pressing the
 * active item keeps it selected: the value is never empty.
 */
export function Segmented<T extends string>({
  value,
  options,
  onValueChange,
  disabled,
  size = "default",
  className,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
}: {
  value: T
  options: readonly { value: T; label: ReactNode; disabled?: boolean }[]
  onValueChange: (value: T) => void
  disabled?: boolean
  size?: "sm" | "default"
  className?: string
  "aria-label"?: string
  "aria-labelledby"?: string
}) {
  return (
    <ToggleGroup
      variant="default"
      size={size}
      spacing={0.5}
      value={[value]}
      disabled={disabled}
      onValueChange={(next) => {
        const [picked] = next
        const option = options.find((candidate) => candidate.value === picked)
        if (option) onValueChange(option.value)
      }}
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      className={cn(
        "w-full rounded-lg bg-muted p-0.5 sm:w-fit [&>*]:flex-1 sm:[&>*]:flex-none",
        className
      )}
    >
      {options.map((option) => (
        <ToggleGroupItem
          key={option.value}
          value={option.value}
          disabled={option.disabled}
          className="px-2.5 font-normal text-muted-foreground hover:bg-background/60 aria-pressed:bg-background aria-pressed:font-medium aria-pressed:text-foreground aria-pressed:shadow-xs dark:aria-pressed:bg-input/40"
        >
          {option.label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  )
}
