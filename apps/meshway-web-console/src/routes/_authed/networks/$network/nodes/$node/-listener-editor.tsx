import type { Listener } from "@meshway/control-client"
import { useState, type ReactNode } from "react"

import { FIELD_SIZE, SwitchRow, TextField } from "@/components/chrome/form"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import {
  draftProblems,
  isListenerKey,
  keepsCertificate,
  keepsUuid,
  schemaRefuses,
  updateOf,
  type DraftProblem,
  type ListenerDraft,
  type ListenerOrigin,
} from "@/lib/nodes/listener-edit"
import {
  problemField,
  protocolOf,
  withPreset,
  type ListenerField,
  type ListenerProblem,
  type Preset,
} from "@/lib/nodes/listener-model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { presetLabel } from "../-labels"
import { ListenerInputs, listenerProblemText } from "../add/-add-node"

const draftProblemText: Record<
  Exclude<DraftProblem, ListenerProblem>,
  () => string
> = {
  ws_path: () => m.nodes_listener_ws_path_invalid(),
  uuid: () => m.nodes_listener_uuid_invalid(),
  certificate_pair: () => m.nodes_listener_certificate_pair(),
  certificate_missing: () => m.nodes_listener_certificate_missing(),
  schema: () => m.nodes_listener_schema(),
}

const textOf = (problem: DraftProblem) =>
  problem in listenerProblemText
    ? listenerProblemText[problem as ListenerProblem]()
    : draftProblemText[problem as Exclude<DraftProblem, ListenerProblem>]()

type SecretChoice = "keep" | "write"

/**
 * One listener's inputs, edited apart from the draft until done: a
 * listener is one value (`setListener` restates it whole), so it enters
 * the draft only once it is valid. Secrets are write-only: the UUID and a
 * provided certificate are kept unless new ones are written; recreating
 * the listener generates new material for what is not written.
 */
