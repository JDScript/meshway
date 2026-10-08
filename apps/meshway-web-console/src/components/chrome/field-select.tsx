import { cn } from "@meshway/ui/cn"
import { ChevronDownIcon } from "lucide-react"
import type { ComponentProps } from "react"

import { NativeSelectOption } from "@/components/ui/native-select"

import { FIELD_SIZE, type FieldSize } from "./form"

/**
 * A native select on glass with the text field's fill, corners and ring
 * (`FieldInput`): a real form control (named, submitted with its form), and
 * the platform's own picker on phones.
 */
export function FieldSelect({
  className,
  size = "default",
  children,
  ...props
}: { size?: FieldSize } & Omit<ComponentProps<"select">, "size">) {
  return (
    <div className="relative w-full has-[select:disabled]:opacity-60">
      <select
        className={cn(
          "w-full min-w-0 appearance-none truncate rounded-tile border border-transparent bg-field pr-9 pl-3.5 text-base transition-colors outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20",
          FIELD_SIZE[size].control,
          className
        )}
        {...props}
      >
        {children}
      </select>
      <ChevronDownIcon
        aria-hidden="true"
        className="pointer-events-none absolute top-1/2 right-3.5 size-4 -translate-y-1/2 text-muted-foreground"
      />
    </div>
  )
}

export { NativeSelectOption as FieldOption }
