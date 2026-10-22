import SwiftUI

@MainActor
func peerSummary(_ peer: ClientPeer, model: AppModel) -> String {
    switch peer.state {
    case .direct: return peer.rtt.map { milliseconds($0.durationUs) } ?? peerStateName(.direct)
    case .relay: return peer.via.isEmpty ? peerStateName(.relay) : model.via(peer.via)
    case .idle, .relayIdle: return peerStateName(.idle)
    case .offline: return peerStateName(.offline)
    case .unreachable: return peerStateName(.unreachable)
    }
}

struct PeerDot: View {
    let peer: ClientPeer
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var color: Color {
        switch peer.state {
        case .direct: .green
        case .relay: .indigo
        case .idle, .relayIdle: .gray
        case .offline: .gray.opacity(0.35)
        case .unreachable: .orange
        }
    }
    var body: some View {
        Circle().fill(color).frame(width: 7, height: 7).accessibilityLabel(peerStateName(peer.state))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: peer.state)
    }
}

struct DevicesView: View {
    @Bindable var model: AppModel
    @State private var selected: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var peer: ClientPeer? { model.peers.first { $0.nodeId == selected } }

    private var connecting: Bool { ConnectionPresentation(model: model).state == .connecting }
    var body: some View {
        // With no device at all the empty state owns the whole content area.
        if !connecting && (model.status?.peers.isEmpty ?? true) {
            EmptyStateView(kind: serverEmptyState(model) ?? .devices).frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.background.opacity(0.45), in: .rect(cornerRadius: 16))
        } else { list }
    }
    private var list: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 6) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
                    TextField("Search devices", text: $model.search).textFieldStyle(.plain)
                    Text(verbatim: "\(model.peers.count)").foregroundStyle(.tertiary)
                }.font(.system(size: 12)).padding(.horizontal, 10).frame(height: 30)
                ScrollView {
                    LazyVStack(spacing: 1) {
                        if connecting { SkeletonRows(count: min(10, max(3, model.peers.count)), addresses: true) }
                        else {
                        ForEach(model.peers, id: \.nodeId) { peer in
                            Button { selected = peer.nodeId } label: {
                                HStack(spacing: 10) {
                                    PeerDot(peer: peer)
                                    Text(peer.node.hostname).fontWeight(.medium).frame(maxWidth: .infinity, alignment: .leading).lineLimit(1)
                                    Text(peer.node.ipv4 ?? peer.node.ipv6 ?? String(localized: "No address"))
                                        .font(.system(size: 11, design: .monospaced)).opacity(0.55).frame(width: 110, alignment: .leading).lineLimit(1)
                                    Text(peerSummary(peer, model: model)).font(.system(size: 11)).opacity(0.55)
                                        .frame(width: 125, alignment: .trailing).lineLimit(1)
                                        .help(model.path(peer))
                                }
                            }.buttonStyle(MenuRowStyle())
                                .background(selected == peer.nodeId ? Color.accentColor.opacity(0.12) : .clear, in: .rect(cornerRadius: 6))
                                .accessibilityAddTraits(selected == peer.nodeId ? .isSelected : [])
                                .contextMenu { PeerCopyActions(peer: peer) }.transition(.opacity)
                        }
                        }
                    }.padding(4)
                }.scrollIndicators(.hidden)
                    .overlay {
                        if !connecting, let kind = EmptyStateKind.devices(query: model.search, count: model.peers.count, server: serverEmptyState(model)) {
                            EmptyStateView(kind: kind)
                        }
                    }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.background.opacity(0.45), in: .rect(cornerRadius: 16))
            // A stable column keeps addresses and rows still as selection changes.
            ZStack(alignment: .top) {
                if let peer {
                    PeerDetail(model: model, peer: peer) { selected = nil }
                        .id(peer.nodeId).transition(reduceMotion ? .opacity : .move(edge: .trailing).combined(with: .opacity))
                } else if let kind = serverEmptyState(model) {
                    EmptyStateView(kind: kind, compact: true)
                }
            }.frame(width: 210).frame(maxHeight: .infinity)
        }.font(.system(size: 12)).clipped()
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: selected)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: model.peers.map(\.nodeId))
            .task {
                #if DEBUG
                if model.sampleMode && CommandLine.arguments.contains("--preview-selected") { selected = model.peers.first?.nodeId }
                #endif
            }
    }
}

struct CompactPeerRow: View {
    let model: AppModel
    let peer: ClientPeer
    @State private var copied = false
    @State private var copyID = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var address: String? { peer.node.ipv4 ?? peer.node.ipv6 }
    var body: some View {
        Button {
            if let address { copy(address); copied = true; copyID += 1 }
        } label: {
            HStack(spacing: 9) {
                PeerDot(peer: peer)
                Text(peer.node.hostname).lineLimit(1)
                Spacer(minLength: 3)
                Text(copied ? String(localized: "Copied") : peerSummary(peer, model: model)).font(.system(size: 11)).opacity(0.55).lineLimit(1)
            }
        }.buttonStyle(MenuRowStyle()).disabled(address == nil)
            .help(address.map { String(localized: "Click to copy") + " \($0) · " + model.path(peer) } ?? String(localized: "No address"))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.15), value: copied)
            .task(id: copyID) {
                guard copied else { return }
                do { try await Task.sleep(for: .seconds(1.5)) } catch { return }
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
        Button("Copy device ID") { copy(peer.nodeId) }
    }
}

