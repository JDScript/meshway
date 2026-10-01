"use client"

import { useActionState } from "react"
import { Button } from "@/components/ui/button"
import { decideDevice } from "@/lib/device-login-actions"
import type { DeviceAuthorization } from "@/lib/device-login"

export function DeviceAuthorizationForm({ value }: { value: DeviceAuthorization }) {
  const [state, action, pending] = useActionState(decideDevice, { error: null })
  return <form action={action} className="flex flex-col gap-4">
    {Object.entries({ ...value.request, state: value.state }).map(([key, item]) => <input key={key} type="hidden" name={key} value={item} />)}
    {state.error ? <p role="alert" className="text-sm text-destructive">{state.error}</p> : null}
    <div className="flex gap-3">
      <Button type="submit" name="decision" value="approve" disabled={pending}>批准接入</Button>
      <Button type="submit" name="decision" value="deny" variant="outline" disabled={pending}>拒绝</Button>
    </div>
  </form>
}
