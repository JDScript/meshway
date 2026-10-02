"use server"

import { ControlApiError, redeemAccountPasswordReset } from "@meshway/control-client"
import { zRedeemPasswordResetRequest } from "@meshway/control-client/schemas"
import { redirect } from "next/navigation"

import { passwordProblem } from "@/lib/account-model"
import { describeError } from "@/lib/controller"
import { controllerUrl } from "@/lib/controller-config"
import { LOGIN_PATH } from "@/lib/session"
import { formField } from "@/lib/sign-in"

export type ResetRedeemState = { status: "idle" } | { status: "error"; message: string }

/**
 * Public: a reset link's one-time secret sets a new password. The controller
 * consumes the link and revokes every session of the account in the same
 * transaction; the person then signs in with the new password.
 */
export async function redeemResetAction(_previous: ResetRedeemState, formData: FormData): Promise<ResetRedeemState> {
  const password = formField(formData, "new_password")
  if (password !== formField(formData, "new_password_confirmation")) return { status: "error", message: "两次输入的密码不一致。" }
  const problem = passwordProblem(password)
  if (problem) return { status: "error", message: problem }
  const body = zRedeemPasswordResetRequest.safeParse({ reset_token: formField(formData, "reset_token"), new_password: password })
  if (!body.success) return { status: "error", message: "重置链接无效，请向实例管理员索取新的链接。" }
  try {
    await redeemAccountPasswordReset(controllerUrl(), body.data)
  } catch (error) {
    if (error instanceof ControlApiError) {
      if (error.status === 401) return { status: "error", message: "重置链接无效、已过期、已被撤销或已使用。请向实例管理员索取新的链接。" }
      if (error.status === 429) return { status: "error", message: "尝试次数过多，请稍后再试。" }
      if (error.status === 400 || error.status === 422) return { status: "error", message: "新密码不符合要求。" }
    }
    return { status: "error", message: `重置失败：${describeError(error, password).replaceAll(body.data.reset_token, "[已隐藏]")}` }
  }
  redirect(`${LOGIN_PATH}?reason=password-reset`)
}
