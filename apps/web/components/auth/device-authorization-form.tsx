"use client"

import { useActionState } from "react"
import { Button } from "@/components/ui/button"
import { decideDevice } from "@/lib/device-login-actions"
import type { DeviceAuthorization } from "@/lib/device-login"
import type { NetworkList } from "@meshway/control-client"

export function DeviceAuthorizationForm({ value, networks }: { value: DeviceAuthorization; networks: NetworkList }) {
  const [state, action, pending] = useActionState(decideDevice, { error: null })
  return <form action={action} className="flex flex-col gap-4">
    {Object.entries({ ...value.request, state: value.state }).map(([key, item]) => <input key={key} type="hidden" name={key} value={item} />)}
    <label className="flex flex-col gap-2 text-sm">接入网络
      <select name="network_id" required className="rounded-md border bg-background p-2" defaultValue={networks.networks[0]?.network_id}>
        {networks.networks.map(network => <option key={network.network_id} value={network.network_id}>{network.name}</option>)}
      </select>
    </label>
    {state.error ? <p role="alert" className="text-sm text-destructive">{state.error}</p> : null}
    <div className="flex gap-3">
      <Button type="submit" name="decision" value="approve" disabled={pending}>批准接入</Button>
      <Button type="submit" name="decision" value="deny" variant="outline" disabled={pending}>拒绝</Button>
    </div>
  </form>
}
