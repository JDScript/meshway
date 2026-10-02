"use server"

import {
  ControlApiError,
  createEnrollmentToken,
  revokeNode,
} from "@meshway/control-client"
import { refresh } from "next/cache"

import {
  enrollmentTtlOptions,
  type EnrollmentState,
  type RevokeState,
} from "@/lib/action-state"
import {
  controllerUrl,
  describeError,
  isCredentialRejected,
} from "@/lib/controller"
import { rejectActionSession } from "@/lib/session"
import { requireNetworkActionContext } from "@/lib/network-context"

const MAX_NODE_ID_LENGTH = 128

function readNodeId(formData: FormData): string | null {
  const raw = formData.get("node_id")
  const nodeId = typeof raw === "string" ? raw.trim() : ""
  return nodeId && nodeId.length <= MAX_NODE_ID_LENGTH ? nodeId : null
}

export async function createEnrollment(
  networkId: string,
  _previous: EnrollmentState,
  formData: FormData
): Promise<EnrollmentState> {
  const { token } = await requireNetworkActionContext(networkId)

  const raw = formData.get("expires_in_seconds")
  const option = enrollmentTtlOptions.find(
    ({ seconds }) => String(seconds) === raw
  )
  if (!option) return { status: "error", message: "请选择有效期。" }

  const baseUrl = controllerUrl()
  try {
    const enrollment = await createEnrollmentToken(baseUrl, token, networkId, {
      expires_in_seconds: option.seconds,
    })
    return {
      status: "created",
      enrollment,
      controllerUrl: baseUrl,
      createdAt: Date.now(),
      initialListener: null,
    }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      return {
        status: "error",
        message: `创建注册令牌失败：${describeError(error, token)}`,
      }
    }
  }
  return rejectActionSession()
}

export async function revokeNodeAction(
  networkId: string,
  _previous: RevokeState,
  formData: FormData
): Promise<RevokeState> {
  const { token } = await requireNetworkActionContext(networkId)

  const nodeId = readNodeId(formData)
  if (!nodeId) return { status: "error", message: "节点 ID 无效。" }

  try {
    const { node } = await revokeNode(controllerUrl(), token, networkId, { node_id: nodeId })
    refresh()
    return { status: "revoked", node }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      if (error instanceof ControlApiError && error.status === 404) {
        refresh()
        return { status: "error", message: "Controller 中没有这个节点。" }
      }
      if (error instanceof ControlApiError && error.status === 409) {
        refresh()
        return { status: "error", message: "该节点已被撤销。" }
      }
      return {
        status: "error",
        message: `撤销失败：${describeError(error, token)}`,
      }
    }
  }
  return rejectActionSession()
}
