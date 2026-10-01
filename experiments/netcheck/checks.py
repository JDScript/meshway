"""Strict JSON contract checks and independent evidence cross-checks (no NAT taxonomy)."""

from datetime import datetime, timezone
import ipaddress
import json
import math
import re

from wire import endpoint

REPORT_FIELDS = {
    "schema_version", "source", "measured_at", "completed_at", "valid_until",
    "bound_socket", "address_family", "udp_observed", "mapping_varies_by_destination",
    "mapping_stable_during_check", "samples",
}
SAMPLE_FIELDS = {"server", "outcome", "mapped_endpoint", "rtt_ms", "elapsed_ms"}
OUTCOMES = {"mapped", "timeout", "invalid_response", "io_error", "stun_error"}


class CheckError(ValueError):
    """Messages are fixed fixture text, never external response/log content."""


def require(condition, message):
    if not condition:
        raise CheckError(message)


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result

    def invalid_constant(_value):
        raise CheckError("non-finite JSON constant")

    require(isinstance(raw, str) and len(raw.encode()) <= 65536, "JSON output exceeds bound")
    try:
        return json.loads(raw, object_pairs_hook=pairs, parse_constant=invalid_constant)
    except (ValueError, TypeError) as error:
        raise CheckError("invalid strict JSON output") from error


def parse_endpoint(value):
    require(isinstance(value, str) and len(value) <= 64, "invalid numeric endpoint")
    match = re.fullmatch(r"\[([^\]]+)\]:(\d+)|([^:]+):(\d+)", value)
    require(match is not None, "endpoint must include numeric address and port")
    address_text, port_text = (match[1], match[2]) if match[1] else (match[3], match[4])
    try:
        address = ipaddress.ip_address(address_text)
        port = int(port_text)
    except ValueError as error:
        raise CheckError("invalid numeric endpoint") from error
    require("%" not in address_text and 1 <= port <= 65535, "invalid endpoint scope or port")
    require((address.version == 6) == bool(match[1]), "invalid endpoint bracket family")
    return address, port


def timestamp(value):
    require(isinstance(value, str) and len(value) <= 40, "timestamp must be RFC3339")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise CheckError("invalid RFC3339 timestamp") from error
    require(parsed.tzinfo is not None and "T" in value, "timestamp must include timezone")
    return parsed.timestamp()


def number(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def validate_report(raw, exit_code, servers, started, finished):
    report = strict_json(raw)
    require(type(report) is dict and set(report) == REPORT_FIELDS, "unexpected report fields")
    require(type(report["schema_version"]) is int and report["schema_version"] == 1, "unsupported report schema")
    require(report["source"] == "independent_udp_probe", "probe source must be independent")
    measured = timestamp(report["measured_at"])
    completed = timestamp(report["completed_at"])
    expires = timestamp(report["valid_until"])
    require(started - 2 <= measured <= completed <= finished + 2, "report does not describe this invocation")
    require(completed < expires <= measured + 30, "report lifetime must be at most 30 seconds")
    require(report["address_family"] in ("ipv4", "ipv6"), "invalid address family")
    bound, _port = parse_endpoint(report["bound_socket"])
    require(report["address_family"] == f"ipv{bound.version}", "bound socket family mismatch")
    require(type(report["udp_observed"]) is bool, "udp_observed must be boolean")
    for field in ("mapping_varies_by_destination", "mapping_stable_during_check"):
        require(report[field] is None or type(report[field]) is bool, "mapping result must be boolean or null")
    samples = report["samples"]
    require(type(samples) is list and len(samples) == 3, "expected A,B,A on one socket")
    require(len(servers) == 2 and parse_endpoint(servers[0])[0] != parse_endpoint(servers[1])[0], "two different server IPs required")
    for sample, server in zip(samples, [servers[0], servers[1], servers[0]]):
        require(type(sample) is dict and set(sample) == SAMPLE_FIELDS, "unexpected sample fields")
        require(sample["server"] == server and sample["outcome"] in OUTCOMES, "unexpected sample server or outcome")
        require(number(sample["elapsed_ms"]), "invalid elapsed time")
        if sample["outcome"] == "mapped":
            address, _port = parse_endpoint(sample["mapped_endpoint"])
            require(address.version == bound.version, "mapped endpoint family mismatch")
            require(number(sample["rtt_ms"]), "mapped sample needs valid RTT")
        else:
            require(sample["mapped_endpoint"] is None and sample["rtt_ms"] is None, "failed sample must not contain a mapping")
    mapped = [sample for sample in samples if sample["outcome"] == "mapped"]
    require(report["udp_observed"] == bool(mapped), "udp_observed disagrees with valid samples")
    require(exit_code == (0 if mapped else 1), "exit code disagrees with observations")
    first, middle, last = samples
    stable = first["outcome"] == last["outcome"] == "mapped" and first["mapped_endpoint"] == last["mapped_endpoint"]
    if not all(sample["outcome"] == "mapped" for sample in samples) or not stable:
        require(report["mapping_varies_by_destination"] is None, "incomplete or changing mapping comparison must be unknown")
    else:
        require(report["mapping_varies_by_destination"] == (first["mapped_endpoint"] != middle["mapped_endpoint"]), "destination comparison disagrees with samples")
    if first["outcome"] == last["outcome"] == "mapped":
        require(report["mapping_stable_during_check"] in (stable, None), "stability disagrees with repeat A")
    else:
        require(report["mapping_stable_during_check"] is None, "stability must be unknown without both A observations")
    return report


def crosscheck(report, router_wan, ports, observed):
    """Expected kernel rule settings are separate from recvfrom and Agent evidence."""
    expected = [None if port is None else endpoint(router_wan, port) for port in ports]
    samples = report["samples"]
    require([s["mapped_endpoint"] for s in samples] == expected, "mapped endpoints differ from kernel NAT scenario")
    require([s["outcome"] for s in samples] == ["timeout" if p is None else "mapped" for p in ports], "unexpected scenario outcome")
    for name, indices in (("a", (0, 2)), ("b", (1,))):
        actual = observed[name]
        wanted = [expected[index] for index in indices if expected[index] is not None]
        require(actual == wanted, "actual observer peers differ from Agent mapping samples")
    if all(port is not None for port in ports):
        require(report["mapping_stable_during_check"] is True, "stable complete check must report stable")
    if all(port is None for port in ports):
        require(report["mapping_stable_during_check"] is None, "failed check stability must remain unknown")


def utc_now():
    return datetime.now(timezone.utc).timestamp()
