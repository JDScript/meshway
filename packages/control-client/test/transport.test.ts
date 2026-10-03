import assert from "node:assert/strict";
import { test } from "node:test";
import * as z from "zod";
import { ControlClient } from "../src/transport.ts";

const operation = {
  method: "GET", path: "/fixture/value", authenticated: false, successStatus: 200,
  success: z.strictObject({ value: z.string() }), errors: [409],
  problem: z.strictObject({ code: z.literal("conflict"), revision: z.int().min(1) }),
} as const;
function client(response: () => Response | Promise<Response>, maxBodyBytes = 1024) {
  return new ControlClient({ baseUrl: "https://controller.example", fetch: async () => response(), maxBodyBytes });
}
test("HTTP failures expose only validated typed problems and retain status on decode failure", async () => {
  const valid = await client(() => Response.json({ code: "conflict", revision: 5 }, { status: 409 })).request(operation);
  assert.deepEqual(valid, { ok: false, error: { kind: "http", status: 409, problem: { code: "conflict", revision: 5 } } });
  for (const response of [
    Response.json({ code: "conflict", revision: 5, password: "secret" }, { status: 409 }),
    new Response("secret invalid JSON", { status: 409, headers: { "content-type": "application/json" } }),
    new Response("secret upstream body", { status: 502 }),
  ]) {
    const result = await client(() => response).request(operation);
    assert.deepEqual(result, { ok: false, error: { kind: "http", status: response.status } });
    assert.equal(JSON.stringify(result).includes("secret"), false);
  }
});
test("successful bodies, malformed UTF8, content type and status are checked", async () => {
  assert.deepEqual(await client(() => Response.json({ value: "ok" })).request(operation), { ok: true, data: { value: "ok" } });
  for (const response of [Response.json({ value: "ok", unknown: true }), new Response("{}"), new Response(new Uint8Array([0xff]), { headers: { "content-type": "application/json" } })]) {
    assert.deepEqual(await client(() => response).request(operation), { ok: false, error: { kind: "decode", status: 200 } });
  }
  assert.deepEqual(await client(() => Response.json({ value: "ok" }, { status: 201 })).request(operation), { ok: false, error: { kind: "http", status: 201 } });
  assert.deepEqual(await client(() => new Response(null, { status: 204 })).request({ ...operation, successStatus: 204, success: undefined }), { ok: true, data: undefined });
});
test("bounded streamed/declared bodies cancel reads and discard error content", async () => {
  let cancelled = false;
  const streamed = () => new Response(new ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(new TextEncoder().encode('{"value":"too big"}')); },
    cancel() { cancelled = true; },
  }), { headers: { "content-type": "application/json" } });
  assert.deepEqual(await client(streamed, 4).request(operation), { ok: false, error: { kind: "body_limit", status: 200 } });
  assert.equal(cancelled, true);
  assert.deepEqual(await client(() => Response.json({ value: "x" }, { headers: { "content-length": "99999" } }), 4).request(operation), { ok: false, error: { kind: "body_limit", status: 200 } });
  assert.deepEqual(await client(() => Response.json({ secret: "x" }, { status: 409 }), 4).request(operation), { ok: false, error: { kind: "http", status: 409 } });
});
test("caller cancellation covers fetch and stalled body reads", async () => {
  for (const streaming of [false, true]) {
    const controller = new AbortController();
    let cancelled = false;
    const transport = client(() => streaming ? new Response(new ReadableStream<Uint8Array>({ cancel() { cancelled = true; } }), { headers: { "content-type": "application/json" } }) : new Promise<Response>(() => {}));
    const pending = transport.request(operation, { signal: controller.signal });
    await new Promise(resolve => setTimeout(resolve, 5));
    controller.abort("secret reason");
    assert.deepEqual(await pending, { ok: false, error: { kind: "cancelled" } });
    if (streaming) assert.equal(cancelled, true);
  }
  assert.deepEqual(await client(() => { throw new Error("secret network exception"); }).request(operation), { ok: false, error: { kind: "network" } });
});
test("cancelled injected fetch discards a late response stream", async () => {
  let deliver: ((response: Response) => void) | undefined;
  const controller = new AbortController();
  const transport = client(() => new Promise<Response>(resolve => { deliver = resolve; }));
  const request = transport.request(operation, { signal: controller.signal });
  controller.abort();
  assert.deepEqual(await request, { ok: false, error: { kind: "cancelled" } });
  let cancelled = false;
  assert.ok(deliver);
  deliver(new Response(new ReadableStream({ cancel() { cancelled = true; } })));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(cancelled, true);
});
test("base URL, request validation and Authorization are controlled without ambient credentials", async () => {
  for (const baseUrl of ["https://user:secret@controller.example", "https://controller.example/prefix", "https://controller.example/?token=secret", "file:///secret"]) assert.throws(() => new ControlClient({ baseUrl }), error => error instanceof Error && !error.message.includes("secret"));
  let calls = 0;
  const transport = new ControlClient({ baseUrl: "https://controller.example/", fetch: async (url, init) => {
    calls++;
    assert.equal(String(url), "https://controller.example/fixture/value");
    assert.equal(new Headers(init?.headers).get("authorization"), "Bearer secret");
    assert.equal(init?.credentials, "omit");
    assert.equal(init?.redirect, "error");
    return Response.json({ value: "ok" });
  } });
  assert.equal((await transport.request({ ...operation, authenticated: true }, { token: "secret" })).ok, true);
  for (const token of ["", "secret\r\nInjected: yes"]) assert.deepEqual(await transport.request({ ...operation, authenticated: true }, { token }), { ok: false, error: { kind: "configuration" } });
  assert.deepEqual(await transport.request({ ...operation, path: "//evil.example/secret" }), { ok: false, error: { kind: "configuration" } });
  assert.deepEqual(await transport.request({ ...operation, method: "POST", body: { schema: z.strictObject({ value: z.string() }), value: { value: 42 } } }), { ok: false, error: { kind: "request" } });
  assert.equal(calls, 1);
});
