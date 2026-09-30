import "server-only"

import {
  getOverview,
  listNodes,
  type NodeList,
  type Overview,
} from "@meshway/control-client"
import { connection } from "next/server"

export function controllerUrl(): string {
  return process.env.MESHWAY_CONTROLLER_URL ?? "http://127.0.0.1:8420"
}

export type ControllerResult<T> =
  | { ok: true; data: T; fetchedAt: number }
  | { ok: false; error: string }

function describeError(error: unknown): string {
  // Node's fetch reports the socket-level reason (ECONNREFUSED, ENOTFOUND…)
  // somewhere down the `cause` chain, possibly under SDK wrapper errors.
  const messages: string[] = []
  let current: unknown = error
  while (current instanceof Error && messages.length < 5) {
    if (!messages.includes(current.message)) messages.push(current.message)
    current = current.cause
  }
  return messages.length > 0 ? messages.join(": ") : String(error)
}

// Each call hits the controller at request time; nothing is cached, so the
// console never shows a stale report as current. Timeouts are enforced by the
// SDK transport and surface here as ordinary errors.
async function request<T>(
  call: (baseUrl: string) => Promise<T>
): Promise<ControllerResult<T>> {
  await connection()
  try {
    const data = await call(controllerUrl())
    return { ok: true, data, fetchedAt: Date.now() }
  } catch (error) {
    return { ok: false, error: describeError(error) }
  }
}

export function fetchOverview(): Promise<ControllerResult<Overview>> {
  return request(getOverview)
}

export function fetchNodes(): Promise<ControllerResult<NodeList>> {
  return request(listNodes)
}
