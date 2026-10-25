"""Offline tests of the field soak harness (scripts/soak/): report from sample
fixtures, planned/unplanned labels from faults.jsonl, trend verdicts, the disk
floor, log cursor continuation, compression/retention and fault markers."""
import json
import sys
import tempfile
import unittest
from datetime import timedelta
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent / "soak"))
import common  # noqa: E402
import fault  # noqa: E402
import logs  # noqa: E402
import report  # noqa: E402
import resources  # noqa: E402

DAY = "2026-11-02"
T0 = "2026-11-02T00:00:00Z"


def at(hours: float) -> str:
    return common.iso(common.parse_time(T0) + timedelta(hours=hours))


def status(phase="running", pid=100, issued="2026-11-02T00:00:00Z", serial=5, last_error=None, detail=None,
           sync_error=None):
    return {"processId": pid, "phase": phase, "sync": "connected", "syncError": sync_error, "dataPlane": "ready",
            "lastError": last_error, "lastErrorDetail": detail,
            "netmap": {"issuedAt": issued, "expiresAt": common.iso(common.parse_time(issued) + timedelta(hours=24))},
            "applied": {"revision": 3, "serial": serial}}


def tracing(level, message, **fields):
    return json.dumps({"timestamp": "x", "level": level, "fields": {"message": message, **fields}, "target": "t"})


class Soak:
    """A temporary soak root with a config and helpers to place fixture records."""

    def __init__(self, extra=None):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        data = {"t0": T0, "days": 7, "min_free_gib": 10,
                "hosts": {"local": {"kind": "local"}},
                "nodes": {"jp": {"host": "local", "unit": "meshwayd"},
                          "hk": {"host": "local", "container": "meshway-hk", "relay_or_exit": True}},
                "server": {"health_url": "https://server.invalid/api/v1/auth/options", "host": "local",
                           "container": "meshway-server"},
                "edges": [["jp", "hk"]], "faults": []}
        data.update(extra or {})
        (self.root / "config.json").write_text(json.dumps(data))
        self.config = common.Config.load(str(self.root / "config.json"))

    def put(self, *parts_and_records):
        *parts, records = parts_and_records
        path = self.root.joinpath(*parts)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a") as handle:
            for record in records:
                handle.write(json.dumps(record) + "\n")

    def close(self):
        self.tmp.cleanup()


