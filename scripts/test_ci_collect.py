"""Offline Actions/report fixtures and isolated Git publication checks."""
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from ci import collect as ci

FIXTURES = Path(__file__).parent / "ci/fixtures"
REPO = "JDScript/meshway"


def fixture(name):
    return json.loads((FIXTURES / f"{name}.json").read_text())


def archive(files):
    data = io.BytesIO()
    with zipfile.ZipFile(data, "w", zipfile.ZIP_DEFLATED) as output:
        for name, body in files.items():
            output.writestr(name, body if isinstance(body, bytes) else json.dumps(body))
    return data.getvalue()


class FakeAPI:
    def __init__(self):
        self.data = fixture("actions")
        self.downloads = []
        self.archives = {}

    def pages(self, endpoint, key, limit=None):
        if endpoint == "runs":
            return self.data["workflow_runs"][:limit]
        rid = endpoint.split("/")[1]
        return self.data[key][rid]

    def download(self, aid, destination):
        self.downloads.append(aid)
        destination.write_bytes(self.archives[aid])

    def add_perf(self):
        run = self.data["workflow_runs"][0]
        run["name"] = "Perf"
        rid = str(run["id"])
        self.data["jobs"][rid] = [{"id": 1, "name": "perf", "runner_name": "planck", "status": "completed",
                                   "conclusion": "success", "started_at": "2026-10-07T00:00:00Z",
                                   "completed_at": "2026-10-07T00:01:30Z"}]
        self.data["artifacts"][rid] = [{"id": 42, "name": "perf-42", "size_in_bytes": 100, "expired": False}]
        self.archives[42] = archive({"circuit/report/results.json": fixture("circuit")})
        return run


