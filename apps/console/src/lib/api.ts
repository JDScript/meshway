import {
  ControlClient,
  type ClientFailure,
  type ClientResult,
  type ErrorResponse,
} from "@meshway/control-client"

import { currentSession, endSession, type Session } from "@/lib/session"

/** The controller serves this console, so the API is always same-origin. */
export const client = new ControlClient({ baseUrl: window.location.origin })

export type Failure = ClientFailure<ErrorResponse>

/** A call that did not succeed, as the generated client reported it. */
export class ApiFailure extends Error {
  readonly failure: Failure
  constructor(failure: Failure) {
    super(`control API ${failure.kind}`)
    this.name = "ApiFailure"
    this.failure = failure
  }
}

/** The controller rejected the credential itself: expired, revoked or signed out. */
export class SessionRejected extends Error {
  constructor() {
    super("session rejected")
    this.name = "SessionRejected"
  }
}

/** The problem code of an HTTP failure whose body the client decoded, else null. */
export function problemCode(failure: Failure) {
  return failure.kind === "http"
    ? (failure.problem?.problem.code ?? null)
    : null
}

/** A 401 means the session is not (or no longer) valid. */
const rejectsSession = (failure: Failure) =>
  failure.kind === "http" && failure.status === 401

/** Unwraps a call made without this tab's session; a failure is thrown as `ApiFailure`. */
export async function call<T>(
  result: Promise<ClientResult<T, ErrorResponse>>
): Promise<T> {
  const settled = await result
  if (settled.ok) return settled.data
  throw new ApiFailure(settled.error)
}

let onRejected: () => void = () => undefined

/** Where to go when the controller rejects this tab's session (set once by the router). */
export function handleSessionRejected(callback: () => void) {
  onRejected = callback
}

/**
 * Calls a generated operation with this tab's session. A 401 ends exactly
 * that session and hands over to the router; every other failure is thrown
 * as `ApiFailure`.
 */
export async function authed<T>(
  operation: (token: string) => Promise<ClientResult<T, ErrorResponse>>
): Promise<T> {
  const session = currentSession()
  if (!session) {
    onRejected()
    throw new SessionRejected()
  }
  const settled = await operation(session.token)
  if (settled.ok) return settled.data
  if (rejectsSession(settled.error)) {
    reject(session)
    throw new SessionRejected()
  }
  throw new ApiFailure(settled.error)
}

function reject(session: Session) {
  endSession(session)
  onRejected()
}
