"use server"

import { authorizeDevice, getOverview } from "@meshway/control-client"
import { redirect } from "next/navigation"
import { controllerUrl, isCredentialRejected } from "@/lib/controller"
import { authorizationPath, deviceCallback, parseDeviceAuthorization } from "@/lib/device-login"
import { endSession, readSession } from "@/lib/session"

export type DeviceLoginState = { error: string | null }

export async function decideDevice(_previous: DeviceLoginState, form: FormData): Promise<DeviceLoginState> {
  const params: Record<string, string | undefined> = {}
  for (const key of ["redirect_uri", "code_challenge", "name", "agent_version", "state"]) {
    const raw = form.getAll(key)
    params[key] = raw.length === 1 && typeof raw[0] === "string" ? raw[0] : undefined
  }
  const value = parseDeviceAuthorization(params)
  if (!value) return { error: "授权请求无效，请从 App 重新发起登录。" }
  const loginPath = `/login?returnTo=${encodeURIComponent(authorizationPath(value))}`
  const session = await readSession()
  if (session.kind !== "active") redirect(loginPath)
  const decision = form.get("decision")
  if (decision !== "approve" && decision !== "deny") return { error: "请选择批准或拒绝。" }
  let callback: string
  try {
    // Both decisions recheck the current human login on the controller.
    if (decision === "approve") {
      const response = await authorizeDevice(controllerUrl(), session.token, value.request)
      callback = deviceCallback(value, { code: response.code })
    } else {
      await getOverview(controllerUrl(), session.token)
      callback = deviceCallback(value, { error: "access_denied" })
    }
  } catch (error) {
    if (isCredentialRejected(error)) {
      await endSession()
      redirect(`${loginPath}&reason=expired`)
    }
    return { error: "Controller 未完成授权，请稍后重试或从 App 重新发起。" }
  }
  redirect(callback)
}
