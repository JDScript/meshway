import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, mkdir, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import {
  ControlClient, getAuthOptions, register, login, getAuthSession, logout,
  createNetwork, listNetworks, getNetwork, renameNetwork, listNetworkMembers,
  createJoinLink, redeemJoinLink, changeMemberRole, getConfiguration,
  getConfigurationSource, validateConfiguration, applyConfiguration,
  compareConfiguration, listConfigurationRevisions,
  type ConfigurationRequest,
} from "../src/index.ts";
import { zListener, zPreparedListener, zPreparationKind, zNodeAddresses, zConfigurationOperation } from "../src/generated/zod.gen.ts";

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
  assert.deepEqual(options.data.networkNameRules, { minCodepoints: 1, maxCodepoints: 128, maxInputUtf8Bytes: 1024, trim: "unicodeWhiteSpace", forbidControlCharacters: true });
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

test("generated TS network operations create real networks and preserve pagination and CAS", async t => {
  const client = await controller(t);
  const issued = await register(client, { email: "network-owner@example.com", password: "valid network password" });
  assert.ok(issued.ok);
  const options = { token: issued.data.token };
  const firstRequest = { requestId: randomUUID(), name: "First network" };
  const first = await createNetwork(client, firstRequest, options);
  assert.ok(first.ok);
  assert.equal(first.data.network.role, "owner");
  assert.equal(first.data.network.currentRevision, 1);
  assert.equal(first.data.network.actions.rename, true);
  const replay = await createNetwork(client, firstRequest, options);
  assert.ok(replay.ok);
  assert.deepEqual(replay.data.network, first.data.network);
  assert.ok((await createNetwork(client, { requestId: randomUUID(), name: "Second network" }, options)).ok);
  const page = await listNetworks(client, { limit: 1 }, options);
  assert.ok(page.ok);
  assert.equal(page.data.items.length, 1);
  assert.equal(page.data.total, 2);
  assert.ok(page.data.nextCursor);
  assert.ok((await createNetwork(client, { requestId: randomUUID(), name: "After the page boundary" }, options)).ok);
  const rest = await listNetworks(client, { limit: 200, cursor: page.data.nextCursor }, options);
  assert.ok(rest.ok);
  assert.equal(rest.data.items.length, 1);
  assert.equal(rest.data.total, 2);
  assert.equal(rest.data.items[0]?.name, "Second network");
  assert.equal(rest.data.nextCursor, null);
  assert.equal(rest.data.asOf, page.data.asOf);
  const path = { networkId: first.data.network.id };
  const detail = await getNetwork(client, path, options);
  assert.ok(detail.ok);
  const members = await listNetworkMembers(client, path, {}, options);
  assert.ok(members.ok);
  assert.equal(members.data.items[0]?.userId, issued.data.user.id);
  assert.equal(members.data.items[0]?.membershipId, detail.data.network.membershipId);
  assert.equal(members.data.total, 1);
  assert.deepEqual(members.data.items[0]?.actions, { changeRole: false, assignableRoles: ["owner"], remove: false });
  const renamed = await renameNetwork(client, path, { requestId: randomUUID(), expectedName: "First network", name: "New name" }, options);
  assert.ok(renamed.ok);
  assert.equal(renamed.data.network.currentRevision, 1);
  const conflict = await renameNetwork(client, path, { requestId: randomUUID(), expectedName: "First network", name: "Lost edit" }, options);
  assert.ok(!conflict.ok && conflict.error.kind === "http");
  assert.equal(conflict.error.status, 409);
  assert.equal(conflict.error.problem?.problem.code, "name_conflict");
});

