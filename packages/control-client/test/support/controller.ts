import type { TestContext } from "node:test";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { ControlClient } from "../../src/index.ts";

export async function controller(t: TestContext, registrationEnabled = true, userAgent?: string) {
  const parent = join(tmpdir(), "opencode");
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(await realpath(parent), "meshway-ts-sdk-"));
  const binary = process.env.MESHWAY_CONTROLLER_BINARY ?? resolve(import.meta.dirname, "../../../..", "target/debug/meshway-controller");
  const child = spawn(binary, ["--data-dir", join(directory, "instance"), "--listen", "127.0.0.1:0", "--registration-enabled", String(registrationEnabled)], { stdio: ["ignore", "pipe", "pipe"] });
  const exited = new Promise<number | null>(resolveExit => child.once("close", resolveExit));
  child.stderr.resume();
  t.after(async () => {
    child.kill("SIGTERM");
    const kill = setTimeout(() => child.kill("SIGKILL"), 3000);
    try { await exited; } finally { clearTimeout(kill); await rm(directory, { recursive: true, force: true }); }
  });
  const baseUrl = await new Promise<string>((ready, reject) => {
    const timeout = setTimeout(() => reject(new Error("Controller startup timed out")), 5000);
    let output = "";
    child.once("error", () => { clearTimeout(timeout); reject(new Error("Build the real meshway-controller binary before SDK tests")); });
    child.once("exit", () => { clearTimeout(timeout); reject(new Error("Controller exited during startup")); });
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      output = (output + chunk).slice(-1024);
      const match = /^Listening on (127\.0\.0\.1:\d+)$/m.exec(output);
      if (match?.[1]) { clearTimeout(timeout); ready(`http://${match[1]}`); }
    });
  });
  return new ControlClient({ baseUrl, ...(userAgent === undefined ? {} : { fetch: (input, init) => {
    const headers = new Headers(init?.headers);
    headers.set("user-agent", userAgent);
    return globalThis.fetch(input, { ...init, headers });
  } }) });
}
