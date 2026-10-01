import { spawn } from "node:child_process";
import type { Readable } from "node:stream";
import type { AgentDataplaneSnapshot } from "../../packages/control-client/src/index.ts";
import { packetProof } from "./packet-readiness.ts";

export type PacketLogProcess = {
  stdout: Readable | null;
  stderr: Readable | null;
  once(event: "error", listener: (error: Error) => void): unknown;
  once(event: "close", listener: (code: number | null, signal: NodeJS.Signals | null) => void): unknown;
  kill(signal: NodeJS.Signals): boolean;
};
export type PacketLogLimits = { lineBytes: number; totalBytes: number; timeoutMs: number };
const LIMITS: PacketLogLimits = { lineBytes: 64 * 1024, totalBytes: 16 * 1024 * 1024, timeoutMs: 10_000 };

/**
 * Scan complete available history, keeping one bounded line, never the whole log.
 * Agent stdout is JSONL; malformed/truncated output could hide a withdrawal and
 * therefore invalidates the scan. Stderr diagnostics are discarded, but any
 * dataplane event there fails closed: separate pipes cannot prove event ordering.
 * Only complete, successful reads may validate the LAST managed_dataplane event.
 * No child output or exception detail is returned, printed, or placed in errors.
 */
export function readPacketLog(
  child: PacketLogProcess,
  accepts: (latestEvent: string) => boolean,
  limits: PacketLogLimits = LIMITS,
): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    let bytes = 0;
    let latest: string | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const complete = (ready: boolean) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      latest = undefined;
      resolve(ready);
    };
    const fail = () => {
      if (settled) return;
      complete(false);
      // Kill only the bounded log-reader CLI, never the container/Agent.
      try { child.kill("SIGKILL"); } catch { /* Private child diagnostics suppressed. */ }
    };
    const streams: { ended: boolean; pending: number }[] = [];
    if (child.stdout === null || child.stderr === null) { fail(); return; }
    for (const [index, stream] of [child.stdout, child.stderr].entries()) {
      const state = { ended: false, pending: 0 };
      streams.push(state);
      let pieces: Buffer[] = [];
      const line = (part: Buffer) => {
        const body = pieces.length === 0 ? part : Buffer.concat([...pieces, part], state.pending + part.length);
        pieces = [];
        state.pending = 0;
        try {
          const text = new TextDecoder("utf-8", { fatal: true }).decode(body);
          if (index === 1) {
            if (text.includes("managed_dataplane")) fail();
            return;
          }
          if (!text.trim()) return;
          const value: unknown = JSON.parse(text);
          if (typeof value !== "object" || value === null || Array.isArray(value)) { fail(); return; }
          if ("event" in value && value.event === "managed_dataplane") latest = text;
        } catch { fail(); }
      };
      stream.on("data", (chunk: unknown) => {
        if (settled) return;
        if (!Buffer.isBuffer(chunk)) { fail(); return; }
        bytes += chunk.length;
        if (bytes > limits.totalBytes) { fail(); return; }
        let offset = 0;
        while (offset < chunk.length && !settled) {
          const end = chunk.indexOf(10, offset);
          const part = chunk.subarray(offset, end === -1 ? chunk.length : end);
          if (state.pending + part.length > limits.lineBytes) { fail(); return; }
          if (end === -1) {
            // Copy a partial line so it cannot pin a much larger input chunk.
            pieces.push(Buffer.from(part));
            state.pending += part.length;
            break;
          }
          line(part);
          offset = end + 1;
        }
      });
      stream.once("end", () => { state.ended = true; pieces = []; });
      stream.once("error", fail);
    }
    child.once("error", fail);
    child.once("close", (code, signal) => {
      if (settled) return;
      if (code !== 0 || signal !== null || !streams.every((stream) => stream.ended && stream.pending === 0) || latest === undefined) {
        complete(false);
        return;
      }
      try { complete(accepts(latest)); } catch { complete(false); }
    });
    if (!Object.values(limits).every((value) => Number.isSafeInteger(value) && value > 0)) fail();
    else timer = setTimeout(fail, limits.timeoutMs);
  });
}

/** Caller already holds demo.py's shared performance lock and ownership fence. */
export async function dockerPacketLogReady(
  name: string, startedAt: string, snapshot: AgentDataplaneSnapshot, lockFd: number,
): Promise<boolean> {
  if (!Number.isFinite(Date.parse(startedAt)) || !Number.isSafeInteger(lockFd) || lockFd < 3) return false;
  try {
    // No --tail: receipt volume must not discard a generation's first proof.
    // Retain the same flock in the Docker child even if the Node parent exits.
    const child = spawn("docker", ["logs", "--since", startedAt, name], { stdio: ["ignore", "pipe", "pipe", lockFd] });
    return await readPacketLog(child, (latest) => packetProof(latest, snapshot));
  } catch { return false; }
}
