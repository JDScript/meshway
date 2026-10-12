// How a Linux device installs the Node and joins (docs/v2/install.md). The
// Server origin the Node reaches and the release to install are the
// deployment's, set when the console is built; without them the commands
// keep their placeholders, said as such.

export type InstallTarget = {
  /** The Server origin a Node reaches (it may differ from the console's). */
  server: string | null
  /** The release tag to install; the installer takes no unpinned pre-release. */
  release: string | null
  /** The GitHub repository the release is in. */
  repository: string
}

const SERVER = "<SERVER_URL>"
const RELEASE = "<RELEASE>"
/** Where the enrolment key is saved: a file, so it stays out of shell history. */
export const KEY_FILE = "/root/enrollment-key"

const present = (value: unknown) =>
  typeof value === "string" && value.trim() !== "" ? value.trim() : null

/** The deployment's install target, from the build's environment. */
export function installTarget(
  env: Readonly<Record<string, unknown>> = import.meta.env
): InstallTarget {
  return {
    server: present(env.VITE_MESHWAY_SERVER_URL)?.replace(/\/+$/, "") ?? null,
    release: present(env.VITE_MESHWAY_RELEASE),
    repository: present(env.VITE_MESHWAY_RELEASE_REPO) ?? "JDScript/meshway",
  }
}

/** The steps, in order: install the service, then join with a saved key. */
export function installCommands(target: InstallTarget) {
  const release = target.release ?? RELEASE
  return {
    install: [
      `curl -fsSLO https://github.com/${target.repository}/releases/download/${release}/install.sh`,
      `sudo sh install.sh --version ${release} --server ${target.server ?? SERVER}`,
    ].join("\n"),
    join: `sudo meshway up --enrollment-key @${KEY_FILE}`,
    status: "sudo meshway status",
    /** Placeholders the build left in. */
    missing: [
      ...(target.server === null ? (["server"] as const) : []),
      ...(target.release === null ? (["release"] as const) : []),
    ],
  }
}
