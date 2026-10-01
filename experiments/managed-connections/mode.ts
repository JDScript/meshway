import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";
import { record } from "./local.ts";
import { epochFloor, upgradeSnapshot } from "./upgrade.ts";
import type { EpochFloor, UpgradeSnapshot } from "./upgrade.ts";

export type DemoMode = "neighbors" | "linux-ipv4-tun";
export const TUN_NAME = "meshway0";
export const TUN_MTU = 1000;
export const TUN_ROUTE_PROTOCOL = "242";
export const BOOTSTRAP_CAPS = ["NET_ADMIN", "SETGID", "SETPCAP", "SETUID"];

export function demoMode(value: unknown = "neighbors"): DemoMode {
  assert(value === "neighbors" || value === "linux-ipv4-tun", "unrecognized demo mode");
  return value;
}

export function bootstrapArguments(user: string, netAdmin = true): string[] {
  assert(/^[1-9][0-9]*:[0-9]+$/.test(user), "non-root recorded UID:GID required");
  const [uid, gid] = user.split(":");
  assert(uid && gid);
  return ["--reuid", uid, "--regid", gid, "--clear-groups", `--bounding-set=-all${netAdmin ? ",+net_admin" : ""}`,
    `--inh-caps=-all${netAdmin ? ",+net_admin" : ""}`, `--ambient-caps=-all${netAdmin ? ",+net_admin" : ""}`, "--no-new-privs"];
}

export function agentArguments(mode: DemoMode): string[] {
  return ["run", ...(mode === "linux-ipv4-tun" ? ["--apply-dataplane", "--tun-name", TUN_NAME, "--tun-mtu", String(TUN_MTU)] : ["--apply-connections"]),
    "--interval-seconds", "2", "--connection-lease-seconds", "15", "--connection-profiles", "/state/profiles.json", "--state", "/state/agent.json"];
}

export type ModeSnapshot = UpgradeSnapshot & { mode: DemoMode };
export type ModeJournal = {
  version: 1; kind: "dataplane-mode";
  source_mode: DemoMode;
  target_mode: DemoMode;
  phase: "prepared" | "restarting" | "verifying" | "complete";
  epoch_floor: EpochFloor | null;
  original: ModeSnapshot;
};

/** A separate strict journal: image changes and mode changes never share intent. */
export function modeJournal(value: unknown): ModeJournal {
  const entry = record(value); const original = record(entry.original);
  assert.deepEqual(Object.keys(entry).sort(), ["version", "kind", "source_mode", "target_mode", "phase", "epoch_floor", "original"].sort());
  assert(entry.version === 1 && entry.kind === "dataplane-mode", "unrecognized mode journal");
  assert(typeof original.mode === "string" && typeof entry.source_mode === "string" && typeof entry.target_mode === "string", "explicit mode required");
  const { mode, ...snapshot } = original;
  assert(entry.phase === "prepared" || entry.phase === "restarting" || entry.phase === "verifying" || entry.phase === "complete", "invalid mode phase");
  const floor = entry.epoch_floor === null ? null : epochFloor(entry.epoch_floor);
  assert((entry.phase === "prepared") === (floor === null), "mode phase lacks runtime epoch fence");
  return { version: 1, kind: "dataplane-mode", source_mode: demoMode(entry.source_mode), target_mode: demoMode(entry.target_mode), phase: entry.phase,
    epoch_floor: floor, original: { ...upgradeSnapshot(snapshot), mode: demoMode(mode) } };
}

export function assertModeSnapshot(journal: ModeJournal, current: ModeSnapshot): void {
  const { mode: _before, ...original } = journal.original;
  const { mode: _after, ...snapshot } = current;
  assert(isDeepStrictEqual(snapshot, original), "image, immutable material/binding or controller plan changed during mode transition");
  assert([journal.original.mode, journal.source_mode, journal.target_mode].includes(current.mode), "unexpected persisted mode");
}

export type ModeActions = {
  snapshot(): Promise<ModeSnapshot>;
  preflight(target: DemoMode, allowedModes: DemoMode[]): Promise<void>;
  saveJournal(journal: ModeJournal): Promise<void>;
  stop(): Promise<void>;
  epochs(): Promise<EpochFloor>;
  replacementsRunning(mode: DemoMode): Promise<boolean>;
  pin(mode: DemoMode): Promise<void>;
  replaceContainers(mode: DemoMode): Promise<void>;
  start(): Promise<void>;
  healthy(floor: EpochFloor | null): Promise<void>;
  finish(): Promise<void>;
};

/** No enroll, publish, access grant or routing mutation exists in this lifecycle. */
export async function changeMode(target: DemoMode, pending: ModeJournal | null, actions: ModeActions, restart = false): Promise<void> {
  const initial = await actions.snapshot();
  if (pending) assertModeSnapshot(pending, initial);
  await actions.preflight(target, pending ? [pending.original.mode, pending.source_mode, pending.target_mode] : [initial.mode]);
  if (!pending && target === initial.mode && !restart) {
    await actions.healthy(null);
    assert.deepEqual(await actions.snapshot(), initial);
    return;
  }
  const journal: ModeJournal = pending ? structuredClone(pending) : { version: 1, kind: "dataplane-mode", source_mode: initial.mode, target_mode: target,
    phase: "prepared", epoch_floor: null, original: initial };
  const check = async () => assertModeSnapshot(journal, await actions.snapshot());
  await check();
  if (journal.target_mode === target && (journal.phase === "verifying" || journal.phase === "complete"))
    assert(initial.mode === target, "metadata mode differs from replacement cohort");
  if (journal.target_mode !== target || ((journal.phase === "verifying" || journal.phase === "complete") && !await actions.replacementsRunning(target))) {
    journal.source_mode = initial.mode; journal.target_mode = target; journal.phase = "prepared"; journal.epoch_floor = null;
  }
  if (journal.phase === "prepared") {
    await actions.saveJournal(journal); // Explicit target is durable BEFORE stop.
    await actions.stop();
    await check();
    journal.epoch_floor = await actions.epochs(); journal.phase = "restarting";
    await actions.saveJournal(modeJournal(journal));
  }
  if (journal.phase === "restarting") {
    await actions.pin(target);
    await actions.replaceContainers(target);
    await check();
    journal.phase = "verifying"; await actions.saveJournal(journal);
    await actions.start();
  }
  assert(journal.epoch_floor !== null);
  await actions.healthy(journal.epoch_floor);
  await check();
  journal.phase = "complete"; await actions.saveJournal(journal);
  await actions.finish();
}
