import "server-only"

import { controllerUrl } from "@/lib/controller"

// Exact allowlist: never use a wildcard rewrite for controller APIs.
type NodePath = "/api/v1/devices/exchange" | "/api/v1/agents/heartbeat" | "/api/v1/agents/configuration" | "/api/v1/agent/dns"
const methods: Record<NodePath, "GET" | "POST"> = {
  "/api/v1/devices/exchange": "POST",
  "/api/v1/agents/heartbeat": "POST",
  "/api/v1/agents/configuration": "GET",
  "/api/v1/agent/dns": "GET",
}
const hopHeaders = ["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "transfer-encoding", "upgrade", "host"]

export async function proxyNodeRequest(request: Request, path: NodePath): Promise<Response> {
  if (request.method !== methods[path]) return new Response(null, { status: 405 })
  const headers = new Headers(request.headers)
  for (const header of hopHeaders) headers.delete(header)
  // The console session is not node authentication. Only the caller's bearer
  // is forwarded; the proxy never adds an administrator credential.
  headers.delete("cookie")
  headers.set("accept-encoding", "identity")
  try {
    const init: RequestInit & { duplex: "half" } = {
      method: methods[path], headers, body: methods[path] === "POST" ? request.body : undefined, duplex: "half",
      cache: "no-store", redirect: "manual", signal: request.signal,
    }
    // A byte-preserving reverse proxy does not consume DTOs: generated SDK
    // calls would decode bodies and lose the original rejection/headers.
    // eslint-disable-next-line no-restricted-globals
    const upstream = await fetch(`${controllerUrl().replace(/\/$/, "")}${path}`, init)
    const responseHeaders = new Headers(upstream.headers)
    for (const header of hopHeaders) responseHeaders.delete(header)
    // fetch transparently decodes compressed responses.
    responseHeaders.delete("content-encoding")
    responseHeaders.delete("content-length")
    responseHeaders.set("cache-control", "no-store")
    return new Response(upstream.body, { status: upstream.status, headers: responseHeaders })
  } catch {
    return new Response("Node API transport unavailable", { status: 502, headers: { "cache-control": "no-store" } })
  }
}
