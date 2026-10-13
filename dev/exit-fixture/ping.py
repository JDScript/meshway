"""`meshway ping` toward targets outside the Mesh (phase 13): the real CLI on
A prints its local decision first, then echoes. Validation reads only the
typed NDJSON events; the human output is kept as the operator's view."""
import json


def events(record):
    """The typed event stream of one `--json` run."""
    return [json.loads(line) for line in record["stdout"].splitlines() if line.strip()]


def decision(stream):
    """The decision precedes every other event; only preparing (a name
    still resolving) may come before it."""
    measured = [event for event in stream if event.get("type") != "preparing"]
    if not measured or measured[0].get("type") != "decision":
        raise RuntimeError("ping did not report its decision first")
    return measured[0]["decision"]


def echoes(stream):
    return [event["echo"] for event in stream if event.get("type") == "echo"]


def exit_ping(record, destination, exit_node, path, rule=None, resolved=None):
    """Decision `exit E` on `path`, then replies from `destination` that
    returned on the same full path, and a successful summary."""
    stream = events(record)
    decided = decision(stream)
    if (decided.get("destination") != destination
            or decided.get("action") != {"type": "exit", "node": exit_node}
            or decided.get("path") != path or decided.get("node") is not None):
        raise RuntimeError("ping decision is not exit E on the A→R→E path")
    if (decided.get("rule") or {}).get("pointer") != rule:
        raise RuntimeError("ping decision names another rule: " + json.dumps(decided.get("rule")))
    if resolved is not None:
        resolution = decided.get("resolution") or {}
        if (resolution.get("name") != resolved or resolution.get("resolver") != {"type": "exit", "node": exit_node}
                or destination not in resolution.get("addresses", [])):
            raise RuntimeError("the name did not resolve through E's resolver")
        association = resolution.get("association") or {}
        if resolved not in association.get("names", []) or association.get("remainingSeconds", 0) <= 0:
            raise RuntimeError("the decision does not show the live association")
    replies = [echo for echo in echoes(stream) if echo["outcome"] == "reply"]
    if (not replies or any(echo["from"] != destination or echo["path"] != path or echo["durationUs"] is None
                           for echo in replies)):
        raise RuntimeError("no echo reply from the destination through E")
    return finished(record, stream)


def local_ping(record, destination, action, rule):
    """Decision direct/unmanaged by `rule`, then replies from this host."""
    stream = events(record)
    decided = decision(stream)
    if (decided.get("destination") != destination or decided.get("action") != {"type": action}
            or decided.get("path") != [] or (decided.get("rule") or {}).get("pointer") != rule):
        raise RuntimeError("ping decision is not " + action + " by its rule")
    replies = [echo for echo in echoes(stream) if echo["outcome"] == "reply"]
    if not replies or any(echo["from"] != destination or echo["path"] for echo in replies):
        raise RuntimeError("no " + action + " echo reply")
    return finished(record, stream)


def denied_ping(record):
    """Decision denied: nothing is probed and the CLI fails."""
    stream = events(record)
    decided = decision(stream)
    if (decided.get("action") or {}).get("type") != "denied" or record["exitCode"] == 0:
        raise RuntimeError("revoked target was not denied")
    if any(event.get("type") in ("echo", "accepted", "sample", "completed") for event in stream):
        raise RuntimeError("a denied target was probed")
    if stream[-1] != {"type": "failed", "code": "permissionDenied"}:
        raise RuntimeError("a denied ping did not fail with permissionDenied")
    return decided["action"]


def finished(record, stream):
    last = stream[-1]
    if record["exitCode"] != 0 or last.get("type") != "completed" or last["success"] < 1:
        raise RuntimeError("ping did not complete with a reply")
    return last
