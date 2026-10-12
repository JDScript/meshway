// Addresses and ports as the controller reads them, for checking what is
// typed before it is sent. The controller checks again either way.

const IPV4 =
  /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/

export const isIpv4 = (value: string) => IPV4.test(value)

/** An IPv6 address without a zone: the URL parser accepts exactly the address forms. */
export function isIpv6(value: string): boolean {
  if (!value.includes(":") || !/^[0-9A-Fa-f:.]+$/.test(value)) return false
  try {
    return new URL(`http://[${value}]/`).hostname !== ""
  } catch {
    return false
  }
}

export const isIp = (value: string) => isIpv4(value) || isIpv6(value)

/** A port number as written: 1 to 65535, no sign, space or leading zero. */
export const parsePort = (value: string): number | null =>
  /^[1-9]\d{0,4}$/.test(value) && Number(value) <= 65535 ? Number(value) : null

function v4Value(value: string): bigint {
  return value.split(".").reduce((sum, part) => sum * 256n + BigInt(part), 0n)
}

function v6Value(value: string): bigint | null {
  let text = value
  // An IPv4 tail (`::ffff:192.0.2.1`) is two groups.
  const tail = /^(.*:)(\d+\.\d+\.\d+\.\d+)$/.exec(text)
  if (tail) {
    const [, head = "", v4 = ""] = tail
    if (!isIpv4(v4)) return null
    const n = v4Value(v4)
    text = `${head}${(n >> 16n).toString(16)}:${(n & 0xffffn).toString(16)}`
  }
  const halves = text.split("::")
  if (halves.length > 2) return null
  const groups = (part: string | undefined) =>
    part === undefined || part === "" ? [] : part.split(":")
  const head = groups(halves[0])
  const rest = groups(halves[1])
  const fill = 8 - head.length - rest.length
  if (halves.length === 1 ? fill !== 0 : fill < 1) return null
  const all = [...head, ...Array<string>(Math.max(fill, 0)).fill("0"), ...rest]
  if (all.some((group) => !/^[0-9a-fA-F]{1,4}$/.test(group))) return null
  return all.reduce((sum, group) => sum * 65536n + BigInt(`0x${group}`), 0n)
}

/**
 * The prefix a range of addresses is, as `first/len`, when it is exactly
 * one (aligned, a power of two long); null when it is not, or the two are
 * not addresses of one family.
 */
export function rangeCidr(first: string, last: string): string | null {
  const v4 = isIpv4(first) && isIpv4(last)
  if (!v4 && !(isIpv6(first) && isIpv6(last))) return null
  const a = v4 ? v4Value(first) : v6Value(first)
  const b = v4 ? v4Value(last) : v6Value(last)
  if (a === null || b === null || b < a) return null
  const size = b - a + 1n
  if ((size & (size - 1n)) !== 0n || a % size !== 0n) return null
  const bits = size.toString(2).length - 1
  return `${first}/${(v4 ? 32 : 128) - bits}`
}
