"""Commands apply exclusively inside the new router container's network namespace."""

SCENARIOS = (
    ("stable_snat", (41000, 41000, 41000)),
    ("destination_dependent_mapping", (41001, 41002, 41001)),
    ("endpoint_restricted_fixture", (41000, 41000, 41000)),
    ("udp_drop", (None, None, None)),
    ("partial", (41000, None, 41000)),
    ("mapping_change_before", (41003, 41003, 41003)),
    ("mapping_change_after", (41004, 41004, 41004)),
)


def router_commands(scenario, client, wan, server_a, server_b, ports):
    def rule(table, chain, *args):
        return ["iptables", "-w", "5", "-t", table, "-A", chain, *args]

    # This namespace and all its rules/conntrack entries belong to this fixture.
    commands = [
        ["conntrack", "-F"],
        ["iptables", "-w", "5", "-t", "nat", "-F"],
        ["iptables", "-w", "5", "-t", "filter", "-F"],
        ["iptables", "-w", "5", "-P", "FORWARD", "DROP"],
        ["iptables", "-w", "5", "-P", "INPUT", "DROP"],
        ["iptables", "-w", "5", "-P", "OUTPUT", "ACCEPT"],
    ]
    if scenario == "udp_drop":
        commands.append(rule("filter", "FORWARD", "-s", client, "-p", "udp", "-m", "comment", "--comment", "netcheck-drop-all", "-j", "DROP"))
    elif scenario == "partial":
        commands.append(rule("filter", "FORWARD", "-s", client, "-d", server_b, "-p", "udp", "--dport", "3478", "-m", "comment", "--comment", "netcheck-drop-b", "-j", "DROP"))
    for name, server, port in (("a", server_a, ports[0]), ("b", server_b, ports[1])):
        # Failed destinations have no SNAT rule. Forward drop is separately counted.
        if port is not None:
            commands.append(rule("nat", "POSTROUTING", "-s", client, "-d", server, "-p", "udp", "--dport", "3478", "-m", "comment", "--comment", f"netcheck-snat-{name}", "-j", "SNAT", "--to-source", f"{wan}:{port}"))
        commands.append(rule("filter", "FORWARD", "-s", client, "-d", server, "-p", "udp", "--dport", "3478", "-m", "comment", "--comment", f"netcheck-forward-{name}", "-j", "ACCEPT"))
    commands.append(rule("filter", "FORWARD", "-d", client, "-m", "conntrack", "--ctstate", "ESTABLISHED,RELATED", "-m", "comment", "--comment", "netcheck-return", "-j", "ACCEPT"))
    if scenario == "endpoint_restricted_fixture":
        # A never-contacted source port cannot match the SNAT reply tuple. The
        # resulting router-local unsolicited packet is counted, then dropped.
        commands.append(rule("filter", "INPUT", "-s", server_b, "-d", wan, "-p", "udp", "--sport", "3479", "--dport", "41000", "-m", "comment", "--comment", "netcheck-canary", "-j", "DROP"))
    return commands
