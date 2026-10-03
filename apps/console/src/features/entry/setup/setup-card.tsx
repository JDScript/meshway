import {
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react"
import { toast } from "sonner"

import { Brand } from "@/components/chrome/brand"
import { TextField } from "@/components/chrome/form"
import { FieldGroup } from "@/components/ui/field"
import { SessionRejected } from "@/lib/api"
import { failureText } from "@/lib/failure"
import { formText } from "@/lib/form"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"

import { ENTRY_CARD, ENTRY_ENTER, EntryLink, EntrySubmit } from "../frame"
import { instanceCall, readInstance, type Instance } from "./instance"
import { SetupSections } from "./setup-sections"

const TOKEN_COMMAND = "meshway-controller admin-token --data-dir <dir>"

type Signed = { token: string; instance: Instance }

/**
 * Instance setup and recovery with the administrator token. The token lives
 * in this component's state only: leaving the page, reloading, or the
 * controller refusing it forgets it.
 */
export function SetupCard({ onHeld }: { onHeld: (held: boolean) => void }) {
  const [signed, setSignedState] = useState<Signed | null>(null)
  const [leaving, setLeaving] = useState(false)

  const setSigned = (next: Signed | null) => {
    setSignedState(next)
    onHeld(next !== null)
  }
  const forget = (rejected: boolean) => {
    setSigned(null)
    if (rejected) toast.error(m.setup_token_rejected(), { id: "setup" })
  }

  return (
    <SizedCard leaving={leaving}>
      <div className="flex items-center justify-between gap-3">
        <Brand />
        {signed ? (
          <div className="-my-2 flex items-center">
            <EntryLink className="text-row" onClick={() => forget(false)}>
              {m.setup_forget()}
            </EntryLink>
          </div>
        ) : null}
      </div>
      <h1 className="mt-10 text-display font-semibold tracking-tight">
        {m.setup_title()}
      </h1>
      {signed ? (
        <SetupSections
          key={signed.token}
          token={signed.token}
          initial={signed.instance}
          onRejected={() => forget(true)}
          onLeave={() => setLeaving(true)}
        />
      ) : (
        <TokenForm onSigned={setSigned} />
      )}
    </SizedCard>
  )
}

// The instance call turns a refused token into SessionRejected; a user's session reads networks but not the instance's reset inventory (403).
const tokenFailure = (error: unknown): string =>
  error instanceof SessionRejected
    ? m.setup_token_rejected()
    : (failureText(error, {
        forbidden: () => m.setup_token_not_instance(),
        unknown: () => m.entry_unavailable(),
      }) ?? m.entry_unavailable())

function TokenForm({ onSigned }: { onSigned: (signed: Signed) => void }) {
  const [pending, setPending] = useState(false)
  const [invalid, setInvalid] = useState(false)
  const field = useRef<HTMLInputElement>(null)

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending) return
    const token = formText(new FormData(event.currentTarget), "token").trim()
    if (!token) {
      setInvalid(true)
      return
    }
    setPending(true)
    try {
      // Nothing to drop yet: a refusal is reported below.
      const instance = await readInstance(instanceCall(token, () => undefined))
      toast.dismiss("setup")
      onSigned({ token, instance })
    } catch (error) {
      setPending(false)
      setInvalid(true)
      toast.error(tokenFailure(error), { id: "setup" })
      field.current?.select()
    }
  }

  return (
    <form
      method="post"
      action="/setup"
      className="mt-7"
      onSubmit={(event) => void submit(event)}
    >
      <FieldGroup className="gap-5">
        <div className="flex flex-col gap-2">
          <TextField
            size="lg"
            ref={field}
            id="setup-token"
            placeholder={m.setup_token_placeholder()}
            label={m.setup_token()}
            name="token"
            type="password"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            autoFocus
            required
            maxLength={512}
            invalid={invalid}
            onInput={() => setInvalid(false)}
          />
          <code className="font-mono text-meta break-words text-muted-foreground select-all">
            {TOKEN_COMMAND}
          </code>
        </div>
        <EntrySubmit pending={pending} className="mt-2">
          {m.setup_continue()}
        </EntrySubmit>
      </FieldGroup>
    </form>
  )
}

/**
 * The card eases to its content's height when the step changes, so the
 * token step and the sections never jump.
 */
function SizedCard({
  leaving,
  children,
}: {
  leaving: boolean
  children: ReactNode
}) {
  const content = useRef<HTMLDivElement>(null)
  const [height, setHeight] = useState<number | undefined>(undefined)
  useLayoutEffect(() => {
    const element = content.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      const size = entry?.borderBoxSize[0]?.blockSize
      if (size !== undefined) setHeight(size)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  return (
    <div
      data-leaving={leaving}
      style={{ height, ...ENTRY_ENTER }}
      className={cn(
        ENTRY_CARD,
        "overflow-hidden transition-[height,opacity,transform,filter]"
      )}
    >
      <div ref={content} className="flex flex-col p-8">
        {children}
      </div>
    </div>
  )
}
