import assert from "node:assert/strict";
import { test } from "node:test";
import { EventDecoder, EventStreamError } from "../src/sse.ts";
import { ControlClient } from "../src/transport.ts";
import * as z from "zod";

const operation = {
  method: "GET", path: "/events", authenticated: true, successStatus: 200,
  success: z.strictObject({ type: z.literal("sample"), value: z.string() }),
  errors: [401, 503], problem: z.strictObject({ code: z.string() }),
} as const;

test("SSE framing preserves split UTF-8, CRLF, multiline JSON and explicit ids", () => {
  const text = '\uFEFF: heartbeat\r\nid: first\r\nevent: message\r\ndata: {"value":\r\ndata: "节点"}\r\n\r\ndata: {}\n\n';
  const bytes = new TextEncoder().encode(text);
  for (let split = 0; split <= bytes.length; split++) {
    const decoder = new EventDecoder(1024);
    const frames = [...decoder.push(bytes.subarray(0, split)), ...decoder.push(bytes.subarray(split))];
    assert.deepEqual(frames, [
      { data: '{"value":\n"节点"}', event: "message", id: "first" },
      { data: "{}", event: "message" },
    ]);
  }
});

test("incomplete events and id-only frames do not create resumable application data", () => {
  const decoder = new EventDecoder(1024);
  assert.deepEqual([...decoder.push(new TextEncoder().encode("id: never-applied\n\ndata: partial\nid: later"))], []);
  decoder.finish();
  assert.deepEqual([...decoder.push(new TextEncoder().encode("data:\n\n"))], [{ data: "", event: "message" }]);
});

test("SSE malformed UTF-8 and per-frame budgets fail without retaining raw input", () => {
  const decoder = new EventDecoder(12);
  assert.throws(() => [...decoder.push(new TextEncoder().encode("data: 1234567890123"))], error => error instanceof EventStreamError && error.kind === "body_limit");
  const invalid = new EventDecoder(1024);
  assert.throws(() => [...invalid.push(new Uint8Array([100, 97, 116, 97, 58, 32, 0xff, 10]))], error => error instanceof EventStreamError && error.kind === "decode");
  const lines = new EventDecoder(16);
  assert.throws(() => [...lines.push(new TextEncoder().encode("data: 12345678\ndata: 12345678\n\n"))], EventStreamError);
});

test("comments and retry hints do not create samples or inherit a prior cursor", () => {
  const decoder = new EventDecoder(64);
  const bytes = new TextEncoder();
  for (let i = 0; i < 1000; i++) assert.deepEqual([...decoder.push(bytes.encode(": keepalive\n"))], []);
  assert.deepEqual([...decoder.push(bytes.encode("retry: 999999\n\n"))], []);
  assert.equal(decoder.retryMs, 60_000);
  assert.deepEqual([...decoder.push(bytes.encode("retry: not-an-integer\n\ndata: {}\nid: bad\0id\n\n"))], [{ data: "{}", event: "message" }]);
  assert.equal(decoder.retryMs, 60_000);
});

test("event delivery validates data and reconnects only from the caller's applied cursor", async () => {
  let requests = 0;
  let applied: string | undefined;
  const seen: Array<string | null> = [];
  const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async (_url, init) => {
    requests++;
    const headers = new Headers(init?.headers);
    assert.equal(headers.get("authorization"), "Bearer private");
    assert.equal(headers.get("accept"), "text/event-stream");
    seen.push(headers.get("last-event-id"));
    if (requests === 1) return new Response('id: received-not-applied\ndata: {"type":"sample","value":"first"}\n\n', { headers: { "content-type": "text/event-stream" } });
    return Response.json({ code: "expired" }, { status: 401 });
  } });
  const stream = client.stream(operation, { token: "private", retryDelayMs: 100, maxReconnects: 1, resumeCursor: () => applied });
  const first = await stream.next();
  assert.ok(!first.done && first.value.ok);
  assert.equal(first.value.data.id, "received-not-applied");
  // A receiver may apply an earlier complete snapshot, not the last decoded id.
  applied = "applied-snapshot";
  const disconnected = await stream.next();
  assert.ok(!disconnected.done && !disconnected.value.ok);
  const rejected = await stream.next();
  assert.ok(!rejected.done && !rejected.value.ok && rejected.value.error.kind === "http");
  assert.equal(rejected.value.error.status, 401);
  assert.deepEqual(seen, [null, "applied-snapshot"]);
  assert.equal((await stream.next()).done, true);
});

test("invalid event payloads terminate without advancing a cursor", async () => {
  for (const text of ['data: {"type":"sample","value":1}\n\n', 'event: other\ndata: {"type":"sample","value":"x"}\n\n', 'data: not-json\n\n']) {
    const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async () => new Response(text, { headers: { "content-type": "text/event-stream" } }) });
    const stream = client.stream(operation, { token: "private" });
    assert.deepEqual((await stream.next()).value, { ok: false, error: { kind: "decode", status: 200 } });
    assert.equal((await stream.next()).done, true);
  }
});

