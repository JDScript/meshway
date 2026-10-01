// Real unsigned Debug windows, isolated controller/Web/Keychain. No App fixtures.
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { createHash, randomBytes, randomUUID } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { homedir, tmpdir } from "node:os";
import { setTimeout as delay } from "node:timers/promises";
import { authorizeDevice, exchangeDevice, getNetwork, listNodes, revokeNode, saveNetwork, setNodeCountry } from "../../../packages/control-client/src/index.ts";
import { executable, root, run, service, unusedPort } from "../../../e2e/harness.ts";

const output = resolve(process.argv[2] ?? join(tmpdir(), "opencode/apple-ui-polish/round-1"));
const capturedFiles = new Set<string>();
await mkdir(output, { recursive: true });
const temporary = await mkdtemp(join(output, ".live-"));
await chmod(temporary, 0o700);
const controllerPort = await unusedPort(), webPort = await unusedPort();
const controllerURL = `http://127.0.0.1:${controllerPort}`, consoleURL = `http://127.0.0.1:${webPort}`;
const data = join(temporary, "controller");
const startController = () => service(executable("meshway-controller"), ["serve", "--data-dir", data, "--bind", `127.0.0.1:${controllerPort}`]);
let controller = startController();
let web: ReturnType<typeof service> | undefined;
let app: ReturnType<typeof spawn> | undefined;
let appExit: Promise<void> | undefined;
let keychainService: string | undefined;
let isolatedBundle = false;
const appExecutable = process.argv[3] ?? join(root, "apps/apple/build/DerivedData/Build/Products/Debug/Meshway.app/Contents/MacOS/Meshway");
let interrupted = false;
process.once("SIGINT", () => { interrupted = true; });
process.once("SIGTERM", () => { interrupted = true; });
async function cleanupNamespace(namespace: string) {
  for (const [command, args] of [
    ["/usr/bin/security", ["delete-generic-password", "-s", namespace, "-a", "identity-journal"]],
    ["/usr/bin/defaults", ["delete", namespace]],
  ] as const) {
    const cleanup = spawn(command, [...args], { stdio: "ignore" });
    await new Promise<void>(resolve => cleanup.once("exit", () => resolve()));
  }
  // defaults delete can leave an empty plist. Remove only our exact UUID file.
  await rm(join(homedir(), "Library/Preferences", `${namespace}.plist`), { force: true });
}
async function waitFor(predicate: () => Promise<boolean>, label: string, timeout = 40_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (interrupted) throw new Error("Snapshot run interrupted");
    if (await predicate()) return;
    await delay(100);
  }
  throw new Error(`Timed out: ${label}`);
}
async function exists(path: string) { try { await readFile(path); return true; } catch { return false; } }
try {
  const bundle = await run("/usr/bin/plutil", ["-extract", "CFBundleIdentifier", "raw", "-o", "-",
    resolve(dirname(appExecutable), "../Info.plist")]);
  if (bundle.stdout.trim() !== "dev.meshway.snapshot.meshway") {
    throw new Error("Build the Debug app with MESHWAY_BUNDLE_PREFIX=dev.meshway.snapshot before capturing");
  }
  isolatedBundle = true;
  await controller.ready(`${controllerURL}/api/v1/overview`, 401);
  web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
    ...process.env, MESHWAY_CONTROLLER_URL: controllerURL, NEXT_TELEMETRY_DISABLED: "1",
  });
  await web.ready(`${consoleURL}/login`, 200);
  const admin = (await readFile(join(data, "admin.token"), "utf8")).trim();
  const network = await getNetwork(controllerURL, admin);
  await saveNetwork(controllerURL, admin, { expected_revision: network.revision, name: network.name,
    dns: { enabled: true, zone: "mesh.example", ttl_seconds: 60 } });
  const names = ["Atlas", "Home studio", "iPhone", "Office workstation — 東京", "Travel companion with a very long device name for truncation review"];
  for (const [index, name] of names.entries()) {
    const verifier = randomBytes(32).toString("base64url");
    const redirect = "meshway://enroll/callback";
    const authorization = await authorizeDevice(controllerURL, admin, {
      name, agent_version: "snapshot-review", redirect_uri: redirect,
      code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    });
    const enrolled = await exchangeDevice(controllerURL, authorization.code, {
      name, agent_version: "snapshot-review", redirect_uri: redirect,
      code_verifier: verifier, node_credential: randomBytes(32).toString("hex"),
    });
    if (index !== 2) {
      const current = await getNetwork(controllerURL, admin);
      await setNodeCountry(controllerURL, admin, { expected_revision: current.revision,
        node_id: enrolled.node.id, country: ["US", "JP", "", "DE", "CA"][index] ?? "US" });
    }
  }
  for (const appearance of ["light", "dark"]) {
    for (const size of ["default", "min", "wide"]) {
      const ipc = join(temporary, `${appearance}-${size}`);
      await mkdir(ipc, { mode: 0o700 });
      keychainService = `dev.meshway.snapshot.${randomUUID()}`;
      // Public cleanup metadata only; never write the credential to a file.
      await writeFile(join(ipc, "namespace"), keychainService, { mode: 0o600 });
      app = spawn(appExecutable, ["-ApplePersistenceIgnoreState", "YES", "-NSQuitAlwaysKeepsWindows", "NO"], { cwd: root, stdio: "ignore", env: { ...process.env,
        MESHWAY_SNAPSHOT_DIR: ipc, MESHWAY_SNAPSHOT_SERVER: consoleURL,
        MESHWAY_SNAPSHOT_KEYCHAIN: keychainService, MESHWAY_SNAPSHOT_APPEARANCE: appearance,
        MESHWAY_SNAPSHOT_SIZE: size } });
      appExit = new Promise<void>((resolve) => { app?.once("exit", () => resolve()); app?.once("error", () => resolve()); });
      await waitFor(async () => {
        if (await exists(join(ipc, "failed")) || app?.exitCode !== null) throw new Error("Debug snapshot app stopped before becoming ready");
        return exists(join(ipc, "ready"));
      }, "AppKit window");
      let sequence = 0;
      async function command(action: string, id = `command-${++sequence}`) {
        await writeFile(join(ipc, "next.json"), JSON.stringify({ action, id }), { mode: 0o600 });
        await rename(join(ipc, "next.json"), join(ipc, "command.json"));
        await waitFor(async () => {
          if (await exists(join(ipc, "failed"))) {
            const reason = (await readFile(join(ipc, "failed"), "utf8")).trim();
            throw new Error(`Debug snapshot operation failed: ${["window", "bitmap", "command", "cancelled", "io"].includes(reason) ? reason : "unknown"}`);
          }
          try { return (await readFile(join(ipc, "done"), "utf8")) === id; } catch { return false; }
        }, action);
      }
      async function capture(state: string) {
        const id = `mac-${state}-${appearance}-${size}`;
        await command("capture", id);
        // A rerun after interruption can otherwise retain an old attached sheet
        // for a state which no longer has one. Replace only this managed ID.
        for (const suffix of ["", "-sheet", "-panel"]) {
          await rm(join(output, `${id}${suffix}.png`), { force: true });
        }
        for (const name of await readdir(ipc)) {
          if (name.startsWith(id) && name.endsWith(".png")) {
            await copyFile(join(ipc, name), join(output, name));
            capturedFiles.add(name);
          }
        }
      }
      await capture("signed-out");
      await command("panel"); await capture("signed-out-panel");
      const signingIn = command("sign-in");
      await waitFor(() => exists(join(ipc, "authorization-request")), "real device-flow request");
      const request = new URL(await readFile(join(ipc, "authorization-request"), "utf8"));
      const required = (name: string) => { const value = request.searchParams.get(name); if (!value) throw new Error("Missing device-flow parameter"); return value; };
      const authorization = await authorizeDevice(controllerURL, admin, {
        redirect_uri: required("redirect_uri"), code_challenge: required("code_challenge"),
        name: required("name"), agent_version: required("agent_version"),
      });
      const callback = new URL(required("redirect_uri"));
      callback.searchParams.set("state", required("state")); callback.searchParams.set("code", authorization.code);
      await writeFile(join(ipc, "authorization-callback"), callback.toString(), { mode: 0o600 });
      await signingIn;
      await command("refresh"); await capture("devices");
      await command("detail"); await capture("detail"); await command("dismiss");
      await command("alias-detail"); await capture("alias-detail"); await command("dismiss");
      await command("network"); await capture("network"); await command("dismiss");
      await command("panel"); await capture("panel");
      await command("search"); await capture("no-matches"); await command("search");
      await controller.stop(); await command("refresh"); await capture("error");
      controller = startController(); await controller.ready(`${controllerURL}/api/v1/overview`, 401);
      const own = (await listNodes(controllerURL, admin)).nodes.find(node => node.name === "Zeyu’s MacBook Pro" && !node.revoked_at);
      if (!own) throw new Error("App did not enroll a real node");
      await revokeNode(controllerURL, admin, { node_id: own.id });
      await command("refresh"); await capture("revoked");
      await command("panel"); await capture("revoked-panel");
      await command("reset");
      app.kill("SIGTERM"); await appExit; app = undefined;
      await cleanupNamespace(keychainService);
      keychainService = undefined;
      console.log(`Captured real windows: ${appearance}/${size}`);
    }
  }
} finally {
  if (app && app.exitCode === null && app.signalCode === null) { app.kill("SIGTERM"); await appExit; }
  if (keychainService) {
    // Delete only this invocation's unique item; never access developer credentials.
    await cleanupNamespace(keychainService);
  }
  if (isolatedBundle) {
    const preferences = spawn("/usr/bin/defaults", ["delete", "dev.meshway.snapshot.meshway"], { stdio: "ignore" });
    await new Promise<void>(resolve => preferences.once("exit", () => resolve()));
    await rm(join(homedir(), "Library/Preferences/dev.meshway.snapshot.meshway.plist"), { force: true });
    await rm(join(homedir(), "Library/Saved Application State/dev.meshway.snapshot.meshway.savedState"), { recursive: true, force: true });
  }
  await web?.stop(); await controller.stop(); await rm(temporary, { recursive: true, force: true });
}
const images = [...capturedFiles].sort();
await writeFile(join(output, "png-paths.txt"), `${images.map(name => join(output, name)).join("\n")}\n`);
console.log(`PNGs: ${output}`);
