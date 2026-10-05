/** Explicit dev.py reset seed. All HTTP requests use the generated, validating SDK. */
import { randomBytes, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { lstat, open, readFile, rename, unlink } from "node:fs/promises";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  ControlClient, getAuthOptions, register, createNetwork, listNetworks,
  createJoinLink, redeemJoinLink, listJoinLinks, listNetworkMembers, logout,
  type ClientResult,
} from "../../packages/control-client/src/index.ts";

export const emails = ["admin@example.com", "member@example.com", "carol@example.com", "dave@example.com"] as const;
export type Passwords = Record<typeof emails[number], string>;
export const serverOrigin = "https://meshway-dev-api.jdscript.dev";
export const consoleOrigin = "http://127.0.0.1:3001";

function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** This is a private local credential file, not an HTTP DTO. Never log it. */
export function previousPasswords(value: unknown): Passwords {
  if (!object(value) || !object(value.accounts)
      || Object.keys(value).some(key => !["server", "console", "email", "password", "networkId", "accounts"].includes(key))
      || !["server", "console", "email", "password", "networkId"].every(key => typeof value[key] === "string")
      || Object.keys(value.accounts).length !== emails.length) throw new Error("Invalid private development credentials");
  const passwords = freshPasswords();
  for (const email of emails) {
    const password = value.accounts[email];
    if (typeof password !== "string" || !password) throw new Error("Invalid private development credentials");
    passwords[email] = password;
  }
  if (value.password !== value.accounts[String(value.email)]) throw new Error("Inconsistent private development credentials");
  return passwords;
}

function freshPasswords(): Passwords {
  return {
    "admin@example.com": randomBytes(24).toString("base64url"),
    "member@example.com": randomBytes(24).toString("base64url"),
    "carol@example.com": randomBytes(24).toString("base64url"),
    "dave@example.com": randomBytes(24).toString("base64url"),
  };
}

function requireResult<T, P>(result: ClientResult<T, P>): T {
  // Do not include response bodies, problem parameters, tokens or credentials.
  if (!result.ok) throw new Error("Development seed SDK request failed");
  return result.data;
}

export async function seed(client: ControlClient, passwords: Passwords) {
  const request = () => ({ signal: AbortSignal.timeout(10_000) });
  const options = requireResult(await getAuthOptions(client, request()));
  if (!options.registrationEnabled) throw new Error("Development registration is disabled");
  for (const password of Object.values(passwords)) {
    const length = [...password].length;
    if (length < options.passwordRules.minCodepoints || length > options.passwordRules.maxCodepoints
        || Buffer.byteLength(password) > options.passwordRules.maxUtf8Bytes) throw new Error("Development password does not meet current rules");
  }
  const sessions: string[] = [];
  try {
    const owner = requireResult(await register(client, { email: emails[0], password: passwords[emails[0]] }, request()));
    sessions.push(owner.token);
    const authenticated = () => ({ ...request(), token: owner.token });
    const existing = requireResult(await listNetworks(client, {}, authenticated()));
    if (existing.total !== 0) throw new Error("Seed requires an empty development account");
    const created = requireResult(await createNetwork(client, { requestId: randomUUID(), name: "Development" }, authenticated()));
    if (created.network.name !== "Development" || created.network.role !== "owner" || created.network.currentRevision !== 1) throw new Error("Unexpected development network");
    const path = { networkId: created.network.id };
    if (owner.user.email !== emails[0]) throw new Error("Unexpected development owner");
    const users = new Map<string, string>([[emails[0], owner.user.id]]);
    const consumedLinks = new Map<string, string>();
    for (const email of emails.slice(1)) {
      const member = requireResult(await register(client, { email, password: passwords[email] }, request()));
      sessions.push(member.token);
      if (member.user.email !== email) throw new Error("Unexpected development account");
      users.set(email, member.user.id);
      const link = requireResult(await createJoinLink(client, path, { requestId: randomUUID(), role: "member", expiresInSeconds: 600 }, authenticated()));
      if (link.secretDelivery !== "available") throw new Error("Development join secret unavailable");
      const joined = requireResult(await redeemJoinLink(client, { token: link.secret }, { ...request(), token: member.token }));
      if (joined.networkId !== path.networkId) throw new Error("Unexpected development membership");
      consumedLinks.set(link.link.id, member.user.id);
    }
    // Join links are single-use. Revoking a consumed link is a state conflict.
    const links = requireResult(await listJoinLinks(client, path, {}, authenticated()));
    if (links.total !== consumedLinks.size || links.items.length !== consumedLinks.size || links.nextCursor !== null
        || links.items.some(link => link.networkId !== path.networkId || link.consumedAt === null || !link.consumedBy
          || consumedLinks.get(link.id) !== link.consumedBy.userId || link.actions.revoke)) throw new Error("Development join-link verification failed");
    const members = requireResult(await listNetworkMembers(client, path, {}, authenticated()));
    if (members.total !== emails.length || members.nextCursor !== null || members.items.length !== emails.length
        || emails.some(email => !members.items.some(member => member.userId === users.get(email)
          && member.role === (email === emails[0] ? "owner" : "member")))) throw new Error("Development membership verification failed");
    const networks = requireResult(await listNetworks(client, {}, authenticated()));
    if (networks.total !== 1 || networks.items[0]?.id !== path.networkId) throw new Error("Development network verification failed");
    // No session tokens are persisted in the development login file.
    for (const token of sessions) requireResult(await logout(client, { ...request(), token }));
    sessions.length = 0;
    return { server: serverOrigin, console: consoleOrigin, email: emails[0], password: passwords[emails[0]],
      networkId: path.networkId, accounts: passwords };
  } finally {
    // Best effort on a partial seed; failure never triggers account/database deletion.
    await Promise.all(sessions.map(token => logout(client, { ...request(), token })));
  }
}

