import { isIpv4, isIpv6 } from "@/lib/ip"

import type { Family } from "./selectors"

// Mesh addresses against the address pools (addressing.md): an address is
// assignable when it is an address of its family inside one of that
// family's pools. Whether it is free, reserved or retiring is the
// controller's to say.

function ipv4Bits(value: string): bigint {
  return value
    .split(".")
    .reduce((bits, part) => (bits << 8n) | BigInt(Number(part)), 0n)
}

function ipv6Bits(value: string): bigint | null {
  let text = value
  // An embedded IPv4 tail (`::ffff:1.2.3.4`) is two groups.
  const tail = text.match(/^(.*:)(\d+\.\d+\.\d+\.\d+)$/)
  if (tail) {
    const [, head = "", v4 = ""] = tail
    if (!isIpv4(v4)) return null
    const bits = ipv4Bits(v4)
    text = `${head}${(bits >> 16n).toString(16)}:${(bits & 0xffffn).toString(16)}`
  }
  const halves = text.split("::")
  if (halves.length > 2) return null
  const groups = (part: string | undefined) =>
    part === undefined || part === "" ? [] : part.split(":")
  const head = groups(halves[0])
  const rest = groups(halves[1])
  const missing = 8 - head.length - rest.length
  if (halves.length === 1 ? missing !== 0 : missing < 1) return null
  const all = [
    ...head,
    ...Array<string>(Math.max(missing, 0)).fill("0"),
    ...rest,
  ]
  let bits = 0n
  for (const group of all) {
    if (!/^[0-9A-Fa-f]{1,4}$/.test(group)) return null
    bits = (bits << 16n) | BigInt(parseInt(group, 16))
  }
  return bits
}

/** The family of an address as typed; null when it is none. */
export const addressFamily = (value: string): Family | null =>
  isIpv4(value) ? "ipv4" : isIpv6(value) ? "ipv6" : null

/** `address` is inside `prefix` (a CIDR of the same family). */
export function inPrefix(address: string, prefix: string): boolean {
  const slash = prefix.indexOf("/")
  if (slash < 0) return false
  const network = prefix.slice(0, slash)
  const length = Number(prefix.slice(slash + 1))
  const family = addressFamily(address)
  if (family === null || addressFamily(network) !== family) return false
  const width = family === "ipv4" ? 32 : 128
  if (!Number.isInteger(length) || length < 0 || length > width) return false
  const read = family === "ipv4" ? ipv4Bits : ipv6Bits
  const a = read(address)
  const n = read(network)
  if (a === null || n === null) return false
  const shift = BigInt(width - length)
  return a >> shift === n >> shift
}

/** `address` is inside one of the pools of its family. */
export const inPools = (address: string, pools: readonly string[]) =>
  pools.some((prefix) => inPrefix(address, prefix))

/** Two spellings of one address are the same address. */
export function sameAddress(a: string, b: string): boolean {
  const family = addressFamily(a)
  if (family === null || addressFamily(b) !== family) return a === b
  return family === "ipv4" ? a === b : ipv6Bits(a) === ipv6Bits(b)
}
