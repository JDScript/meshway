// Native App fixtures use plain Node process/HTTP helpers, with no browser runner.
import { execFile, spawn } from "node:child_process";
import { createServer } from "node:net";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";

export const root = resolve(import.meta.dirname, "../../..");
export const run = promisify(execFile);
export const executable = (name: string) =>
  join(root, "target", "debug", `${name}${process.platform === "win32" ? ".exe" : ""}`);

export async function unusedPort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    server.close();
    throw new Error("Expected a TCP address");
  }
  await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  return address.port;
}

export function service(command: string, args: string[], environment: NodeJS.ProcessEnv = process.env) {
  const child = spawn(command, args, { cwd: root, env: environment, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk: Buffer) => { output = `${output}${chunk.toString()}`.slice(-32_768); });
  }
  let launchError: Error | undefined;
  const exited = new Promise<void>(resolve => {
    child.once("exit", () => resolve());
    child.once("error", error => { launchError = error; resolve(); });
  });
  return {
    async ready(url: string, status: number) {
      url = url.replace(/\/api\/v1\/overview$/, "/api/v1/networks");
      const cancel = new AbortController();
      const poll = async () => {
        const deadline = performance.now() + 25_000;
        while (!cancel.signal.aborted && performance.now() < deadline) {
          try {
            const response = await fetch(url, { redirect: "manual", signal: AbortSignal.any([cancel.signal, AbortSignal.timeout(1000)]) });
            await response.body?.cancel();
            if (response.status === status) return;
          } catch {
            if (cancel.signal.aborted) return;
          }
          await delay(100, undefined, { signal: cancel.signal });
        }
        if (!cancel.signal.aborted) throw new Error(`Service readiness deadline elapsed: ${url}`);
      };
      try {
        await Promise.race([
          poll(),
          exited.then(() => { throw new Error(`Service stopped during startup: ${launchError?.message ?? output}`); }),
        ]);
      } finally { cancel.abort(); }
    },
    async stop() {
      if (child.exitCode !== null || child.signalCode !== null || launchError) return;
      child.kill("SIGINT");
      const deadline = new AbortController();
      try {
        await Promise.race([
          exited,
          delay(5000, undefined, { signal: deadline.signal }).then(async () => {
            child.kill("SIGKILL");
            await exited;
          }),
        ]);
      } finally { deadline.abort(); }
    },
  };
}
