"use server"

import { listNetworks } from "@meshway/control-client"
import { redirect } from "next/navigation"

import type { LoginState } from "@/lib/action-state"
import { safeLoginReturn } from "@/lib/device-login"
import {
  controllerUrl,
  describeError,
  isCredentialRejected,
} from "@/lib/controller"
import { endSession, LOGIN_PATH, readSession, startSession } from "@/lib/session"

const MAX_TOKEN_LENGTH = 1024
const VISIBLE_ASCII = /^[\x21-\x7E]+$/

export async function login(
  _previous: LoginState,
  formData: FormData
): Promise<LoginState> {
  const raw = formData.get("token")
  const token = typeof raw === "string" ? raw.trim() : ""
  if (!token) return { status: "malformed", message: "请输入管理员令牌。" }
  if (token.length > MAX_TOKEN_LENGTH || !VISIBLE_ASCII.test(token)) {
    return {
      status: "malformed",
      message: "管理员令牌只包含可见 ASCII 字符，不含空格。请检查是否完整复制。",
    }
  }

  try {
    await listNetworks(controllerUrl(), token)
  } catch (error) {
    if (isCredentialRejected(error)) return { status: "invalid" }
    return { status: "unavailable", detail: describeError(error, token) }
  }

  await startSession(token)
  redirect(safeLoginReturn(formData.get("returnTo")))
}

export async function logout(): Promise<void> {
  await endSession()
  redirect(LOGIN_PATH)
}

/**
 * Drops the session cookie only when it is actually unusable: expired locally
 * or rejected by the controller. A still-valid session is left untouched, so
 * this cannot be abused to sign an administrator out, and a controller outage
 * is not mistaken for an authentication failure.
 */
export async function discardRejectedSession(): Promise<void> {
  const session = await readSession()
  if (session.kind === "none") return
  if (session.kind === "expired") {
    await endSession()
    return
  }
  try {
    await listNetworks(controllerUrl(), session.token)
  } catch (error) {
    if (isCredentialRejected(error)) await endSession()
  }
}
