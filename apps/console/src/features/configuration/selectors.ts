// The values the section forms write, read the way the configuration
// reference states them (identity.md#selectors, access.md, dns.md,
// addressing.md). Only their form is checked here; whether a user, group,
// tag or node exists is the controller's to say.

export type SelectorKind =
  "all" | "user" | "group" | "tag" | "node" | "internet" | "domain" | "cidr"

/** Node IDs are UUIDs, as the controller writes them. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

/** Something before and after one "@", no spaces: an email as typed. */
export const isEmail = (value: string) => /^[^\s@]+@[^\s@]+$/.test(value)

/** `group:<name>` or `tag:<name>` with a name. */
export const isNamed = (value: string, prefix: "group:" | "tag:") =>
  value.startsWith(prefix) && value.length > prefix.length && !/\s/.test(value)

const IPV4 =
  /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/

export const isIpv4 = (value: string) => IPV4.test(value)

/** An IPv6 address: the URL parser accepts exactly the address forms. */
export function isIpv6(value: string): boolean {
  if (!value.includes(":") || /[[\]/%\s]/.test(value)) return false
  try {
    return new URL(`http://[${value}]/`).hostname.length > 2
  } catch {
    return false
  }
}

export type Family = "ipv4" | "ipv6"

/** The family of a CIDR prefix (`100.64.0.0/10`, `fd42::/48`); null when it is none. */
export function cidrFamily(value: string): Family | null {
  const slash = value.indexOf("/")
  if (slash < 0 || slash !== value.lastIndexOf("/")) return null
  const address = value.slice(0, slash)
  const length = value.slice(slash + 1)
  if (!/^(0|[1-9]\d*)$/.test(length)) return null
  const bits = Number(length)
  if (isIpv4(address)) return bits <= 32 ? "ipv4" : null
  if (isIpv6(address)) return bits <= 128 ? "ipv6" : null
  return null
}

/** A DNS name or suffix as typed: labels of letters, digits and hyphens, an optional root dot. */
export function isDomain(value: string): boolean {
  const name = value.endsWith(".") ? value.slice(0, -1) : value
  if (name.length === 0 || name.length > 253) return false
  if (isIpv4(name)) return false
  return name
    .split(".")
    .every((label) =>
      /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/.test(label)
    )
}

/**
 * What a selector names. Destinations (`external`) also take the internet,
 * a domain suffix and a CIDR prefix; sources and topology ends do not.
 */
export function selectorKind(
  value: string,
  external: boolean
): SelectorKind | null {
  if (value === "*") return "all"
  if (value.startsWith("group:"))
    return isNamed(value, "group:") ? "group" : null
  if (value.startsWith("tag:")) return isNamed(value, "tag:") ? "tag" : null
  if (value.startsWith("node:"))
    return UUID.test(value.slice(5)) ? "node" : null
  if (external) {
    if (value === "autogroup:internet") return "internet"
    if (value.startsWith("domain:"))
      return isDomain(value.slice(7)) ? "domain" : null
    if (cidrFamily(value) !== null) return "cidr"
  }
  return isEmail(value) ? "user" : null
}

const port = (value: string): number | null =>
  /^[1-9]\d{0,4}$/.test(value) && Number(value) <= 65535 ? Number(value) : null

/**
 * An access protocol entry: `*`, or `tcp:` / `udp:` with `*`, a port or a
 * range, or `icmp:*`.
 */
export function isProtocol(value: string): boolean {
  if (value === "*") return true
  const colon = value.indexOf(":")
  if (colon < 0) return false
  const protocol = value.slice(0, colon)
  const ports = value.slice(colon + 1)
  if (protocol !== "tcp" && protocol !== "udp" && protocol !== "icmp")
    return false
  if (ports === "*") return true
  if (protocol === "icmp") return false
  const dash = ports.indexOf("-")
  if (dash < 0) return port(ports) !== null
  const from = port(ports.slice(0, dash))
  const to = port(ports.slice(dash + 1))
  return from !== null && to !== null && from <= to
}

/** A port number as typed: 1 to 65535. */
export const parsePort = port