class CollectorTests(unittest.TestCase):
    def test_real_api_metadata(self):
        api = FakeAPI()
        index = ci.collect(api, REPO, ci.empty_index(REPO))
        self.assertEqual(len(index["runs"]), 2)
        self.assertEqual(index["runs"][0]["id"], 37359803215)
        self.assertEqual(index["runs"][0]["workflow"], "Check")
        self.assertEqual(index["runs"][0]["branch"], "main")
        self.assertTrue(index["runs"][0]["jobs"])
        self.assertFalse(api.downloads)
        self.assertNotIn("\n", index["runs"][0]["message"])

    def test_pagination(self):
        api = ci.GitHub(REPO)
        with patch.object(api, "request", side_effect=[{"jobs": list(range(100))}, {"jobs": [100]}]) as request:
            self.assertEqual(len(api.pages("runs/1/jobs?filter=latest", "jobs")), 101)
            self.assertIn("&per_page=100&page=2", request.call_args.args[0])

    def test_incremental_expired_deleted_and_rerun(self):
        api = FakeAPI()
        run = api.add_perf()
        first = ci.collect(api, REPO, ci.empty_index(REPO))
        a = first["runs"][0]["artifacts"][0]
        self.assertEqual(a["runner"], "planck")
        self.assertEqual(a["seconds"], 90)
        self.assertEqual(len(a["reports"][0]["metrics"]), 6)
        run["run_attempt"] = 2
        api.data["jobs"][str(run["id"])][0]["runner_name"] = "mars-14900k"
        api.data["artifacts"][str(run["id"])][0]["expired"] = True
        second = ci.collect(api, REPO, first)
        self.assertEqual(len(second["runs"]), 2)
        self.assertEqual(second["runs"][0]["attempt"], 2)
        self.assertEqual(second["runs"][0]["artifacts"][0]["runner"], "planck")
        self.assertTrue(second["runs"][0]["artifacts"][0]["expired"])
        self.assertEqual(api.downloads, [42])
        api.data["artifacts"][str(run["id"])] = []
        third = ci.collect(api, REPO, second, limit=1)
        self.assertEqual(len(third["runs"]), 2)  # retain older evidence outside refresh window
        self.assertEqual(third["runs"][0]["artifacts"][0]["reports"], a["reports"])

    def test_failed_download_retry_and_unknown_runner(self):
        api = FakeAPI()
        run = api.add_perf()
        api.archives[42] = b"not a zip"
        first = ci.collect(api, REPO, ci.empty_index(REPO))
        self.assertEqual(first["runs"][0]["artifacts"][0]["state"], "error")
        api.archives[42] = archive({"circuit/report/results.json": fixture("circuit")})
        api.data["jobs"][str(run["id"])][0]["name"] = "renamed-job"
        second = ci.collect(api, REPO, first)
        self.assertEqual(second["runs"][0]["artifacts"][0]["state"], "ok")
        self.assertIsNone(second["runs"][0]["artifacts"][0]["runner"])
        self.assertEqual(api.downloads, [42, 42])

    def test_real_circuit_profile_filter_and_cpu_denominator(self):
        metrics = ci.parse_perf(fixture("circuit"), "real", "circuit")["metrics"]
        self.assertEqual(len(metrics), 6)
        pps = next(m for m in metrics if m["label"] == "L0 2 nodes pps")
        cpu = next(m for m in metrics if m["label"] == "L0 2 nodes CPU")
        self.assertAlmostEqual(pps["value"], 99177.7081994362)
        self.assertAlmostEqual(cpu["value"], 10.07237935)
        self.assertEqual(cpu["samples"], 3)
        self.assertIn('"profile": 0', cpu["scope"])

    def test_real_dataplane_medians_rtt_retransmits_and_scope(self):
        report = ci.parse_perf(fixture("dataplane"), "real", "dataplane")
        metrics = {m["label"]: m for m in report["metrics"]}
        self.assertAlmostEqual(metrics["manager-relay TCP"]["value"], 1628.89, delta=0.01)
        self.assertAlmostEqual(metrics["secure-relay TCP sender RTT"]["value"], (17.434 + 22.905) / 2, delta=0.01)
        self.assertEqual(metrics["secure-relay TCP retransmits"]["value"], 0)
        self.assertIn('"udp_rate": "500M"', metrics["manager-relay UDP"]["scope"])
        self.assertEqual(metrics["manager-relay UDP"]["samples"], 2)
        self.assertGreater(report["loadMax"], report["loadMin"])

    def test_real_crypto_environment_and_operation_separation(self):
        report = ci.parse_perf(fixture("crypto"), "real", "crypto")
        metrics = [m for m in report["metrics"] if m["label"] == "crypto ring.seal.inplace.new"]
        self.assertEqual(len(metrics), 2)
        self.assertTrue(all(m["samples"] == 5 for m in metrics))
        self.assertNotEqual(metrics[0]["scope"], metrics[1]["scope"])
        self.assertTrue(all(0.1 < m["value"] < 2 for m in metrics))

    def test_missing_and_failed_samples_are_not_zero(self):
        doc = fixture("dataplane")
        for s in doc["results"]:
            s["status"] = "failed"
        report = ci.parse_perf(doc, "failed", "dataplane")
        self.assertTrue(all(m["value"] is None and m["samples"] == 0 for m in report["metrics"]))
        doc = fixture("dataplane")
        for s in doc["results"]:
            s.pop("tcpStreams", None)
        report = ci.parse_perf(doc, "old", "dataplane")
        self.assertTrue(all(m["value"] is None for m in report["metrics"] if "RTT" in m["label"]))

    def test_matrix_adapter_and_boolean_validation(self):
        document = {"short": True, "rows": [{"row": "nat/off", "passed": False, "reportId": "evidence",
                                             "seconds": 30.5, "load": {"before": [1, 2, 3]}},
                                            {"row": "tcp/off", "passed": None}]}
        matrix = ci.parse_matrix(document, "matrix-id")
        self.assertTrue(matrix["short"])
        self.assertFalse(matrix["rows"][0]["passed"])
        self.assertIsNone(matrix["rows"][1]["passed"])
        self.assertIn("before", matrix["rows"][0]["load"])
        document["rows"][0]["passed"] = "false"
        with self.assertRaises(ValueError):
            ci.parse_matrix(document, "bad")

    def test_zip_binary_hash_and_path_safety(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "archive.zip"
            path.write_bytes(archive({"meshway": b"binary content", "meshwayd": b"daemon"}))
            result = ci.extract_archive(path, "node-aarch64-unknown-linux-musl-" + "a" * 40)
            self.assertEqual(result["builds"][0]["bytes"], 14)
            self.assertEqual(result["builds"][0]["sha256"], hashlib.sha256(b"binary content").hexdigest())
            path.write_bytes(archive({"../results.json": {}}))
            with self.assertRaises(ValueError):
                ci.extract_archive(path, "perf-1")
            self.assertFalse((Path(directory).parent / "results.json").exists())

    def test_unknown_schema_and_branch_loop(self):
        with self.assertRaises(ValueError):
            ci.load_index(b'{"schemaVersion":2}', REPO)
        api = FakeAPI()
        api.data["workflow_runs"][0]["head_branch"] = "ci-results"
        index = ci.collect(api, REPO, ci.empty_index(REPO))
        self.assertEqual(len(index["runs"]), 1)

    def test_orphan_publish_preserves_worktree_and_history(self):
        with tempfile.TemporaryDirectory() as directory:
            cwd = Path(directory)
            ci.git("init", "-q", cwd=cwd)
            (cwd / "user-work.txt").write_text("untouched")
            before = ci.git("status", "--porcelain", cwd=cwd).stdout
            first = ci.publish(ci.empty_index(REPO), cwd=cwd)
            self.assertEqual(ci.git("rev-list", "--parents", "-n1", first, cwd=cwd).stdout.decode().strip(), first)
            index = copy.deepcopy(ci.empty_index(REPO))
            index["updatedAt"] = "2026-10-07T00:00:00Z"
            second = ci.publish(index, first, cwd=cwd)
            self.assertEqual(ci.publication_base(first, cwd), second)
            with self.assertRaises(RuntimeError):
                ci.publish(index, first, cwd=cwd, expected_local=first)
            self.assertEqual(ci.git("rev-parse", f"{second}^", cwd=cwd).stdout.decode().strip(), first)
            self.assertEqual(json.loads(ci.git("show", f"{second}:index.json", cwd=cwd).stdout), index)
            self.assertEqual(ci.git("status", "--porcelain", cwd=cwd).stdout, before)
            self.assertEqual((cwd / "user-work.txt").read_text(), "untouched")


if __name__ == "__main__":
    unittest.main()
