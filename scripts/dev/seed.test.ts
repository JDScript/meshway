import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdtemp, mkdir, writeFile, chmod, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import {
  ControlClient, type AuthOptionsResponse, type SessionIssuedResponse,
  type NetworkSummary, type NetworkResponse, type NetworkListResponse,
  type NetworkMembersResponse, type JoinLinkCreated, type JoinLinkRedeemed, type JoinLinkView, type JoinLinkListResponse,
} from "../../packages/control-client/src/index.ts";
import { zRegisterRequest } from "../../packages/control-client/src/generated/zod.gen.ts";
import { seed, previousPasswords, emails, serverOrigin, consoleOrigin, type Passwords } from "./seed.ts";

const passwords: Passwords = {
  "admin@example.com": "private owner test password",
  "member@example.com": "private member test password",
  "carol@example.com": "private carol test password",
  "dave@example.com": "private dave test password",
};
const now = "2026-10-05T12:00:00Z";
const options: AuthOptionsResponse = {
  requestId: randomUUID(), registrationEnabled: true,
  passwordRules: { minCodepoints: 12, maxCodepoints: 256, maxUtf8Bytes: 1024, lengthUnit: "unicodeCodepoints" },
  networkNameRules: { minCodepoints: 1, maxCodepoints: 128, maxInputUtf8Bytes: 1024, trim: "unicodeWhiteSpace", forbidControlCharacters: true },
};

function fixture(poison: "none" | "registration" | "network" = "none") {
  const network: NetworkSummary = {
    id: randomUUID(), membershipId: randomUUID(), name: "Development", role: "owner", currentRevision: 1, createdAt: now,
    actions: { rename: true, editConfiguration: true, readConfiguration: true, manageMembers: true, joinLinkRoles: ["admin", "member"] },
  };
  const sessions = emails.map(email => ({ requestId: randomUUID(), token: randomUUID(), user: { id: randomUUID(), email },
    session: { id: randomUUID(), createdAt: now, expiresAt: "2026-10-06T12:00:00Z" } } satisfies SessionIssuedResponse));
  const owner = sessions[0];
  assert.ok(owner);
  const members: NetworkMembersResponse = {
    requestId: randomUUID(), asOf: now, nextCursor: null, total: 4,
    items: sessions.map((session, index) => ({ userId: session.user.id, email: session.user.email,
      membershipId: index === 0 ? network.membershipId : randomUUID(), role: index === 0 ? "owner" : "member",
      joinedAt: now, version: 1, actions: { changeRole: index !== 0, assignableRoles: ["owner", "admin", "member"], remove: index !== 0 } })),
  };
  const calls: string[] = [];
  const links: JoinLinkView[] = [];
  let registrations = 0;
  let networkCreated = false;
  let joinCount = 0;
  const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { "Content-Type": "application/json" } });
  const fetcher: typeof fetch = async (input, init) => {
    const path = new URL(String(input)).pathname;
    const method = init?.method;
    calls.push(`${method} ${path}`);
    if (path === "/api/v1/auth/options") return json(options);
    if (path === "/api/v1/auth/logout") return new Response(null, { status: 204 });
    if (path === "/api/v1/auth/register") {
      assert.equal(typeof init?.body, "string");
      const body = zRegisterRequest.parse(JSON.parse(String(init?.body)));
      assert.equal(body.email, emails[registrations]);
      const session = sessions[registrations++];
      assert.ok(session);
      if (poison === "registration") return json({ ...session, extraSecret: "never print this" }, 201);
      return json(session, 201);
    }
    if (path === "/api/v1/networks" && method === "GET") return json({ requestId: randomUUID(), asOf: now,
      total: networkCreated ? 1 : 0, items: networkCreated ? [network] : [], nextCursor: null } satisfies NetworkListResponse);
    if (path === "/api/v1/networks" && method === "POST") {
      networkCreated = true;
      if (poison === "network") return json({ network: { ...network, role: "globalAdmin" }, requestId: randomUUID() }, 201);
      return json({ network, requestId: randomUUID() } satisfies NetworkResponse, 201);
    }
    if (path === `/api/v1/networks/${network.id}/join-links` && method === "GET") return json({
      requestId: randomUUID(), asOf: now, items: links, total: links.length, nextCursor: null,
    } satisfies JoinLinkListResponse);
    if (path === `/api/v1/networks/${network.id}/join-links` && method === "POST") {
      const link: JoinLinkView = { id: randomUUID(), networkId: network.id, role: "member", createdAt: now,
        expiresAt: "2026-10-05T12:10:00Z", consumedAt: null, consumedBy: null, revokedAt: null,
        issuedBy: { userId: owner.user.id, email: emails[0] }, actions: { revoke: true } };
      links.push(link);
      return json({ requestId: randomUUID(), secretDelivery: "available", secret: randomUUID(), link } satisfies JoinLinkCreated, 201);
    }
    if (path === "/api/v1/join-links/redeem") {
      joinCount++;
      const member = members.items[joinCount];
      const link = links.at(-1);
      assert.ok(member);
      assert.ok(link);
      link.consumedAt = now;
      link.consumedBy = { userId: member.userId, email: member.email };
      link.actions.revoke = false;
      return json({ requestId: randomUUID(), membershipId: member.membershipId, networkId: network.id } satisfies JoinLinkRedeemed);
    }
    if (path === `/api/v1/networks/${network.id}/members`) return json(members);
    assert.fail("Unexpected SDK operation");
  };
  return { client: new ControlClient({ baseUrl: "http://127.0.0.1:8420", fetch: fetcher }), network, calls };
}

