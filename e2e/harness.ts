import { expect } from "@playwright/test";
import { execFile, spawn } from "node:child_process";
import { createServer } from "node:net";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";

export const root = resolve(import.meta.dirname, "..");
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
  await new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  return address.port;
}

export function service(command: string, args: string[], environment: NodeJS.ProcessEnv = process.env) {
  const child = spawn(command, args, { cwd: root, env: environment, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk: Buffer) => { output = `${output}${chunk.toString()}`.slice(-32_768); });
  }
  let launchError: Error | undefined;
  const exited = new Promise<void>((resolve) => {
    child.once("exit", () => resolve());
    child.once("error", (error) => { launchError = error; resolve(); });
  });
  return {
    async ready(url: string, status: number) {
      await Promise.race([
        expect.poll(async () => {
          try {
            const response = await fetch(url, { redirect: "manual", signal: AbortSignal.timeout(1000) });
            await response.body?.cancel();
            return response.status;
          } catch { return 0; }
        }, { timeout: 25_000, message: `Service readiness: ${url}` }).toBe(status),
        exited.then(() => { throw new Error(`Service stopped during startup: ${launchError?.message ?? output}`); }),
      ]);
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
