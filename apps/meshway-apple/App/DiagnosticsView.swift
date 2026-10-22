import SwiftUI

struct DiagnosticsView: View {
    @Bindable var model: AppModel
    var netcheckOnly = false
    @State private var checking = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var ping: PingPresentation { PingPresentation(events: model.pingEvents) }
    var body: some View {
        VStack(spacing: 12) {
            if !netcheckOnly { pingCard }
            netcheckCard
            Spacer(minLength: 0)
        }.padding(4).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
    private var pingCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
                TextField("Device, IP or domain", text: $model.pingTarget).textFieldStyle(.plain).onSubmit { model.ping() }
                Button(model.pingRunning ? String(localized: "Cancel") : "Ping") {
                    if model.pingRunning { model.cancelPing() } else { model.ping() }
                }.buttonStyle(.glassProminent).tint(.teal)
                    .disabled(!model.pingRunning && (model.sampleMode || model.pingTarget.isEmpty))
                    .help(unavailableHelp(model, fallback: model.pingTarget.isEmpty ? String(localized: "Enter a target first") : String(localized: "Sends four probes")))
            }.padding(8).background(.background.opacity(0.65), in: .rect(cornerRadius: 12))
            ZStack(alignment: .topLeading) {
                if model.pingEvents.isEmpty && !model.pingRunning {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 8) {
                            ForEach(Array((model.status?.peers ?? []).prefix(3)), id: \.nodeId) { peer in
                                Button { model.pingTarget = peer.node.hostname } label: { Text(verbatim: peer.node.hostname) }.buttonStyle(.glass).controlSize(.small)
                            }
                        }
                        Text("Shows the route, then measures latency.")
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                    }.padding(.top, 4)
                } else {
                    VStack(alignment: .leading, spacing: 7) {
                        if let decision = ping.decision { decisionRow(decision) }
                        else { MeshLoader(label: String(localized: "Finding route…"), labelVisible: true).frame(height: 26) }
                        if !ping.readings.isEmpty {
                            PingBars(readings: Array(ping.readings.suffix(24)))
                            HStack(spacing: 24) {
                                stat(String(localized: "Median"), ping.median.map { ms($0) } ?? "—")
                                stat(String(localized: "Min / max"), ping.durations.first.flatMap { low in ping.durations.last.map { ms(low) + " / " + ms($0) } } ?? "—")
                                stat(String(localized: "Replies"), ping.counts.map { "\($0.success) / \($0.sent)" } ?? "—")
                                stat(String(localized: "Loss"), ping.counts.flatMap { $0.sent > 0 ? (Double($0.timeout) / Double($0.sent)).formatted(.percent.precision(.fractionLength(0))) : nil } ?? "—")
                                    .help("Timeouts out of probes sent")
                            }
                            Text(ping.readings.contains(where: \.setup) ? "Lighter bars include connection setup and are left out of the stats." : "Empty slots had no reply.")
                                .font(.system(size: 10)).foregroundStyle(.tertiary)
                        } else if model.pingRunning { MeshLoader(label: String(localized: "Measuring…"), labelVisible: true).padding(.top, 18) }
                        else { Text("No replies yet").font(.caption).foregroundStyle(.secondary).padding(.top, 18) }
                    }
                }
            }.frame(height: 142, alignment: .top)
            Group {
                if let issue = ping.failure ?? model.issue { IssueRow(issue: issue, compact: true) } else { Color.clear }
            }.frame(height: 30, alignment: .top)
        }.padding(14).frame(maxWidth: .infinity).background(.background.opacity(0.5), in: .rect(cornerRadius: 18))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: model.pingEvents.count)
    }
    private func ms(_ value: Double) -> String { value.formatted(.number.precision(.fractionLength(2))) + " ms" }
    private func stat(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(value).font(.system(size: 13, weight: .semibold)).contentTransition(reduceMotion ? .identity : .numericText())
            Text(title).font(.system(size: 11)).foregroundStyle(.secondary)
        }
    }
    private func decisionRow(_ decision: ClientDecision) -> some View {
        HStack(spacing: 8) {
            ScrollView(.horizontal) {
                HStack(spacing: 5) {
                    ForEach(Array(path(decision).enumerated()), id: \.offset) { index, name in
                        if index > 0 { Image(systemName: "arrow.right").foregroundStyle(.tertiary) }
                        Text(verbatim: name).padding(.horizontal, 8).padding(.vertical, 4).background(.quaternary, in: Capsule())
                    }
                }
            }.scrollIndicators(.hidden)
            Text(summary(decision)).foregroundStyle(.secondary).lineLimit(1).help(summary(decision))
                .frame(maxWidth: 270, alignment: .trailing)
        }.font(.system(size: 11)).frame(height: 26)
    }
    private func path(_ decision: ClientDecision) -> [String] {
        var result = decision.path.map { id in id == model.status?.identity?.nodeId ? String(localized: "This Mac") : model.name(id) }
        // Empty path is unknown/unmanaged, not evidence of a direct circuit.
        if let destination = decision.destination, decision.node == nil { result.append(destination) }
        return result.isEmpty ? [String(localized: "Route unknown")] : result
    }
    private func summary(_ decision: ClientDecision) -> String {
        var parts: [String] = []
        if let profile = decision.profile { parts.append(String(localized: "Profile \(profile)")) }
        if let rule = decision.rule { parts.append(String(localized: "Rule \(rule.remark.isEmpty ? rule.pointer : rule.remark)")) }
        let action: String?
        switch decision.action {
        case .mesh: action = "Mesh"
        case .exit: action = String(localized: "Exit")
        case .direct: action = String(localized: "Direct from this Mac")
        case .unmanaged: action = String(localized: "This Mac’s network")
        case .denied: action = String(localized: "Blocked")
        case nil: action = nil
        }
        return parts.joined(separator: " · ") + (action.map { (parts.isEmpty ? "" : " → ") + $0 } ?? "")
    }
    private var netcheckCard: some View {
        Group {
            if let report = model.netcheck {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("Network check").fontWeight(.semibold)
                        RelativeTime(date: report.observedAt).font(.caption)
                        Spacer()
                        checkButton(String(localized: "Check again"))
                    }
                    if let issue = NetcheckPresentation(report: report).issue { IssueRow(issue: issue, compact: true) }
                    ScrollView {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 200), alignment: .leading)], alignment: .leading, spacing: 8) {
                            ForEach(NetcheckPresentation(report: report).facts) { fact in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(verbatim: fact.title).foregroundStyle(.secondary).lineLimit(1)
                                    Text(verbatim: fact.value).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                                }.font(.system(size: 11)).padding(9).frame(maxWidth: .infinity, alignment: .leading).help(fact.help ?? "")
                                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 10))
                            }
                        }
                    }
                }
            } else {
                QuietEmptyState(symbol: "waveform.path", title: String(localized: "Check this Mac’s network"),
                                explanation: String(localized: "Tests the server, UDP, IPv4, IPv6 and NAT."),
                                compact: true, actionTitle: checking ? String(localized: "Checking…") : String(localized: "Check"), action: runCheck,
                                actionDisabled: model.busy || model.sampleMode || checking,
                                actionHelp: unavailableHelp(model, fallback: String(localized: "Run a network check")))
            }
        }.frame(height: netcheckOnly ? 250 : 170).padding(14).frame(maxWidth: .infinity)
            .background(.background.opacity(0.5), in: .rect(cornerRadius: 18))
            .overlay(alignment: .topTrailing) { if checking { MeshLoader(label: String(localized: "Checking…")).padding(20) } }
    }
    private func checkButton(_ title: String) -> some View {
        Button(checking ? String(localized: "Checking…") : title, action: runCheck).buttonStyle(.glass).controlSize(.small)
            .disabled(model.busy || model.sampleMode || checking).help(unavailableHelp(model, fallback: String(localized: "Run a network check")))
    }
    private func runCheck() {
        checking = true
        Task { await model.perform(.netcheck); checking = false }
    }
}

struct PingBars: View {
    let readings: [PingReading]
    var body: some View {
        let maximum = max(readings.compactMap(\.duration).max() ?? 1, 1)
        HStack(alignment: .bottom, spacing: 4) {
            ForEach(readings) { reading in
                ZStack(alignment: .bottom) {
                    RoundedRectangle(cornerRadius: 3).strokeBorder(.quaternary).frame(height: 42)
                    if let duration = reading.duration {
                        RoundedRectangle(cornerRadius: 3).fill(.teal.opacity(reading.setup ? 0.35 : 0.85))
                            .frame(height: max(2, 42 * Double(duration) / Double(maximum)))
                    }
                }.frame(width: 9).help(reading.duration.map { milliseconds($0) + (reading.setup ? " · " + String(localized: "includes setup") : "") } ?? String(localized: "No reply"))
            }
            Spacer(minLength: 0)
        }.frame(height: 42).accessibilityLabel(Text("Recent latency samples"))
    }
}
