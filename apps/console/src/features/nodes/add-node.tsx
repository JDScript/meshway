import {
  createEnrollmentToken,
  createManagedEnrollmentToken,
  type ManagedListenerSettings,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useMemo, useState } from "react"

import { CloseButton } from "@/components/chrome/close-button"
import { TextField } from "@/components/chrome/form"
import {
  CopyField,
  GlassDialog,
  SecretField,
} from "@/components/chrome/glass-dialog"
import { Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { connectionsQuery } from "@/features/network/queries"
import { useCanManage } from "@/features/network/role"
import { surfaceTitleId } from "@/features/shell/page-frame"
import { apiBase, authed } from "@/lib/api"
import { dateTime } from "@/lib/format"
import { useSecretMutation } from "@/lib/secret"
import { m } from "@/paraglide/messages.js"

import {
  fieldLabelClass,
  ListenerAddress,
  listenerProblemText,
  presetLabel,
} from "./listener-form"
import {
  endpointText,
  initialListener,
  listenerProblems,
  newListener,
  presetOf,
  PRESETS,
  settingsOf,
  withPreset,
  type ListenerFields,
  type Preset,
} from "./listener-model"
import { managedListenersQuery } from "./queries"

const EXPIRIES = ["600", "3600", "86400"] as const
type Expiry = (typeof EXPIRIES)[number]
const expiryLabel: Record<Expiry, () => string> = {
  "600": () => m.nodes_add_expiry_10m(),
  "3600": () => m.nodes_add_expiry_1h(),
  "86400": () => m.nodes_add_expiry_24h(),
}

const NONE = "none"

const STATE_PATH = ".meshway/agent.json"
const MAX_NAME = 128
const SAFE = /^[A-Za-z0-9_\-.,:/=@%+]+$/
const quote = (arg: string) =>
  SAFE.test(arg) ? arg : `'${arg.replaceAll("'", `'\\''`)}'`
const command = (args: readonly string[]) => args.map(quote).join(" ")

/**
 * A command as it is shown: it breaks at its spaces and after a flag's
 * `=`, never inside a word, except within the secret, which may break
 * anywhere (it is one long word).
 */
function CommandText({
  args,
  secret,
}: {
  args: readonly string[]
  secret?: string
}) {
  return args.map((arg, index) => {
    const shown = quote(arg)
    const at = shown.indexOf("=")
    const [head, tail] =
      at === -1 ? ["", shown] : [shown.slice(0, at + 1), shown.slice(at + 1)]
    const cut = secret ? tail.indexOf(secret) : -1
    return (
      <span key={index}>
        {index > 0 ? " " : null}
        {head}
        {head ? <wbr /> : null}
        {cut === -1 || !secret ? (
          tail
        ) : (
          <>
            {tail.slice(0, cut)}
            <span className="break-all">{secret}</span>
            {tail.slice(cut + secret.length)}
          </>
        )}
      </span>
    )
  })
}

type Request = {
  expiry: Expiry
  name: string
  listener: ManagedListenerSettings | null
}

/**
 * Adding a node: a one-time enrolment token, optionally bound to the
 * node's first managed listener. The token is shown once, with the
 * commands that use it, and is never kept anywhere else.
 */
export function AddNode({
  networkId,
  onClose,
}: {
  networkId: string
  onClose: () => void
}) {
  const canManage = useCanManage(networkId)
  const listeners = useQuery(managedListenersQuery(networkId))
  const connections = useQuery(connectionsQuery(networkId))
  const taken = useMemo(
    () =>
      new Set([
        ...(listeners.data?.listeners.map((listener) => listener.settings.id) ??
          []),
        ...(connections.data?.configuration.listeners.map(
          (listener) => listener.id
        ) ?? []),
      ]),
    [listeners.data, connections.data]
  )

  const [name, setName] = useState("")
  const [expiry, setExpiry] = useState<Expiry>("3600")
  const [choice, setChoice] = useState<Preset | typeof NONE>(NONE)
  // Kept while "no listener" is chosen, so nothing typed is lost.
  const [fields, setFields] = useState<ListenerFields>(() =>
    newListener("native_udp", new Set())
  )
  const [attempted, setAttempted] = useState(false)
  const [sent, setSent] = useState<Request | null>(null)
  const listener = choice === NONE ? null : initialListener(fields, taken)

  const trimmed = name.trim()
  const nameValid = trimmed.length > 0 && [...trimmed].length <= MAX_NAME
  const listenerIssues = listener ? listenerProblems(listener, { taken }) : []
  const problem = !nameValid
    ? m.nodes_add_name_invalid({ max: String(MAX_NAME) })
    : listenerIssues[0]
      ? listenerProblemText[listenerIssues[0].kind]()
      : null
  const invalid = new Set<string>(
    attempted ? listenerIssues.map((issue) => issue.field) : []
  )

  const secret = useSecretMutation((request: Request) =>
    authed((base, token) =>
      request.listener
        ? createManagedEnrollmentToken(base, token, networkId, {
            expires_in_seconds: Number(request.expiry),
            initial_listener: request.listener,
          })
        : createEnrollmentToken(base, token, networkId, {
            expires_in_seconds: Number(request.expiry),
          })
    )
  )

  const submit = () => {
    setAttempted(true)
    if (problem !== null || secret.pending || !canManage) return
    const request: Request = {
      expiry,
      name: trimmed,
      listener: listener ? settingsOf(listener, "") : null,
    }
    setSent(request)
    secret.create(request)
  }

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            {m.nodes_add_title()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
      />
      <form
        className="flex min-h-0 flex-1 flex-col"
        onSubmit={(event) => {
          event.preventDefault()
          submit()
        }}
      >
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
          <div className="flex max-w-workspace flex-col gap-8">
            <div className="grid gap-4 sm:grid-cols-[1fr_auto]">
              <TextField
                id="add-node-name"
                name="node-name"
                label={m.nodes_add_name()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={MAX_NAME}
                placeholder="node-1"
                value={name}
                invalid={attempted && !nameValid}
                disabled={!canManage}
                onChange={(event) => setName(event.target.value)}
              />
              <div className="flex flex-col gap-2">
                <span className={fieldLabelClass}>{m.nodes_add_expiry()}</span>
                <SegmentedControl<Expiry>
                  label={m.nodes_add_expiry()}
                  value={expiry}
                  options={EXPIRIES.map((value) => ({
                    value,
                    label: expiryLabel[value](),
                  }))}
                  onChange={setExpiry}
                  className="h-10 self-start"
                />
              </div>
            </div>

            <Section title={m.nodes_add_listener_section()}>
              <div className="flex flex-col gap-4">
                <SegmentedControl<Preset | typeof NONE>
                  label={m.nodes_add_listener_section()}
                  value={choice}
                  options={[
                    { value: NONE, label: m.nodes_add_listener_none() },
                    ...PRESETS.map((preset) => ({
                      value: preset,
                      label: presetLabel[preset](),
                    })),
                  ]}
                  onChange={(next) => {
                    if (!canManage) return
                    setChoice(next)
                    if (next !== NONE)
                      setFields((current) => withPreset(current, next, null))
                  }}
                  className="h-10 self-start"
                />
                {choice !== NONE ? (
                  <div className="swap-in flex flex-col gap-3">
                    <ListenerAddress
                      idPrefix="add-listener"
                      fields={fields}
                      invalid={invalid}
                      disabled={!canManage}
                      onChange={setFields}
                    />
                    <div className="flex flex-col gap-1 text-meta text-muted-foreground">
                      <p>{m.nodes_add_listener_dial()}</p>
                      <p>{m.nodes_add_listener_defaults()}</p>
                    </div>
                  </div>
                ) : null}
              </div>
            </Section>
          </div>
        </div>
        {canManage ? (
          <footer className="flex h-16 shrink-0 items-center gap-3 border-t border-divider px-6">
            <span className="min-w-0 flex-1 truncate text-row text-warning">
              {attempted ? problem : null}
            </span>
            <Button
              type="submit"
              className="h-9 rounded-full px-4"
              disabled={secret.pending}
              aria-busy={secret.pending}
            >
              {secret.pending ? <Spinner /> : null}
              {m.nodes_add_submit()}
            </Button>
          </footer>
        ) : null}
      </form>

      <GlassDialog
        open={secret.result !== null}
        onOpenChange={(open) => {
          if (!open) secret.clear()
        }}
        title={m.nodes_add_token_title()}
        footer={
          <Button
            className="h-9 rounded-full px-4"
            onClick={() => secret.clear()}
          >
            {m.dialog_done()}
          </Button>
        }
      >
        {secret.result && sent ? (
          <TokenResult
            token={secret.result.token}
            tokenId={secret.result.token_id}
            expiresAt={secret.result.expires_at}
            request={sent}
          />
        ) : null}
      </GlassDialog>
    </div>
  )
}

function TokenResult({
  token,
  tokenId,
  expiresAt,
  request,
}: {
  token: string
  tokenId: string
  expiresAt: string
  request: Request
}) {
  const agent = ["cargo", "run", "--locked", "-p", "meshway-agent"]
  const enroll = [
    ...agent,
    "--",
    "enroll",
    `--controller=${apiBase()}`,
    `--token=${token}`,
    `--name=${request.name}`,
    `--state=${STATE_PATH}`,
  ]
  const run = request.listener
    ? [
        ...agent,
        "--features",
        "managed-connections",
        "--",
        "run",
        `--state=${STATE_PATH}`,
        "--apply-connections",
      ]
    : [...agent, "--", "run", `--state=${STATE_PATH}`]
  const listener = request.listener
  const reachable = listener?.advertised_endpoints[0]
  return (
    <>
      <div className="grid grid-cols-2 gap-2">
        <Tile label={m.nodes_add_expires()}>
          {dateTime(expiresAt) ?? expiresAt}
        </Tile>
        <Tile label={m.nodes_add_token_id()}>
          <span className="font-mono font-normal">{tokenId}</span>
        </Tile>
        <div className="col-span-2">
          <Tile label={m.nodes_add_listener_section()}>
            {listener ? (
              <>
                {presetLabel[presetOf(listener)]()} ·{" "}
                <span className="font-mono">
                  {reachable
                    ? endpointText(reachable.host, reachable.port)
                    : endpointText(listener.bind_address, listener.bind_port)}
                </span>
              </>
            ) : (
              m.nodes_add_listener_none()
            )}
          </Tile>
        </div>
      </div>
      <SecretField label={m.nodes_add_token()} value={token} />
      <CopyField
        label={m.nodes_add_enroll_command()}
        value={command(enroll)}
        once
      >
        <CommandText args={enroll} secret={token} />
      </CopyField>
      <CopyField label={m.nodes_add_run_command()} value={command(run)}>
        <CommandText args={run} />
      </CopyField>
    </>
  )
}
