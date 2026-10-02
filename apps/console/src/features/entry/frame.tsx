import { cn } from "@/lib/utils"
import { useEffect, type ComponentProps, type ReactNode } from "react"

import { Brand } from "@/components/chrome/brand"
import { LanguageSwitch, ThemeSwitch } from "@/components/chrome/preferences"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { showWelcomeGlobe } from "@/features/globe/store"

/**
 * A page outside the network shell: the preferences in the corner and one
 * card beside the globe, placed exactly like the sign-in.
 */
export function EntryPage({
  wide = false,
  holdsSecret = false,
  children,
}: {
  /** Room for lists (the instance setup). */
  wide?: boolean
  /**
   * A secret is held in memory only. Changing the language reloads the
   * page, which would lose it, so the switch is not offered meanwhile.
   */
  holdsSecret?: boolean
  children: ReactNode
}) {
  useEffect(() => {
    showWelcomeGlobe()
  }, [])
  return (
    <main className="pointer-events-none relative flex min-h-svh flex-col">
      <div className="enter glass pointer-events-auto m-4 flex h-11 items-center gap-0.5 self-end rounded-full px-1.5 md:m-5">
        {holdsSecret ? null : <LanguageSwitch />}
        <ThemeSwitch />
      </div>
      <div className="flex flex-1 items-center px-6 pb-16 md:px-[8vw]">
        <div
          className={cn(
            "pointer-events-auto w-full",
            wide ? "max-w-110" : "max-w-sm"
          )}
        >
          {children}
        </div>
      </div>
    </main>
  )
}

/** The sign-in card's shape: brand, title, then the content. */
export function EntryCard({
  title,
  action,
  leaving = false,
  className,
  children,
}: {
  title: string
  /** Beside the brand, e.g. leaving the instance credential. */
  action?: ReactNode
  /** Dissolves as the sign-in card does when it hands over. */
  leaving?: boolean
  className?: string
  children?: ReactNode
}) {
  return (
    <div
      data-leaving={leaving}
      style={{
        "--enter-from": "16px",
        "--enter-scale": "0.985",
        "--enter-delay": "60ms",
      }}
      className={cn(
        "enter glass w-full max-w-95 rounded-sheet p-8 transition-[opacity,transform,filter] duration-500 ease-entrance data-[leaving=true]:-translate-y-3 data-[leaving=true]:scale-[0.98] data-[leaving=true]:opacity-0 data-[leaving=true]:blur-md motion-reduce:transition-none",
        className
      )}
    >
      <div className="flex items-center justify-between gap-3">
        <Brand />
        {/* Kept to the brand's height so the title sits where the sign-in's does. */}
        {action ? (
          <div className="-my-2 flex items-center">{action}</div>
        ) : null}
      </div>
      <h1 className="mt-10 text-display font-semibold tracking-tight">
        {title}
      </h1>
      {children}
    </div>
  )
}

export const ENTRY_BUTTON = "h-11 w-full rounded-tile"

/** A submit button with the sign-in's spinner, which never moves its label. */
export function EntrySubmit({
  pending,
  disabled = false,
  variant,
  children,
  ...props
}: {
  pending: boolean
  disabled?: boolean
  variant?: ComponentProps<typeof Button>["variant"]
  children: ReactNode
} & Pick<ComponentProps<typeof Button>, "name" | "value" | "className">) {
  return (
    <Button
      type="submit"
      size="lg"
      variant={variant}
      disabled={pending || disabled}
      aria-busy={pending}
      {...props}
      className={cn(ENTRY_BUTTON, props.className)}
    >
      <span className="relative inline-flex items-center justify-center">
        <Spinner
          className={
            pending
              ? "absolute -left-6 opacity-100"
              : "absolute -left-6 opacity-0"
          }
        />
        {children}
      </span>
    </Button>
  )
}

/** A quiet line at the card's foot: a fact and a way elsewhere. */
export function EntryFoot({ children }: { children: ReactNode }) {
  return (
    <div className="mt-6 flex min-h-5 items-center justify-between gap-3 text-meta text-muted-foreground">
      {children}
    </div>
  )
}

/** A text action in the card (its foot, or beside the brand). */
export function EntryLink({
  children,
  className,
  ...props
}: Omit<ComponentProps<"button">, "type">) {
  return (
    <button
      type="button"
      className={cn(
        "shrink-0 rounded-sm font-medium text-foreground underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
        className
      )}
      {...props}
    >
      {children}
    </button>
  )
}
