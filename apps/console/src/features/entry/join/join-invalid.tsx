import { useNavigate } from "@tanstack/react-router"

import { m } from "@/paraglide/messages.js"

import { EntryCard, EntryFoot, EntryLink } from "../frame"

/** No usable join link: none was held, or the controller refused it. */
export function JoinInvalid() {
  const navigate = useNavigate()
  return (
    <EntryCard title={m.join_invalid()}>
      <p className="mt-7 text-row text-muted-foreground">
        {m.problem_invalid_join_link()}
      </p>
      <EntryFoot>
        <span />
        <EntryLink onClick={() => void navigate({ to: "/" })}>
          {m.join_open_console()}
        </EntryLink>
      </EntryFoot>
    </EntryCard>
  )
}
