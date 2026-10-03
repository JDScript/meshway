"""Track a Docker creation before dispatch and defer cancellation until it settles."""
from dataclasses import dataclass
import json
import signal


@dataclass
class Creation:
    kind: str
    name: str
    label: str
    owner: str
    image: str | None = None
    network: str | None = None
    require_stopped: bool = False
    settled: bool = False

    def perform(self, operation):
        pending = []
        previous = {sig: signal.signal(sig, lambda number, _frame: pending.append(number))
                    for sig in (signal.SIGINT, signal.SIGTERM)}
        try:
            result = operation()
            self.settled = True
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
        if pending:
            raise KeyboardInterrupt("cancelled after Docker creation settled")
        return result

    def cleanup(self, docker):
        # A successful listing establishes absence; an arbitrary inspect failure
        # cannot be treated as proof that a create never committed.
        args = ("ps", "--all", "--no-trunc") if self.kind == "container" else ("network", "ls", "--no-trunc")
        entries = [json.loads(line) for line in docker(*args, "--format", "{{json .}}").splitlines() if line]
        key = "Names" if self.kind == "container" else "Name"
        matches = [entry for entry in entries if entry[key].lstrip("/") == self.name]
        if len(matches) > 1:
            raise RuntimeError("ambiguous Docker resource name: " + self.name)
        if matches:
            identifier = matches[0]["ID"]
            command = ("inspect", identifier) if self.kind == "container" else ("network", "inspect", identifier)
            values = json.loads(docker(*command))
            if len(values) != 1:
                raise RuntimeError("ambiguous Docker inspection: " + self.name)
            value = values[0]
            labels = value["Config"].get("Labels", {}) if self.kind == "container" else value.get("Labels", {})
            if value["Name"].lstrip("/") != self.name or labels.get(self.label) != self.owner:
                raise RuntimeError("Docker resource ownership differs: " + self.name)
            if self.kind == "container":
                host = value["HostConfig"]
                if value["Image"] != self.image or host["NetworkMode"] != self.network or host["Privileged"]:
                    raise RuntimeError("Docker container identity differs: " + self.name)
                if self.require_stopped and value["State"]["Running"] is not False:
                    raise RuntimeError("extraction container unexpectedly running: " + self.name)
                docker("rm", "--force", identifier)
            else:
                docker("network", "rm", identifier)
        if not self.settled:
            # A create timeout/lost transport result is still uncertain even if
            # no object is visible yet. Retain its exact name and report failure.
            raise RuntimeError("Docker creation outcome remains uncertain: " + self.name)
