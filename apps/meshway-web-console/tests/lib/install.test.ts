import assert from "node:assert/strict"
import { test } from "vitest"

import { installCommands, installTarget } from "@/lib/install"

test("the install commands pin the build's release and Server", () => {
  const target = installTarget({
    VITE_MESHWAY_SERVER_URL: "https://mesh.example.com/",
    VITE_MESHWAY_RELEASE: "v0.1.0-alpha.3",
  })
  assert.deepEqual(target, {
    server: "https://mesh.example.com",
    release: "v0.1.0-alpha.3",
    repository: "JDScript/meshway",
  })
  const commands = installCommands(target)
  assert.equal(
    commands.install,
    [
      "curl -fsSLO https://github.com/JDScript/meshway/releases/download/v0.1.0-alpha.3/install.sh",
      "sudo sh install.sh --version v0.1.0-alpha.3 --server https://mesh.example.com",
    ].join("\n")
  )
  // The key is read from a file, never written into the command.
  assert.equal(
    commands.join,
    "sudo meshway up --enrollment-key @/root/enrollment-key"
  )
  assert.deepEqual(commands.missing, [])
})

test("without the build's values the commands keep placeholders, said as missing", () => {
  const commands = installCommands(installTarget({ VITE_MESHWAY_RELEASE: " " }))
  assert.match(commands.install, /download\/<RELEASE>\/install\.sh/)
  assert.match(commands.install, /--server <SERVER_URL>$/)
  assert.deepEqual(commands.missing, ["server", "release"])
})
