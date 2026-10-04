import type { GeoIpData, IpInfo, LocationSource } from "@meshway/control-client"

import { locationAbsentLabel } from "@/lib/state-labels"
import { m } from "@/paraglide/messages.js"
import { getLocale } from "@/paraglide/runtime.js"

// What the controller's GeoIP and ASN datasets say of an address
// (ip-info.md), in words: the place (most specific first, the country in
// the reader's language), the network it is in, and on what a location
// rests.

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

/** The place a GeoIP result names, most specific first ("Shibuya City, Tokyo, 日本"); null when it names none. */
export function placeText(data: GeoIpData): string | null {
  const parts = [
    data.city,
    data.region,
    data.country ? countryName(data.country) : null,
  ].filter((part): part is string => typeof part === "string" && part !== "")
  return parts.length > 0 ? [...new Set(parts)].join(", ") : null
}

/** Where GeoIP places an address, or why it does not. */
export function geoText(info: IpInfo): string {
  switch (info.geoip.status) {
    case "available":
      return placeText(info.geoip.data) ?? m.nodes_geo_empty()
    case "notFound":
      return m.nodes_geo_not_found()
    case "unavailable":
      return locationAbsentLabel[info.geoip.reason]()
    case "notApplicable":
      return m.nodes_geo_not_applicable()
  }
}

/** The network an address is in ("AS63949 Akamai Technologies, Inc."), or why it is not known. */
export function asnText(info: IpInfo): string {
  switch (info.asn.status) {
    case "available":
      return info.asn.data.organization
        ? `AS${info.asn.data.number} ${info.asn.data.organization}`
        : `AS${info.asn.data.number}`
    case "notFound":
      return m.nodes_asn_not_found()
    case "unavailable":
      return m.nodes_asn_unavailable()
    case "notApplicable":
      return m.nodes_asn_not_applicable()
  }
}

/** An address's country and AS number, as short as a list row takes them ("日本 · AS63949"); null when neither is known. */
export function shortGeoText(info: IpInfo): string | null {
  const country =
    info.geoip.status === "available" && info.geoip.data.country
      ? countryName(info.geoip.data.country)
      : null
  const asn =
    info.asn.status === "available" ? `AS${info.asn.data.number}` : null
  const parts = [country, asn].filter((part): part is string => part !== null)
  return parts.length > 0 ? parts.join(" · ") : null
}

/** The IP of `host:port` (`[v6]:port` too), as State's IP info is keyed. */
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
