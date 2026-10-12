import { CopyField } from "@/components/chrome/glass-dialog"
import { installCommands, installTarget, KEY_FILE } from "@/lib/install"
import { m } from "@/paraglide/messages.js"

/** A command block that breaks at its lines, not anywhere. */
function Command({ label, value }: { label: string; value: string }) {
  return (
    <CopyField label={label} value={value}>
      {value.split("\n").map((line) => (
        <span key={line} className="block">
          {line}
        </span>
      ))}
    </CopyField>
  )
}

/**
 * Installing the Node on a Linux device and joining with an enrolment key,
 * as docs/v2/install.md has it. `joining`: the key was just created, so
 * only the step that uses it is left.
 */
export function InstallSteps({ joining = false }: { joining?: boolean }) {
  const commands = installCommands(installTarget())
  return (
    <div className="flex flex-col gap-3">
      {joining ? null : (
        <Command
          label={m.nodes_install_step_install()}
          value={commands.install}
        />
      )}
      <Command
        label={
          joining
            ? m.nodes_install_step_join_now({ file: KEY_FILE })
            : m.nodes_install_step_join({ file: KEY_FILE })
        }
        value={`${commands.join}\n${commands.status}`}
      />
      {!joining && commands.missing.length > 0 ? (
        <p className="text-meta text-warning">
          {commands.missing.length === 2
            ? m.nodes_install_missing_both()
            : commands.missing[0] === "server"
              ? m.nodes_install_missing_server()
              : m.nodes_install_missing_release()}
        </p>
      ) : (
        <p className="text-meta text-muted-foreground">
          {m.nodes_install_requirements()}
        </p>
      )}
    </div>
  )
}
