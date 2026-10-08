#!/usr/bin/env python3
"""Collect GitHub Actions metadata and bounded artifact summaries (no builds)."""
import argparse
from collections import defaultdict
from datetime import datetime, timezone
import hashlib
import json
import shutil
import urllib.error
import urllib.request
import math
import os
from pathlib import Path, PurePosixPath
import re
import statistics
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_VERSION = 1
REF = "refs/heads/ci-results"
WORKFLOWS = {"Check", "Validate", "Perf", "Build"}
MAX_ARCHIVE = 256 * 1024 * 1024
MAX_JSON = 32 * 1024 * 1024
UNSET = object()


def object_value(value):
    if not isinstance(value, dict):
        raise ValueError("expected object")
    return value


def array(value):
    if not isinstance(value, list):
        raise ValueError("expected array")
    return value


def text(value):
    if not isinstance(value, str):
        raise ValueError("expected string")
    return value


def number(value):
    return value if type(value) in (int, float) and math.isfinite(value) else None


def ratio_us(nanoseconds, count):
    ns, n = number(nanoseconds), number(count)
    return ns / n / 1000 if ns is not None and n and n > 0 else None


def duration(start, end):
    if not start or not end:
        return None
    return max(0, (datetime.fromisoformat(end) - datetime.fromisoformat(start)).total_seconds())


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode()


def empty_index(repo):
    return {"schemaVersion": SCHEMA_VERSION, "repository": repo, "updatedAt": None, "runs": []}


def load_index(data, repo):
    index = object_value(json.loads(data))
    if index.get("schemaVersion") != SCHEMA_VERSION or index.get("repository") != repo:
        raise ValueError("index schema/repository mismatch")
    array(index["runs"])
    return index


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class GitHub:
    """Minimal Actions API client: stdlib HTTP, token from GH_TOKEN/GITHUB_TOKEN.

    Artifact downloads redirect to blob storage, which rejects the GitHub
    bearer token; the redirect is followed manually without credentials.
    """

    API = "https://api.github.com/"

    def __init__(self, repo):
        self.prefix = f"repos/{repo}/actions/"
        self.token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")

    def _open(self, url, authenticated):
        headers = {"Accept": "application/vnd.github+json", "User-Agent": "meshway-ci-collect",
                   "X-GitHub-Api-Version": "2022-11-28"}
        if authenticated and self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        opener = urllib.request.build_opener(_NoRedirect())
        try:
            return opener.open(urllib.request.Request(url, headers=headers), timeout=180)
        except urllib.error.HTTPError as error:
            if error.code in (301, 302, 303, 307, 308) and error.headers.get("Location"):
                return self._open(error.headers["Location"], authenticated=False)
            # Do not persist raw HTTP bodies/redirect URLs or credentials in the index.
            raise RuntimeError(f"GitHub API failed (HTTP {error.code})") from None
        except urllib.error.URLError:
            raise RuntimeError("GitHub API failed") from None

    def request(self, endpoint, output=None):
        with self._open(self.API + self.prefix + endpoint, authenticated=True) as response:
            if output is None:
                return json.loads(response.read())
            shutil.copyfileobj(response, output, 1 << 20)
            return None

    def pages(self, endpoint, key, limit=None):
        rows = []
        page = 1
        while limit is None or len(rows) < limit:
            separator = "&" if "?" in endpoint else "?"
            result = self.request(f"{endpoint}{separator}per_page=100&page={page}")
            batch = array(object_value(result)[key])
            rows.extend(batch)
            if len(batch) < 100:
                break
            page += 1
        return rows if limit is None else rows[:limit]

    def download(self, artifact_id, destination):
        with destination.open("wb") as output:
            self.request(f"artifacts/{artifact_id}/zip", output)
        if destination.stat().st_size > MAX_ARCHIVE:
            raise ValueError("archive exceeds 256 MiB")


def job_summary(job, repo, run_id):
    return {
        "id": job["id"], "name": text(job["name"]),
        "runner": job.get("runner_name") or None,
        "status": text(job["status"]), "conclusion": job.get("conclusion"),
        "startedAt": job.get("started_at"), "completedAt": job.get("completed_at"),
        "seconds": duration(job.get("started_at"), job.get("completed_at")),
        "url": f"https://github.com/{repo}/actions/runs/{run_id}/job/{job['id']}",
    }


