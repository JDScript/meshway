import { useQueryClient } from "@tanstack/react-query"

import { ipGeoKey } from "@/api/queries/ip-geo"
import { SwitchRow } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { m } from "@/paraglide/messages.js"
import { setGeoLookup, useGeoLookupOn } from "@/state/geo-lookup"

/** Whether this browser asks geojs.io where nodes are; off forgets what it found. */
export function GeoLookup() {
  const on = useGeoLookupOn()
  const queryClient = useQueryClient()
  return (
    <Section title={m.account_geo()}>
      <SwitchRow
        id="geo-lookup"
        label={m.account_geo_lookup()}
        checked={on}
        onCheckedChange={(next) => {
          setGeoLookup(next)
          if (!next) queryClient.removeQueries({ queryKey: ipGeoKey })
        }}
      />
      <p className="px-3.5 text-meta text-muted-foreground">
        {m.account_geo_lookup_note()}
      </p>
    </Section>
  )
}
