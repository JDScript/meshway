import type { Diagnostic, NetworkPolicy } from "@meshway/control-client"
import { HistoryIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"

import { TEMPLATE_POLICY } from "./document"
import { withNodeNames, type SelectorNames } from "./selectors"

/**
 * A network still on the v2 source: what the controller proposes for v3,
 * what it could not convert, and starting the v3 draft from the proposal
 * (or from the template when there is none). Saving the draft switches the
 * network to v3.
 */
export function LegacyNotice({
  proposal,
  diagnostics,
  started,
  canManage,
  names,
  onStart,
}: {
  proposal: NetworkPolicy | null
  diagnostics: readonly Diagnostic[]
  /** The draft already holds a v3 policy. */
  started: boolean
  canManage: boolean
  names: SelectorNames
  onStart: (policy: NetworkPolicy) => void
}) {
  return (
    <section className="flex flex-col gap-3 rounded-row border border-warning/30 bg-warning/6 p-3.5">
      <div className="flex min-w-0 items-center gap-3">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-warning/14 text-warning">
          <HistoryIcon aria-hidden="true" className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-row font-medium">
            {m.policy_legacy_title()}
          </h3>
          <p className="truncate text-meta text-muted-foreground">
            {started
              ? m.policy_legacy_started()
              : proposal
                ? m.policy_legacy_proposal()
                : m.policy_legacy_no_proposal()}
          </p>
        </div>
        {canManage && !started ? (
          <Button
            type="button"
            size="sm"
            className="h-8 shrink-0 rounded-full px-3.5"
            onClick={() => onStart(proposal ?? TEMPLATE_POLICY)}
          >
            {proposal ? m.policy_legacy_adopt() : m.policy_legacy_template()}
          </Button>
        ) : null}
      </div>
      {diagnostics.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          <h4 className="text-meta text-muted-foreground">
            {m.policy_legacy_unconverted({ count: diagnostics.length })}
          </h4>
          <ul className="flex flex-col gap-1">
            {diagnostics.map((diagnostic, index) => (
              <li
                key={`${diagnostic.field}-${index}`}
                className="flex min-w-0 flex-col gap-0.5 rounded-tile bg-fill-subtle px-3 py-2 @lg/workspace:flex-row @lg/workspace:gap-3"
              >
                <code className="shrink-0 font-mono text-meta text-muted-foreground @lg/workspace:w-48 @lg/workspace:truncate">
                  {diagnostic.field}
                </code>
                <span className="min-w-0 text-meta break-words">
                  {withNodeNames(diagnostic.message, names)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  )
}
