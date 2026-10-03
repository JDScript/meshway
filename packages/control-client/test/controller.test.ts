import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import {
  ControlClient, getAuthOptions, register, login, getAuthSession, logout,
} from "../src/index.ts";

async function controller(t: TestContext, registrationEnabled = true) {
  const parent = join(tmpdir(), "opencode");
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(await realpath(parent), "meshway-ts-sdk-"));
  const binary = process.env.MESHWAY_CONTROLLER_BINARY ?? resolve(import.meta.dirname, "../../..", "target/debug/meshway-controller");
  const child = spawn(binary, ["--data-dir", join(directory, "instance"), "--listen", "127.0.0.1:0", "--registration-enabled", String(registrationEnabled)], { stdio: ["ignore", "pipe", "pipe"] });
  const exited = new Promise<number | null>(resolveExit => child.once("close", resolveExit));
  child.stderr.resume(); // Runtime test failures never reproduce raw service output.
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
  return new ControlClient({ baseUrl });
}

test("generated TS SDK authenticates through the normal Controller binary", async t => {
  const client = await controller(t);
  const options = await getAuthOptions(client);
  assert.ok(options.ok);
  assert.equal(options.data.registrationEnabled, true);
  assert.equal(options.data.passwordRules.lengthUnit, "unicodeCodepoints");
  const password = "🔐".repeat(options.data.passwordRules.minCodepoints);
  assert.ok(new TextEncoder().encode(password).length <= options.data.passwordRules.maxUtf8Bytes);
  const issued = await register(client, { email: "  ALICE@BÜCHER.example  ", password });
  assert.ok(issued.ok);
  assert.equal(issued.data.user.email, "alice@xn--bcher-kva.example");
  const authenticated = { token: issued.data.token };
  const session = await getAuthSession(client, authenticated);
  assert.ok(session.ok);
  assert.equal(session.data.user.id, issued.data.user.id);
  assert.equal(session.data.session.id, issued.data.session.id);
  const duplicate = await register(client, { email: issued.data.user.email, password });
  assert.ok(!duplicate.ok && duplicate.error.kind === "http");
  assert.equal(duplicate.error.status, 409);
  assert.equal(duplicate.error.problem?.problem.code, "email_taken");
  const weak = await register(client, { email: "weak@example.com", password: "short" });
  assert.ok(!weak.ok && weak.error.kind === "http" && weak.error.problem?.problem.code === "password_weak");
  assert.deepEqual(weak.error.problem.problem.params, options.data.passwordRules);
  const wrong = await login(client, { email: issued.data.user.email, password: "wrong" });
  assert.ok(!wrong.ok && wrong.error.kind === "http");
  assert.equal(wrong.error.problem?.problem.code, "invalid_credentials");
  assert.deepEqual(await logout(client, authenticated), { ok: true, data: undefined });
  const revoked = await getAuthSession(client, authenticated);
  assert.ok(!revoked.ok && revoked.error.kind === "http");
  assert.equal(revoked.error.problem?.problem.code, "invalid_session");
  const next = await login(client, { email: issued.data.user.email, password });
  assert.ok(next.ok);
  assert.equal(next.data.user.id, issued.data.user.id);
  assert.notEqual(next.data.session.id, issued.data.session.id);
  assert.ok((await logout(client, { token: next.data.token })).ok);
});

test("generated TS SDK decodes server-enforced registration disablement", async t => {
  const client = await controller(t, false);
  const options = await getAuthOptions(client);
  assert.ok(options.ok);
  assert.equal(options.data.registrationEnabled, false);
  const result = await register(client, { email: "disabled@example.com", password: "test password with length" });
  assert.ok(!result.ok && result.error.kind === "http");
  assert.equal(result.error.status, 403);
  assert.equal(result.error.problem?.problem.code, "registration_disabled");
});

test("generated TS operations validate malformed real envelopes before returning data", async t => {
  const real = await controller(t);
  const issued = await register(real, { email: "decoding@example.com", password: "test password with length" });
  assert.ok(issued.ok);
  const invalidLogin = await login(real, { email: issued.data.user.email, password: "wrong" });
  assert.ok(!invalidLogin.ok && invalidLogin.error.kind === "http" && invalidLogin.error.problem);
  const envelope = issued.data;
  function fault(status: number, value: unknown, media = "application/json", maxBodyBytes = 4096) {
    return new ControlClient({ baseUrl: "https://response-fixture.example", maxBodyBytes,
      fetch: async () => new Response(JSON.stringify(value), { status, headers: { "content-type": media } }),
    });
  }
  const malformed: unknown[] = [
    { ...envelope, user: [envelope.user.id, envelope.user.email] },
    { ...envelope, session: [envelope.session.id, envelope.session.createdAt, envelope.session.expiresAt] },
    { ...envelope, user: { ...envelope.user, id: "00000000-0000-0000-0000-000000000000" } },
    { ...envelope, user: { ...envelope.user, id: "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA" } },
    { ...envelope, user: { ...envelope.user, unknown: "private" } },
    { ...envelope, unknown: "private" },
    { ...envelope, token: null },
    { ...envelope, session: { ...envelope.session, createdAt: "2026-02-30T00:00:00Z" } },
    { ...envelope, session: { ...envelope.session, createdAt: envelope.session.createdAt + "\n" } },
  ];
  for (const value of malformed) assert.deepEqual(await login(fault(200, value), { email: "test@example.com", password: "private" }), { ok: false, error: { kind: "decode", status: 200 } });
  const error = invalidLogin.error.problem;
  const { requestId: _requestId, ...missingRequestId } = error;
  for (const value of [
    [error.requestId, error.status, error.problem],
    { ...error, unknown: "private" },
    { ...error, problem: { ...error.problem, code: "unknown" } },
    { ...error, problem: { ...error.problem, unknown: "private" } },
    { ...error, problem: { ...error.problem, params: [] } },
    { ...error, status: 65536 }, missingRequestId,
  ]) assert.deepEqual(await login(fault(401, value), { email: "test@example.com", password: "private" }), { ok: false, error: { kind: "http", status: 401 } });
  const nullable = await login(fault(401, { ...error, requestId: null }), { email: "test@example.com", password: "private" });
  assert.ok(!nullable.ok && nullable.error.kind === "http" && nullable.error.problem);
  assert.equal(nullable.error.problem.requestId, null);
  assert.deepEqual(await getAuthSession(fault(200, envelope, "text/html"), { token: "private" }), { ok: false, error: { kind: "decode", status: 200 } });
  assert.deepEqual(await getAuthSession(fault(200, envelope, "application/json", 4), { token: "private" }), { ok: false, error: { kind: "body_limit", status: 200 } });
});
