import {
  queryOptions,
  useQueries,
  type UseQueryResult,
} from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { lookUpIp, type IpGeo } from "@/api/geojs"
import type { AddressGeo, AddressScope, GeoOf } from "@/lib/geo"
import {
  GEO_KEPT_MS,
  keepGeo,
  keptGeo,
  useGeoLookupOn,
} from "@/state/geo-lookup"

export const ipGeoKey = ["geojs"] as const

/** What geojs says of a public address: kept for a week in this browser, asked again after. */
const ipGeoQuery = (ip: string, on: boolean) =>
  queryOptions({
    queryKey: [...ipGeoKey, ip],
    queryFn: async () => {
      const geo = await lookUpIp(ip)
      keepGeo(ip, geo)
      return geo
    },
    enabled: on,
    initialData: () => keptGeo(ip)?.geo,
    initialDataUpdatedAt: () => keptGeo(ip)?.at,
    staleTime: GEO_KEPT_MS,
    refetchOnWindowFocus: false,
    retry: 2,
  })

/**
 * Where each of `addresses` is: geojs's answer for each public one (while
 * this browser looks them up), and that the others are not public.
 */
export function useAddressGeo(
  addresses: ReadonlyMap<string, AddressScope>
): GeoOf {
  const on = useGeoLookupOn()
  const asked = useMemo(
    () =>
      [...addresses]
        .filter(([, scope]) => scope === "public")
        .map(([ip]) => ip)
        .sort(),
    [addresses]
  )
  const combine = useCallback(
    (results: UseQueryResult<IpGeo>[]) =>
      new Map(asked.map((ip, index) => [ip, results[index]])),
    [asked]
  )
  const answers = useQueries({
    queries: asked.map((ip) => ipGeoQuery(ip, on)),
    combine,
  })
  return useCallback(
    (ip: string): AddressGeo | null => {
      const scope = addresses.get(ip)
      if (scope === undefined) return null
      if (scope === "notPublic") return { kind: "notPublic" }
      if (!on) return { kind: "off" }
      const answer = answers.get(ip)
      if (answer?.data !== undefined) return { kind: "found", geo: answer.data }
      return answer?.isError ? { kind: "failed" } : { kind: "pending" }
    },
    [addresses, answers, on]
  )
}