async function privatePath(path: string, directory = false) {
  const info = await lstat(path);
  if (!(directory ? info.isDirectory() : info.isFile()) || info.uid !== process.getuid?.()
      || (info.mode & 0o777) !== (directory ? 0o700 : 0o600) || (!directory && info.nlink !== 1)) throw new Error("Invalid private seed path");
}

async function passwordsAt(path: string) {
  try { await privatePath(path); }
  catch (error) {
    if (object(error) && error.code === "ENOENT") return freshPasswords();
    throw error;
  }
  return previousPasswords(JSON.parse(await readFile(path, "utf8")));
}

async function publish(path: string, value: Awaited<ReturnType<typeof seed>>) {
  // The previous login was archived by dev.py. Refuse to replace an unexpected file.
  try { await lstat(path); throw new Error("Unexpected development login file"); }
  catch (error) { if (!object(error) || error.code !== "ENOENT") throw error; }
  const temporary = path + ".tmp-" + randomUUID();
  try {
    const file = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
    try { await file.writeFile(JSON.stringify(value, null, 2) + "\n"); await file.sync(); }
    finally { await file.close(); }
    await rename(temporary, path);
    const directory = await open(resolve(path, ".."), constants.O_RDONLY);
    try { await directory.sync(); } finally { await directory.close(); }
  } finally {
    await unlink(temporary).catch(error => { if (!object(error) || error.code !== "ENOENT") throw error; });
  }
}

async function main(args: string[]) {
  if (args[0] === "--check" && args.length === 2 && args[1]) {
    const root = resolve(args[1]);
    await privatePath(join(root, ".meshway"), true);
    await passwordsAt(join(root, ".meshway/dev-login.json"));
    return;
  }
  if (args.length !== 2 || !args[0] || !args[1]) throw new Error("Seed is invoked by dev.py reset server");
  const root = resolve(args[0]);
  const backup = resolve(args[1]);
  if (resolve(backup, "..") !== join(root, ".meshway/backups")) throw new Error("Invalid reset backup path");
  await privatePath(join(root, ".meshway"), true);
  await privatePath(join(root, ".meshway/backups"), true);
  await privatePath(backup, true);
  await privatePath(join(backup, "reset.json"));
  const journal: unknown = JSON.parse(await readFile(join(backup, "reset.json"), "utf8"));
  if (!object(journal) || journal.phase !== "seeding" || journal.backup !== backup) throw new Error("Reset is not ready to seed");
  const passwords = await passwordsAt(join(backup, "dev-login.json"));
  const credentials = await seed(new ControlClient({ baseUrl: "http://127.0.0.1:8420" }), passwords);
  await publish(join(root, ".meshway/dev-login.json"), credentials);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch(() => {
    console.error("Development seed failed; retain the reset backup and inspect private reset state.");
    process.exitCode = 1;
  });
}
