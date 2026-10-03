import { Fragment, type ReactNode } from "react"

import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

/**
 * That something holds unsaved edits, in the one draft colour (`draft`):
 * a dot beside a row's title, or the word where there is room for it.
 */
export function DraftMark({ variant = "dot" }: { variant?: "dot" | "text" }) {
  if (variant === "text")
    return (
      <span className="shrink-0 text-meta text-draft">{m.draft_mark()}</span>
    )
  return (
    <span
      role="img"
      aria-label={m.draft_mark()}
      className="size-1.5 shrink-0 rounded-full bg-draft"
    />
  )
}

/**
 * The line under an editor's title: the revision it shows (`r42`), or that
 * it is a draft and which revision it started from; then any other facts.
 * Null `revision` while unread.
 */
export function RevisionLine({
  revision,
  draftBase,
  facts = [],
}: {
  revision: number | null
  /** The revision the draft was loaded at; null without a draft. */
  draftBase: number | null
  facts?: readonly ReactNode[]
}) {
  return (
    <p className="flex h-4 items-center gap-1.5 text-meta text-muted-foreground">
      {revision === null ? (
        <Skeleton className="h-2.5 w-24 rounded-full" />
      ) : (
        <span
          className={cn(
            "font-mono tabular-nums",
            draftBase !== null && "text-draft"
          )}
        >
          {draftBase !== null
            ? m.revision_draft({ revision: draftBase })
            : m.revision({ revision })}
        </span>
      )}
      {facts.map((fact, index) => (
        <Fragment key={index}>
          <span aria-hidden="true">·</span>
          {fact}
        </Fragment>
      ))}
    </p>
  )
}
