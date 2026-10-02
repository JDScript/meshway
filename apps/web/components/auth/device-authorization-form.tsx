"use client"

import type { AccountRole } from "@meshway/control-client"
import { CircleAlertIcon } from "lucide-react"
import { useActionState, useId } from "react"

import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { roleLabels } from "@/lib/account-model"
import type { DeviceAuthorization } from "@/lib/device-login"
import { decideDevice } from "@/lib/device-login-actions"

export type DeviceNetworkChoice = { networkId: string; name: string; ipv4Prefix: string; role: AccountRole }

/**
 * Approve or deny one App sign-in. The request fields travel back as hidden
 * inputs and are revalidated on the server; only the chosen network is new.
 * Any member, including a Member, may approve their own device.
 */
export function DeviceAuthorizationForm({ value, networks }: { value: DeviceAuthorization; networks: DeviceNetworkChoice[] }) {
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
          defaultValue={networks[0]?.networkId}
          disabled={networks.length === 0}
        >
          {networks.map((network) => (
            <NativeSelectOption key={network.networkId} value={network.networkId}>
              {network.name}（{network.ipv4Prefix}）· {roleLabels[network.role]}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <p className="text-xs text-muted-foreground">设备归属你在该网络的成员身份；一台设备一次只接入一张网络。</p>
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
        <Button type="submit" name="decision" value="approve" size="lg" disabled={pending || networks.length === 0}>
          批准接入
        </Button>
      </div>
    </form>
  )
}
