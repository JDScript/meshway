import assert from "node:assert/strict";
import { test } from "node:test";
import { randomUUID } from "node:crypto";
import { controller } from "./support/controller.ts";
import {
  ControlClient, getAuthOptions, register, login, getAuthSession, logout,
  createNetwork, listNetworks, getNetwork, renameNetwork, listNetworkMembers,
  createJoinLink, redeemJoinLink, changeMemberRole, getConfiguration,
  getConfigurationSource, validateConfiguration, applyConfiguration,
  compareConfiguration, listConfigurationRevisions,
  changeAccountEmail, getMemberRemovalImpact, removeMember,
  listAccountActivity, listNetworkActivity, listAccountSessions, changeAccountPassword,
  type ConfigurationRequest,
} from "../src/index.ts";
import { zListener, zPreparedListener, zPreparationKind, zNodeAddresses, zConfigurationOperation } from "../src/generated/zod.gen.ts";

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

test("activity actions decode through the generated SDK with their actual dotted names", async t => {
  const client = await controller(t);
  const email = "activity-schema@example.test";
  const password = "valid activity schema password";
  assert.ok((await register(client, { email, password })).ok);
  const session = await login(client, { email, password });
  assert.ok(session.ok);
  const options = { token: session.data.token };
  const created = await createNetwork(client, { requestId: randomUUID(), name: "Activity schema" }, options);
  assert.ok(created.ok);
  const account = await listAccountActivity(client, {}, options);
  assert.ok(account.ok);
  assert.ok(account.data.items.some(event => event.action === "session.login"));
  assert.ok(account.data.items.some(event => event.action === "account.register"));
  const network = await listNetworkActivity(client, { networkId: created.data.network.id }, {}, options);
  assert.ok(network.ok);
  assert.ok(network.data.items.some(event => event.action === "network.create" && event.target.type === "network"));
});