test("generated SDK seed publishes four credentials and one verified Development network", async () => {
  const { client, network, calls } = fixture();
  const result = await seed(client, passwords);
  assert.deepEqual(result, { server: serverOrigin, console: consoleOrigin, email: emails[0],
    password: passwords[emails[0]], networkId: network.id, accounts: passwords });
  assert.equal(calls.filter(call => call === "POST /api/v1/networks").length, 1);
  assert.equal(calls.filter(call => call.endsWith("/revoke")).length, 0);
  assert.equal(calls.filter(call => call.endsWith("/join-links") && call.startsWith("GET")).length, 1);
  assert.equal(calls.filter(call => call.endsWith("/logout")).length, 4);
  assert.equal(JSON.stringify(result).includes("token"), false);
});

test("strict response failures stop seed without a retry or deletion", async () => {
  for (const poisoned of ["registration", "network"] as const) {
    const { client, calls } = fixture(poisoned);
    await assert.rejects(seed(client, passwords), { message: "Development seed SDK request failed" });
    assert.equal(calls.filter(call => call.endsWith("/register")).length, 1);
    assert.equal(calls.some(call => call.startsWith("DELETE")), false);
    if (poisoned === "network") assert.equal(calls.at(-1), "POST /api/v1/auth/logout");
  }
});

test("private seed preserves account passwords, rejects malformed inputs and checks password rules before registration", async () => {
  const value = { server: serverOrigin, console: consoleOrigin, email: emails[0], password: passwords[emails[0]], networkId: randomUUID(), accounts: passwords };
  assert.deepEqual(previousPasswords(value), passwords);
  assert.throws(() => previousPasswords({ ...value, accounts: { ...passwords, "dave@example.com": 4 } }), /Invalid private/);
  assert.throws(() => previousPasswords({ ...value, password: "different" }), /Inconsistent private/);
  assert.throws(() => previousPasswords({ ...value, extra: true }), /Invalid private/);
  const { client, calls } = fixture();
  await assert.rejects(seed(client, { ...passwords, "dave@example.com": "short" }), /current rules/);
  assert.deepEqual(calls, ["GET /api/v1/auth/options"]);
});

test("CLI preflight accepts private credentials and rejects unsafe input without printing secrets", async () => {
  const root = await mkdtemp(join(tmpdir(), "meshway-seed-check-"));
  try {
    const directory = join(root, ".meshway");
    const file = join(directory, "dev-login.json");
    await mkdir(directory, { mode: 0o700 });
    const value = { server: serverOrigin, console: consoleOrigin, email: emails[0], password: passwords[emails[0]], networkId: randomUUID(), accounts: passwords };
    await writeFile(file, JSON.stringify(value), { mode: 0o600 });
    const check = () => spawnSync(process.execPath, ["--experimental-strip-types", fileURLToPath(new URL("./seed.ts", import.meta.url)), "--check", root], { encoding: "utf8", timeout: 10_000 });
    const good = check();
    assert.equal(good.status, 0);
    assert.equal(good.stdout, "");
    assert.equal(good.stderr, "");
    await chmod(file, 0o644);
    const unsafe = check();
    assert.equal(unsafe.status, 1);
    await chmod(file, 0o600);
    await writeFile(file, JSON.stringify({ ...value, accounts: "SECRET_INVALID_CREDENTIAL_INPUT" }));
    const malformed = check();
    assert.equal(malformed.status, 1);
    assert.equal(malformed.stdout, "");
    assert.equal(malformed.stderr.includes("SECRET_INVALID_CREDENTIAL_INPUT"), false);
    assert.equal(malformed.stderr.includes(passwords[emails[0]]), false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
