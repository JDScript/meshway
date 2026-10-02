import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { listNetworks, listNodes } from "../../../packages/control-client/src/index.ts";
import { executable, root, run, service, unusedPort } from "./processes.ts";

const directory = await mkdtemp(join(process.env.TMPDIR ?? tmpdir(), "meshway-apple-signin-"));
const controllerPort = await unusedPort(), webPort = await unusedPort();
const controllerURL = `http://127.0.0.1:${controllerPort}`, consoleURL = `http://127.0.0.1:${webPort}`;
const data = join(directory, "controller"), adminFile = join(data, "admin.token");
const controller = service(executable("meshway-controller"), ["serve", "--data-dir", data, "--bind", `127.0.0.1:${controllerPort}`]);
let web: ReturnType<typeof service> | undefined;
let app: ReturnType<typeof spawn> | undefined;
let appExited: Promise<void> | undefined;
try {
  await controller.ready(`${controllerURL}/api/v1/overview`, 401);
  web = service(process.execPath, [join(root, "apps/web/node_modules/next/dist/bin/next"), "start", join(root, "apps/web"), "--hostname", "127.0.0.1", "--port", String(webPort)], {
    ...process.env, MESHWAY_CONTROLLER_URL: controllerURL, NEXT_TELEMETRY_DISABLED: "1",
  });
  await web.ready(`${consoleURL}/login`, 200);
  const appExecutable = process.argv[2];
  if (appExecutable) {
    app = spawn(appExecutable, ["-controllerURL", consoleURL], { cwd: root, stdio: "ignore" });
    let launchError = false;
    appExited = new Promise<void>((resolve) => {
      app?.once("exit", () => resolve());
      app?.once("error", () => { launchError = true; resolve(); });
    });
    await delay(1500);
    if (launchError || app.exitCode !== null || app.signalCode !== null) throw new Error("Unsigned macOS app failed to remain running");
    console.log("Unsigned macOS app launched against isolated console; native sign-in UI was not automated.");
  }
  const result = await run("swift", ["test", "--package-path", join(root, "apps/apple/MeshwayKit"), "--filter", "liveSwiftKeychainFFIProxyEnrollmentRecoveryAndRevocation"], {
    cwd: root, maxBuffer: 4 * 1024 * 1024,
    env: { ...process.env, MESHWAY_APP_TEST_CONSOLE: consoleURL, MESHWAY_APP_TEST_CONTROLLER: controllerURL,
      MESHWAY_APP_TEST_ADMIN_FILE: adminFile, MESHWAY_APP_TEST_NODE: process.execPath,
      MESHWAY_APP_TEST_HELPER: join(root, "apps/apple/scripts/signin-test-approval.ts") },
  });
  process.stdout.write(result.stdout); process.stderr.write(result.stderr);
  const admin = (await readFile(adminFile, "utf8")).trim();
  const networkId = (await listNetworks(controllerURL, admin)).networks[0]?.network_id;
  if (!networkId) throw new Error("Isolated controller has no network");
  const nodes = (await listNodes(controllerURL, admin, networkId)).nodes;
  if (nodes.length !== 2 || !nodes.some(node => node.name === "Swift Keychain smoke" && node.revoked_at)
    || !nodes.some(node => node.name === "Swift inventory peer" && !node.revoked_at)) {
    throw new Error("Swift smoke did not leave the revoked self and real inventory peer");
  }
  console.log("Verified: Swift + real Keychain → UniFFI → generated Rust client → console proxy → controller; recovery, heartbeat, real peer inventory, DNS and revocation.");
  console.log("Browser approval used a test-only generated-client fixture; ASWebAuthenticationSession UI was not driven.");
} finally {
  if (app && app.exitCode === null && app.signalCode === null) {
    app.kill("SIGTERM");
    await Promise.race([appExited, delay(3000)]);
    if (app.exitCode === null && app.signalCode === null) { app.kill("SIGKILL"); await appExited; }
  }
  await web?.stop(); await controller.stop(); await rm(directory, { recursive: true, force: true });
}
