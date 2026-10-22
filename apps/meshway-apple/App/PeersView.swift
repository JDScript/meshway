import SwiftUI

struct PeerRow: View {
    let model: AppModel
    let peer: ClientPeer
    @State private var hovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "desktopcomputer").font(.title3).foregroundStyle(.secondary).frame(width: 32)
            VStack(alignment: .leading, spacing: 5) {
                Text(peer.node.hostname).font(.body.weight(.medium)).lineLimit(1)
                Text(peer.node.ipv4 ?? peer.node.ipv6 ?? "No address").font(peer.node.ipv4 == nil && peer.node.ipv6 == nil ? .caption : .caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                Text(model.peerPathDetail(peer) ?? "No active channel").font(.caption).foregroundStyle(.tertiary).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 7) {
                PathBadge(peer: peer)
                Text(peer.rtt.map { milliseconds($0.durationUs) } ?? "Not measured").font(.caption).foregroundStyle(.secondary).contentTransition(.numericText())
                    .help(peer.rtt.map { "Measured " + $0.observedAt.formatted() } ?? "No successful RTT measurement")
            }.frame(width: 100, alignment: .trailing)
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
        }.padding(.horizontal, 10).frame(height: 78).contentShape(Rectangle())
            .background(hovered ? Color.primary.opacity(0.045) : Color.clear, in: RoundedRectangle(cornerRadius: 10))
            .onHover { hovered = $0 }.animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: hovered)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: peer.rtt?.durationUs)
    }
}

struct PathBadge: View {
    let peer: ClientPeer
    private var title: String { peer.state == .idle || peer.state == .relayIdle ? "Idle" : human(peer.state) }
    var body: some View {
        Text(title).font(.system(size: 10, weight: .medium)).lineLimit(1).padding(.horizontal, 7).padding(.vertical, 3)
            .foregroundStyle(peer.state == .direct ? Color.green : Color.secondary)
            .background((peer.state == .direct ? Color.green : Color.secondary).opacity(0.09), in: Capsule())
            .help(peer.state == .idle || peer.state == .relayIdle ? "Idle path; no established channel" : title)
    }
}

