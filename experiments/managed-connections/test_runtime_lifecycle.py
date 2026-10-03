"""Host fault injection of Docker commit/return windows; no Docker daemon calls."""
from contextlib import nullcontext, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import build


spec = importlib.util.spec_from_file_location("runtime_smoke", Path(__file__).with_name("runtime-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
A, B = ("sha256:" + char * 64 for char in ("a", "b"))


class Daemon:
    def __init__(self, interrupt=None, timeout=False, force_image=None):
        self.interrupt, self.timeout, self.force_image = interrupt, timeout, force_image
        self.containers, self.networks, self.calls, self.started_images = {}, {}, [], []

    def run(self, argv, **kwargs):
        args = argv[1:]
        self.calls.append(args)
        result = ""
        if args[:2] == ["image", "inspect"]:
            # The user-provided mutable tag points to B after this observation.
            result = [{"Id": A, "Os": "linux", "Architecture": "arm64", "Config": {}}]
        elif args[0] == "info":
            result = "arm64"
        elif args[:2] == ["buildx", "inspect"]:
            result = "Driver: docker"
        elif args[:2] == ["image", "ls"]:
            result = ""
        elif args[0] == "create":
            name = args[args.index("--name") + 1]
            image = self.force_image or (A if A in args else B)
            labels = dict(args[i + 1].split("=", 1) for i, arg in enumerate(args[:-1]) if arg == "--label")
            self.containers[name] = {"Id": name + "-id", "Name": "/" + name, "Image": image,
                "Config": {"Labels": labels}, "State": {"Running": False},
                "HostConfig": {"Privileged": False, "NetworkMode": args[args.index("--network") + 1]}}
            result = name + "-id"
            self.after_commit("container", argv)
        elif args[:2] == ["network", "create"]:
            name = args[-1]
            labels = dict(args[i + 1].split("=", 1) for i, arg in enumerate(args[:-1]) if arg == "--label")
            self.networks[name] = {"Id": name + "-id", "Name": name, "Labels": labels}
            result = name + "-id"
            self.after_commit("network", argv)
        elif args[0] == "ps":
            result = "\n".join(json.dumps({"ID": v["Id"], "Names": n}) for n, v in self.containers.items())
        elif args[:2] == ["network", "ls"]:
            result = "\n".join(json.dumps({"ID": v["Id"], "Name": n}) for n, v in self.networks.items())
        elif args[0] == "inspect":
            result = [next(v for v in self.containers.values() if args[1] in (v["Id"], v["Name"].lstrip("/")))]
        elif args[:2] == ["network", "inspect"]:
            result = [next(v for v in self.networks.values() if args[2] in (v["Id"], v["Name"]))]
        elif args[0] == "start":
            value = next(v for v in self.containers.values() if args[1] == v["Id"])
            value["State"]["Running"] = True
            self.started_images.append(value["Image"])
        elif args[0] == "exec":
            if "python3" in args:
                result = {"uid": os.getuid(), "gid": os.getgid(), "caps": "0000000000001000", "nnp": "1"}
        elif args[0] == "rm":
            name = next(n for n, v in self.containers.items() if v["Id"] == args[-1])
            del self.containers[name]
        elif args[:2] == ["network", "rm"]:
            name = next(n for n, v in self.networks.items() if v["Id"] == args[-1])
            del self.networks[name]
        else:
            raise AssertionError(args)
        return subprocess.CompletedProcess(argv, 0, result if isinstance(result, str) else json.dumps(result), "")

    def after_commit(self, kind, argv):
        if self.interrupt == kind:
            if self.timeout:
                raise subprocess.TimeoutExpired(argv, 120)
            signal.raise_signal(signal.SIGTERM)


def one_node(docker, container, owner, logs, result, networks):
    name = owner + "-network"
    docker("network", "create", "--label", "meshway.underlay.acceptance=" + owner, name)
    networks.append(name)
    container("agent", name, "agent")


class RuntimeLifecycle(unittest.TestCase):
    def run_smoke(self, daemon):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(smoke, "TEMP", Path(directory)), \
                    patch.object(smoke, "performance_lock", lambda: nullcontext(999)), \
                    patch.object(smoke, "validate_descriptor", lambda _fd: None), \
                    patch.object(smoke, "normal_acceptance", one_node), \
                    patch.object(smoke.subprocess, "run", daemon.run), redirect_stdout(io.StringIO()):
                code = smoke.run("meshway-managedtest:mutable")
            path = next(Path(directory).glob("meshway-runtime-checks/*/result.json"))
            return code, json.loads(path.read_text())

    def test_build_cancellation_cleans_committed_extraction_before_return(self):
        daemon = Daemon(interrupt="container")
        with patch.object(build, "performance_lock", lambda: nullcontext(999)), \
                patch.object(build, "validate_descriptor", lambda _fd: None), \
                patch.object(build, "inventory", lambda _root: {"format_version": 1, "files": {}}), \
                patch.object(build.subprocess, "run", daemon.run):
            with self.assertRaises(KeyboardInterrupt):
                build.build(SimpleNamespace(source_root=Path.cwd(), binary_image=A,
                    tag="meshway-managedtest:new", developer=False))
        self.assertEqual(daemon.containers, {})

    def test_smoke_cancellation_cleans_network_and_container_commit_windows(self):
        for kind in ("network", "container"):
            with self.subTest(kind=kind):
                daemon = Daemon(interrupt=kind)
                code, receipt = self.run_smoke(daemon)
                self.assertEqual(code, 1)
                self.assertEqual(daemon.containers, {})
                self.assertEqual(daemon.networks, {})
                self.assertEqual(receipt["cleanup_errors"], [])
                self.assertTrue(all(item["settled"] for item in receipt["creation_intents"]))

    def test_timeout_does_not_report_certain_cleanup_for_unknown_create_result(self):
        daemon = Daemon(interrupt="container", timeout=True)
        code, receipt = self.run_smoke(daemon)
        self.assertEqual(code, 1)
        self.assertEqual(daemon.containers, {})
        self.assertTrue(any("uncertain" in message for message in receipt["cleanup_errors"]))

    def test_tag_change_cannot_change_started_image(self):
        daemon = Daemon()
        code, receipt = self.run_smoke(daemon)
        self.assertEqual(code, 0)
        self.assertEqual(receipt["image"], A)
        self.assertEqual(daemon.started_images, [A])
        self.assertEqual(daemon.containers, {})

    def test_wrong_inspected_image_is_rejected_before_start_and_not_removed(self):
        daemon = Daemon(force_image=B)
        code, receipt = self.run_smoke(daemon)
        self.assertEqual(code, 1)
        self.assertEqual(daemon.started_images, [])
        self.assertEqual(len(daemon.containers), 1)
        self.assertTrue(receipt["cleanup_errors"])


if __name__ == "__main__":
    unittest.main()