def artifact_job(name, jobs):
    if name.startswith("perf-arm64-"):
        matches = [j for j in jobs if j["name"] == "perf-arm64"]
    elif name.startswith("perf-"):
        matches = [j for j in jobs if j["name"] == "perf"]
    elif name.startswith("validate-matrix-"):
        matches = [j for j in jobs if j["name"] == "matrix"]
    elif name.startswith("node-"):
        target = build_target(name)
        matches = [j for j in jobs if target and target in j["name"]]
    else:
        matches = []
    return matches[0] if len(matches) == 1 else None


def build_target(name):
    match = re.fullmatch(r"node-(.+)-[0-9a-f]{40}", name)
    return match[1] if match else None


def parse_matrix(document, report_id):
    """CI1 adapter: only this function depends on the matrix.json wire shape.

    matrix.json v1 (scripts/validate_matrix.py): top-level `scope` is
    `acceptance` or `diagnostic-short`; each row carries `row`, `passed`,
    `reportId`, `scope`, `elapsedSeconds` and `loadBeforeStart.loadAverage`.
    The index keeps the compact `short`/`seconds`/`load` form.
    """
    doc = object_value(document)
    if doc.get("version") != 1:
        raise ValueError("unsupported matrix schema")
    scope = text(doc.get("scope"))
    if scope not in ("acceptance", "diagnostic-short"):
        raise ValueError("matrix.scope must be acceptance or diagnostic-short")
    rows = []
    for value in array(doc.get("rows")):
        row = object_value(value)
        if row.get("passed") is not None and type(row["passed"]) is not bool:
            raise ValueError("matrix row.passed must be boolean or null")
        before = row.get("loadBeforeStart")
        load_text = None
        if isinstance(before, dict) and isinstance(before.get("loadAverage"), list):
            load_text = "/".join(f"{number(v):.2f}" for v in before["loadAverage"][:3] if isinstance(v, (int, float)))[:64] or None
        row_scope = row.get("scope")
        if row_scope is not None and text(row_scope) not in ("acceptance", "diagnostic-short"):
            raise ValueError("matrix row.scope must be acceptance or diagnostic-short")
        rows.append({"row": text(row["row"]), "passed": row.get("passed"),
                     "reportId": text(row["reportId"]) if row.get("reportId") is not None else None,
                     "seconds": number(row.get("elapsedSeconds")), "load": load_text})
    return {"reportId": report_id, "short": scope == "diagnostic-short", "rows": rows}