class TimeAndStorage(unittest.TestCase):
    def test_times_parse_nanoseconds_offsets_and_z(self):
        self.assertEqual(common.iso(common.parse_time("2026-10-23T09:39:43.191123456Z")), "2026-10-23T09:39:43.191123Z")
        self.assertEqual(common.iso(common.parse_time("2026-10-23T11:39:43+02:00")), "2026-10-23T09:39:43.000000Z")

    def test_the_example_config_refuses_to_load_until_its_placeholders_are_filled(self):
        example = Path(__file__).resolve().parent / "soak" / "config.example.json"
        with self.assertRaisesRegex(ValueError, "REPLACE"):
            common.Config.load(str(example))
        data = json.loads(example.read_text().replace("REPLACE-2026-11-01T00:00:00Z", T0).replace("REPLACE", "x"))
        config = common.Config(data, example)
        self.assertEqual(len(config.data["edges"]), 10)
        self.assertEqual(config.soak_days()[-1], "2026-11-08")
        self.assertEqual(sorted(fault.planned_at(config, f).strftime("%d %H") for f in config.data["faults"])[:2],
                         ["03 10", "04 10"])

    def test_proc_header_is_split_from_status_json(self):
        proc, rest = common.proc_line('@@proc 24 1500 1792745868 100\n{"phase": "running"}')
        self.assertEqual(proc, {"pid": 24, "start": 1792745883.0})
        self.assertEqual(json.loads(rest)["phase"], "running")
        self.assertEqual(common.proc_line("@@proc none 0 1 100\n{}")[0], {"pid": None, "start": None})

    def test_gate_forms_go_over_ssh_as_one_quoted_command_and_locally_with_command(self):
        soak = Soak({"hosts": {"p": {"kind": "ssh", "target": "u@h"}, "l": {"kind": "local"}},
                     "nodes": {"x": {"host": "p", "container": "c"}, "y": {"host": "l"}}, "ssh": {"identity": "/k"}})
        argv = common.gate_argv(soak.config, "p", ["journal", "x", "--after-cursor", "s=a;i=1"])
        self.assertEqual(argv[-2:], ["u@h", "journal x --after-cursor 's=a;i=1'"])
        self.assertIn("IdentitiesOnly=yes", argv)
        self.assertIn("AddressFamily=inet", argv)
        local = common.gate_argv(soak.config, "l", ["status", "y"])
        self.assertEqual(local[1:], [str(common.GATE), "--spec", str(soak.root / "gate" / "l.json"), "--command",
                                     "status y", "l"])
        soak.close()

    def test_past_days_are_compressed_and_read_back(self):
        soak = Soak()
        directory = soak.root / "logs" / "jp"
        soak.put("logs", "jp", "2026-11-01.jsonl", [{"t": "2026-11-01T23:59:59Z", "msg": "a"}])
        soak.put("logs", "jp", f"{DAY}.jsonl", [{"t": "2026-11-02T00:00:01Z", "msg": "b"}])
        try:
            done = common.compress_before(directory, DAY)
        except OSError as error:
            self.skipTest(str(error))
        self.assertEqual([p.name for p in done], ["2026-11-01.jsonl.zst"])
        self.assertFalse((directory / "2026-11-01.jsonl").exists())
        self.assertEqual(common.read_day(directory, "2026-11-01")[0]["msg"], "a")
        self.assertTrue((directory / f"{DAY}.jsonl").exists())
        soak.close()

    def test_retention_removes_only_dated_files_older_than_the_period(self):
        soak = Soak()
        soak.put("samples", "jp", "2026-09-30.jsonl", [{}])
        soak.put("samples", "jp", "2026-10-05.jsonl", [{}])
        soak.put("reports", "2026-09-30.md", [{}])
        soak.put("samples", "jp", "notes.jsonl", [{}])
        removed = common.prune(soak.config, "2026-11-02")
        self.assertEqual(sorted(p.name for p in removed), ["2026-09-30.jsonl", "2026-09-30.md"])
        self.assertTrue((soak.root / "samples/jp/2026-10-05.jsonl").exists())
        self.assertTrue((soak.root / "samples/jp/notes.jsonl").exists())
        soak.close()

    def test_below_the_disk_floor_nothing_is_written_and_the_report_is_red(self):
        soak = Soak()
        with mock.patch.object(common, "free_gib", return_value=4.2), \
                mock.patch.object(common, "now", return_value=common.parse_time(at(3))):
            self.assertFalse(common.append_by_day(soak.config, soak.root / "samples/jp", [{"t": at(1)}]))
            result = report.analyze(soak.config, [DAY])
            text = report.render(soak.config, result, "t")
        self.assertFalse((soak.root / "samples/jp").exists())
        self.assertEqual(result["disk"]["stops"][0]["free_gib"], 4.2)
        self.assertIn("🔴 **磁盘剩余 4.2 GiB < 10.0 GiB", text)
        soak.close()


