import {
  createObservation,
  type CreateObservation,
  type ObservationServiceScope,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useId, useState, type FormEvent, type ReactNode } from "react"

import { authed, client } from "@/api/client"
import { toastFailureWith, type FailureText } from "@/api/failure"
import { writeTask } from "@/api/queries/observation"
import { BusyLabel } from "@/components/chrome/busy-label"
import { FieldOption, FieldSelect } from "@/components/chrome/field-select"
import { FieldInput } from "@/components/chrome/form"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { parsePort } from "@/lib/ip"
import { m } from "@/paraglide/messages.js"
import { attempts } from "@/state/attempts"

type ServiceType = ObservationServiceScope["type"]
type Mode = "auto" | "explicit"

/** The periods offered, within the controller's 1–30 s. */
const INTERVALS = [1, 2, 5, 10, 30] as const
/** Measured: the controller provides no traffic counts (`traffic` comes back unsupported). */
const METRICS: CreateObservation["metrics"] = ["rtt", "probeLoss"]

/**
 * By network and source node. The subscription ID is the attempt's: an
 * outcome left unknown sends the same subscription again.
 */
const starts =
  attempts<Omit<CreateObservation, "subscriptionId">>("observation-start")

/** What the controller's refusals of a new observation mean here. */
const createFailureText: FailureText = {
  permission_denied: () => m.observation_denied(),
  state_conflict: () => m.observation_conflict(),
  observation_limit: () => m.observation_limit(),
  not_found: () => m.observation_not_found(),
}

/**
 * A new observation from `sourceId`: a target among the nodes this account
 * sees, the exact service, automatic or one explicit path among `paths`,
 * and the period. A distinct attempt takes a fresh subscription ID; after
 * an attempt whose outcome is unknown the same input sends the same one.
 */
export function StartForm({
  networkId,
  sourceId,
  targets,
  pathsTo,
  nameOf,
  onStarted,
}: {
  networkId: string
  sourceId: string
  targets: readonly string[]
  pathsTo: (target: string) => string[][]
  nameOf: (id: string) => string
  onStarted: (subscriptionId: string) => void
}) {
  const id = useId()
  const queryClient = useQueryClient()
  const [target, setTarget] = useState(targets[0] ?? "")
  const chosenTarget = targets.includes(target) ? target : (targets[0] ?? "")
  const [service, setService] = useState<ServiceType>("icmpEcho")
  const [port, setPort] = useState("")
  const [mode, setMode] = useState<Mode>("auto")
  const paths = chosenTarget ? pathsTo(chosenTarget) : []
  const [pathKey, setPathKey] = useState("")
  const chosenPath =
    paths.find((path) => path.join(">") === pathKey) ?? paths[0] ?? null
  const [interval, setPeriod] = useState<(typeof INTERVALS)[number]>(1)
  const [busy, setBusy] = useState(false)

  const portValue = service === "icmpEcho" ? null : parsePort(port.trim())
  const scope: ObservationServiceScope | null =
    service === "icmpEcho"
      ? { type: "icmpEcho" }
      : portValue === null
        ? null
        : { type: service, port: portValue }
  const ready =
    chosenTarget !== "" &&
    scope !== null &&
    (mode === "auto" || chosenPath !== null)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!ready || busy) return
    const selection: CreateObservation["selection"] =
      mode === "auto" || !chosenPath
        ? { mode: "auto" }
        : { mode: "explicit", path: chosenPath }
    const input = {
      sourceNodeId: sourceId,
      targetNodeId: chosenTarget,
      service: scope,
      selection,
      metrics: METRICS,
      intervalMs: interval * 1000,
    }
    setBusy(true)
    try {
      const { task, subscriptionId } = await starts.send(
        `${networkId}/${sourceId}`,
        JSON.stringify(input),
        () => input,
        async (attempt) => ({
          subscriptionId: attempt.requestId,
          task: await authed((token) =>
            createObservation(
              client,
              { networkId },
              { ...attempt.body, subscriptionId: attempt.requestId },
              { token }
            )
          ),
        })
      )
      writeTask(queryClient, networkId, task)
      onStarted(subscriptionId)
    } catch (error) {
      toastFailureWith(createFailureText)(error)
    } finally {
      setBusy(false)
    }
  }

  const pathText = (path: readonly string[]) => path.map(nameOf).join(" → ")

  return (
    <form
      onSubmit={(event) => void submit(event)}
      className="flex flex-col gap-3"
    >
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Labelled id={`${id}-target`} label={m.observation_target()}>
          <FieldSelect
            id={`${id}-target`}
            name="target"
            value={chosenTarget}
            disabled={!targets.length}
            onChange={(event) => setTarget(event.target.value)}
          >
            {targets.length ? (
              targets.map((node) => (
                <FieldOption key={node} value={node}>
                  {nameOf(node)}
                </FieldOption>
              ))
            ) : (
              <FieldOption value="">{m.observation_no_target()}</FieldOption>
            )}
          </FieldSelect>
        </Labelled>
        <Labelled id={`${id}-interval`} label={m.observation_interval()}>
          <FieldSelect
            id={`${id}-interval`}
            name="interval"
            value={String(interval)}
            onChange={(event) => {
              const value = INTERVALS.find(
                (item) => String(item) === event.target.value
              )
              if (value) setPeriod(value)
            }}
          >
            {INTERVALS.map((value) => (
              <FieldOption key={value} value={String(value)}>
                {m.observation_seconds({ value: String(value) })}
              </FieldOption>
            ))}
          </FieldSelect>
        </Labelled>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <SegmentedControl
          size="sm"
          label={m.observation_service()}
          value={service}
          options={[
            { value: "icmpEcho", label: m.observation_service_icmp() },
            { value: "tcp", label: m.observation_service_tcp() },
            { value: "udp", label: m.observation_service_udp() },
          ]}
          onChange={setService}
        />
        {service === "icmpEcho" ? null : (
          <FieldInput
            aria-label={m.observation_port()}
            name="port"
            inputMode="numeric"
            autoComplete="off"
            placeholder={m.observation_port_placeholder()}
            value={port}
            aria-invalid={port !== "" && portValue === null ? true : undefined}
            onChange={(event) => setPort(event.target.value)}
            className="h-8 w-28 md:text-meta"
          />
        )}
        <SegmentedControl
          size="sm"
          label={m.observation_mode()}
          value={mode}
          options={[
            { value: "auto", label: m.observation_mode_auto() },
            { value: "explicit", label: m.observation_mode_explicit() },
          ]}
          onChange={setMode}
        />
      </div>

      {mode === "explicit" ? (
        <Labelled id={`${id}-path`} label={m.observation_path()}>
          <FieldSelect
            id={`${id}-path`}
            name="path"
            value={chosenPath?.join(">") ?? ""}
            disabled={!paths.length}
            onChange={(event) => setPathKey(event.target.value)}
          >
            {paths.map((path) => (
              <FieldOption key={path.join(">")} value={path.join(">")}>
                {pathText(path)}
              </FieldOption>
            ))}
          </FieldSelect>
        </Labelled>
      ) : null}

      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="text-meta text-muted-foreground">
          {m.observation_measures()}
        </span>
        <Button type="submit" size="sm" disabled={!ready || busy}>
          <BusyLabel busy={busy}>{m.observation_start()}</BusyLabel>
        </Button>
      </div>
    </form>
  )
}

function Labelled({
  id,
  label,
  children,
}: {
  id: string
  label: string
  children: ReactNode
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={id} className="text-meta text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  )
}
