import { spawn } from "node:child_process";

/** Never let subprocess errors serialize argv, input, stdout, or stderr. */
export async function command(
  executable: string,
  args: string[],
  options: { timeout?: number; input?: string; cwd?: string } = {},
): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd: options.cwd, stdio: ["pipe", "pipe", "pipe"] });
    const chunks: Buffer[] = [];
    let bytes = 0;
    let exceeded = false;
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; child.kill("SIGKILL"); }, options.timeout ?? 30_000);
    child.stdout.on("data", (chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes <= 4 * 1024 * 1024) chunks.push(chunk);
      else { exceeded = true; child.kill("SIGKILL"); }
    });
    child.stderr.resume();
    child.stdin.on("error", () => {});
    child.once("error", () => { clearTimeout(timer); reject(new Error("subprocess could not start; private details suppressed")); });
    child.once("close", (code) => {
      clearTimeout(timer);
      if (code !== 0 || timedOut || exceeded) reject(new Error(`subprocess failed (exit ${code ?? "signal"}, timeout ${timedOut}, output_limit ${exceeded}); private details suppressed`));
      else resolve(Buffer.concat(chunks).toString("utf8"));
    });
    child.stdin.end(options.input ?? "");
  });
}

export function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("expected a JSON object");
  return Object.fromEntries(Object.entries(value));
}

export function parse(text: string): Record<string, unknown> {
  const value: unknown = JSON.parse(text);
  return record(value);
}

export function text(value: unknown): string {
  if (typeof value !== "string") throw new Error("expected a string");
  return value;
}

function number(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error("expected a nonnegative integer");
  return value;
}

export function boolean(value: unknown): boolean {
  if (typeof value !== "boolean") throw new Error("expected a boolean");
  return value;
}

export function array(value: unknown): unknown[] {
  if (!Array.isArray(value)) throw new Error("expected an array");
  return value;
}

export type SocketObservation = {
  listeners: { protocol: "tcp" | "udp"; host: string; port: number }[];
  established: { local_host: string; local_port: number; peer_host: string; peer_port: number }[];
  agent_running: boolean;
  agent_udp_ports: number[];
  controller_running: boolean;
  ipv4_forwarding: boolean;
  ipv6_forwarding: boolean;
  interfaces: string[];
};

export function sockets(value: Record<string, unknown>): SocketObservation {
  return {
    listeners: array(value.listeners).map((item) => {
      const entry = record(item);
      if (entry.protocol !== "tcp" && entry.protocol !== "udp") throw new Error("invalid socket protocol");
      return { protocol: entry.protocol, host: text(entry.host), port: number(entry.port) };
    }),
    established: array(value.established).map((item) => {
      const entry = record(item);
      return { local_host: text(entry.local_host), local_port: number(entry.local_port), peer_host: text(entry.peer_host), peer_port: number(entry.peer_port) };
    }),
    agent_running: boolean(value.agent_running), controller_running: boolean(value.controller_running),
    agent_udp_ports: array(value.agent_udp_ports).map(number),
    ipv4_forwarding: boolean(value.ipv4_forwarding), ipv6_forwarding: boolean(value.ipv6_forwarding),
    interfaces: array(value.interfaces).map(text),
  };
}
