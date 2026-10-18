import SwiftUI

struct DiagnosticsView: View {
    @Bindable var model: AppModel
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Label("Diagnostics", systemImage: "waveform.path.ecg").font(.headline)
                Spacer()
                Button("Run netcheck") { Task { await model.perform(.netcheck) } }.disabled(model.busy || model.sampleMode)
            }
            if let report = model.netcheck {
                HStack { Text("Network check").font(.subheadline.weight(.semibold)); Spacer(); RelativeTime(date: report.observedAt) }
                LabeledContent("Server", value: report.serverReachable.map { $0 ? "Reachable" : "Unreachable" } ?? "Not probed")
                if let rtt = report.serverRttUs { LabeledContent("Server RTT", value: milliseconds(rtt)) }
                if let reason = report.unavailable { Text("Network not checked: \(human(reason))").foregroundStyle(.secondary) }
                else {
                    Text("Fresh IPv4 mapping: \(report.ipv4 ? "yes" : "no") · IPv6: \(report.ipv6 ? "yes" : "no") · UDP: \(report.udp ? "yes" : "no")").font(.caption)
                    ForEach(Array(report.observers.enumerated()), id: \.offset) { _, observer in
                        Text("STUN \(observer.observer) · \(observer.rttUs.map(milliseconds) ?? "RTT ambiguous")").font(.caption.monospaced())
                        if let probe = observer.sizeProbe { Text("DF probe \(probe.size) bytes · \(probe.answered ? "answered" : "unanswered (not a size measurement)")").font(.caption).foregroundStyle(.secondary) }
                        if let mtu = observer.pathMtu { Text("Route MTU \(mtu)").font(.caption).foregroundStyle(.secondary) }
                    }
                    ForEach(Array(report.sockets.enumerated()), id: \.offset) { _, socket in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(socket.boundAddress).font(.caption.monospaced())
                            Text("Port: \(human(socket.portMapping)) · Mapping: \(human(socket.endpointMapping))").font(.caption).foregroundStyle(.secondary)
                            ForEach(Array(socket.mappings.enumerated()), id: \.offset) { _, mapping in Text("\(mapping.mapped) via \(mapping.observer)\(mapping.fresh ? "" : " (earlier)")").font(.caption.monospaced()) }
                        }
                    }
                }
            }
            Divider()
            HStack {
                TextField("Node name, Mesh IP, public IP or domain", text: $model.pingTarget).textFieldStyle(.roundedBorder).onSubmit { model.ping() }
                if model.pingRunning { Button("Cancel") { model.cancelPing() } }
                else { Button("Ping") { model.ping() }.disabled(model.sampleMode || model.pingTarget.isEmpty) }
            }
            if !model.pingEvents.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(model.pingEvents.enumerated()), id: \.offset) { _, event in Text(describe(event)).font(.caption.monospaced()).textSelection(.enabled) }
                }
            }
        }.card()
    }
    private func describe(_ event: ClientPingEvent) -> String {
        switch event {
        case .preparing(let elapsed): return "Preparing · \(elapsed) ms (not RTT)"
        case .accepted(_, _, let target): return "Measuring \(target.node.hostname)"
        case .decision(let decision):
            var text = "Destination \(decision.destination ?? "unresolved")"
            if let profile = decision.profile { text += " · profile \(profile)" }
            if let rule = decision.rule {
                text += " · rule \(rule.pointer) \(rule.remark)"
                if let match = rule.ruleSet { text += " [\(match.key): \(match.entry)]" }
            }
            if let action = decision.action { text += " · \(actionText(action))" }
            if !decision.path.isEmpty { text += "\nPath " + decision.path.map(model.name).joined(separator: " → ") }
            if let resolution = decision.resolution {
                text += "\nDNS \(resolution.name) · \(resolverText(resolution.resolver))"
                if let association = resolution.association { text += "\nTrusted association: \(association.names.joined(separator: ", ")) · \(association.remainingSeconds)s remaining at decision" }
            }
            return text
        case .sample(_, _, _, _, _, _, let probe):
            guard let probe else { return "Sample without probe data" }
            var text = "\(human(probe.outcome)) · \(probe.durationUs.map(milliseconds) ?? "RTT not measured") · " + probe.path.map(model.name).joined(separator: " → ")
            if let channel = probe.channels?.request { text += "\n\(channel.transport) \(channel.remoteAddress) · \(acquisitionText(channel.acquisition))" }
            return text
        case .echo(let echo):
            var text = "\(echo.sequence): \(human(echo.outcome)) · \(echo.durationUs.map(milliseconds) ?? "RTT not measured")\(echo.setup ? " (includes setup)" : "")"
            if let from = echo.from { text += " from \(from)" }
            if let error = echo.error { text += " · ICMP \(error.kind)/\(error.code)" }
            if !echo.path.isEmpty { text += "\n" + echo.path.map(model.name).joined(separator: " → ") }
            return text
        case .completed(let sent, let success, let timeout, let failed): return "\(sent) sent · \(success) replies · \(timeout) timeouts · \(failed) failed"
        case .failed(let code): return "Ping failed: \(human(code))"
        }
    }
    private func actionText(_ action: ClientPingAction) -> String {
        switch action {
        case .mesh: "Mesh"
        case .direct: "Direct"
        case .unmanaged: "Unmanaged"
        case .exit(let node): "Exit via \(model.name(node))"
        case .denied(let reason): "Denied: \(human(reason))"
        }
    }
    private func resolverText(_ resolver: ClientResolver) -> String {
        switch resolver {
        case .magicDns: "MagicDNS"
        case .system: "System resolver"
        case .unavailable: "No DNS path"
        case .exit(let node): "Exit resolver \(model.name(node))"
        case .resolvers(let keys): "Configured resolvers: \(keys.joined(separator: ", "))"
        }
    }
    private func acquisitionText(_ acquisition: ClientAcquisition) -> String {
        switch acquisition {
        case .publishedEndpoint: "Published endpoint"
        case .localEndpoint: "Local endpoint"
        case .stunEndpoint: "STUN endpoint"
        case .acceptedInbound: "Accepted inbound"
        case .coordinatedUdp(_, let source): "Coordinated UDP · \(human(source)) endpoint"
        }
    }
}