struct CompactPeerRow: View {
    let model: AppModel
    let peer: ClientPeer
    @State private var hovered = false
    @State private var copied = false
    @State private var copyID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var address: String? { peer.node.ipv4 ?? peer.node.ipv6 }
    var body: some View {
        Button {
            if let address { copy(address); copied = true; copyID += 1 }
        } label: {
            HStack(spacing: 9) {
                Image(systemName: copied ? "checkmark.circle.fill" : "desktopcomputer")
                    .font(.body).foregroundStyle(copied ? Color.green : Color.secondary).frame(width: 22)
                    .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                VStack(alignment: .leading, spacing: 4) {
                    Text(peer.node.hostname).font(.callout.weight(.medium)).lineLimit(1)
                    Text(address ?? "No address").font(address == nil ? .caption : .caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                }.frame(maxWidth: .infinity, alignment: .leading)
                VStack(alignment: .trailing, spacing: 4) {
                    PathBadge(peer: peer)
                    Text(peer.rtt.map { milliseconds($0.durationUs) } ?? "—").font(.system(size: 10)).foregroundStyle(.secondary)
                        .contentTransition(.numericText()).frame(width: 63, alignment: .trailing)
                }.frame(width: 63, alignment: .trailing)
            }.padding(.horizontal, 7).frame(height: 54).contentShape(Rectangle())
                .background(hovered ? Color.primary.opacity(0.06) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
        }.buttonStyle(.plain).disabled(address == nil)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: copied)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: peer.rtt?.durationUs)
            .onHover { hovered = $0 }.animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: hovered)
            .help(copied ? "Address copied" : "Click to copy address · \(model.path(peer)) · \(peer.rtt.map { "RTT measured " + $0.observedAt.formatted() } ?? "RTT not measured")")
            .task(id: copyID) {
                guard copied else { return }
                do { try await Task.sleep(for: .seconds(1.6)) } catch { return }
                copied = false
            }
    }
}
struct PeerCopyActions: View {
    let peer: ClientPeer
    var body: some View {
        Button("Copy name") { copy(peer.node.hostname) }
        if let value = peer.node.ipv4 { Button("Copy IPv4") { copy(value) } }
        if let value = peer.node.ipv6 { Button("Copy IPv6") { copy(value) } }
        Button("Copy Node ID") { copy(peer.nodeId) }
    }
}
struct PeerDetail: View {
    let model: AppModel
    let peer: ClientPeer
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack { Text(peer.node.hostname).font(.title2.weight(.semibold)); Spacer(); Button("Done") { dismiss() } }
            AddressRow(title: "Mesh IPv4", value: peer.node.ipv4)
            AddressRow(title: "Mesh IPv6", value: peer.node.ipv6)
            LabeledContent("Node ID") { Text(peer.nodeId); CopyButton(value: peer.nodeId, title: "Copy Node ID") }
            Divider()
            LabeledContent("State", value: human(peer.state))
            LabeledContent("Path", value: model.path(peer))
            LabeledContent("Transport", value: peer.transport ?? "Not reported")
            if let mtu = peer.mtu { LabeledContent("MTU", value: "\(mtu.path) outer / \(mtu.inner) inner") }
            else { LabeledContent("MTU", value: missingMtu) }
            if let rtt = peer.rtt {
                HStack { Text("Last successful RTT"); Spacer(); Text(milliseconds(rtt.durationUs)); RelativeTime(date: rtt.observedAt) }
            } else { LabeledContent("RTT", value: "No successful measurement") }
            Button("Ping this peer", systemImage: "waveform.path") { model.pingTarget = peer.node.hostname; model.ping(); AppNavigation.shared.page = .diagnostics; dismiss() }.disabled(model.sampleMode || model.pingRunning)
                .help(unavailableHelp(model, fallback: model.pingRunning ? "Wait for the current ping to finish" : "Measure this peer"))
        }.padding(28).frame(width: 560).textSelection(.enabled)
    }
    private var missingMtu: String {
        guard peer.state == .direct else { return "Not applicable to this path" }
        guard let transport = peer.transport else { return "Not reported" }
        return transport == "native_udp" ? "Not measured" : "Not applicable to this transport"
    }
}
struct ProfilesView: View {
    let model: AppModel
    let selection: ClientSelection
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Form {
            Section("Current selection") {
                LabeledContent("Netmap profile", value: selection.appliedProfile)
                LabeledContent("Installation", value: model.netmapApplied ? "Applied" : "Waiting to apply")
                LabeledContent("Requested", value: selection.requestedProfile ?? "Following default")
                LabeledContent("Default", value: selection.defaultProfile)
                LabeledContent("Selection", value: selection.locked ? "Locked by administrator" : "Available")
            }
            Section("Available profiles") {
                ForEach(selection.profiles, id: \.name) { profile in
                    HStack(spacing: 12) {
                        Image(systemName: profile.name == selection.appliedProfile ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(profile.name == selection.appliedProfile ? Color.accentColor : Color.secondary)
                            .font(.title3).contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                        VStack(alignment: .leading, spacing: 4) {
                            Text(profile.name).fontWeight(.medium)
                            Text(summary(profile)).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                        }
                        Spacer()
                        Button("Select") { Task { await model.perform(.selectProfile(profile: profile.name)) } }
                            .disabled(!model.canRequestSelection || selection.locked || model.busy || model.sampleMode)
                            .help(unavailableHelp(model, fallback: selection.locked ? "Locked by administrator" : (!model.canRequestSelection ? "Requires an active identity, netmap and instance" : "Request this profile from the Server")))
                    }.frame(height: 58)
                }
                Button("Use default profile") { Task { await model.perform(.clearProfile) } }
                    .disabled(!model.canRequestSelection || model.busy || model.sampleMode || selection.requestedProfile == nil)
                    .help(unavailableHelp(model, fallback: !model.canRequestSelection ? "Requires an active identity, netmap and instance" : (selection.requestedProfile == nil ? "Already following the default profile" : "Clear the persistent profile request")))
            }
            Section("Selection details") {
                LabeledContent("Requested availability", value: selection.requestedUnavailable.map(human) ?? "No issue reported")
                LabeledContent("Automatic profile", value: selection.auto?.profile ?? "Not applicable")
                LabeledContent("Automatic basis", value: selection.auto.map { human($0.basis) } ?? "Not applicable")
                LabeledContent("Declared region", value: selection.auto?.region ?? (selection.auto == nil ? "Not applicable" : "Not declared"))
            }
        }.formStyle(.grouped)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: selection.appliedProfile)
    }
    private func summary(_ p: ClientProfile) -> String {
        switch p.kind {
        case .mesh: return "Mesh only; other external traffic unmanaged"
        case .auto: return "Selects an allowed profile using the declared region"
        case .global: return "All external traffic · " + action(p.fallback)
        case .custom:
            var parts = ["\(p.rules) rules"]
            if !p.exits.isEmpty { parts.append("exits " + p.exits.map(model.name).joined(separator: ", ")) }
            if p.fallback != nil { parts.append("otherwise " + action(p.fallback)) }
            return parts.joined(separator: " · ")
        }
    }
    private func action(_ value: ClientRouteAction?) -> String {
        guard let value else { return "not specified" }
        switch value {
        case .direct: return "direct"
        case .unmanaged: return "unmanaged"
        case .exit(let nodes, let via): return "exit " + nodes.map(selector).joined(separator: ", ") + (via.isEmpty ? "" : " via " + via.map(selector).joined(separator: " → "))
        }
    }
    private func selector(_ value: String) -> String { value.hasPrefix("node:") ? model.name(String(value.dropFirst(5))) : value }
}