def parse_perf(document, report_id, kind):
    doc = object_value(document)
    if doc.get("schemaVersion") != 1:
        raise ValueError("unsupported performance schema")
    options = object_value(doc.get("options", {}))
    # Explicit scalar allowlist: never copy commands, environment, raw logs or secrets.
    scope_keys = ("workers", "allocator", "cpus", "memory", "mtu", "payload", "udp_rate",
                  "tcp_streams", "seconds", "omit", "udp_buffer", "rounds", "warmup", "iterations", "cpu_set")
    scope = {k: options.get(k) for k in scope_keys if k in options and
             (options[k] is None or type(options[k]) in (str, int, float, bool))}
    groups = defaultdict(list)
    attempted = defaultdict(int)

    def add(label, unit, dimensions, value, ok):
        key = (label, unit, json.dumps({**scope, **dimensions}, sort_keys=True, allow_nan=False))
        attempted[key] += 1
        if ok and number(value) is not None:
            groups[key].append(value)

    if kind == "circuit":
        if doc.get("layer") != "L0":
            raise ValueError("expected L0 report")
        for s in array(doc.get("results")):
            if s.get("nodes") not in (2, 3, 5) or s.get("packetBytes") != 1280 or s.get("profileRate") != 0:
                continue
            dims = {"nodes": s["nodes"], "bytes": 1280, "profile": 0}
            ok = s.get("status") == "ok"
            add(f"L0 {s['nodes']} nodes pps", "pps", dims, s.get("packetsPerSecond"), ok)
            measured = object_value(s.get("measured", {}))
            add(f"L0 {s['nodes']} nodes CPU", "µs/packet", dims,
                ratio_us(s.get("harnessCpuNs"), measured.get("deliveredPackets")), ok)
    elif kind == "dataplane":
        for s in array(doc.get("results")):
            if s.get("mode") not in ("secure-direct", "secure-relay", "manager-relay") or s.get("profile") != 0:
                continue
            dims = {"batch": s.get("batch"), "profile": 0, "traffic": s.get("traffic")}
            ok = s.get("status") == "ok"
            mode = s["mode"]
            if s.get("traffic") == "udp":
                add(f"{mode} UDP", "pps", dims, s.get("packetsPerSecond"), ok)
            elif s.get("traffic") == "tcp":
                add(f"{mode} TCP", "Mbps", dims, s.get("goodputMbps"), ok)
                streams = array(s.get("tcpStreams") or [])
                rtts = [number(t.get("mean_rtt")) for t in streams]
                retransmits = [number(t.get("retransmits")) for t in streams]
                rtt = statistics.mean(rtts) / 1000 if rtts and None not in rtts else None
                retransmit = sum(retransmits) if retransmits and None not in retransmits else None
                add(f"{mode} TCP sender RTT", "ms", dims, rtt, ok)
                add(f"{mode} TCP retransmits", "count", dims, retransmit, ok)
    elif kind == "crypto":
        for env in array(doc.get("environments")):
            for s in array(env.get("samples", [])):
                op = text(s["operation"])
                if s.get("bytes") != 1280 or not (op.startswith(("chacha", "hmac")) or op.startswith("ring.")):
                    continue
                add(f"crypto {op}", "µs/op", {"environment": text(env["name"]), "bytes": 1280},
                    ratio_us(s.get("cpuNs"), s.get("iterations")), env.get("status") == "ok")
    else:
        raise ValueError("unknown report kind")
    metrics = []
    for (label, unit, scope_json), count in sorted(attempted.items()):
        values = groups[(label, unit, scope_json)]
        metrics.append({"label": label, "unit": unit, "scope": scope_json,
                        "value": statistics.median(values) if values else None,
                        "samples": len(values), "attempted": count})
    loads = []
    for s in doc.get("results", []) + doc.get("environments", []):
        for key in ("hostLoadBefore", "hostLoadAfter", "hostLoadInitial", "hostLoadFinal"):
            load = s.get(key)
            if isinstance(load, list) and load and number(load[0]) is not None:
                loads.append(load[0])
    return {"reportId": report_id, "kind": kind, "revision": doc.get("revision"),
            "dirty": doc.get("dirty"), "quick": options.get("quick", False),
            "sourcesUnchanged": doc.get("sourcesUnchanged"),
            "loadMin": min(loads) if loads else None, "loadMax": max(loads) if loads else None,
            "metrics": metrics}


def extract_archive(path, name):
    reports, matrices, builds = [], [], []
    with zipfile.ZipFile(path) as archive:
        members = archive.infolist()
        if len(members) > 10000 or sum(m.file_size for m in members) > MAX_ARCHIVE:
            raise ValueError("expanded archive exceeds limits")
        seen = set()
        for member in members:
            p = PurePosixPath(member.filename)
            if p.is_absolute() or ".." in p.parts or member.filename in seen:
                raise ValueError("invalid/duplicate archive path")
            seen.add(member.filename)
            if member.is_dir():
                continue
            if name.startswith("node-") and p.name in ("meshway", "meshwayd"):
                if not build_target(name):
                    raise ValueError("unrecognized build artifact target")
                with archive.open(member) as source:
                    digest = hashlib.file_digest(source, "sha256").hexdigest()
                builds.append({"target": build_target(name), "binary": p.name,
                               "bytes": member.file_size, "sha256": digest})
            elif p.name in ("results.json", "matrix.json"):
                if member.file_size > MAX_JSON:
                    raise ValueError("report exceeds 32 MiB")
                doc = json.loads(archive.read(member))
                if p.name == "matrix.json" and name.startswith("validate-matrix-"):
                    matrices.append(parse_matrix(doc, p.parent.name))
                elif p.name == "results.json" and name.startswith("perf-"):
                    kinds = [k for k in ("circuit", "dataplane", "crypto") if k in p.parts[:-1]]
                    if len(kinds) != 1:
                        raise ValueError("report path must include circuit/dataplane/crypto")
                    reports.append(parse_perf(doc, p.parent.name, kinds[0]))
        if not reports and not matrices and not builds:
            raise ValueError("artifact contains no recognized reports/binaries")
    return {"reports": reports, "matrices": matrices, "builds": builds}


