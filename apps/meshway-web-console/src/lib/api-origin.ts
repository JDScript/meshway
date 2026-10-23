// Where the console's API calls (JSON and the event stream) go. Served by
// the Server or a reverse proxy in front of it, the console is same-origin
// with the API. Hosted elsewhere (Cloudflare Pages), the build names the
// Server's origin in `VITE_MESHWAY_API_ORIGIN`, and that Server lists this
// console's origin in `--console-origin` so it answers cross-origin.

export class InvalidApiOrigin extends Error {
  constructor(value: string) {
    super(
      `VITE_MESHWAY_API_ORIGIN must be an https origin (http on loopback only) without path, query or credentials: ${JSON.stringify(value)}`
    )
    this.name = "InvalidApiOrigin"
  }
}

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]"])

/**
 * The configured API origin, `scheme://host[:port]` (the Server's own rule
 * for a Console origin), or null when none is set. An invalid value throws.
 */
export function configuredApiOrigin(value: unknown): string | null {
  if (typeof value !== "string" || value.trim() === "") return null
  const text = value.trim()
  let url: URL
  try {
    url = new URL(text)
  } catch {
    throw new InvalidApiOrigin(text)
  }
  const secure =
    url.protocol === "https:" ||
    (url.protocol === "http:" && LOOPBACK.has(url.hostname))
  if (
    !secure ||
    url.username !== "" ||
    url.password !== "" ||
    url.pathname !== "/" ||
    url.search !== "" ||
    url.hash !== "" ||
    text.includes("?") ||
    text.includes("#")
  )
    throw new InvalidApiOrigin(text)
  return url.origin
}

/**
 * The origin the client calls. The development server always proxies
 * `/api` on its own origin, so there the build variable does not apply.
 */
export function apiOrigin(
  env: Readonly<Record<string, unknown>>,
  ownOrigin: string
): string {
  if (env.DEV === true) return ownOrigin
  return configuredApiOrigin(env.VITE_MESHWAY_API_ORIGIN) ?? ownOrigin
}
