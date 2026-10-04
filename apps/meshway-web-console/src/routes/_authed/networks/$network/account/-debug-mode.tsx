import { SwitchRow } from "@/components/chrome/form"
import { Section } from "@/components/chrome/section"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { m } from "@/paraglide/messages.js"
import { setDebugEnabled, useDebugEnabled } from "@/state/debug"

/** This browser's debug mode, and the keys that show or hide its panel. */
export function DebugMode() {
  const enabled = useDebugEnabled()
  return (
    <Section
      title={m.account_debug()}
      action={
        <KbdGroup>
          <Kbd>Ctrl</Kbd>
          <Kbd>`</Kbd>
        </KbdGroup>
      }
    >
      <SwitchRow
        id="debug-mode"
        label={m.account_debug_mode()}
        checked={enabled}
        onCheckedChange={setDebugEnabled}
      />
    </Section>
  )
}