class LogCursor(unittest.TestCase):
    @staticmethod
    def entry(_self, cursor, seconds, message):
        return json.dumps({"__CURSOR": cursor, "__REALTIME_TIMESTAMP": str(1793577600_000000 + seconds * 1_000_000),
                           "_PID": "77", "PRIORITY": "4", "_BOOT_ID": "abcdef0123", "MESSAGE": message})

    def test_journal_continues_after_the_stored_cursor(self):
        soak = Soak({"nodes": {"jp": {"host": "local", "unit": "meshwayd"}}, "server": {}})
        calls = []

        def fake(argv, limit, timeout):
            calls.append(argv)
            command = argv[argv.index("--command") + 1]
            if "--after-cursor" not in command:
                return [LogCursor.entry(None, "c1", 10, "one"), LogCursor.entry(None, "c2", 20, "two")], False, ""
            self.assertEqual(command, "journal jp --after-cursor c2")
            return [LogCursor.entry(None, "c3", 30, tracing("WARN", "Node sync failed; retrying", error="Network"))], False, ""
        with mock.patch.object(logs, "stream", side_effect=fake):
            logs.collect(soak.config)
            logs.collect(soak.config)
        self.assertIn("journal jp --since 1793577600", calls[0])
        self.assertEqual(common.read_state(soak.config, "logs.json")["jp"]["cursor"], "c3")
        lines = common.read_day(soak.root / "logs/jp", DAY)
        self.assertEqual([line["msg"][:3] for line in lines], ["one", "two", '{"t'])
        self.assertNotIn("cursor", lines[0])
        soak.close()

    def test_docker_windows_are_disjoint_and_a_truncated_batch_resumes_after_its_last_line(self):
        soak = Soak({"nodes": {"hk": {"host": "local", "container": "meshway-hk"}}, "server": {}})
        scripts = []

        def fake(argv, limit, timeout):
            scripts.append(argv[argv.index("--command") + 1] + " ")
            if len(scripts) == 1:
                return ["2026-11-02T01:00:00.123456789Z a", "2026-11-02T01:00:01.000000001Z b"], True, ""
            return ["2026-11-02T01:00:02.5Z c"], False, ""
        moment = common.parse_time("2026-11-02T02:00:00Z")
        with mock.patch.object(logs, "stream", side_effect=fake), mock.patch.object(common, "now", return_value=moment):
            logs.collect(soak.config)
            self.assertEqual(common.read_state(soak.config, "logs.json")["hk"]["since_ns"], 1793581201000000002)
            logs.collect(soak.config)
        self.assertEqual(scripts[0], "docker-logs hk 1793577600.000000000 1793584770.000000000 ")
        self.assertEqual(scripts[1], "docker-logs hk 1793581201.000000002 1793584770.000000000 ")
        self.assertEqual(common.read_state(soak.config, "logs.json")["hk"]["since_ns"], 1793584770000000001)
        self.assertEqual([line["msg"] for line in common.read_day(soak.root / "logs/hk", DAY)], ["a", "b", "c"])
        soak.close()


class LogGaps(unittest.TestCase):
    def test_a_rotated_previous_window_records_a_possible_gap(self):
        soak = Soak({"nodes": {"hk": {"host": "local", "container": "meshway-hk"}}, "server": {}})
        common.write_state(soak.config, "logs.json", {"hk": {"since_ns": 1793581200000000000,
                                                             "previous": [1793577600000000000, 1793581199999999999, 5]}})
        counted = []

        def probe(argv, timeout):
            counted.append(argv[argv.index("--command") + 1])
            return common.Result(0, "@@count 0\n" if len(counted) == 1 else "@@count 3\n", "", 1, argv)
        moment = common.parse_time("2026-11-02T02:00:00Z")
        with mock.patch.object(logs, "stream", return_value=(["2026-11-02T01:30:00Z x"], False, "")), \
                mock.patch.object(common, "run", side_effect=probe), mock.patch.object(common, "now", return_value=moment):
            self.assertTrue(logs.collect(soak.config)["hk"]["gap"])
            with mock.patch.object(common, "now", return_value=moment + timedelta(hours=1)):
                self.assertNotIn("gap", logs.collect(soak.config)["hk"])
        self.assertEqual(counted[0], "docker-logs-count hk 1793577600.000000000 1793581199.999999999")
        self.assertEqual(counted[1], "docker-logs-count hk 1793581200.000000000 1793584770.000000000")
        gaps = common.read_jsonl(soak.root / "state/log-gaps.jsonl")
        self.assertEqual([(g["node"], g["since"]) for g in gaps], [("hk", "2026-11-02T00:59:59.999999Z")])
        with mock.patch.object(common, "now", return_value=moment):
            result = report.analyze(soak.config, [DAY])
            text = report.render(soak.config, result, "t")
        self.assertIn("可能缺行", text)
        self.assertEqual(result["gates"][-1]["result"], "fail")
        soak.close()

    def test_an_unreadable_journal_cursor_resumes_from_the_last_stored_time(self):
        soak = Soak({"nodes": {"jp": {"host": "local", "unit": "meshwayd"}}, "server": {}})
        common.write_state(soak.config, "logs.json", {"jp": {"cursor": "gone", "last_t": "2026-11-02T01:00:00.000000Z"}})
        commands = []

        def fake(argv, limit, timeout):
            commands.append(argv[argv.index("--command") + 1])
            if "--after-cursor" in commands[-1]:
                return [], False, "rc 1: Failed to seek to cursor"
            return [LogCursor.entry(None, "c9", 3601, "again")], False, ""
        with mock.patch.object(logs, "stream", side_effect=fake):
            logs.collect(soak.config)
        self.assertEqual(commands, ["journal jp --after-cursor gone", "journal jp --since 1793581200"])
        self.assertEqual(common.read_state(soak.config, "logs.json")["jp"]["cursor"], "c9")
        self.assertIn("cursor", common.read_jsonl(soak.root / "state/log-gaps.jsonl")[0]["reason"])
        soak.close()


