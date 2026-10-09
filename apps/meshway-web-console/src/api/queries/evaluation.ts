import { evaluateConfiguration } from "@meshway/control-client"
import { queryOptions } from "@tanstack/react-query"

import { authed, client } from "@/api/client"

import { networkKey } from "./network"

/** Plans asked at once; the rest wait, so a large network does not trip the controller's limit. */
const AT_ONCE = 4
let running = 0
const waiting: (() => void)[] = []

async function inTurn<T>(signal: AbortSignal, run: () => Promise<T>) {
  if (running >= AT_ONCE)
    await new Promise<void>((resolve) => waiting.push(resolve))
  running += 1
  try {
    signal.throwIfAborted()
    return await run()
  } finally {
    running -= 1
    waiting.shift()?.()
  }
}

/**
 * The controller's plan from `source` to node `target` over the current
 * version (`evaluateConfiguration`). A plan of one revision never changes,
 * so it is kept per revision. Owners and admins (`readConfiguration`).
 */
export const routeQuery = (
  networkId: string,
  revision: number,
  source: string,
  target: string
) =>
  queryOptions({
    queryKey: [
      ...networkKey(networkId),
      "route",
      revision,
      source,
      target,
    ] as const,
    queryFn: ({ signal }) =>
      inTurn(signal, () =>
        authed((token) =>
          evaluateConfiguration(
            client,
            { networkId },
            {
              configuration: { type: "current" },
              source,
              target: { type: "node", nodeId: target },
            },
            { token, signal }
          )
        )
      ),
    staleTime: Infinity,
    gcTime: 10 * 60_000,
    retry: false,
  })
