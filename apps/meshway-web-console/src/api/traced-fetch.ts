import { zErrorResponse } from "@meshway/control-client/schemas"

import { debugEnabled } from "@/state/debug"
import { traceRequest } from "@/state/debug-trace"

/** The problem code of an error response, read from a copy of it; null when it has none. */
async function problemCodeOf(response: Response): Promise<string | null> {
  const media = response.headers.get("content-type")?.split(";")[0]?.trim()
  if (media !== "application/json") return null
  try {
    const parsed = zErrorResponse.safeParse(await response.clone().json())
    return parsed.success ? parsed.data.problem.code : null
  } catch {
    return null
  }
}

/**
 * `fetcher`, noting each request while the debug mode is on: its method,
 * path and query, status, time to the response headers and problem code,
 * and the controller's clock from `Date`. Never a header or a body: those
 * carry the session and the secrets some responses return once.
 */
export function tracedFetch(fetcher: typeof fetch): typeof fetch {
  return async (input, init) => {
    if (!debugEnabled()) return fetcher(input, init)
    const url = new URL(input instanceof Request ? input.url : String(input))
    const entry = {
      method: init?.method ?? "GET",
      path: `${url.pathname}${url.search}`,
      stream: new Headers(init?.headers).get("accept") === "text/event-stream",
    }
    const sent = Date.now()
    const started = performance.now()
    try {
      const response = await fetcher(input, init)
      const received = Date.now()
      const ms = Math.round(performance.now() - started)
      const date = Date.parse(response.headers.get("date") ?? "")
      const code = response.ok ? null : await problemCodeOf(response)
      traceRequest(
        { ...entry, status: response.status, ms, code, failed: null },
        sent,
        Number.isFinite(date) ? { date, sent, received } : null
      )
      return response
    } catch (error) {
      traceRequest(
        {
          ...entry,
          status: null,
          ms: Math.round(performance.now() - started),
          code: null,
          failed: init?.signal?.aborted ? "cancelled" : "network",
        },
        sent,
        null
      )
      throw error
    }
  }
}
