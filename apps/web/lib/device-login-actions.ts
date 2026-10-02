"use server"

import { authorizeDevice, getAccountSession } from "@meshway/control-client"
import { zNetworkSnapshot } from "@meshway/control-client/schemas"
import { redirect } from "next/navigation"
import { controllerUrl, isCredentialRejected, isPermissionDenied } from "@/lib/controller"
import { authorizationPath, deviceCallback, parseDeviceAuthorization } from "@/lib/device-login"
import { SIGN_IN_FIELD } from "@/lib/action-scope"
import { isRenderedSignIn, readSession } from "@/lib/session"

/** `stale`: the page was rendered for another sign-in than the browser now holds. */
export type DeviceLoginState = { error: string | null; stale?: boolean }

/**
 * Approve or deny one App sign-in as the signed-in *user*. The controller
 * binds the one-time code to this user's live session and membership in the
 * chosen network. The administrator token is not a user and never approves.
 */
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
  // An unusable cookie is cleared by the login page, never by this (possibly late) response.
  if (session.kind !== "active") redirect(session.kind === "expired" ? `${loginPath}&reason=expired` : loginPath)
  // The decision belongs to the sign-in this page was rendered for. If the
  // browser now holds another one (another tab signed in), nothing is
  // approved or denied with it; the page must be reloaded and confirmed again.
  if (!(await isRenderedSignIn(form.get(SIGN_IN_FIELD)))) {
    return { error: "这个页面属于之前的登录，浏览器现在是另一个账户或已重新登录。未批准也未拒绝，请刷新页面后重新确认。", stale: true }
  }
  if (session.credential.kind !== "user") {
    return { error: "管理员令牌会话不代表用户，不能批准设备。请使用账户登录后再批准。" }
  }
  const token = session.credential.token
  const decision = form.get("decision")
  if (decision !== "approve" && decision !== "deny") return { error: "请选择批准或拒绝。" }
  let callback: string
  try {
    // Both decisions recheck the current user session on the controller.
    if (decision === "approve") {
      const networkId = zNetworkSnapshot.shape.network_id.safeParse(form.get("network_id"))
      if (!networkId.success) return { error: "请选择接入网络。" }
      const response = await authorizeDevice(controllerUrl(), token, networkId.data, value.request)
      callback = deviceCallback(value, { code: response.code })
    } else {
      await getAccountSession(controllerUrl(), token)
      callback = deviceCallback(value, { error: "access_denied" })
    }
  } catch (error) {
    if (isCredentialRejected(error)) redirect(`${loginPath}&reason=expired`)
    if (isPermissionDenied(error)) return { error: "你不是所选网络的成员，不能把设备接入该网络。" }
    return { error: "Controller 未完成授权，请稍后重试或从 App 重新发起。" }
  }
  redirect(callback)
}
