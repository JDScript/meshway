"use client"

import type { NetworkList } from "@meshway/control-client"
import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId } from "react"

import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import type { DeviceAuthorization } from "@/lib/device-login"
import { decideDevice } from "@/lib/device-login-actions"

/**
 * Approve or deny one App sign-in. The request fields travel back as hidden
 * inputs and are revalidated on the server; only the chosen network is new.
 */
export function DeviceAuthorizationForm({ value, networks }: { value: DeviceAuthorization; networks: NetworkList }) {
  const [state, action, pending] = useActionState(decideDevice, { error: null })
  const selectId = useId()
  return (
    <form action={action} className="flex flex-col gap-4">
      {Object.entries({ ...value.request, state: value.state }).map(([key, item]) => (
        <input key={key} type="hidden" name={key} value={item} />
      ))}
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={selectId}>接入网络</Label>
        <NativeSelect
          id={selectId}
          name="network_id"
          required
          className="w-full"
          defaultValue={networks.networks[0]?.network_id}
        >
          {networks.networks.map((network) => (
            <NativeSelectOption key={network.network_id} value={network.network_id}>
              {network.name}（{network.network.ipv4_prefix}）
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <p className="text-xs text-muted-foreground">一台设备一次只接入一张网络；之后可在 App 中保存其他网络的接入。</p>
      </div>
      {state.error ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlertIcon className="mt-px size-3.5 shrink-0" />
          {state.error}
        </p>
      ) : null}
      <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <Button type="submit" name="decision" value="deny" variant="outline" size="lg" disabled={pending}>
          拒绝
        </Button>
        <Button type="submit" name="decision" value="approve" size="lg" disabled={pending || networks.networks.length === 0}>
          批准接入
        </Button>
      </div>
    </form>
  )
}
