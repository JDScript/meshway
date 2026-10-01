"use server"

import {
  ControlApiError,
  createEnrollmentToken,
  revokeNode,
  setNodeCountry,
} from "@meshway/control-client"
import { refresh } from "next/cache"

import {
  enrollmentTtlOptions,
  type CountryState,
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
const MAX_REVISION = 4_294_967_295
const COUNTRY = /^[A-Za-z]{2}$/

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

/**
 * Administrator-managed country metadata. Conditional on the global
 * configuration revision the form was rendered with; a stale revision is a
 * 409 and nothing is written.
 */
export async function setNodeCountryAction(
  networkId: string,
  _previous: CountryState,
  formData: FormData
): Promise<CountryState> {
  const { token } = await requireNetworkActionContext(networkId)

  const nodeId = readNodeId(formData)
  if (!nodeId) return { status: "error", message: "节点 ID 无效。" }
  const rawCountry = formData.get("country")
  const country = typeof rawCountry === "string" ? rawCountry.trim() : ""
  if (country !== "" && !COUNTRY.test(country)) {
    return { status: "error", message: "国家代码必须是两位字母，或留空以清除。" }
  }
  const rawRevision = formData.get("expected_revision")
  const expectedRevision =
    typeof rawRevision === "string" && /^[0-9]{1,10}$/.test(rawRevision)
      ? Number(rawRevision)
      : 0
  if (expectedRevision < 1 || expectedRevision > MAX_REVISION) {
    return { status: "error", message: "配置版本无效，请重新读取页面。" }
  }

  try {
    const { node } = await setNodeCountry(controllerUrl(), token, networkId, {
      node_id: nodeId,
      country: country === "" ? null : country.toUpperCase(),
      expected_revision: expectedRevision,
    })
    refresh()
    return { status: "saved", node }
  } catch (error) {
    if (!isCredentialRejected(error)) {
      if (error instanceof ControlApiError && error.status === 409) {
        refresh()
        return {
          status: "conflict",
          message: `配置版本已不是 ${expectedRevision}：其他修改已先保存。未写入任何内容；页面已重新读取，请确认最新数据后重试。`,
        }
      }
      if (error instanceof ControlApiError && error.status === 404) {
        refresh()
        return { status: "error", message: "Controller 中没有这个节点。" }
      }
      if (error instanceof ControlApiError && error.status === 400) {
        return { status: "error", message: "Controller 拒绝了该国家代码：必须是两位字母或留空。" }
      }
      return {
        status: "error",
        message: `保存国家失败：${describeError(error, token)}`,
      }
    }
  }
  return rejectActionSession()
}