test("management SDK exposes membership actions and applies versioned configuration through the real Controller", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "manage-owner@example.com", password: "valid management password" });
  const user = await register(client, { email: "manage-member@example.com", password: "valid management password" });
  assert.ok(owner.ok && user.ok);
  const options = { token: owner.data.token };
  const padded = `\u3000${"😀".repeat(128)}\u00a0`;
  const created = await createNetwork(client, { requestId: randomUUID(), name: padded }, options);
  assert.ok(created.ok);
  assert.equal(created.data.network.name, "😀".repeat(128));
  const path = { networkId: created.data.network.id };
  const link = await createJoinLink(client, path, { requestId: randomUUID(), role: "member", expiresInSeconds: 600 }, options);
  assert.ok(link.ok && link.data.secretDelivery === "available");
  const joined = await redeemJoinLink(client, { token: link.data.secret }, { token: user.data.token });
  assert.ok(joined.ok);
  const members = await listNetworkMembers(client, path, { limit: 1 }, options);
  assert.ok(members.ok && members.data.nextCursor);
  assert.equal(members.data.total, 2);
  const rest = await listNetworkMembers(client, path, { cursor: members.data.nextCursor }, options);
  assert.ok(rest.ok);
  assert.equal(rest.data.total, 2);
  const member = [...members.data.items, ...rest.data.items].find(item => item.userId === user.data.user.id);
  assert.ok(member);
  assert.deepEqual(member.actions, { changeRole: true, assignableRoles: ["owner", "admin", "member"], remove: true });
  const changed = await changeMemberRole(client, { ...path, membershipId: member.membershipId }, { expectedVersion: member.version, role: "admin" }, options);
  assert.ok(changed.ok);
  assert.equal(changed.data.member.role, "admin");
  assert.equal(changed.data.member.version, member.version + 1);

  const source = await getConfigurationSource(client, path, {}, options);
  assert.ok(source.ok);
  const initial = source.data.revision;
  const edit: ConfigurationRequest = { type: "operations", expectedRevision: initial, requestId: randomUUID(), ops: [{ type: "replaceAccess", access: [] }] };
  const preview = await validateConfiguration(client, path, edit, options);
  assert.ok(preview.ok);
  assert.equal(preview.data.changeCount, 1);
  assert.deepEqual(preview.data.changes, [{ path: "/access", kind: "changed" }]);
  assert.deepEqual(preview.data.preview.access, []);
  const applied = await applyConfiguration(client, path, edit, options);
  assert.ok(applied.ok);
  assert.equal(applied.data.revision, initial + 1);
  const replay = await applyConfiguration(client, path, edit, options);
  assert.ok(replay.ok);
  assert.equal(replay.data.revision, applied.data.revision);
  const stale = await applyConfiguration(client, path, { ...edit, requestId: randomUUID() }, options);
  assert.ok(!stale.ok && stale.error.kind === "http");
  assert.equal(stale.error.problem?.problem.code, "revision_conflict");
  const current = await getConfiguration(client, path, options);
  assert.ok(current.ok);
  assert.deepEqual(current.data.configuration.access, []);
  const comparison = await compareConfiguration(client, path, { fromRevision: initial, toRevision: applied.data.revision }, options);
  assert.ok(comparison.ok);
  assert.equal(comparison.data.configurationChanged, true);
  const restored = await applyConfiguration(client, path, { type: "restore", expectedRevision: applied.data.revision, requestId: randomUUID(), restoreFrom: initial }, options);
  assert.ok(restored.ok);
  assert.equal(restored.data.revision, initial + 2);
  const history = await listConfigurationRevisions(client, path, {}, options);
  assert.ok(history.ok);
  assert.equal(history.data.items.find(item => item.current)?.cause, "restore");
});

test("published configuration validators preserve closed listener variants and material requirements", () => {
  const removal = { type: "setListener", nodeId: "12345678-1234-0234-0234-123456789abc", key: "edge" };
  assert.equal(zConfigurationOperation.safeParse(removal).success, false);
  assert.equal(zConfigurationOperation.safeParse({ ...removal, listener: null }).success, true);
  const native = { type: "native_udp", listenPort: 8421 };
  assert.equal(zListener.safeParse(native).success, true);
  assert.equal(zPreparedListener.safeParse(native).success, true);
  assert.equal(zListener.safeParse({ ...native, tcp: {} }).success, false);
  const vless = { type: "vless", listenPort: 443, tls: { serverName: "edge.example" } };
  assert.equal(zListener.safeParse(vless).success, true);
  assert.equal(zPreparedListener.safeParse(vless).success, false);
  assert.equal(zPreparedListener.safeParse({ ...vless, uuid: "11111111-1111-4111-8111-111111111111", tls: { ...vless.tls, certificate: "structural certificate", privateKey: "structural key" } }).success, true);
  for (const value of ["addressV4", { material: "certificateValidation" }]) assert.equal(zPreparationKind.safeParse(value).success, true);
  for (const value of ["unknown", { material: "certificateValidation", extra: true }, ["addressV4"]]) assert.equal(zPreparationKind.safeParse(value).success, false);
  for (const value of [{ ipv4: "100.64.0.1", ipv6: "fd00::1" }, { ipv4: null, ipv6: "::ffff:192.0.2.1" }]) assert.equal(zNodeAddresses.safeParse(value).success, true);
  for (const value of [{ ipv4: "100.064.0.1", ipv6: null }, { ipv4: null, ipv6: "fe80::1%en0" }, { ipv4: null }]) assert.equal(zNodeAddresses.safeParse(value).success, false);
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
