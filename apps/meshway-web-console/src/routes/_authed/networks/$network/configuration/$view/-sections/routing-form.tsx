import type {
  EgressRouting,
  RoutingConfiguration,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { ListIcon } from "lucide-react"

import { Section } from "@/components/chrome/section"
import {
  Notes,
  type Names,
  type Note,
} from "@/components/configuration/form-kit"
import { pointerSegments } from "@/lib/configuration/jsonc"
import { profileChoices } from "@/lib/configuration/routing"
import { m } from "@/paraglide/messages.js"

import { Hint, type CatalogSearch } from "./route-kit"
import { EgressRules } from "./routing-egress"
import { MeshRoutes } from "./routing-mesh"
import { Assignments, Auto, Profiles } from "./routing-profiles"

/** Traffic routing: Mesh routes, then egress (public rules, profiles, the automatic profile, assignments). */
export function RoutingForm({
  routing,
  editable,
  names,
  notes,
  exits,
  ruleSets,
  catalog,
  onChange,
  onOpenRuleSets,
}: {
  routing: RoutingConfiguration
  editable: boolean
  names: Names
  notes: readonly Note[]
  /** Node IDs enabled as exits in the configuration. */
  exits: ReadonlySet<string>
  /** The draft's rule set keys. */
  ruleSets: readonly string[]
  /** Where destinations search the catalogs; none when this account cannot. */
  catalog: CatalogSearch | null
  onChange: (routing: RoutingConfiguration) => void
  onOpenRuleSets: () => void
}) {
  const egress = routing.egress ?? {}
  const setEgress = (next: EgressRouting) =>
    onChange({ ...routing, egress: next })
  const common = { editable, names, notes, exits, ruleSets, catalog }
  // A rule set's content, checked with the rules that name it.
  const outside = notes
    .filter(
      (note) => !note.path.startsWith("/routing/") && note.path !== "/routing"
    )
    .map((note) => {
      const [section, key] = pointerSegments(note.path) ?? []
      if (section === "ruleSets" && key !== undefined)
        return {
          ...note,
          message: m.configuration_routing_set_note({
            key,
            message: note.message,
          }),
        }
      return note
    })

  return (
    <div className="flex flex-col gap-8">
      {outside.length > 0 ? (
        <div className="flex flex-col gap-2 rounded-tile bg-fill-subtle px-3.5 py-3">
          <Notes notes={outside} />
          {outside.some((note) => note.path.startsWith("/ruleSets/")) ? (
            <Button
              type="button"
              size="sm"
              variant="secondary"
              className="self-start rounded-full"
              onClick={onOpenRuleSets}
            >
              <ListIcon />
              {m.configuration_routing_open_rule_sets()}
            </Button>
          ) : null}
        </div>
      ) : null}
      <MeshRoutes
        rules={routing.mesh ?? []}
        editable={editable}
        names={names}
        notes={notes}
        onChange={(mesh) => onChange({ ...routing, mesh })}
      />
      <Section
        title={m.configuration_routing_public()}
        count={(egress.rules ?? []).length}
      >
        <Hint>{m.configuration_routing_public_hint()}</Hint>
        <EgressRules
          {...common}
          path={["routing", "egress", "rules"]}
          rules={egress.rules ?? []}
          end={m.configuration_routing_public_end()}
          onChange={(rules) => setEgress({ ...egress, rules })}
        />
      </Section>
      <Profiles {...common} routing={routing} onChange={onChange} />
      <Auto
        auto={egress.auto ?? null}
        choices={profileChoices(routing, [...exits], false)}
        editable={editable}
        names={names}
        notes={notes}
        onChange={(auto) => {
          const next = { ...egress }
          if (auto) next.auto = auto
          else delete next.auto
          setEgress(next)
        }}
      />
      <Assignments
        assignments={egress.assignments ?? []}
        choices={profileChoices(routing, [...exits], true)}
        editable={editable}
        names={names}
        notes={notes}
        onChange={(assignments) => setEgress({ ...egress, assignments })}
      />
    </div>
  )
}
