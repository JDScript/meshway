import { OpenConsoleLink } from "@/components/entry/fallback"
import { EntryCard, EntryFoot } from "@/components/entry/frame"
import type { JoinRefusal } from "@/lib/join-refusal"
import { m } from "@/paraglide/messages.js"

const reason: Record<JoinRefusal, () => string> = {
  invalid: () => m.problem_invalid_join_link(),
  used: () => m.problem_join_link_used(),
  revoked: () => m.problem_join_link_revoked(),
  expired: () => m.problem_join_link_expired(),
  issuer_unavailable: () => m.problem_join_link_issuer_unavailable(),
}

/**
 * No usable join link: none was held, or the controller refused it. Only a
 * real link's refusal says why; any other token says it is not valid.
 */
export function JoinInvalid({
  refusal = "invalid",
}: {
  refusal?: JoinRefusal
}) {
  return (
    <EntryCard
      title={refusal === "invalid" ? m.join_invalid() : m.join_unusable()}
    >
      <p className="mt-7 text-row text-muted-foreground">{reason[refusal]()}</p>
      <EntryFoot>
        <span />
        <OpenConsoleLink />
      </EntryFoot>
    </EntryCard>
  )
}
