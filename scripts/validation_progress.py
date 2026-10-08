"""Read-only, bounded progress projection; private errors never enter stdout."""
import datetime
import json
from pathlib import Path
import re
import sys


def read_json(path):
    try:
        if path.stat().st_size > 2 * 1024 * 1024:
            return None
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None


def snapshot(root, identity, now=None):
    root = Path(root)
    now = now or datetime.datetime.now(datetime.timezone.utc)
    result = read_json(root / "jobs" / identity / "result.json")
    directory = root / "matrix" / identity
    matrix = read_json(directory / "matrix.json")
    if not isinstance(matrix, dict):
        return {"result": result, "progress": None}
    try:
        started = datetime.datetime.strptime(identity.split("-")[0], "%Y%m%dT%H%M%SZ").replace(tzinfo=datetime.timezone.utc)
        elapsed = max(0, int((now - started).total_seconds()))
    except ValueError:
        elapsed = None
    completed = {row["row"]: row for row in matrix.get("rows", [])}
    rows = []
    for name in matrix.get("selectedRows", []):
        if not isinstance(name, str) or not re.fullmatch(r"[a-z+-]+(?:/[a-z-]+)?", name):
            continue
        if name in completed:
            rows.append({"row": name, "status": "passed" if completed[name].get("passed") is True else "failed", "phase": None})
            continue
        row_dir = directory / "rows" / name.replace("/", "-")
        phase = None
        if row_dir.is_dir():
            latest = ("", "")
            for report in row_dir.iterdir():
                if not report.is_dir() or report.is_symlink():
                    continue
                for role in ("controller", "a", "r", "b"):
                    try:
                        with (report / (role + ".log")).open("rb") as stream:
                            stream.seek(max(0, stream.seek(0, 2) - 16384))
                            tail = stream.read().decode(errors="replace")
                    except OSError:
                        continue
                    for line in tail.splitlines():
                        try:
                            event = json.loads(line)
                            fields = event.get("fields", {})
                            value, timestamp = fields.get("phase"), event.get("timestamp")
                        except (ValueError, AttributeError):
                            continue
                        if isinstance(value, str) and re.fullmatch(r"[a-z0-9_-]{1,96}", value) and isinstance(timestamp, str):
                            latest = max(latest, (timestamp, role + ":" + value))
            phase = latest[1] or None
        rows.append({"row": name, "status": "running" if row_dir.is_dir() else "queued", "phase": phase})
    return {"result": result, "progress": {"status": matrix.get("status"), "elapsedSeconds": elapsed, "rows": rows}}


def display(progress):
    if progress is None:
        return None
    elapsed = progress["elapsedSeconds"]
    duration = "?" if elapsed is None else f"{elapsed // 60}m{elapsed % 60:02d}s"
    rows = "; ".join(row["row"] + "=" + (row["phase"] or row["status"]) for row in progress["rows"])
    return f"[remote {duration}] {progress['status']} | {rows}"


if __name__ == "__main__":
    print(json.dumps(snapshot(sys.argv[1], sys.argv[2])))