test("session metadata and account errors retain their distinct meanings", async t => {
  const agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/140.0.0.0 Safari/537.36";
  const client = await controller(t, true, agent);
  const email = "session-metadata@example.test";
  const password = "valid metadata test password";
  const session = await register(client, { email, password });
  assert.ok(session.ok);
  const options = { token: session.data.token };
  const sessions = await listAccountSessions(client, {}, options);
  assert.ok(sessions.ok);
  const current = sessions.data.items.find(item => item.current);
  assert.ok(current?.lastUsedAt);
  assert.equal(current.clientLabel, "Chrome · Linux");
  assert.ok(Date.parse(current.lastUsedAt) >= Date.parse(current.createdAt));
  assert.equal(JSON.stringify(sessions.data).includes(agent), false);

  const created = await createNetwork(client, { requestId: randomUUID(), name: "Cursor errors" }, options);
  assert.ok(created.ok);
  const path = { networkId: created.data.network.id };
  const query = { cursor: "invalid-cursor" };
  for (const result of [
    await listNetworks(client, query, options),
    await listNetworkMembers(client, path, query, options),
    await listAccountSessions(client, query, options),
    await listAccountActivity(client, query, options),
    await listNetworkActivity(client, path, query, options),
    await listConfigurationRevisions(client, path, query, options),
  ]) {
    assert.ok(!result.ok && result.error.kind === "http");
    assert.equal(result.error.status, 400);
    assert.equal(result.error.problem?.problem.code, "invalid_cursor");
  }
  for (const result of [
    await changeAccountPassword(client, { currentPassword: "incorrect current password", password: "valid replacement password" }, options),
    await changeAccountEmail(client, { password: "incorrect current password", email: "unused-metadata@example.test" }, options),
  ]) {
    assert.ok(!result.ok && result.error.kind === "http");
    assert.equal(result.error.status, 400);
    assert.equal(result.error.problem?.problem.code, "invalid_current_password");
  }
  assert.ok((await getAuthSession(client, options)).ok);
  assert.ok((await logout(client, options)).ok);
  const revoked = await changeAccountPassword(client, { currentPassword: password, password: "valid replacement password" }, options);
  assert.ok(!revoked.ok && revoked.error.kind === "http");
  assert.equal(revoked.error.status, 401);
  assert.equal(revoked.error.problem?.problem.code, "invalid_session");
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

test("SDK preserves retired references across email changes and rejoin without a hidden document rebind", async t => {
  const client = await controller(t);
  const password = "valid retired reference password";
  const owner = await register(client, { email: "retired-owner@example.test", password });
  const member = await register(client, { email: "retired-member@example.test", password });
  assert.ok(owner.ok && member.ok);
  const options = { token: owner.data.token };
  const created = await createNetwork(client, { requestId: randomUUID(), name: "Retired references" }, options);
  assert.ok(created.ok);
  const path = { networkId: created.data.network.id };
  async function join(token: string) {
    const link = await createJoinLink(client, path, { requestId: randomUUID(), role: "member", expiresInSeconds: 600 }, options);
    assert.ok(link.ok && link.data.secretDelivery === "available");
    const result = await redeemJoinLink(client, { token: link.data.secret }, { token });
    assert.ok(result.ok);
    return result.data.membershipId;
  }
  const membershipId = await join(member.data.token);
  const edited = await applyConfiguration(client, path, {
    type: "operations", expectedRevision: created.data.network.currentRevision, requestId: randomUUID(),
    ops: [{ type: "replaceGroups", groups: { "group:team": [member.data.user.email] } }],
  }, options);
  assert.ok(edited.ok);
  const impact = await getMemberRemovalImpact(client, { ...path, membershipId }, options);
  assert.ok(impact.ok && impact.data.nodeCount === 0);
  const removed = await removeMember(client, { ...path, membershipId }, { expectedRevision: impact.data.revision, expectedVersion: impact.data.version }, options);
  assert.ok(removed.ok);
  const departed = await getConfigurationSource(client, path, {}, options);
  assert.ok(departed.ok);
  const email = "retired-renamed@example.test";
  const changed = await changeAccountEmail(client, { password, email }, { token: member.data.token });
  assert.ok(changed.ok);
  assert.equal(changed.data.user.id, member.data.user.id);
  const afterEmail = await getConfigurationSource(client, path, {}, options);
  assert.ok(afterEmail.ok);
  assert.equal(afterEmail.data.revision, departed.data.revision);
  assert.equal(afterEmail.data.sourceJsonc === departed.data.sourceJsonc, true);
  assert.deepEqual(afterEmail.data.configuration.groups?.["group:team"], [member.data.user.email]);
  const session = await login(client, { email, password });
  assert.ok(session.ok);
  assert.notEqual(await join(session.data.token), membershipId);
  const source = await getConfigurationSource(client, path, {}, options);
  assert.ok(source.ok);
  assert.deepEqual(source.data.configuration.groups?.["group:team"], [member.data.user.email]);
  const inputs: ConfigurationRequest[] = [
    { type: "operations", expectedRevision: source.data.revision, requestId: randomUUID(), ops: [] },
    { type: "document", expectedRevision: source.data.revision, requestId: randomUUID(), sourceJsonc: source.data.sourceJsonc },
  ];
  for (const input of inputs) {
    const preview = await validateConfiguration(client, path, input, options);
    assert.ok(preview.ok);
    assert.equal(preview.data.changeCount, 0);
    assert.equal(preview.data.sourceChanged, false);
    assert.ok(preview.data.unavailableDependencies.length > 0);
    const saved = await applyConfiguration(client, path, input, options);
    assert.ok(saved.ok);
    assert.equal(saved.data.revision, source.data.revision);
  }
});

test("SDK form edits preserve hidden headers and explicit empty headers clear them", async t => {
  const client = await controller(t);
  const owner = await register(client, { email: "headers-owner@example.test", password: "valid hidden headers password" });
  assert.ok(owner.ok);
  const options = { token: owner.data.token };
  const created = await createNetwork(client, { requestId: randomUUID(), name: "Hidden headers" }, options);
  assert.ok(created.ok);
  const path = { networkId: created.data.network.id };
  const initial = await getConfigurationSource(client, path, {}, options);
  assert.ok(initial.ok);
  const defaults = initial.data.configuration.defaults;
  assert.ok(defaults?.dialers?.vless);
  const credential = "test-header-value-not-returned-to-the-form";
  const seeded = await applyConfiguration(client, path, {
    type: "operations", expectedRevision: initial.data.revision, requestId: randomUUID(),
    ops: [{ type: "replaceDefaults", defaults: { ...defaults, dialers: { ...defaults.dialers, vless: { ...defaults.dialers.vless, websocket: { headers: { Authorization: credential } } } } } }],
  }, options);
  assert.ok(seeded.ok);
  const view = await getConfiguration(client, path, options);
  assert.ok(view.ok && view.data.configuration.defaults.dialers.vless);
  assert.equal(JSON.stringify(view.data).includes(credential), false);
  const visible = view.data.configuration.defaults;
  const { websocketHeaderNames, ...vless } = view.data.configuration.defaults.dialers.vless;
  assert.deepEqual(websocketHeaderNames, ["Authorization"]);
  const publicDialers = {
    ...(visible.dialers.native_udp ? { native_udp: visible.dialers.native_udp } : {}),
    ...(visible.dialers.native_tcp ? { native_tcp: visible.dialers.native_tcp } : {}),
    ...(visible.dialers.trojan ? { trojan: visible.dialers.trojan } : {}),
  };
  const form = { access: visible.access, topology: visible.topology, dialers: { ...publicDialers, vless: { ...vless, priority: 301, websocket: {} } } };
  const updated = await applyConfiguration(client, path, {
    type: "operations", expectedRevision: view.data.revision, requestId: randomUUID(),
    ops: [{ type: "replaceDefaults", defaults: form }],
  }, options);
  assert.ok(updated.ok);
  // An explicit privileged audit read verifies the form did not need the value.
  const retained = await getConfigurationSource(client, path, {}, options);
  assert.ok(retained.ok);
  assert.equal(retained.data.configuration.defaults?.dialers?.vless?.websocket?.headers?.Authorization, credential);
  assert.equal(retained.data.configuration.defaults?.dialers?.vless?.priority, 301);
  const cleared = await applyConfiguration(client, path, {
    type: "operations", expectedRevision: updated.data.revision, requestId: randomUUID(),
    ops: [{ type: "replaceDefaults", defaults: { ...form, dialers: { ...form.dialers, vless: { ...form.dialers.vless, websocket: { headers: {} } } } } }],
  }, options);
  assert.ok(cleared.ok);
  const after = await getConfiguration(client, path, options);
  assert.ok(after.ok);
  assert.deepEqual(after.data.configuration.defaults.dialers.vless?.websocketHeaderNames, []);
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