test("aborting a pending event stream closes its reader", async () => {
  let cancelled = false;
  const signal = new AbortController();
  const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async () => new Response(new ReadableStream({ cancel() { cancelled = true; } }), { headers: { "content-type": "text/event-stream" } }) });
  const stream = client.stream(operation, { token: "private", signal: signal.signal });
  const pending = stream.next();
  await new Promise(resolve => setImmediate(resolve));
  signal.abort();
  assert.deepEqual((await pending).value, { ok: false, error: { kind: "cancelled" } });
  await stream.return(undefined);
  assert.equal(cancelled, true);
});

test("SSE rejects duplicate keys in data and HTTP problems", async () => {
  for (const response of [
    new Response('data: {"type":"sample","value":"first","\\u0076alue":"second"}\n\n', { headers: { "content-type": "text/event-stream" } }),
    new Response('{"code":"first","code":"second"}', { status: 401, headers: { "content-type": "application/json" } }),
  ]) {
    const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async () => response });
    const stream = client.stream(operation, { token: "private", maxReconnects: 0 });
    assert.deepEqual((await stream.next()).value, { ok: false, error: { kind: response.status === 200 ? "decode" : "http", status: response.status } });
    assert.equal((await stream.next()).done, true);
  }
});

test("idle SSE cancels the connection and resumes only from the caller-applied cursor", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let cancelled = false;
  let applied: string | undefined;
  const cursors: Array<string | null> = [];
  const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async (_url, init) => {
    cursors.push(new Headers(init?.headers).get("last-event-id"));
    if (cursors.length === 2) return Response.json({ code: "expired" }, { status: 401 });
    return new Response(new ReadableStream<Uint8Array>({
      start(controller) { controller.enqueue(new TextEncoder().encode('id: merely-decoded\ndata: {"type":"sample","value":"first"}\n\n')); },
      cancel() { cancelled = true; },
    }), { headers: { "content-type": "text/event-stream" } });
  } });
  const stream = client.stream(operation, { token: "private", idleTimeoutMs: 1000, retryDelayMs: 100, maxReconnects: 1, resumeCursor: () => applied });
  const first = await stream.next();
  assert.ok(!first.done && first.value.ok);
  // Consumer processing time is not network inactivity: no pending I/O yet.
  t.mock.timers.tick(10_000);
  assert.equal(cancelled, false);
  applied = "complete-applied-snapshot";
  const pending = stream.next();
  await new Promise(resolve => setImmediate(resolve));
  t.mock.timers.tick(1000);
  assert.deepEqual((await pending).value, { ok: false, error: { kind: "network", status: 200 } });
  assert.equal(cancelled, true);
  const next = stream.next();
  await new Promise(resolve => setImmediate(resolve));
  t.mock.timers.tick(100);
  const rejected = await next;
  assert.ok(!rejected.done && !rejected.value.ok && rejected.value.error.kind === "http");
  assert.equal(rejected.value.error.status, 401);
  assert.deepEqual(cursors, [null, applied]);
  assert.equal((await stream.next()).done, true);
});

test("real keepalive bytes refresh idle deadlines, empty chunks do not", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let source: ReadableStreamDefaultController<Uint8Array> | undefined;
  let cancelled = false;
  const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async () => new Response(new ReadableStream<Uint8Array>({
    start(controller) { source = controller; }, cancel() { cancelled = true; },
  }), { headers: { "content-type": "text/event-stream" } }) });
  const stream = client.stream(operation, { token: "private", idleTimeoutMs: 1000, maxReconnects: 0 });
  const pending = stream.next();
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(source);
  t.mock.timers.tick(900);
  source.enqueue(new TextEncoder().encode(": keepalive\n\n"));
  await new Promise(resolve => setImmediate(resolve));
  t.mock.timers.tick(900);
  assert.equal(cancelled, false);
  source.enqueue(new Uint8Array());
  await new Promise(resolve => setImmediate(resolve));
  t.mock.timers.tick(100);
  assert.deepEqual((await pending).value, { ok: false, error: { kind: "network", status: 200 } });
  assert.equal(cancelled, true);
  assert.equal((await stream.next()).done, true);
});

test("stream idle timeout also bounds headers and error body reads", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  for (const status of [undefined, 503]) {
    let deliver: ((response: Response) => void) | undefined;
    let cancelled = false;
    const client = new ControlClient({ baseUrl: "https://controller.example", fetch: async () => status === undefined
      ? new Promise<Response>(resolve => { deliver = resolve; })
      : new Response(new ReadableStream({ cancel() { cancelled = true; } }), { status, headers: { "content-type": "application/json" } }) });
    const stream = client.stream(operation, { token: "private", maxReconnects: 0 });
    const pending = stream.next();
    await new Promise(resolve => setImmediate(resolve));
    t.mock.timers.tick(30_000);
    assert.deepEqual((await pending).value, { ok: false, error: { kind: "network", ...(status === undefined ? {} : { status }) } });
    if (status === undefined) {
      assert.ok(deliver);
      deliver(new Response(new ReadableStream({ cancel() { cancelled = true; } })));
      await new Promise(resolve => setImmediate(resolve));
    }
    assert.equal(cancelled, true);
    assert.equal((await stream.next()).done, true);
  }
});
