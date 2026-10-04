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
