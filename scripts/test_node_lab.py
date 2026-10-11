"""Namespace-safe lab deploy regressions; host mocks only, no Docker/services."""
import json
import hashlib
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import node_lab


class Deploy(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="meshway-node-lab-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / ".meshway").mkdir(mode=0o700)
        self.lab = self.root / ".meshway/node-lab"
        self.lab.mkdir(mode=0o700)
        (self.root / "scripts/dev").mkdir(parents=True)
        (self.root / "scripts/dev/lab.ts").touch()
        self.release = self.lab / "releases" / ("e" * 64)
        self.hashes = {node: str(index + 1) * 64 for index, node in enumerate(node_lab.NODES)}
        self.volumes = {node: {"Name": f"meshway-{node}-state", "CreatedAt": "2026-10-05T00:00:00Z",
                               "Driver": "local", "Scope": "local", "Mountpoint": f"/volumes/{node}/_data",
                               "Labels": {node_lab.LABEL: "1", "com.docker.compose.project": "meshway-node-lab",
                                          "com.docker.compose.volume": node + "-state"}} for node in node_lab.NODES}
        self.containers = {node: self.container(node, self.hashes[node]) for node in node_lab.NODES}
        self.statuses = {node: self.status(node) for node in node_lab.NODES}
        self.actions = []
        self.fail_down = False
        self.fail_down_before = False
        self.down_still_active = False
        self.down_blocked_underlay = False
        self.recreated_identity = {}
        self.fail_compose = False
        self.partial_compose = False
        self.fail_up = False
        self.pending_up = False
        self.fail_stop = None
        self.stop_fails_after_exit = False
        self.fail_start = None
        self.missing_volumes = set()
        self.generation = 0
        self.calls = []
        self.addCleanup(patch.stopall)
        patch.object(node_lab, "ROOT", self.root).start()
        patch.object(node_lab, "selected_release", side_effect=lambda lab: (self.release, {
            "platform": "linux/arm64", "sha256": {"meshwayd": "a" * 64}})).start()
        patch.object(node_lab, "selected_image", return_value={"imageId": "sha256:" + "b" * 64, "sourceDigest": "c" * 64}).start()
        patch.object(node_lab, "deployment_digest", return_value="d" * 64).start()
        patch.object(node_lab, "daemon_proof", side_effect=lambda node, container, status: {
            "sha256": "a" * 64, "release": container["binaryRelease"], "processId": status["processId"],
            "startTicks": 123, "mountNamespace": "mnt:[123]"}).start()
        patch.object(node_lab.subprocess, "run", side_effect=self.external).start()
        patch.object(node_lab, "CONVERGENCE_SECONDS", 0).start()
        self.output = patch("builtins.print").start()

    def container(self, node, config_hash):
        return {"Id": "old-" + node, "RestartCount": 0, "Image": "sha256:" + "b" * 64, "binaryRelease": str(self.release),
                "State": {"Running": True, "Status": "running", "StartedAt": "original-run-" + node, "FinishedAt": ""},
                "Mounts": [{"Type": "volume", "Name": f"meshway-{node}-state", "Source": f"/volumes/{node}/_data",
                            "Destination": "/var/lib/meshway", "RW": True}], "Config": {"Labels": {
            node_lab.LABEL: "1", "com.docker.compose.project": "meshway-node-lab",
            "com.docker.compose.service": node, "com.docker.compose.config-hash": config_hash}}}

    def status(self, node, up=True, mode="tun"):
        return {"processId": 42, "identity": {"nodeId": node + "-identity", "networkId": "lab-network"},
                "desiredUp": up, "networkMode": mode,
                "dataPlane": "controlOnly" if mode == "controlOnly" else "ready" if up else "stopped",
                "phase": ("controlReady" if mode == "controlOnly" else "running") if up else "down",
                "authenticated": up, "lastError": None,
                "saved": {"revision": 7}, "applied": {"revision": 7} if up and mode == "tun" else None}

    def change(self, *nodes):
        for node in nodes:
            self.hashes[node] = "f" * 64

    def external(self, args, **kwargs):
        args = list(args)
        self.calls.append((args, kwargs))
        if args[:1] == ["node"]:
            self.actions.append("configure")
            return subprocess.CompletedProcess(args, 0, "", "")
        self.assertEqual(args[0], "docker")
        if args[1:3] in (["container", "inspect"], ["volume", "inspect"], ["network", "inspect"]):
            kind, name = args[1], args[3]
            if kind == "container":
                node = name.removeprefix("meshway-")
                self.assertIn(node, node_lab.NODES)
                if node not in self.containers:
                    return subprocess.CompletedProcess(args, 1, "", "Error: No such container: " + name)
                value = self.containers[node]
            else:
                if name in self.missing_volumes:
                    return subprocess.CompletedProcess(args, 1, "", "Error: No such volume: " + name)
                role = name.removeprefix("meshway-") if kind == "volume" else "default"
                value = {"Labels": {node_lab.LABEL: "1", "com.docker.compose.project": "meshway-node-lab",
                                    "com.docker.compose." + kind: role}}
                if kind == "volume":
                    value = self.volumes[role.removesuffix("-state")]
            return subprocess.CompletedProcess(args, 0, json.dumps([value]), "")
        if args[1] == "compose":
            self.assertEqual(args[2:4], ["-f", str(self.root / "dev/node-lab/compose.yaml")])
            self.assertEqual(kwargs["env"]["MESHWAY_NODE_RELEASE"], str(self.release))
            if args[4:] == ["config", "--hash", "*"]:
                return subprocess.CompletedProcess(args, 0, "".join(f"{node} {value}\n" for node, value in self.hashes.items()), "")
            self.assertEqual(args[4:11], ["up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "60"])
            selected = args[11:]
            self.assertTrue(selected)
            self.assertTrue(set(selected) <= set(node_lab.NODES))
            self.assertTrue(all(node not in self.containers or self.containers[node]["Config"]["Labels"]["com.docker.compose.config-hash"] != self.hashes[node]
                                for node in selected))
            self.actions.append("compose")
            if self.fail_compose:
                raise subprocess.CalledProcessError(1, args)
            self.generation += 1
            for node in selected:
                container = self.containers.get(node)
                if container and container["Config"]["Labels"]["com.docker.compose.config-hash"] == self.hashes[node]:
                    continue
                self.containers[node] = self.container(node, self.hashes[node])
                self.containers[node]["Id"] = f"new-{node}-{self.generation}"
                self.missing_volumes.discard(f"meshway-{node}-state")
                previous = self.statuses.get(node)
                self.statuses[node] = self.status(node, up=previous["desiredUp"] if previous else False)
                self.statuses[node]["identity"] = previous["identity"] if previous else None
                if self.recreated_identity.get(node):
                    self.statuses[node]["identity"] = self.recreated_identity[node]
                if self.partial_compose:
                    raise subprocess.CalledProcessError(1, args)
            return subprocess.CompletedProcess(args, 0, "", "")
        if args[1] == "stop":
            self.assertEqual(args[2:4], ["--time", "-1"])
            self.assertEqual(len(args), 5)
            self.assertEqual(kwargs["timeout"], 45)
            node = next(node for node, container in self.containers.items() if container["Id"] == args[4])
            journal = node_lab.deploy_journal(self.lab)
            self.assertEqual(journal["phase"], "stop-stopping")
            self.assertTrue(all(entry["quiesced"] for entry in journal["nodes"].values()))
            self.assertTrue(journal["nodes"][node]["stopRequested"])
            self.actions.append("stop:" + node)
            if self.fail_stop == node and not self.stop_fails_after_exit:
                raise subprocess.TimeoutExpired(args, 45)
            self.containers[node]["State"].update(Running=False, Status="exited", FinishedAt="finished-" + node)
            if self.fail_stop == node:
                raise subprocess.TimeoutExpired(args, 45)
            return subprocess.CompletedProcess(args, 0, "", "")
        if args[1] == "start":
            self.assertEqual(len(args), 3)
            self.assertEqual(kwargs["timeout"], 45)
            node = next(node for node, container in self.containers.items() if container["Id"] == args[2])
            journal = node_lab.deploy_journal(self.lab)
            self.assertEqual(journal["phase"], "resuming")
            self.assertTrue(journal["nodes"][node]["startRequested"])
            self.actions.append("start:" + node)
            if self.fail_start == node:
                raise subprocess.CalledProcessError(1, args)
            self.generation += 1
            self.containers[node]["State"].update(Running=True, Status="running", StartedAt=f"restarted-{node}-{self.generation}")
            return subprocess.CompletedProcess(args, 0, "", "")
        if args[1] == "exec":
            node = next(node for node, container in self.containers.items() if container["Id"] == args[2])
            self.assertTrue(self.containers[node]["State"]["Running"])
            self.assertEqual(args[3], "/opt/meshway/meshway")
            command = args[4]
            self.assertEqual(args[5:], ["--json"])
            self.assertEqual(kwargs["timeout"], 45)
            if command == "down":
                self.actions.append("down:" + node)
                # The original intent must already be durable before CLI CAS.
                journal = node_lab.deploy_journal(self.lab)
                self.assertTrue(journal["nodes"][node]["desiredUp"])
                self.assertTrue(journal["nodes"][node]["downRequested"])
                if self.fail_down_before:
                    raise subprocess.CalledProcessError(1, args)
                self.statuses[node].update(desiredUp=False, dataPlane="stopped", phase="down", applied=None, authenticated=False)
                if self.down_still_active:
                    self.statuses[node]["dataPlane"] = "ready"
                if self.down_blocked_underlay:
                    # The intent commits, but a daemon whose underlay never
                    # started reports the problem and the CLI exits nonzero.
                    self.statuses[node].update(dataPlane="failed", phase="blocked", lastError="underlay")
                    raise subprocess.CalledProcessError(1, args)
                if self.fail_down:
                    raise subprocess.CalledProcessError(1, args)
            elif command == "up":
                self.actions.append("up:" + node)
                identity = self.statuses[node]["identity"]
                self.statuses[node] = self.status(node)
                self.statuses[node]["identity"] = identity
                if self.pending_up or self.fail_up:
                    self.statuses[node].update(dataPlane="preparing", phase="synchronizing", applied=None)
                if self.fail_up:
                    raise subprocess.CalledProcessError(1, args)
            else:
                self.assertEqual(command, "status")
            return subprocess.CompletedProcess(args, 0, json.dumps(self.statuses[node]), "")
        self.fail("unexpected external command: " + str(args))

    def deploy(self):
        node_lab.operate("deploy", self.lab)

    def stop(self):
        node_lab.operate("stop", self.lab)

    def test_managed_stop_then_same_hash_deploy_restores_only_original_up(self):
        self.statuses["sha"] = self.status("sha", up=False)
        self.statuses["nyc"] = self.status("nyc", mode="controlOnly")
        originals = {node: container["Id"] for node, container in self.containers.items()}
        self.stop()
        stopped = node_lab.deploy_journal(self.lab)
        self.assertEqual(stopped["phase"], "managed-stopped")
        self.assertEqual(self.actions, ["down:lax", "stop:lax", "stop:sha", "stop:nyc"])
        self.assertTrue(all(not container["State"]["Running"] for container in self.containers.values()))
        self.stop()
        self.assertEqual(node_lab.deploy_journal(self.lab), stopped)
        self.assertEqual(len(self.actions), 4)
        self.deploy()
        self.assertEqual(self.actions[4:], ["start:lax", "start:sha", "start:nyc", "up:lax"])
        self.assertEqual({node: container["Id"] for node, container in self.containers.items()}, originals)
        self.assertEqual([self.statuses[node]["desiredUp"] for node in node_lab.NODES], [True, False, True])
        journal = node_lab.deploy_journal(self.lab)
        self.assertEqual(journal["phase"], "complete")
        for node in node_lab.NODES:
            self.assertEqual(journal["nodes"][node]["identity"], stopped["nodes"][node]["identity"])
            self.assertEqual(journal["nodes"][node]["stateVolume"], stopped["nodes"][node]["stateVolume"])
        node_lab.private(self.lab / "deploy.json")
        self.assertLess((self.lab / "deploy.json").stat().st_size, 32768)

    def test_completed_managed_stop_accepts_new_release_and_config_without_losing_identity(self):
        self.stop()
        stopped = node_lab.deploy_journal(self.lab)
        self.release = self.lab / "releases" / ("d" * 64)
        self.change(*node_lab.NODES)
        self.deploy()
        journal = node_lab.deploy_journal(self.lab)
        self.assertEqual(journal["plan"]["release"], str(self.release))
        self.assertEqual(journal["phase"], "complete")
        self.assertEqual(self.actions[6:], ["start:lax", "start:sha", "start:nyc", "compose", "up:lax", "up:sha", "up:nyc"])
        for node in node_lab.NODES:
            self.assertEqual(self.actions.count("down:" + node), 1)
            self.assertNotEqual(self.containers[node]["Id"], stopped["nodes"][node]["containerId"])
            self.assertEqual(self.statuses[node]["identity"], stopped["nodes"][node]["identity"])
            self.assertTrue(self.statuses[node]["desiredUp"])
            self.assertEqual(journal["nodes"][node]["stateVolume"], stopped["nodes"][node]["stateVolume"])

    def test_managed_stop_down_failure_resumes_original_intent_before_any_container_stop(self):
        for before in (True, False):
            with self.subTest(before=before):
                self.fail_down_before, self.fail_down = before, not before
                offset = len(self.actions)
                with self.assertRaises(subprocess.CalledProcessError):
                    self.stop()
                journal = node_lab.deploy_journal(self.lab)
                self.assertEqual(journal["phase"], "stop-preparing")
                self.assertTrue(journal["nodes"]["lax"]["desiredUp"])
                self.assertEqual(self.actions[offset:], ["down:lax"])
                with self.assertRaisesRegex(node_lab.DevError, "Finish the incomplete managed stop"):
                    self.deploy()
                self.fail_down_before = self.fail_down = False
                self.stop()
                self.deploy()
                self.assertTrue(all(status["desiredUp"] for status in self.statuses.values()))

    def test_managed_stop_never_stops_container_without_confirmed_tun_quiescence(self):
        self.down_still_active = True
        with self.assertRaisesRegex(node_lab.DevError, "did not confirm quiescence"):
            self.stop()
        self.assertEqual(self.actions, ["down:lax"])
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "stop-preparing")

    def test_partial_container_stop_failure_is_resumable_without_force_or_losing_up(self):
        for after in (True, False):
            with self.subTest(after=after):
                self.fail_stop, self.stop_fails_after_exit = "sha", after
                with self.assertRaises(subprocess.TimeoutExpired):
                    self.stop()
                journal = node_lab.deploy_journal(self.lab)
                self.assertEqual(journal["phase"], "stop-stopping")
                self.assertTrue(all(entry["desiredUp"] for entry in journal["nodes"].values()))
                before = self.actions[:]
                self.fail_stop = None
                self.stop()
                self.assertEqual(self.actions[len(before):], ["stop:nyc"] if after else ["stop:sha", "stop:nyc"])
                self.deploy()
                self.assertTrue(all(status["desiredUp"] for status in self.statuses.values()))

    def test_managed_stop_proof_rejects_changed_container_label_volume_mount_or_stop_generation(self):
        self.stop()
        baseline = (self.lab / "deploy.json").read_bytes()
        count = len(self.actions)
        container = self.containers["lax"]
        changes = [(container, "Id", "replaced-container"),
                   (container["Config"]["Labels"], "com.docker.compose.service", "foreign"),
                   (self.volumes["lax"]["Labels"], "com.docker.compose.volume", "foreign"),
                   (self.volumes["lax"], "CreatedAt", "replacement-volume-creation"),
                   (self.volumes["lax"], "Driver", "another-driver"),
                   (self.volumes["lax"], "Options", {"device": "/another-volume"}),
                   (container["Mounts"][0], "Source", "/another-volume"),
                   (container["State"], "StartedAt", "outside-restart"),
                   (container["State"], "FinishedAt", "outside-stop"),
                   (container, "RestartCount", 1)]
        for owner, key, value in changes:
            with self.subTest(key=key), patch.dict(owner, {key: value}):
                with self.assertRaises((node_lab.DevError, SystemExit)):
                    self.deploy()
                self.assertEqual((self.lab / "deploy.json").read_bytes(), baseline)
                self.assertEqual(len(self.actions), count)
        with patch.dict(container["State"], {"Running": True, "Status": "running"}):
            with self.assertRaisesRegex(node_lab.DevError, "restarted outside this plan"):
                self.deploy()
        self.assertEqual(len(self.actions), count)

    def test_identity_changed_while_stopped_fails_before_any_up_or_compose(self):
        self.stop()
        original = node_lab.deploy_journal(self.lab)["nodes"]["lax"]["identity"]
        self.statuses["lax"]["identity"]["nodeId"] = "another-identity"
        count = len(self.actions)
        with self.assertRaisesRegex(node_lab.DevError, "identity/intent/quiescence conflict"):
            self.deploy()
        self.assertEqual(self.actions[count:], ["start:lax"])
        journal = node_lab.deploy_journal(self.lab)
        self.assertEqual(journal["phase"], "resuming")
        self.assertEqual(journal["nodes"]["lax"]["identity"], original)
        self.assertTrue(journal["nodes"]["lax"]["desiredUp"])

    def test_partial_managed_restart_keeps_plan_and_resumes_once(self):
        self.stop()
        self.fail_start = "sha"
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        journal = node_lab.deploy_journal(self.lab)
        self.assertEqual(journal["phase"], "resuming")
        old_release = self.release
        self.release = self.lab / "releases/changed-mid-resume"
        before = self.actions[:]
        with self.assertRaisesRegex(node_lab.DevError, "different release/config plan"):
            self.deploy()
        self.assertEqual(self.actions, before)
        self.release, self.fail_start = old_release, None
        self.deploy()
        self.assertEqual(self.actions[len(before):], ["start:sha", "start:nyc", "up:lax", "up:sha", "up:nyc"])
        self.assertEqual(self.actions.count("start:lax"), 1)

    def test_completed_deploy_cannot_reuse_prior_stop_proof_for_unknown_stopped_container(self):
        self.stop()
        self.deploy()
        before = self.actions[:]
        baseline = (self.lab / "deploy.json").read_bytes()
        self.containers["lax"]["State"].update(Running=False, Status="exited", FinishedAt="unmanaged-stop")
        for operation in (self.deploy, self.stop):
            with self.assertRaisesRegex(node_lab.DevError, "cannot verify Node intent"):
                operation()
            self.assertEqual(self.actions, before)
            self.assertEqual((self.lab / "deploy.json").read_bytes(), baseline)

    def test_stop_cannot_overwrite_an_incomplete_ordinary_deploy(self):
        self.change("lax")
        self.fail_down = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        baseline = (self.lab / "deploy.json").read_bytes()
        with self.assertRaisesRegex(node_lab.DevError, "Finish the incomplete deploy"):
            self.stop()
        self.assertEqual((self.lab / "deploy.json").read_bytes(), baseline)
        self.assertEqual(self.actions, ["down:lax"])

    def test_same_hash_never_downs_or_restores(self):
        self.deploy()
        self.assertEqual(self.actions, [])
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "complete")

    def test_daemon_replacement_or_image_change_cannot_reuse_plan(self):
        self.change("lax")
        self.fail_down = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        self.statuses["nyc"]["processId"] += 1
        with self.assertRaisesRegex(node_lab.DevError, "daemon process/image/release conflict"):
            self.deploy()
        self.assertEqual(self.actions, ["down:lax"])

    def test_matching_compose_hash_does_not_hide_wrong_running_image(self):
        self.containers["lax"]["Image"] = "sha256:" + "9" * 64
        with self.assertRaisesRegex(node_lab.DevError, "daemon process/image/release conflict"):
            self.deploy()
        self.assertEqual(self.actions, [])

    def test_rebuild_stops_only_changed_tun_before_compose_then_restores(self):
        self.change("lax", "sha")
        self.statuses["sha"] = self.status("sha", mode="controlOnly")
        original_ids = {node: container["Id"] for node, container in self.containers.items()}
        self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose", "up:lax"])
        journal = node_lab.deploy_journal(self.lab)
        for node in node_lab.NODES:
            self.assertEqual(journal["nodes"][node]["containerId"], original_ids[node])
            self.assertTrue(journal["nodes"][node]["desiredUp"])
            self.assertEqual(journal["nodes"][node]["identity"], self.statuses[node]["identity"])
        node_lab.private(self.lab / "deploy.json")
        self.assertLess((self.lab / "deploy.json").stat().st_size, 32768)

    def test_down_failure_retains_original_up_and_same_plan_can_resume(self):
        self.change("lax")
        self.fail_down = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        self.assertEqual(self.actions, ["down:lax"])
        self.assertFalse(self.statuses["lax"]["desiredUp"])
        self.assertTrue(node_lab.deploy_journal(self.lab)["nodes"]["lax"]["desiredUp"])
        self.fail_down = False
        self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose", "up:lax"])

    def test_down_must_confirm_dataplane_stopped_before_compose(self):
        self.change("lax")
        self.down_still_active = True
        with self.assertRaisesRegex(node_lab.DevError, "did not confirm stopped"):
            self.deploy()
        self.assertEqual(self.actions, ["down:lax"])
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "preparing")

    def test_blocked_underlay_without_installation_counts_as_quiesced(self):
        self.change("lax")
        self.down_blocked_underlay = True
        self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose", "up:lax"])
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "complete")

    def test_blocked_daemon_without_identity_adopts_first_observed_identity(self):
        self.change("lax")
        self.down_blocked_underlay = True
        self.statuses["lax"]["identity"] = None
        self.recreated_identity["lax"] = {"nodeId": "lax-identity", "networkId": "lab-network"}
        self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose", "up:lax"])
        journal = node_lab.deploy_journal(self.lab)
        self.assertEqual(journal["phase"], "complete")
        self.assertEqual(journal["nodes"]["lax"]["identity"], {"nodeId": "lax-identity", "networkId": "lab-network"})

    def test_blocked_underlay_with_desired_up_is_not_quiesced(self):
        self.assertFalse(node_lab.underlay_blocked(
            {"desiredUp": True, "phase": "blocked", "dataPlane": "failed", "lastError": "underlay"}))
        self.assertFalse(node_lab.underlay_blocked(
            {"desiredUp": False, "phase": "blocked", "dataPlane": "failed", "lastError": "tun"}))

    def test_compose_failure_and_partial_recreation_resume_original_up(self):
        for partial in (False, True):
            with self.subTest(partial=partial):
                self.change("lax", "sha")
                self.fail_compose, self.partial_compose = not partial, partial
                with self.assertRaises(subprocess.CalledProcessError):
                    self.deploy()
                journal = node_lab.deploy_journal(self.lab)
                self.assertEqual(journal["phase"], "composing")
                self.assertTrue(journal["nodes"]["lax"]["desiredUp"])
                self.fail_compose = self.partial_compose = False
                self.deploy()
                self.assertTrue(self.statuses["lax"]["desiredUp"])
                self.assertTrue(self.statuses["sha"]["desiredUp"])
                self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "complete")
                # Make the next iteration another real replacement.
                for node in ("lax", "sha"):
                    self.containers[node]["Config"]["Labels"]["com.docker.compose.config-hash"] = "a" * 64
        self.assertEqual(self.actions.count("down:lax"), 2)
        self.assertEqual(self.actions.count("up:lax"), 2)

    def test_retry_refuses_changed_release_hash_identity_or_original_container(self):
        self.change("lax")
        self.fail_down = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        baseline = (self.lab / "deploy.json").read_bytes()
        changes = [(self, "release", self.lab / "releases/another"),
                   (self.hashes, "nyc", "b" * 64),
                   (self.statuses["nyc"]["identity"], "nodeId", "different"),
                   (self.statuses["nyc"]["identity"], "networkId", "different"),
                   (self.containers["lax"], "Id", "foreign-replacement")]
        for owner, key, value in changes:
            with self.subTest(key=key), patch.dict(owner, {key: value}) if isinstance(owner, dict) else patch.object(owner, key, value):
                with self.assertRaisesRegex(node_lab.DevError, "different release/config plan|conflict"):
                    self.deploy()
                self.assertEqual((self.lab / "deploy.json").read_bytes(), baseline)
                self.assertEqual(self.actions, ["down:lax"])

    def test_compose_retry_can_recreate_removed_container_only_with_retained_state_volume(self):
        self.change("lax")
        self.fail_compose = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        del self.containers["lax"]
        self.missing_volumes.add("meshway-lax-state")
        with self.assertRaisesRegex(node_lab.DevError, "state volume is missing"):
            self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose"])
        self.missing_volumes.clear()
        self.fail_compose = False
        self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose", "compose", "up:lax"])
        self.assertEqual(self.statuses["lax"]["identity"]["nodeId"], "lax-identity")

    def test_foreign_compose_role_or_project_is_rejected_before_mutation(self):
        labels = self.containers["lax"]["Config"]["Labels"]
        for key in (node_lab.LABEL, "com.docker.compose.project", "com.docker.compose.service"):
            with self.subTest(key=key), patch.dict(labels, {key: "foreign"}):
                with self.assertRaisesRegex(SystemExit, "not owned"):
                    self.deploy()
                self.assertFalse((self.lab / "deploy.json").exists())
        self.assertEqual(self.actions, [])

    def test_new_and_previously_down_nodes_never_auto_up(self):
        del self.containers["lax"], self.statuses["lax"]
        self.missing_volumes.add("meshway-lax-state")
        self.statuses["sha"] = self.status("sha", up=False)
        self.statuses["nyc"] = self.status("nyc", up=False, mode="controlOnly")
        self.change("sha", "nyc")
        self.deploy()
        self.assertEqual(self.actions, ["compose"])
        self.assertTrue(all(not status["desiredUp"] for status in self.statuses.values()))

    def test_stopped_existing_container_cannot_skip_namespace_preflight(self):
        self.containers["lax"]["State"]["Running"] = False
        with self.assertRaisesRegex(node_lab.DevError, "cannot verify Node intent"):
            self.deploy()
        self.assertEqual(self.actions, [])

    def test_orphaned_state_volume_is_not_treated_as_a_fresh_down_node(self):
        del self.containers["lax"]
        with self.assertRaisesRegex(node_lab.DevError, "namespace/intent is unknown"):
            self.deploy()
        self.assertEqual(self.actions, [])

    def test_up_failure_keeps_restoration_phase_and_retry_does_not_recreate(self):
        self.change("lax")
        self.fail_up = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "restoring")
        self.fail_up = False
        with self.assertRaisesRegex(node_lab.DevError, "pending or failed"):
            self.deploy()
        self.statuses["lax"] = self.status("lax")
        self.deploy()
        self.assertEqual(self.actions, ["down:lax", "compose", "up:lax"])

    def test_ipc_success_without_applied_is_not_deploy_success(self):
        self.change("lax")
        self.pending_up = True
        with self.assertRaisesRegex(node_lab.DevError, "pending or failed"):
            self.deploy()
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "restoring")
        self.assertTrue(any('"dataPlane": "preparing"' in str(call) for call in self.output.call_args_list))

    def test_restored_intent_converging_through_a_transient_retry_completes(self):
        self.change("lax")
        self.pending_up = True
        retrying = dict(self.status("lax"), phase="retrying", lastError="network", applied=None)
        polls = []

        def sleep(_seconds):
            polls.append(True)
            self.statuses["lax"] = self.status("lax")
        patch.object(node_lab, "CONVERGENCE_SECONDS", 60).start()
        patch.object(node_lab.time, "sleep", side_effect=sleep).start()
        real = node_lab.snapshot

        def snapshot(*args, **kwargs):
            containers, statuses = real(*args, **kwargs)
            if not polls and statuses.get("lax", {}).get("desiredUp"):
                statuses["lax"] = retrying
            return containers, statuses
        patch.object(node_lab, "snapshot", side_effect=snapshot).start()
        self.deploy()
        self.assertTrue(polls)
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "complete")

    def test_blocked_restoration_fails_without_waiting(self):
        self.change("lax")
        self.pending_up = True
        patch.object(node_lab, "CONVERGENCE_SECONDS", 60).start()
        sleep = patch.object(node_lab.time, "sleep").start()
        real = node_lab.snapshot

        def snapshot(*args, **kwargs):
            containers, statuses = real(*args, **kwargs)
            if statuses.get("lax", {}).get("desiredUp"):
                statuses["lax"] = dict(statuses["lax"], phase="blocked", lastError="credentials")
            return containers, statuses
        patch.object(node_lab, "snapshot", side_effect=snapshot).start()
        with self.assertRaisesRegex(node_lab.DevError, "pending or failed"):
            self.deploy()
        sleep.assert_not_called()

    def test_post_compose_identity_change_is_rejected_and_preserves_journal(self):
        self.change("lax")
        self.partial_compose = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        self.statuses["lax"]["identity"]["nodeId"] = "foreign"
        before = self.actions[:]
        with self.assertRaisesRegex(node_lab.DevError, "identity conflict"):
            self.deploy()
        self.assertEqual(self.actions, before)
        self.assertEqual(node_lab.deploy_journal(self.lab)["phase"], "composing")

    def test_nonprivate_oversized_and_symlink_journals_are_rejected(self):
        path = self.lab / "deploy.json"
        node_lab.save(path, {"oversized": "x" * 32768})
        with self.assertRaisesRegex(node_lab.DevError, "size bound"):
            self.deploy()
        path.chmod(0o644)
        with self.assertRaisesRegex(node_lab.DevError, "private path"):
            self.deploy()
        path.unlink()
        target = self.lab / "target.json"
        node_lab.save(target, {})
        path.symlink_to(target)
        with self.assertRaisesRegex(node_lab.DevError, "private path"):
            self.deploy()
        self.assertEqual(self.actions, [])

    def test_configure_uses_owned_nodes_common_login_and_operation_lock(self):
        # Preserve the existing lock inode when upgrading its old default mode.
        lock_path = self.lab / "operation.lock"
        lock_path.touch(mode=0o644)
        lock_inode = lock_path.stat().st_ino
        with patch.object(node_lab, "lab_dir", return_value=self.lab):
            node_lab.main(["configure"])
        command = next(args for args, _ in self.calls if args[0] == "node")
        self.assertEqual(command, ["node", "--experimental-strip-types", str(self.root / "scripts/dev/lab.ts"),
                                   str(self.root), str(self.lab.parent / "dev-login.json"),
                                   "meshway-lax", "meshway-sha", "meshway-nyc"])
        self.assertEqual(self.actions, ["configure"])
        path = self.lab / "operation.lock"
        node_lab.private(path)
        self.assertEqual(path.stat().st_ino, lock_inode)
        with path.open("r+") as lock, patch.object(node_lab, "lab_dir", return_value=self.lab):
            node_lab.fcntl.flock(lock, node_lab.fcntl.LOCK_EX | node_lab.fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                node_lab.main(["configure"])
        self.assertEqual(self.actions, ["configure"])

    def test_configure_refuses_foreign_nodes_or_unfinished_namespace_transition(self):
        self.change("lax")
        self.fail_down = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy()
        with self.assertRaisesRegex(node_lab.DevError, "Finish the incomplete deploy"):
            node_lab.operate("configure", self.lab)
        self.containers["nyc"]["Config"]["Labels"][node_lab.LABEL] = "foreign"
        with self.assertRaisesRegex(SystemExit, "not owned"):
            node_lab.operate("configure", self.lab)
        self.assertEqual(self.actions, ["down:lax"])


class ImageSelection(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="meshway-image-selection-")
        self.addCleanup(self.temp.cleanup)
        self.lab = Path(self.temp.name)
        self.directory = self.lab / "images/candidate"
        self.directory.mkdir(parents=True, mode=0o700)
        self.manifest = {"status": "built", "platform": "linux/arm64", "imageId": "sha256:" + "a" * 64,
                         "sourceDigest": "b" * 64, "base": node_lab.base_image(),
                         "packages": [{"name": "iproute2"}], "sources": {"dev/node-lab/Dockerfile": "c" * 64}}
        self.image = {"Id": self.manifest["imageId"], "Os": "linux", "Architecture": "arm64", "Config": {
            "Labels": {"io.meshway.node-lab.image": "1", "io.meshway.node-lab.source": "b" * 64},
            "Entrypoint": ["/usr/local/lib/meshway-lab/entrypoint.sh"]}}
        node_lab.save(self.lab / "selected-image.json", {"manifest": str(self.directory / "image-manifest.json")})
        self.save_manifest()
        self.addCleanup(patch.stopall)
        patch.object(node_lab, "image_builder", return_value=SimpleNamespace(source_digest=lambda: "b" * 64)).start()
        self.run = patch.object(node_lab, "run", side_effect=lambda *args, **kw: json.dumps([self.image])).start()

    def save_manifest(self):
        (self.directory / "image-manifest.json").write_text(json.dumps(self.manifest))

    def test_selection_reads_only_immutable_local_id(self):
        self.assertEqual(node_lab.selected_image(self.lab, "linux/arm64"), self.manifest)
        self.run.assert_called_once_with("docker", "image", "inspect", self.manifest["imageId"], capture=True, timeout=45)

    def test_stale_source_wrong_arch_foreign_image_volume_and_entrypoint_are_rejected(self):
        for owner, key, value in [
            (self.manifest, "sourceDigest", "c" * 64), (self.manifest, "platform", "linux/amd64"),
            (self.manifest, "status", "candidate"), (self.manifest, "imageId", "ubuntu:latest"),
            (self.image, "Architecture", "amd64"), (self.image, "Id", "sha256:" + "c" * 64),
            (self.image["Config"], "Volumes", {"/state": {}}), (self.image["Config"], "User", "1000"),
            (self.image["Config"], "Entrypoint", ["/bin/sh"]),
            (self.image["Config"]["Labels"], "io.meshway.node-lab.source", "c" * 64),
            (self.manifest, "packages", []), (self.manifest, "sources", {}),
        ]:
            with self.subTest(key=key), patch.dict(owner, {key: value}):
                self.save_manifest()
                with self.assertRaises(node_lab.DevError):
                    node_lab.selected_image(self.lab, "linux/arm64")
            self.save_manifest()

    def test_selection_cannot_escape_private_image_directory(self):
        node_lab.save(self.lab / "selected-image.json", {"manifest": str(self.lab / "foreign/image-manifest.json")})
        with self.assertRaisesRegex(node_lab.DevError, "outside this lab"):
            node_lab.selected_image(self.lab, "linux/arm64")
        self.run.assert_not_called()

    def test_failed_built_image_verification_retains_previous_selection(self):
        before = (self.lab / "selected-image.json").read_bytes()
        self.image["Config"]["Volumes"] = {"/unexpected": {}}
        builder = SimpleNamespace(source_digest=lambda: "b" * 64, build=lambda output, platform: self.manifest)
        with patch.object(node_lab, "image_builder", return_value=builder), patch.object(
                node_lab, "docker_target", return_value=("aarch64-unknown-linux-musl", "linux/arm64", 183)):
            with self.assertRaises(node_lab.DevError):
                node_lab.build_image(self.lab)
        self.assertEqual((self.lab / "selected-image.json").read_bytes(), before)


class DaemonProof(unittest.TestCase):
    def setUp(self):
        self.digest = hashlib.sha256(b"daemon binary, distinct from supervisor").hexdigest()
        self.container = {"Id": "owned-container", "Mounts": [
            {"Destination": "/opt/meshway", "Source": "/releases/exact", "Type": "bind", "RW": False}]}
        # /proc stat field 22 is the boot-relative start time; command may have spaces.
        fields = ["S"] + ["0"] * 18 + ["12345"]
        self.lines = ["/opt/meshway/meshwayd", self.digest + "  /proc/42/exe",
                      "42 (meshway daemon) " + " ".join(fields), "Uid:\t0\t0\t0\t0", "mnt:[7]", "mnt:[7]",
                      json.dumps({"sha256": {"meshwayd": self.digest}})]

    def test_hashes_status_pid_and_checks_exact_executable_mount_uid_namespace(self):
        with patch.object(node_lab, "run", return_value="\n".join(self.lines)) as run:
            proof = node_lab.daemon_proof("lax", self.container, {"processId": 42})
        self.assertEqual(proof["processId"], 42)
        self.assertEqual(proof["sha256"], self.digest)
        self.assertEqual(proof["startTicks"], 12345)
        command = run.call_args.args[5]
        self.assertIn("sha256sum /proc/42/exe", command)
        self.assertNotIn("sha256sum /proc/1/exe", command)

    def test_substituted_process_uid_namespace_or_hash_is_not_a_valid_receipt(self):
        for index, value in [(0, "/usr/bin/python3"), (1, "0" * 64 + "  /proc/42/exe"),
                             (3, "Uid: 1000 1000 1000 1000"), (4, "mnt:[8]"),
                             (2, self.lines[2].replace("42 (", "43 ("))]:
            with self.subTest(index=index):
                lines = self.lines.copy()
                lines[index] = value
                with patch.object(node_lab, "run", return_value="\n".join(lines)):
                    with self.assertRaises(node_lab.DevError):
                        node_lab.daemon_proof("lax", self.container, {"processId": 42})
        for pid in (True, 0, -1, "42; anything", 4194305):
            with self.subTest(pid=pid), self.assertRaises(node_lab.DevError):
                node_lab.daemon_proof("lax", self.container, {"processId": pid})
        self.container["Mounts"][0]["RW"] = True
        with self.assertRaisesRegex(node_lab.DevError, "read-only bind"):
            node_lab.daemon_proof("lax", self.container, {"processId": 42})


class ImageBuildPreflight(unittest.TestCase):
    def test_native_engine_and_default_builder_are_required(self):
        builder = node_lab.image_builder()
        native = {"ID": "engine-a", "OSType": "linux", "Architecture": "aarch64"}
        for outputs, message in [
            ([json.dumps({**native, "Architecture": "x86_64"})], "native Linux Docker architecture"),
            ([json.dumps(native), json.dumps({**native, "ID": "engine-b"})], "selected Docker engine"),
            ([json.dumps(native), json.dumps(native), "Driver: docker-container\n"], "ordinary default Docker builder"),
        ]:
            with self.subTest(message=message), tempfile.TemporaryDirectory() as temporary, patch.object(
                    builder.shutil, "disk_usage", return_value=SimpleNamespace(free=60 * 1024 ** 3)), patch.object(
                    builder.subprocess, "check_output", side_effect=outputs), patch.object(builder.subprocess, "run") as run:
                output = Path(temporary) / "new-output"
                with self.assertRaisesRegex(RuntimeError, message):
                    builder.build(output, "linux/arm64")
                self.assertFalse(output.exists())
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()


class ImageManifestShapeTest(unittest.TestCase):
    """An existing image and a freshly built one have the same manifest shape."""

    def archive(self, packages):
        import io, tarfile
        data = json.dumps(packages).encode()
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as tar:
            info = tarfile.TarInfo("packages.json")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        return buffer.getvalue()

    def test_existing_image_is_described_with_its_baked_package_list(self):
        builder = node_lab.image_builder()
        lock = json.loads((node_lab.ROOT / "dev/node-lab/assets.lock.json").read_text())
        packages = [{"name": name, "version": "1", "architecture": "arm64", "filename": "pool/" + name + ".deb",
                     "sha256": "0" * 64} for name in lock["packages"]]
        image = {"Id": "sha256:" + "a" * 64, "Os": "linux", "Architecture": "arm64",
                 "Config": {"Labels": {"io.meshway.node-lab.source": builder.source_digest()},
                            "Entrypoint": ["/usr/local/lib/meshway-lab/entrypoint.sh"]}}
        outputs = iter([json.dumps([image]), "created\n", self.archive(packages)])
        with patch.object(builder.subprocess, "check_output", side_effect=lambda *a, **k: next(outputs)) as calls, \
                patch.object(builder.subprocess, "run") as remove:
            manifest = builder.describe(image["Id"], "linux/arm64")
        self.assertEqual(manifest["packages"], packages)
        self.assertEqual(set(manifest) - {"buildCommand", "provenance"},
                         {"sourceDigest", "platform", "base", "packages", "sources", "imageId", "status"})
        self.assertEqual(manifest["status"], "built")
        self.assertIn("--network", calls.call_args_list[1].args[0])
        self.assertEqual(remove.call_args.args[0], ["docker", "rm", "-f", "created"])
        # A package list missing a locked package is not that image's manifest.
        outputs = iter([json.dumps([image]), "created\n", self.archive(packages[1:])])
        with patch.object(builder.subprocess, "check_output", side_effect=lambda *a, **k: next(outputs)), \
                patch.object(builder.subprocess, "run"):
            with self.assertRaises(RuntimeError):
                builder.describe(image["Id"], "linux/arm64")

    def test_a_manifest_without_packages_is_not_a_verified_image(self):
        manifest = {"status": "built", "platform": "linux/arm64", "sourceDigest": node_lab.image_builder().source_digest(),
                    "base": node_lab.base_image(), "imageId": "sha256:" + "a" * 64}
        with self.assertRaises(node_lab.DevError):
            node_lab.verify_image(manifest, "linux/arm64")
