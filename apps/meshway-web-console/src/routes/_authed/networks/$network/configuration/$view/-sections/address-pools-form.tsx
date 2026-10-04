import type { AddressPools } from "@meshway/control-client"

import { Section } from "@/components/chrome/section"
import { ChipSetField } from "@/components/configuration/chips"
import {
  Notes,
  notesWithin,
  prefixChips,
  refusedAt,
  type Note,
} from "@/components/configuration/form-kit"
import { within } from "@/lib/configuration/form-problems"
import { pointerOf } from "@/lib/configuration/jsonc"
import { m } from "@/paraglide/messages.js"

/** The prefixes Mesh addresses are assigned from, per family; an empty family has no pool. */
export function AddressPoolsForm({
  pools,
  editable,
  notes,
  onChange,
}: {
  pools: AddressPools
  editable: boolean
  notes: readonly Note[]
  onChange: (pools: AddressPools) => void
}) {
  return (
    <div className="flex flex-col gap-6">
      {(["ipv4", "ipv6"] as const).map((family) => {
        const at = pointerOf(["addressPools", family])
        const label =
          family === "ipv4"
            ? m.configuration_pool_ipv4()
            : m.configuration_pool_ipv6()
        return (
          <Section key={family} title={label} count={pools[family].length}>
            <ChipSetField
              values={pools[family]}
              kind={prefixChips(family)}
              label={label}
              addLabel={m.configuration_pool_add({ family: label })}
              editable={editable}
              invalid={refusedAt(notes, at)}
              onChange={(next) => onChange({ ...pools, [family]: next })}
            />
            <Notes notes={notesWithin(notes, at)} />
          </Section>
        )
      })}
      {/* Elsewhere: the pools as a whole, and node addresses outside them. */}
      <Notes
        notes={notes.filter(
          (note) =>
            !within(note.path, "/addressPools/ipv4") &&
            !within(note.path, "/addressPools/ipv6")
        )}
      />
    </div>
  )
}