struct PeerDetail: View {
    let model: AppModel
    let peer: ClientPeer
    var close: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(peer.node.hostname).font(.system(size: 15, weight: .semibold)).lineLimit(2)
                Spacer(minLength: 0)
                Button(action: close) { Image(systemName: "xmark").font(.system(size: 10)).foregroundStyle(.tertiary) }.buttonStyle(.plain).help("Close")
            }
            VStack(alignment: .leading, spacing: 7) {
                Text("Address").foregroundStyle(.secondary)
                if let value = peer.node.ipv4 { CopyText(value: value) }
                if let value = peer.node.ipv6 { CopyText(value: value) }
                if peer.node.ipv4 == nil && peer.node.ipv6 == nil { Text("None") }
            }
            VStack(alignment: .leading, spacing: 7) {
                Text("Route").foregroundStyle(.secondary)
                Text(path).fixedSize(horizontal: false, vertical: true)
                if let transport = peer.transport { Text(verbatim: transport).foregroundStyle(.secondary) }
            }
            VStack(alignment: .leading, spacing: 7) {
                Text("Latency").foregroundStyle(.secondary)
                Text(peer.rtt.map { milliseconds($0.durationUs) } ?? String(localized: "Not measured"))
                    .help(peer.rtt.map { String(localized: "Measured \($0.observedAt.formatted(.relative(presentation: .named)))") } ?? String(localized: "Not measured"))
            }
            Spacer()
            Button(action: {
                model.pingTarget = peer.node.hostname; model.ping(); AppNavigation.shared.page = .diagnostics
            }) { Label { Text(verbatim: "Ping") } icon: { Image(systemName: "waveform.path") } }.buttonStyle(.glass).frame(maxWidth: .infinity)
                .disabled(model.sampleMode || model.pingRunning)
                .help(unavailableHelp(model, fallback: model.pingRunning ? String(localized: "A ping is already running") : String(localized: "Measure latency to this device")))
        }.font(.system(size: 12)).padding(16).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(.background.opacity(0.55), in: .rect(cornerRadius: 16))
    }
    private var path: String {
        if peer.state == .direct { return peerStateName(.direct) }
        if peer.state == .relay { return peerSummary(peer, model: model) }
        return model.path(peer)
    }
}

struct ProfilesView: View {
    let model: AppModel
    let selection: ClientSelection
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Exit profiles").font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary).padding(.horizontal, 10)
            ProfileRows(model: model, selection: selection)
            if selection.profiles.count == 1 && selection.profiles.first?.kind == .mesh { EmptyStateView(kind: .mesh) }
            Spacer()
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct ProfileRows: View {
    let model: AppModel
    let selection: ClientSelection
    var compact = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var selectHelp: String {
        selection.locked ? String(localized: "Locked by your admin") : (!model.canRequestSelection ? String(localized: "Available when connected") : String(localized: "Switch to this profile"))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            ForEach(selection.profiles, id: \.name) { profile in
                Button { Task { await model.perform(.selectProfile(profile: profile.name)) } } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark").font(.system(size: 11, weight: .semibold)).frame(width: 12)
                            .opacity(profile.name == selection.appliedProfile && model.netmapApplied ? 1 : 0)
                        Text(verbatim: profile.name).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(exitNames(profile))
                            .font(.system(size: 11)).opacity(0.55).lineLimit(1)
                    }
                }.buttonStyle(MenuRowStyle())
                    .disabled(!model.canRequestSelection || selection.locked || model.busy || model.sampleMode)
                    .help(unavailableHelp(model, fallback: selectHelp))
                if profile.name == selection.appliedProfile && selection.appliedProfile != selection.defaultProfile {
                    Button(String(localized: "Use default (\(selection.defaultProfile))")) { Task { await model.perform(.clearProfile) } }
                        .buttonStyle(.link).font(.system(size: 11)).padding(.leading, 30).frame(height: 20)
                        .disabled(!model.canRequestSelection || model.busy || model.sampleMode || selection.requestedProfile == nil)
                        .help(unavailableHelp(model, fallback: !model.canRequestSelection ? String(localized: "Available when connected") : (selection.requestedProfile == nil ? String(localized: "Already using the default") : String(localized: "Go back to the default profile"))))
                }
            }
            if let unavailable = selection.requestedUnavailable {
                IssueRow(issue: AppIssue(.selection(unavailable)), compact: true).padding(.horizontal, 10)
            } else if !model.netmapApplied || model.notice != nil {
                Text(model.notice ?? String(localized: "Applying…"))
                    .font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(2).padding(.horizontal, 10)
            }
        }.font(.system(size: 13))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: selection.appliedProfile)
    }
    private func exitNames(_ profile: ClientProfile) -> String {
        guard !profile.exits.isEmpty else { return "" }
        let names = profile.exits.compactMap { id in model.status?.peers.first { $0.nodeId == id }?.node.hostname }
        guard names.count == profile.exits.count else { return String(localized: "Exit name unknown") }
        return String(localized: "Via \(names.formatted(.list(type: .and)))")
    }
}