def collect(api, repo, previous, limit=200):
    runs = {r["id"]: r for r in previous["runs"]}
    for raw in api.pages("runs", "workflow_runs", limit):
        if raw.get("name") not in WORKFLOWS or raw.get("head_branch") == "ci-results":
            continue
        rid = raw["id"]
        old = runs.get(rid, {})
        jobs = [job_summary(j, repo, rid) for j in api.pages(f"runs/{rid}/jobs?filter=latest", "jobs")]
        old_artifacts = {a["id"]: a for a in old.get("artifacts", [])}
        artifacts = []
        for a in api.pages(f"runs/{rid}/artifacts", "artifacts"):
            name = text(a["name"])
            job = artifact_job(name, jobs)
            cached = old_artifacts.get(a["id"])
            if job and a.get("created_at") and job["startedAt"] and a["created_at"] < job["startedAt"]:
                # A rerun may expose artifacts from an earlier attempt on another runner.
                job = None
            item = {"id": a["id"], "name": name, "bytes": a["size_in_bytes"],
                    "expired": bool(a["expired"]), "url": f"https://github.com/{repo}/actions/runs/{rid}/artifacts/{a['id']}",
                    "jobId": job["id"] if job else None, "runner": job["runner"] if job else None,
                    "seconds": job["seconds"] if job else None,
                    "state": "ignored", "error": None, "reports": [], "matrices": [], "builds": []}
            if cached and cached["state"] == "ok":
                for key in ("state", "reports", "matrices", "builds"):
                    item[key] = cached[key]
                for key in ("jobId", "runner", "seconds"):
                    item[key] = cached[key]
            elif name.startswith(("perf-", "validate-matrix-", "node-")):
                if a["expired"]:
                    item["state"] = "expired"
                elif a["size_in_bytes"] > MAX_ARCHIVE:
                    item.update(state="error", error="archive exceeds 256 MiB")
                else:
                    try:
                        # Never extract files or execute artifact contents.
                        with tempfile.TemporaryDirectory(prefix="status-artifact-") as directory:
                            path = Path(directory) / "artifact.zip"
                            api.download(a["id"], path)
                            item.update(extract_archive(path, name))
                        item["state"] = "ok"
                    except (RuntimeError, ValueError, KeyError, TypeError, OSError, zipfile.BadZipFile) as error:
                        item.update(state="error", error=f"artifact parsing/download failed ({type(error).__name__})")
            artifacts.append(item)
        # Deleted artifacts keep their already-collected evidence, explicitly unavailable.
        current_ids = {a["id"] for a in artifacts}
        for aid, cached in old_artifacts.items():
            if aid not in current_ids:
                artifacts.append({**cached, "expired": True})
        commit = raw.get("head_commit") or {}
        runs[rid] = {"id": rid, "attempt": raw.get("run_attempt", 1), "workflow": raw["name"],
                     "event": raw["event"], "branch": raw.get("head_branch"), "commit": raw["head_sha"],
                     "message": (commit.get("message") or "").split("\n", 1)[0],
                     "createdAt": raw["created_at"], "updatedAt": raw["updated_at"],
                     "status": raw["status"], "conclusion": raw.get("conclusion"),
                     "url": f"https://github.com/{repo}/actions/runs/{rid}", "jobs": jobs, "artifacts": artifacts}
    return {"schemaVersion": SCHEMA_VERSION, "repository": repo,
            "updatedAt": datetime.now(timezone.utc).isoformat(),
            "runs": sorted(runs.values(), key=lambda r: (r["createdAt"], r["id"]), reverse=True)}


def git(*args, data=None, cwd=ROOT, check=True):
    return subprocess.run(["git", *args], input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          cwd=cwd, check=check, timeout=120)


def ref_tip(ref, cwd=ROOT):
    result = git("rev-parse", "--verify", ref, cwd=cwd, check=False)
    return result.stdout.decode().strip() if result.returncode == 0 else None