export function ListenerEditor({
  idPrefix,
  origin,
  initial,
  presets,
  takenKeys,
  newKey,
  onDone,
  onCancel,
}: {
  idPrefix: string
  origin: ListenerOrigin
  initial: ListenerDraft
  presets: readonly Preset[]
  /** Keys a new listener cannot take. */
  takenKeys: readonly string[]
  /** A new listener's suggested key for a transport; null when editing one. */
  newKey: ((preset: Preset) => string) | null
  onDone: (key: string | null, listener: Listener, recreate: boolean) => void
  onCancel: () => void
}) {
  const [draft, setDraft] = useState(initial)
  const [key, setKey] = useState(newKey?.(initial.fields.preset) ?? "")
  // The key follows the transport until it is typed.
  const [keyTyped, setKeyTyped] = useState(false)
  const [attempted, setAttempted] = useState(false)
  const [uuidChoice, setUuidChoice] = useState<SecretChoice>("keep")
  const [certChoice, setCertChoice] = useState<SecretChoice>("keep")
  const existing = origin.view !== null
  const vless = protocolOf(draft.fields.preset) === "vless"
  const keyProblem =
    newKey === null
      ? null
      : !isListenerKey(key)
        ? m.nodes_listener_key_invalid()
        : takenKeys.includes(key)
          ? m.nodes_listener_key_taken()
          : null

  // Secrets chosen as kept are sent as kept, whatever was typed before.
  const certKeepable = keepsCertificate(draft, origin)
  const writesCert = certChoice === "write" || !certKeepable
  const effective: ListenerDraft = {
    ...draft,
    uuid: uuidChoice === "write" ? draft.uuid : "",
    certificate: writesCert ? draft.certificate : "",
    privateKey: writesCert ? draft.privateKey : "",
  }
  const problems = [
    ...draftProblems(effective, origin),
    ...(schemaRefuses(effective, origin) ? (["schema"] as const) : []),
  ]
  const problem = keyProblem ?? (problems[0] ? textOf(problems[0]) : null)
  const invalid = new Set<ListenerField>(
    attempted
      ? problems.flatMap((entry) =>
          entry in problemField ? [problemField[entry as ListenerProblem]] : []
        )
      : []
  )
  const set = (next: Partial<ListenerDraft>) =>
    setDraft((current) => ({ ...current, ...next }))
  const field = (name: string) => `${idPrefix}-${name}`

  const done = () => {
    setAttempted(true)
    if (problem !== null) return
    const listener = updateOf(effective, origin)
    if (!listener) return
    onDone(newKey === null ? null : key, listener, draft.recreate)
  }

  const keepsUuidNow = keepsUuid(draft, origin)

  return (
    <div
      role="group"
      aria-label={m.nodes_listener_editing()}
      className="flex flex-col gap-4 rounded-surface border border-draft/40 p-3"
      onKeyDown={(event) => {
        if (event.key === "Enter" && event.target instanceof HTMLInputElement) {
          event.preventDefault()
          done()
        }
      }}
    >
      {newKey !== null ? (
        <TextField
          id={field("key")}
          name="listener-key"
          label={m.nodes_listener_key()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={64}
          className="font-mono"
          placeholder="vless-ws"
          invalid={attempted && keyProblem !== null}
          value={key}
          onChange={(event) => {
            setKey(event.target.value)
            setKeyTyped(true)
          }}
        />
      ) : null}
      <SegmentedControl<Preset>
        label={m.nodes_add_listener()}
        value={draft.fields.preset}
        options={presets.map((preset) => ({
          value: preset,
          label: presetLabel[preset](),
        }))}
        onChange={(preset) => {
          set({ fields: withPreset(draft.fields, preset) })
          if (newKey !== null && !keyTyped) setKey(newKey(preset))
        }}
        className="self-start"
      />
      <ListenerInputs
        idPrefix={field("in")}
        generated={false}
        fields={draft.fields}
        invalid={invalid}
        onChange={(fields) => set({ fields })}
      >
        <div className="flex flex-col gap-4 pt-4">
          {draft.fields.preset === "vless_websocket" ? (
            <div className="grid grid-cols-2 gap-x-2">
              <TextField
                id={field("ws-path")}
                name="ws-path"
                label={m.nodes_listener_ws_path()}
                autoComplete="off"
                spellCheck={false}
                maxLength={256}
                className="font-mono"
                placeholder="/mesh"
                invalid={attempted && problems.includes("ws_path")}
                value={draft.wsPath}
                onChange={(event) => set({ wsPath: event.target.value })}
              />
              <TextField
                id={field("ws-host")}
                name="ws-host"
                label={m.nodes_listener_ws_host()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={253}
                className="font-mono"
                placeholder="cdn.example.com"
                value={draft.wsHost}
                onChange={(event) => set({ wsHost: event.target.value })}
              />
            </div>
          ) : null}
          <TextField
            id={field("remark")}
            name="listener-remark"
            label={m.nodes_listener_remark()}
            autoComplete="off"
            maxLength={256}
            placeholder={m.nodes_settings_remark_placeholder()}
            value={draft.remark}
            onChange={(event) => set({ remark: event.target.value })}
          />
        </div>
      </ListenerInputs>

      <SwitchRow
        id={field("enabled")}
        label={m.nodes_listener_enabled()}
        checked={draft.enabled}
        onCheckedChange={(enabled) => set({ enabled })}
      />

      {vless ? (
        <div className="flex flex-col gap-3">
          <SecretRow
            label={m.nodes_listener_uuid()}
            kept={
              keepsUuidNow ? m.nodes_secret_keep() : m.nodes_secret_generate()
            }
            choice={uuidChoice}
            onChoice={setUuidChoice}
          >
            <TextField
              id={field("uuid")}
              name="uuid"
              label={m.nodes_listener_uuid_new()}
              autoComplete="off"
              spellCheck={false}
              maxLength={36}
              className="font-mono"
              placeholder="xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx"
              invalid={attempted && problems.includes("uuid")}
              value={draft.uuid}
              onChange={(event) => set({ uuid: event.target.value })}
            />
          </SecretRow>
          <div className="flex min-h-12 flex-wrap items-center justify-between gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
            <span className="text-row font-medium">
              {m.nodes_listener_certificate()}
            </span>
            <SegmentedControl
              size="sm"
              label={m.nodes_listener_certificate()}
              value={draft.tlsMode}
              options={[
                { value: "managed", label: m.nodes_tls_managed() },
                { value: "provided", label: m.nodes_tls_provided() },
              ]}
              onChange={(tlsMode) => set({ tlsMode })}
            />
          </div>
          {draft.tlsMode === "provided" ? (
            <SecretRow
              label={m.nodes_listener_certificate_pem()}
              kept={certKeepable ? m.nodes_secret_keep() : null}
              choice={writesCert ? "write" : "keep"}
              onChoice={setCertChoice}
            >
              <div className="grid gap-2">
                <PemField
                  id={field("certificate")}
                  label={m.nodes_listener_certificate_chain()}
                  placeholder="-----BEGIN CERTIFICATE-----"
                  invalid={
                    attempted &&
                    problems.some((entry) => entry.startsWith("certificate"))
                  }
                  value={draft.certificate}
                  onChange={(certificate) => set({ certificate })}
                />
                <PemField
                  id={field("private-key")}
                  label={m.nodes_listener_private_key()}
                  placeholder="-----BEGIN PRIVATE KEY-----"
                  invalid={
                    attempted &&
                    problems.some((entry) => entry.startsWith("certificate"))
                  }
                  value={draft.privateKey}
                  onChange={(privateKey) => set({ privateKey })}
                />
              </div>
            </SecretRow>
          ) : null}
        </div>
      ) : null}

      {existing && vless ? (
        <SwitchRow
          id={field("recreate")}
          label={m.nodes_listener_recreate()}
          checked={draft.recreate}
          onCheckedChange={(recreate) => set({ recreate })}
        />
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-row text-warning">
          {attempted ? problem : null}
        </span>
        <Button
          type="button"
          variant="ghost"
          className="h-9 min-w-16 rounded-full px-4"
          onClick={onCancel}
        >
          {m.dialog_cancel()}
        </Button>
        <Button
          type="button"
          className="h-9 min-w-20 rounded-full px-4"
          onClick={done}
        >
          {m.nodes_listener_done()}
        </Button>
      </div>
    </div>
  )
}

/** A secret: kept as it is (or generated), or a new value written here. */
function SecretRow({
  label,
  kept,
  choice,
  onChoice,
  children,
}: {
  label: string
  /** What happens unless written; null: it must be written. */
  kept: string | null
  choice: SecretChoice
  onChoice: (choice: SecretChoice) => void
  children: ReactNode
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex min-h-12 flex-wrap items-center justify-between gap-3 rounded-tile bg-fill-subtle py-2 pr-2 pl-3.5">
        <span className="text-row font-medium">{label}</span>
        {kept !== null ? (
          <SegmentedControl<SecretChoice>
            size="sm"
            label={label}
            value={choice}
            options={[
              { value: "keep", label: kept },
              { value: "write", label: m.nodes_secret_write() },
            ]}
            onChange={onChoice}
          />
        ) : (
          <span className="pr-1.5 text-row text-muted-foreground">
            {m.nodes_secret_required()}
          </span>
        )}
      </div>
      {choice === "write" || kept === null ? children : null}
    </div>
  )
}

function PemField({
  id,
  label,
  placeholder,
  invalid,
  value,
  onChange,
}: {
  id: string
  label: string
  placeholder: string
  invalid: boolean
  value: string
  onChange: (value: string) => void
}) {
  return (
    <Field data-invalid={invalid || undefined}>
      <FieldLabel htmlFor={id} className={FIELD_SIZE.default.label}>
        {label}
      </FieldLabel>
      <Textarea
        id={id}
        name={id}
        autoComplete="off"
        spellCheck={false}
        rows={4}
        aria-invalid={invalid || undefined}
        placeholder={placeholder}
        className={cn(
          "rounded-tile border-transparent bg-field font-mono text-meta dark:bg-field"
        )}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  )
}
