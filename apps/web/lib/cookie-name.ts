/** Cookies are scoped to a hostname, not a port. Give independent consoles on
 * the same host separate server-configured names for all workspace cookies. */
export function cookieNamespace(): string | undefined {
  const namespace = process.env.MESHWAY_WEB_COOKIE_NAMESPACE
  if (!namespace) return undefined
  if (!/^[a-zA-Z0-9_-]{1,64}$/.test(namespace)) {
    throw new Error("Invalid MESHWAY_WEB_COOKIE_NAMESPACE")
  }
  return namespace
}

export function consoleCookieName(name: string): string {
  const namespace = cookieNamespace()
  return namespace ? `${name}_${namespace}` : name
}
