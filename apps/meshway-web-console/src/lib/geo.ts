import type {
  DerivedNodeState,
  GeoObservation,
  LocationAbsentReason,
  LocationSource,
} from "@meshway/control-client"

import type { IpGeo } from "@/api/geojs"
import { m } from "@/paraglide/messages.js"
import { getLocale } from "@/paraglide/runtime.js"

// Where a node's addresses are, in words. The server shows the addresses it
// has seen for a node, public or not, and the one its location rests on
// (ip-info.md); geojs says where each public one is and the network it is in
// (api/geojs.ts). A place is most specific first, its country in the
// reader's language.

export type AddressScope = "public" | "notPublic"

/** No addresses: a node without a State. */
export const NO_ADDRESSES: ReadonlyMap<string, AddressScope> = new Map()

/** The address the server rests a node's location on, as observed. */
export type LocationAddress = Pick<
  GeoObservation,
  "address" | "endpointSource" | "observedAt" | "expiresAt"
>

export type NodeAddresses = {
  location: LocationAddress | null
  /** Why the server rests no location on an address. */
  locationAbsent: LocationAbsentReason | null
  /** Every address the server shows for the node, public or not. */
  scopes: ReadonlyMap<string, AddressScope>
}

/** A node's addresses as its State shows them; the server marks one it does not look up (private, Mesh) not applicable. */
export function nodeAddresses(derived: DerivedNodeState): NodeAddresses {
  const scopes = new Map<string, AddressScope>(
    Object.entries(derived.ipInfo).map(([ip, info]) => [
      ip,
      info.geoip.status === "notApplicable" ? "notPublic" : "public",
    ])
  )
  const observed = derived.location
  if (!observed)
    return {
      location: null,
      locationAbsent: derived.locationAbsent ?? null,
      scopes,
    }
  scopes.set(observed.address, "public")
  return {
    location: {
      address: observed.address,
      endpointSource: observed.endpointSource,
      observedAt: observed.observedAt,
      expiresAt: observed.expiresAt,
    },
    locationAbsent: null,
    scopes,
  }
}

/** Where an address is: geojs's answer, or why there is none. */
export type AddressGeo =
  | { kind: "found"; geo: IpGeo }
  | { kind: "pending" }
  | { kind: "failed" }
  /** This browser does not look up locations. */
  | { kind: "off" }
  /** Never looked up: not a public address. */
  | { kind: "notPublic" }

/** Where an address is, or why that is not known; null for an address the server does not show. */
export type GeoOf = (ip: string) => AddressGeo | null

/** A country or region code in the reader's language (`JP`: 日本, Japan); the code when it names none. */
export function countryName(code: string): string {
  try {
    return (
      new Intl.DisplayNames([getLocale()], { type: "region" }).of(code) ?? code
    )
  } catch {
    return code
  }
}

/** The place geojs names, most specific first ("Shibuya City, Tokyo, 日本"); null when it names none. */
export function placeText(geo: IpGeo): string | null {
  const parts = [
    geo.city,
    geo.region,
    geo.country ? countryName(geo.country) : null,
  ].filter((part): part is string => typeof part === "string" && part !== "")
  return parts.length > 0 ? [...new Set(parts)].join(", ") : null
}

/** Where an address is, or why that is not known. */
export function geoText(geo: AddressGeo): string {
  switch (geo.kind) {
    case "found":
      return placeText(geo.geo) ?? m.nodes_geo_empty()
    case "pending":
      return m.geo_pending()
    case "failed":
      return m.geo_failed()
    case "off":
      return m.geo_off()
    case "notPublic":
      return m.nodes_geo_not_applicable()
  }
}

/** The network an address is in ("AS63949 Akamai Technologies, Inc."), or why that is not known. */
export function asnText(geo: AddressGeo): string {
  switch (geo.kind) {
    case "found": {
      const asn = geo.geo.asn
      if (!asn) return m.nodes_asn_not_found()
      return asn.organization
        ? `AS${asn.number} ${asn.organization}`
        : `AS${asn.number}`
    }
    case "notPublic":
      return m.nodes_asn_not_applicable()
    case "pending":
    case "failed":
    case "off":
      return geoText(geo)
  }
}

/** An address's country and AS number, as short as a list row takes them ("日本 · AS63949"); null when neither is known. */
export function shortGeoText(geo: AddressGeo): string | null {
  if (geo.kind !== "found") return null
  const country = geo.geo.country ? countryName(geo.geo.country) : null
  const asn = geo.geo.asn ? `AS${geo.geo.asn.number}` : null
  const parts = [country, asn].filter((part): part is string => part !== null)
  return parts.length > 0 ? parts.join(" · ") : null
}

/** The IP of `host:port` (`[v6]:port` too), as the server keys a node's addresses. */
export function hostOf(address: string): string {
  const bracketed = /^\[([^\]]+)\]:\d+$/.exec(address)
  if (bracketed?.[1]) return bracketed[1]
  const colon = address.lastIndexOf(":")
  return colon > 0 && address.indexOf(":") === colon
    ? address.slice(0, colon)
    : address
}

/** On what a location rests: a STUN observation or an interface's own address. */
export const LOCATION_SOURCE: Record<LocationSource, () => string> = {
  stun: () => m.location_source_stun(),
  interface: () => m.location_source_interface(),
}