class Labels(unittest.TestCase):
    def windows(self, soak, records):
        soak.put("faults.jsonl", records)
        return report.fault_windows(soak.config)

    def test_window_covers_start_to_end_plus_grace_and_star_excludes_the_server(self):
        soak = Soak()
        windows = self.windows(soak, [
            {"t": at(10), "event": "start", "id": "f", "affects": ["jp"], "grace_min": 15},
            {"t": at(10.1), "event": "end", "id": "f", "ok": True},
            {"t": at(12), "event": "start", "id": "s", "affects": ["server", "*"]},
            {"t": at(12.5), "event": "end", "id": "s", "ok": True},
            {"t": at(13), "event": "start", "id": "dry", "affects": ["jp"], "dry": True},
        ])
        p = lambda node, h: report.planned(windows, node, common.parse_time(at(h)))  # noqa: E731
        self.assertEqual(p("jp", 10.3), "f")
        self.assertIsNone(p("jp", 10.4))
        self.assertIsNone(p("hk", 10.05))
        self.assertEqual(p("hk", 12.2), "s")
        self.assertEqual(p("server", 12.2), "s")
        self.assertIsNone(p("jp", 13))
        self.assertIsNone(report.planned(windows, "hk", common.parse_time(at(12.2)), wildcard=False))
        self.assertEqual(report.planned(windows, "server", common.parse_time(at(12.2)), wildcard=False), "s")
        soak.close()

    def test_an_open_window_lasts_until_now(self):
        soak = Soak()
        windows = self.windows(soak, [{"t": at(1), "event": "start", "id": "o", "affects": ["jp"]}])
        with mock.patch.object(common, "now", return_value=common.parse_time(at(30))):
            self.assertEqual(report.planned(windows, "jp", common.parse_time(at(29))), "o")
            self.assertIsNone(report.planned(windows, "jp", common.parse_time(at(31))))
        soak.close()