def fetch_results(cwd=ROOT):
    result = git("ls-remote", "--exit-code", "origin", REF, cwd=cwd, check=False)
    if result.returncode == 2:
        return None
    if result.returncode:
        raise RuntimeError("cannot query origin ci-results")
    git("fetch", "--no-tags", "origin", f"+{REF}:refs/remotes/origin/ci-results", cwd=cwd)
    return ref_tip("refs/remotes/origin/ci-results", cwd)


def publication_base(remote, cwd=ROOT):
    """Resume a failed push without dropping either side's committed evidence."""
    local = ref_tip(REF, cwd)
    if not local or not remote or local == remote:
        return local or remote
    if git("merge-base", "--is-ancestor", local, remote, cwd=cwd, check=False).returncode == 0:
        return remote
    if git("merge-base", "--is-ancestor", remote, local, cwd=cwd, check=False).returncode == 0:
        return local
    raise RuntimeError("local ci-results diverged from origin; reconcile before publishing")


def publish(index, parent=None, cwd=ROOT, expected_local=UNSET):
    """Write only a Git tree/ref. First commit has no parent (orphan branch)."""
    old = ref_tip(REF, cwd)
    if expected_local is not UNSET and old != expected_local:
        raise RuntimeError("ci-results changed during collection; retry")
    if old and parent and old != parent:
        # Never discard a locally advanced results history.
        if git("merge-base", "--is-ancestor", old, parent, cwd=cwd, check=False).returncode:
            raise RuntimeError("local ci-results diverged from origin; reconcile before publishing")
    parent = parent or old
    blob = git("hash-object", "-w", "--stdin", data=encoded(index), cwd=cwd).stdout.decode().strip()
    tree = git("mktree", data=f"100644 blob {blob}\tindex.json\n".encode(), cwd=cwd).stdout.decode().strip()
    args = ["-c", "user.name=Meshway Status", "-c", "user.email=status@meshway.invalid", "commit-tree", tree]
    if parent:
        args.extend(["-p", parent])
    commit = git(*args, data=b"Update Actions status index\n", cwd=cwd).stdout.decode().strip()
    git("update-ref", REF, commit, old or "0" * 40, cwd=cwd)
    return commit


def write_atomic(path, index):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            output.write(encoded(index))
            output.close()
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "JDScript/meshway"))
    parser.add_argument("--limit", type=int, default=200, help="recent API runs to refresh; older evidence is retained")
    parser.add_argument("--output", type=Path, default=ROOT / ".meshway/ci/index.json")
    parser.add_argument("--publish", action="store_true", help="commit index to local orphan ci-results branch")
    parser.add_argument("--push", action="store_true", help="with --publish: fetch origin baseline, then push ci-results")
    parser.add_argument("--sync", action="store_true", help="sync ci-results into docs/public/status/index.json")
    parser.add_argument("--local", action="store_true", help="sync local ci-results without fetching")
    args = parser.parse_args()
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", args.repo) or args.limit < 1:
        parser.error("valid owner/repo and positive --limit required")
    if args.push and (not args.publish or args.sync):
        parser.error("--push requires --publish and cannot be used with --sync")
    local_before = ref_tip(REF)
    parent = fetch_results() if args.push or (args.sync and not args.local) else local_before
    if args.push:
        parent = publication_base(parent)
    if args.sync and not parent:
        parent = ref_tip(REF)
    previous = load_index(git("show", f"{parent}:index.json").stdout, args.repo) if parent else empty_index(args.repo)
    if args.sync:
        write_atomic(ROOT / "docs/public/status/index.json", previous)
        print(f"Synced {len(previous['runs'])} runs" + (" (ci-results does not exist yet)" if not parent else ""))
        return
    if not args.publish and args.output.exists():
        previous = load_index(args.output.read_bytes(), args.repo)
    index = collect(GitHub(args.repo), args.repo, previous, args.limit)
    write_atomic(args.output, index)
    if args.publish:
        print(f"ci-results: {publish(index, parent, expected_local=local_before)}")
    if args.push:
        git("push", "origin", f"{REF}:{REF}")
    print(f"Collected {len(index['runs'])} runs → {args.output}")


if __name__ == "__main__":
    main()
