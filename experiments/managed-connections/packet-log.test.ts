import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { PassThrough } from "node:stream";
import test from "node:test";
import { readPacketLog } from "./packet-log.ts";
import type { PacketLogLimits, PacketLogProcess } from "./packet-log.ts";

const active = JSON.stringify({ event: "managed_dataplane", tun_ready: true, reason: "generation_active", revision: 12, digest: "d".repeat(64) });
const withdrawn = JSON.stringify({ event: "managed_dataplane", tun_ready: false, reason: "generation_withdrawn" });
const limits: PacketLogLimits = { lineBytes: 1024, totalBytes: 1024 * 1024, timeoutMs: 1_000 };
class FakeChild extends EventEmitter implements PacketLogProcess {
  stdout = new PassThrough();
  stderr = new PassThrough();
  signals: NodeJS.Signals[] = [];
  kill(signal: NodeJS.Signals): boolean { this.signals.push(signal); return true; }
}
async function close(child: FakeChild, code = 0, signal: NodeJS.Signals | null = null) {
  const ended = Promise.all([once(child.stdout, "end"), once(child.stderr, "end")]);
  child.stdout.end(); child.stderr.end();
  await ended;
  child.emit("close", code, signal);
}
function start(child: FakeChild, options = limits) {
  const seen: string[] = [];
  const ready = readPacketLog(child, (latest) => { seen.push(latest); return latest === active; }, options);
  return { ready, seen };
}

test("generation proof survives more than 2048 later receipts without returning receipt/secret text", async () => {
  const child = new FakeChild();
  const scan = start(child);
  child.stdout.write(active + "\n");
  for (let sequence = 0; sequence < 5000; sequence++) child.stdout.write(JSON.stringify({ receipt: sequence, credential: "unrelated-secret" }) + "\n");
  child.stderr.write("private retry diagnostic unrelated-secret\n");
  await close(child);
  assert.equal(await scan.ready, true);
  assert.deepEqual(scan.seen, [active], "only the latest managed event reaches the existing proof validator");
  assert.deepEqual(child.signals, []);
});

test("later withdraw, lease expiry, runtime/device failure or incomplete managed event invalidates older ready", async () => {
  for (const later of [withdrawn,
    JSON.stringify({ event: "managed_dataplane", tun_ready: false, reason: "lease_expired" }),
    JSON.stringify({ event: "managed_dataplane", tun_ready: false, reason: "runtime_or_device_failure" }),
    JSON.stringify({ event: "managed_dataplane" }),
  ]) {
    const child = new FakeChild(); const scan = start(child);
    child.stdout.write(active + "\n" + later + "\n");
    for (let index = 0; index < 2200; index++) child.stdout.write('{"receipt":true}\n');
    await close(child);
    assert.equal(await scan.ready, false);
    assert.deepEqual(scan.seen, [later]);
  }
});

test("latest replacement generation is selected, even across split JSON/UTF-8 chunks", async () => {
  const child = new FakeChild(); const scan = start(child);
  const bytes = Buffer.from(withdrawn + '\n{"receipt":"心跳"}\n' + active + "\n");
  for (let offset = 0; offset < bytes.length; offset += 3) child.stdout.write(bytes.subarray(offset, offset + 3));
  await close(child);
  assert.equal(await scan.ready, true);
  assert.deepEqual(scan.seen, [active]);
});

test("missing evidence and a different revision/digest are not replaced with a previous active event", async () => {
  for (const log of ['{"receipt":true}\n', active + "\n" + active.replace('"revision":12', '"revision":11') + "\n",
    active + "\n" + active.replace("d".repeat(64), "e".repeat(64)) + "\n"]) {
    const child = new FakeChild(); const scan = start(child);
    child.stdout.write(log); await close(child);
    assert.equal(await scan.ready, false);
  }
});

test("malformed JSON, non-object output, truncated final line or invalid UTF-8 cannot keep old ready", async () => {
  for (const suffix of [Buffer.from('{"event":"managed_dataplane", BROKEN}\n'), Buffer.from("[]\n"),
    Buffer.from("null\n"), Buffer.from('{"event":"managed_dataplane"'), Buffer.from(active),
    Buffer.from([0xff, 10])]) {
    const child = new FakeChild(); const scan = start(child);
    child.stdout.write(active + "\n"); child.stdout.write(suffix);
    await close(child);
    assert.equal(await scan.ready, false);
    assert.deepEqual(scan.seen, []);
  }
});

test("line and aggregate byte limits fail closed and kill only the log-reader process", async () => {
  for (const options of [{ ...limits, lineBytes: 256 }, { ...limits, totalBytes: 300 }]) {
    const child = new FakeChild(); const scan = start(child, options);
    child.stdout.write(active + "\n");
    child.stdout.write(JSON.stringify({ receipt: "secret".repeat(60) }) + "\n");
    await close(child);
    assert.equal(await scan.ready, false);
    assert.deepEqual(scan.seen, []);
    assert.deepEqual(child.signals, ["SIGKILL"]);
  }
});

test("stderr is also byte bounded and cannot carry unordered dataplane proof", async () => {
  for (const stderr of [withdrawn + "\n", active + "\n", "private-secret".repeat(100) + "\n", "truncated private diagnostic"]) {
    const child = new FakeChild(); const scan = start(child);
    child.stdout.write(active + "\n"); child.stderr.write(stderr);
    await close(child);
    assert.equal(await scan.ready, false);
    assert.deepEqual(scan.seen, []);
  }
});

test("timeout, nonzero exit, signal, spawn error and stream error cannot return green or leak private errors", async () => {
  const timeout = new FakeChild(); const scan = start(timeout, { ...limits, timeoutMs: 5 });
  timeout.stdout.write(active + "\n");
  assert.equal(await scan.ready, false);
  assert.deepEqual(scan.seen, []);
  assert.deepEqual(timeout.signals, ["SIGKILL"]);
  for (const failure of ["exit", "signal", "spawn", "stream"] as const) {
    const child = new FakeChild(); const scan = start(child);
    child.stdout.write(active + "\n");
    if (failure === "spawn") child.emit("error", new Error("admin-bearer=private-secret"));
    if (failure === "stream") child.stdout.emit("error", new Error("node-key=private-secret"));
    await close(child, failure === "exit" ? 1 : 0, failure === "signal" ? "SIGTERM" : null);
    assert.equal(await scan.ready, false);
    assert.deepEqual(scan.seen, []);
  }
});

test("private validator errors are suppressed, and close without complete streams is unknown", async () => {
  const child = new FakeChild();
  const ready = readPacketLog(child, () => { throw new Error("private-secret"); }, limits);
  child.stdout.write(active + "\n"); await close(child);
  assert.equal(await ready, false);
  const incomplete = new FakeChild(); const scan = start(incomplete);
  incomplete.stdout.write(active + "\n"); incomplete.emit("close", 0, null);
  assert.equal(await scan.ready, false);
  assert.deepEqual(scan.seen, []);
});
