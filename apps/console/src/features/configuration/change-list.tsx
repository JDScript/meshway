import type {
  ConfigurationChange,
  ConfigurationView,
} from "@meshway/control-client"

import { EmptyLine, Section } from "@/components/chrome/section"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { groupChanges, valueAt, type Found } from "./changes"
import { changeKindLabel, sectionName } from "./labels"

/** A value as JSON, two spaces deep, scrolling when long. */
export function JsonBlock({
  value,
  className,
}: {
  value: unknown
  className?: string
}) {
  return (
    <pre
      className={cn(
        "max-h-60 min-w-0 overflow-auto rounded-tile bg-fill-subtle px-3 py-2 font-mono text-meta break-all whitespace-pre-wrap",
        className
      )}
    >
      {JSON.stringify(value, null, 2)}
    </pre>
  )
}

function Side({
  label,
  found,
  read,
}: {
  label: string
  found: Found
  /** The view this side comes from has been read. */
  read: boolean
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className="text-meta text-muted-foreground">{label}</span>
      {!read ? (
        <Skeleton className="h-9 w-full rounded-tile" />
      ) : found.found ? (
        <JsonBlock value={found.value} />
      ) : (
        <div className="flex h-9 items-center rounded-tile bg-fill-subtle px-3 text-meta text-muted-foreground">
          {m.configuration_change_absent()}
        </div>
      )}
    </div>
  )
}

/**
 * The controller's changes by section, each with its value before and
 * after, read from two non-secret views by the change's pointer. A change
 * neither view shows (a secret) says so instead of a value.
 */
export function ChangeList({
  changes,
  before,
  after,
  labels,
}: {
  changes: readonly ConfigurationChange[]
  /** undefined while unread. */
  before: ConfigurationView | undefined
  after: ConfigurationView | undefined
  /** What each side is (two revisions); "before" and "after" otherwise. */
  labels?: { before: string; after: string }
}) {
  return (
    <div className="flex flex-col gap-6">
      {groupChanges(changes).map((group) => (
        <Section
          key={group.section}
          title={sectionName(group.section)}
          count={group.changes.length}
        >
          <ul className="flex flex-col gap-4">
            {group.changes.map((change) => {
              const was = before
                ? valueAt(before, change.path)
                : { found: false as const }
              const now = after
                ? valueAt(after, change.path)
                : { found: false as const }
              const hidden =
                before !== undefined &&
                after !== undefined &&
                !was.found &&
                !now.found
              return (
                <li
                  key={`${change.kind}:${change.path}`}
                  className="flex flex-col gap-2"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      className={cn(
                        "shrink-0 rounded-full px-2 py-0.5 text-meta",
                        change.kind === "added" && "bg-success/15 text-success",
                        change.kind === "removed" &&
                          "bg-destructive/15 text-destructive",
                        change.kind === "changed" && "bg-fill-raised"
                      )}
                    >
                      {changeKindLabel[change.kind]()}
                    </span>
                    <code className="min-w-0 truncate font-mono text-meta">
                      {change.path || "/"}
                    </code>
                  </div>
                  {hidden ? (
                    <EmptyLine size="sm">
                      {m.configuration_change_hidden()}
                    </EmptyLine>
                  ) : (
                    <div className="grid gap-2 sm:grid-cols-2">
                      <Side
                        label={
                          labels?.before ?? m.configuration_change_before()
                        }
                        found={was}
                        read={before !== undefined}
                      />
                      <Side
                        label={labels?.after ?? m.configuration_change_after()}
                        found={now}
                        read={after !== undefined}
                      />
                    </div>
                  )}
                </li>
              )
            })}
          </ul>
        </Section>
      ))}
    </div>
  )
}