class Trend(unittest.TestCase):
    def series(self, hours, value):
        return [{"t": at(h), "pid": 1, "start": 9.0, "rss_kib": value(h), "fds": 30} for h in range(hours)]

    def test_steady_rss_with_noise_is_flat_and_a_leak_is_growing(self):
        flat = report.trend(self.series(150, lambda h: 30000 + (h % 7) * 300), "rss_kib", {})
        self.assertEqual(flat[0]["verdict"], "flat")
        leak = report.trend(self.series(150, lambda h: 30000 + h * 20), "rss_kib", {})  # 480 KiB/day ≈ 3.3 MiB/week
        self.assertEqual(leak[0]["verdict"], "flat")  # under max(8 MiB, 10 %)
        leak = report.trend(self.series(150, lambda h: 30000 + h * 60), "rss_kib", {})  # 1440 KiB/day ≈ 10 MiB/week
        self.assertEqual(leak[0]["verdict"], "growing")
        self.assertAlmostEqual(leak[0]["slope_per_day"], 1440, places=3)

    def test_segments_split_at_restarts_and_short_segments_are_not_judged(self):
        values = self.series(10, lambda h: 30000 + h * 1000) + [
            {"t": at(10 + h), "pid": 2, "start": 99.0, "fds": 30 + (h // 10), "rss_kib": 30000} for h in range(60)]
        verdicts = {t["pid"]: t["verdict"] for t in report.trend(values, "rss_kib", {})}
        self.assertEqual(verdicts, {1: "too short", 2: "flat"})
        fds = {t["pid"]: t for t in report.trend(values, "fds", {})}
        self.assertEqual(fds[2]["verdict"], "growing")  # +1 fd / 10 h ≈ 16.8 per week > 8


class Resources(unittest.TestCase):
    def test_schedstat_window_is_converted_to_percent_of_one_core(self):
        output = "\n".join([
            "@@pid 24 1500 1792745868 100 2",
            "@@a 1000.0 40", "T 24 1000000000 meshwayd", "T 25 2000000000 tokio-rt-worker",
            "N Udp: InDatagrams OutDatagrams", "N Udp: 100 100", "N Tcp: RtoAlgorithm InSegs OutSegs", "N Tcp: 1 10 10",
            "@@b 1060.0 52", "T 24 1060000000 meshwayd", "T 25 3800000000 tokio-rt-worker", "T 26 5 new",
            "N Udp: InDatagrams OutDatagrams", "N Udp: 160 160", "N Tcp: RtoAlgorithm InSegs OutSegs", "N Tcp: 1 70 70",
            "@@mem 31000 33000", "@@fd 40 21", "@@exe 0123456789abcdef", "@@load 0.1 0.2 0.3"])
        record = resources.parse(output)
        self.assertEqual(record["max_thread_pct"], 3.0)
        self.assertEqual(record["process_pct"], 3.1)
        self.assertEqual(record["children_pct"], 0.2)
        self.assertEqual(record["netns_pps"], 2.0)
        self.assertEqual((record["rss_kib"], record["fds"], record["sockets"]), (31000, 40, 21))
        self.assertEqual(record["start"], 1792745883.0)
        self.assertEqual(resources.parse("@@error no meshwayd")["error"], "no meshwayd")


class Health(unittest.TestCase):
    def test_health_requests_name_the_collector(self):
        seen = []

        class Response:
            status = 200
            def __enter__(self): return self
            def __exit__(self, *a): return False
            def read(self, n): return b""

        def urlopen(request, timeout):
            seen.append(request.get_header("User-agent"))
            return Response()

        with mock.patch("urllib.request.urlopen", urlopen):
            self.assertEqual(common.health("https://server.example/api/v1/auth/options", 5), {"http": 200})
            config = mock.Mock(server={"health_url": "https://server.example/api/v1/auth/options"})
            self.assertIn("server_healthy_s", fault.wait_health(config, 0.0, 5))
        self.assertEqual(seen, ["meshway-soak", "meshway-soak"])


class ServerStarts(unittest.TestCase):
    def test_a_container_started_before_the_range_is_not_a_start_in_it(self):
        baseline = {"t": "2026-11-01T23:55:00Z", "container": {"startedAt": "2026-10-30T00:00:00Z"}}
        records = [{"t": at(0.25), "health": {"ok": True}, "container": {"startedAt": "2026-11-01T23:59:00Z"}},
                   {"t": at(5), "health": {"ok": True}, "container": {"startedAt": at(4.9)}}]
        since = common.parse_time(at(0.25))
        out = report.server_analysis(records, baseline, [], since=since)
        self.assertEqual([s["t"] for s in out["starts"]], [at(4.9)])
        self.assertEqual(len(report.server_analysis(records, baseline, [])["starts"]), 2)


class Report(unittest.TestCase):
    def fixture(self):
        soak = Soak()
        soak.put("faults.jsonl", [
            {"t": at(10), "event": "start", "id": "d5-jp-restart", "affects": ["jp"], "grace_min": 15,
             "planned_at": at(10), "action": "systemd_restart"},
            {"t": at(10.02), "event": "end", "id": "d5-jp-restart", "ok": True, "detail": {"recovered_s": 6.1}},
        ])
        samples = []
        for i in range(288):
            hours = i / 12
            pid = 100 if hours < 10 else 200 if hours < 15 else 300  # planned at 10:00, unplanned at 15:00
            phase, error = ("blocked", "underlay") if 20 <= hours < 20.2 else ("running", None)
            if 21 <= hours < 21.1:
                phase, error = "blocked", None
            issued = at(int(hours // 6) * 6)
            samples.append({"t": at(hours), "node": "jp", "proc": {"pid": pid, "start": float(pid)},
                            "status": status(phase, pid, issued, serial=5 if hours < 8 else 6, last_error=error)})
        samples[100]["status"] = None
        soak.put("samples", "jp", f"{DAY}.jsonl", samples)
        soak.put("samples", "jp", "2026-11-01.jsonl", [{"t": "2026-11-01T23:59:00Z", "proc": {"pid": 100, "start": 100.0},
                                                       "status": status(pid=100)}])
        soak.put("samples", "hk", f"{DAY}.jsonl", [{"t": at(h / 12), "proc": {"pid": 7, "start": 7.0},
                                                   "status": status(pid=7)} for h in range(288)])
        soak.put("samples", "server", f"{DAY}.jsonl", [
            {"t": at(1), "health": {"ok": True, "http": 200}, "container": {"startedAt": "2026-11-01T00:00:00Z"}},
            {"t": at(2), "health": {"ok": False, "error": "timed out"}, "container": {"startedAt": "2026-11-01T00:00:00Z"}}])
        soak.put("logs", "jp", f"{DAY}.jsonl", [
            {"t": at(10.01), "pid": 1, "sys": True, "prio": 6, "msg": "Started meshwayd.service - Meshway Node daemon."},
            {"t": at(10.01), "pid": 200, "prio": 6, "msg": tracing("WARN", "Node sync failed; retrying", error="Contract")},
            {"t": at(3), "pid": 100, "prio": 6, "msg": tracing("WARN", "Node sync failed; retrying", error="Network")},
            {"t": at(3), "pid": 100, "prio": 6, "msg": tracing("WARN", "Node sync failed; retrying", error="Network")},
            {"t": at(4), "pid": 100, "prio": 6, "msg": tracing("WARN", "Node sync failed; retrying", error="Contract")},
            {"t": at(20), "pid": 300, "prio": 6,
             "msg": tracing("WARN", "Required packet plane component failed; suspending packet plane", problem="Tun")},
            {"t": at(20.3), "pid": 300, "prio": 6,
             "msg": tracing("INFO", "Required packet plane component recovered; netmap reinstalled", trigger="retry")},
            {"t": at(21), "pid": 300, "prio": 6,
             "msg": tracing("WARN", "Node control problem", phase="Blocked", problem="Contract")},
            {"t": at(22), "pid": 300, "prio": 3, "msg": "plain text error"},
            {"t": at(10.005), "pid": 100, "prio": 6, "msg": tracing("INFO", "Node stopped", failed=False)},
            {"t": at(23.97), "pid": 300, "prio": 6, "msg": tracing("INFO", "Node stopped", failed=True)},
        ])
        resource_records = []
        for h in range(24):
            resource_records.append({"t": at(h + 0.5), "node": "hk", "pid": 7, "start": 7.0, "window_s": 60,
                                     "max_thread_pct": 6.2 if h == 5 else 1.0, "process_pct": 9.5 if h == 6 else 3.0,
                                     "children_pct": 2.0, "netns_pps": 10.0, "rss_kib": 30000, "fds": 30, "sockets": 12,
                                     "threads": [{"name": "tokio-rt-worker", "pct": 1.0}]})
        resource_records.append({"t": at(23.9), "node": "hk", "error": "no meshwayd"})
        soak.put("samples", "hk", "resources", f"{DAY}.jsonl", resource_records)
        soak.put("samples", "probes", f"{DAY}.jsonl", [
            {"t": at(23.5), "kind": "ping", "src": "jp", "dst": "hk", "sent": 10, "success": 9},
            {"t": at(23.6), "kind": "throughput", "src": "jp", "dst": "hk", "mbit_limit": 50, "seconds": 10,
             "received_mbit": 49.7, "retransmits": 0}])
        return soak

    def test_daily_report_labels_events_and_judges_the_gates(self):
        soak = self.fixture()
        with mock.patch.object(common, "now", return_value=common.parse_time("2026-11-03T00:10:00Z")):
            result = report.analyze(soak.config, [DAY])
            report.write(soak.config, DAY, result, "日报")
        jp = result["nodes"]["jp"]
        self.assertEqual([(s["previous_pid"], s["pid"], s["planned"]) for s in jp["status"]["starts"]],
                         [(100, 200, "d5-jp-restart"), (200, 300, None)])
        self.assertEqual(jp["status"]["unreachable"], 1)
        self.assertEqual(jp["status"]["expected"], 288)
        self.assertEqual(jp["status"]["lease_renewals"], 3)
        self.assertEqual(jp["status"]["serials"][0]["from"], 5)
        self.assertEqual([(b["has_cause"], b["causes"]) for b in jp["status"]["blocked"]],
                         [(True, [["underlay", None]]), (False, [[None, None]])])
        failures = jp["logs"]["sync_failures"]
        self.assertEqual([(f["error"], f["planned"], f["reachability"]) for f in failures],
                         [("Contract", "d5-jp-restart", False), ("Network", None, True), ("Contract", None, False)])
        self.assertEqual(len(jp["logs"]["withdrawals"]), 1)
        self.assertEqual(len(jp["logs"]["reinstalls"]), 1)
        self.assertEqual(jp["logs"]["systemd"][0]["planned"], "d5-jp-restart")
        self.assertEqual([x["planned"] for x in jp["logs"]["stops"]], ["d5-jp-restart", None])
        restart_gate = next(g for g in result["gates"] if g["gate"].startswith("零人工重启"))
        self.assertIn("计划外 2：jp@2026-11-02T15:00:00, jp@2026-11-02T23:58:12（仅日志 Node stopped）", restart_gate["evidence"])
        self.assertEqual(jp["logs"]["categories"]["WARN Node sync failed; retrying [Network]"], 1)  # duplicate removed
        self.assertEqual(jp["logs"]["categories"]["ERROR plain text error"], 1)
        hk = result["nodes"]["hk"]["resources"]
        self.assertEqual(hk["missing"], 1)
        self.assertEqual(sorted(v["gate"].split()[0] for v in hk["violations"]), ["process", "thread"])
        self.assertEqual(result["server"]["health_failed"][0]["planned"], None)
        verdict = {g["gate"]: g["result"] for g in result["gates"]}
        self.assertEqual(verdict["零人工重启（计划外进程启动 = 0）"], "fail")
        self.assertEqual(verdict["每次 blocked 都有已证实原因"], "fail")
        self.assertEqual(verdict["空闲 CPU（每线程 < 5 %；中继／出口进程合计与 nft 子进程）"], "fail")
        self.assertEqual(verdict["RSS／fd／socket 无持续增长"], "incomplete")
        self.assertEqual(verdict["计划外 sync 失败只属于 tunnel／Server 可达性"], "fail")
        self.assertEqual(verdict["每日 ping 10/10"], "fail")
        text = (soak.root / "reports" / f"{DAY}.md").read_text()
        self.assertIn("（D1）", text)
        self.assertIn("| d5-jp-restart |", text)
        self.assertIn("计划内（d5-jp-restart）", text)
        self.assertIn("**计划外**", text)
        self.assertIn("49.7 Mbit/s", text)
        self.assertTrue(json.loads((soak.root / "reports" / f"{DAY}.json").read_text())["gates"])
        soak.close()

    def test_a_clean_day_passes_and_missing_probes_are_incomplete_not_zero(self):
        soak = Soak()
        soak.put("samples", "jp", f"{DAY}.jsonl", [{"t": at(h / 12), "proc": {"pid": 1, "start": 1.0},
                                                   "status": status(pid=1)} for h in range(288)])
        soak.put("samples", "hk", f"{DAY}.jsonl", [{"t": at(h / 12), "proc": {"pid": 2, "start": 2.0},
                                                   "status": status(pid=2)} for h in range(288)])
        with mock.patch.object(common, "now", return_value=common.parse_time("2026-11-03T00:10:00Z")):
            result = report.analyze(soak.config, [DAY])
        verdict = {g["gate"]: g["result"] for g in result["gates"]}
        self.assertEqual(verdict["零人工重启（计划外进程启动 = 0）"], "pass")
        self.assertEqual(verdict["计划外 sync 失败只属于 tunnel／Server 可达性"], "pass")
        self.assertEqual(verdict["每日 ping 10/10"], "incomplete")
        self.assertEqual(verdict["空闲 CPU（每线程 < 5 %；中继／出口进程合计与 nft 子进程）"], "incomplete")
        soak.close()


class Faults(unittest.TestCase):
    def config(self, faults):
        soak = Soak({"faults": faults})
        return soak, soak.config

    def events(self, soak):
        return [(r["id"], r["event"], r.get("ok")) for r in common.read_jsonl(soak.root / "faults.jsonl")]

    def test_schedule_days_count_from_the_t0_date(self):
        soak, config = self.config([{"id": "a", "day": 3, "at": "10:00", "action": "kill9", "node": "jp"}])
        self.assertEqual(common.iso(fault.planned_at(config, config.data["faults"][0])), "2026-11-04T10:00:00.000000Z")
        soak.close()

    def test_a_failing_action_still_writes_its_end_marker(self):
        soak, config = self.config([{"id": "a", "day": 1, "at": "10:00", "action": "kill9", "node": "jp"}])
        broken = mock.Mock(side_effect=RuntimeError("ssh died"))
        with mock.patch.dict(fault.ACTIONS, {"kill9": (broken, None)}):
            self.assertFalse(fault.execute(config, config.data["faults"][0]))
        self.assertEqual(self.events(soak), [("a", "start", None), ("a", "end", False)])
        self.assertIn("ssh died", common.read_jsonl(soak.root / "faults.jsonl")[1]["error"])
        soak.close()

    def test_due_runs_once_skips_missed_faults_and_recover_closes_interrupted_ones(self):
        faults = [{"id": "early", "day": 1, "at": "08:00", "action": "kill9", "node": "jp"},
                  {"id": "now", "day": 1, "at": "10:00", "action": "nft_drop", "node": "hk", "match": "udp"},
                  {"id": "later", "day": 2, "at": "10:00", "action": "kill9", "node": "jp"}]
        soak, config = self.config(faults)
        ran = mock.Mock(return_value={"x": 1})
        cleanup = mock.Mock()
        moment = common.parse_time(at(10.1))
        with mock.patch.dict(fault.ACTIONS, {"kill9": (ran, None), "nft_drop": (ran, cleanup)}), \
                mock.patch.object(common, "now", return_value=moment):
            fault.due(config)
            fault.due(config)
            self.assertEqual(ran.call_count, 1)
            fault.mark(config, {"event": "start", "id": "later", "affects": ["jp"]})
            soak.put("faults.jsonl", [{"t": at(10.2), "event": "start", "id": "now", "affects": ["hk"]}])
            self.assertEqual(sorted(fault.recover(config)), ["later", "now"])
        self.assertEqual(cleanup.call_count, 1)
        events = self.events(soak)
        self.assertEqual(events[0], ("early", "skipped", None))
        self.assertIn(("now", "end", True), events)
        ends = [r for r in common.read_jsonl(soak.root / "faults.jsonl") if r["event"] == "end" and r.get("interrupted")]
        self.assertEqual(sorted(r["id"] for r in ends), ["later", "now"])
        soak.close()

    def test_fault_actions_ask_the_gate_for_their_forms(self):
        faults = [{"id": "u", "day": 2, "at": "10:00", "action": "server_upgrade", "version": "v0.1.0-alpha.23"},
                  {"id": "r", "day": 4, "at": "10:00", "action": "server_rotate_key"},
                  {"id": "s", "day": 3, "at": "10:00", "action": "server_stop", "duration_s": 5},
                  {"id": "k", "day": 5, "at": "12:00", "action": "kill9", "node": "hk"},
                  {"id": "n", "day": 6, "at": "12:00", "action": "nft_drop", "node": "hk", "match": "tcp443", "duration_s": 600},
                  {"id": "e", "day": 6, "at": "14:00", "action": "exit_profile", "node": "jp", "profile": "us-exit",
                   "duration_s": 600}]
        soak, config = self.config(faults)
        words = []
        with mock.patch.object(fault, "DRY", True), mock.patch("builtins.print"), \
                mock.patch.object(fault, "gate", side_effect=lambda c, t, w, timeout: words.append(w) or
                                  common.Result(0, "", "", 0, w)):
            for item in faults:
                self.assertTrue(fault.execute(config, item), item["id"])
        self.assertEqual(words, [
            ["server-upgrade", "v0.1.0-alpha.23"], ["server-rotate-key"], ["server-stop"], ["server-start"],
            ["inspect", "hk"], ["kill9", "hk"], ["nft-drop", "hk", "tcp443", "600"], ["nft-clear", "hk"],
            ["exit-select", "jp", "us-exit"], ["curl-loop", "jp", "600"], ["exit-default", "jp"]])
        soak.close()

    def test_kill9_refuses_a_target_without_a_restart_policy(self):
        soak, config = self.config([{"id": "k", "day": 1, "at": "10:00", "action": "kill9", "node": "hk"}])
        answers = {"status": common.Result(0, '{"processId": 7}', "", 0, []),
                   "inspect": common.Result(0, '{"Status":"running"} 0 no sha256:x', "", 0, [])}
        with mock.patch.object(fault, "gate", side_effect=lambda c, t, w, timeout: answers[w[0]]):
            self.assertFalse(fault.execute(config, config.data["faults"][0]))
        end = common.read_jsonl(soak.root / "faults.jsonl")[-1]
        self.assertIn("restart policy 'no'", end["error"])
        soak.close()

    def test_dry_runs_do_not_claim_the_schedule(self):
        soak, config = self.config([{"id": "a", "day": 1, "at": "10:00", "action": "kill9", "node": "jp"}])
        with mock.patch.object(fault, "DRY", True), mock.patch("builtins.print"):
            fault.execute(config, config.data["faults"][0])
        self.assertFalse((soak.root / "faults.jsonl").exists())
        soak.close()


if __name__ == "__main__":
    unittest.main()
