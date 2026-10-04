import { InfoIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { m } from "@/paraglide/messages.js"

/**
 * The draft holds the other kind of edits (the source's, in a form; the
 * forms', in the source): what to do first, in place of editing here.
 */
export function DraftNotice({
  text,
  primaryLabel,
  onPrimary,
  onDiscard,
  busy = false,
}: {
  text: string
  primaryLabel: string
  onPrimary: () => void
  onDiscard: () => void
  busy?: boolean
}) {
  return (
    <div className="flex min-h-14 flex-wrap items-center gap-x-3 gap-y-2 rounded-row border border-draft/40 bg-draft/8 py-2.5 pr-2.5 pl-3.5">
      <InfoIcon aria-hidden="true" className="size-4 shrink-0 text-draft" />
      <span className="min-w-0 flex-[1_1_14rem] text-row">{text}</span>
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-8 min-w-16 rounded-full px-3.5"
          disabled={busy}
          onClick={onDiscard}
        >
          {m.configuration_commit_discard()}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="secondary"
          className="h-8 min-w-20 rounded-full px-3.5"
          disabled={busy}
          onClick={onPrimary}
        >
          {primaryLabel}
        </Button>
      </span>
    </div>
  )
}
