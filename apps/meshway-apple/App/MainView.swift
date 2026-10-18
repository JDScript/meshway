import SwiftUI

struct MainView: View {
    @Bindable var model: AppModel
    @State private var selectedPeer: String?
    @State private var showLogout = false
    @State private var advanced = false

    var body: some View {
        ScrollViewReader { proxy in
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                ConnectionHeader(model: model, compact: false)
                if model.sampleMode { SampleBanner() }
                if let issue = model.issue { NoticeCard(title: "Action needs attention", message: issue, error: true) }
                if let issue = model.ipcIssue { NoticeCard(title: "Node connection", message: issue, error: true) }
                if let notice = model.notice { NoticeCard(title: "Node operation", message: notice, error: false) }
                if model.status == nil { setup }
                if let status = model.status {
                    if status.phase == .awaitingApproval { ApprovalCard(model: model, status: status) }
                    if status.identity == nil && status.phase == .down { enrollment }
                    nodeCard(status)
                    if let routing = model.selection { ProfilesView(model: model, selection: routing) }
                    peerList(status).id("peers")
                    DiagnosticsView(model: model).id("diagnostics")
                    HStack {
                        Button("Rotate key", systemImage: "key.horizontal") { Task { await model.identityOperation(logout: false) } }
                            .disabled(!model.canRotateKey || model.busy || model.sampleMode)
                        Spacer()
                        Button("Log out of network…", role: .destructive) { showLogout = true }
                            .disabled(!model.canLogout || model.busy || model.sampleMode)
                    }
                }
            }.padding(28).frame(maxWidth: 1000)
                .frame(maxWidth: .infinity)
        }
        .task {
            #if DEBUG
            if model.sampleMode, let section = model.previewSection { proxy.scrollTo(section, anchor: .top) }
            #endif
        }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .toolbar {
            ToolbarItem { Button("Refresh", systemImage: "arrow.clockwise") { Task { await model.refresh() } }.disabled(model.busy || model.sampleMode) }
        }
        .confirmationDialog("Permanently remove this node from its network?", isPresented: $showLogout, titleVisibility: .visible) {
            Button("Log out", role: .destructive) { Task { await model.identityOperation(logout: true) } }
            Button("Cancel", role: .cancel) {}
        } message: { Text("The Server will delete this node. Its local identity is cleared after confirmation. An offline request remains pending.") }
        .sheet(isPresented: Binding(get: { selectedPeer != nil }, set: { if !$0 { selectedPeer = nil } })) {
            if let peer = model.status?.peers.first(where: { $0.nodeId == selectedPeer }) {
                PeerDetail(model: model, peer: peer)
            } else { Text("This peer is no longer in the installed netmap.").padding(32) }
        }
    }

    private var setup: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Set up Meshway on this Mac", systemImage: "network.badge.shield.half.filled").font(.title3.weight(.semibold))
            Text("Meshway runs inside a macOS network extension. Approve the extension and VPN configuration when macOS asks.").foregroundStyle(.secondary)
            HStack {
                Text("Network Extension").foregroundStyle(.secondary)
                Spacer()
                Text(model.tunnel.extensionState)
                Button("Enable extension…") { model.tunnel.activate() }.disabled(model.sampleMode)
            }
            Text(model.tunnel.message).font(.caption).foregroundStyle(.secondary)
            Divider()
            enrollment
            HStack {
                Text("VPN · \(model.vpnLabel)").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(model.tunnel.hasConfiguration ? "Connect" : "Create VPN & connect…") { Task { await model.connect(true) } }
                    .buttonStyle(.borderedProminent).disabled(model.busy || model.server.isEmpty || model.sampleMode)
            }
        }.card()
    }
    private var enrollment: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Server").font(.headline)
            TextField("https://mesh.example", text: $model.server).textFieldStyle(.roundedBorder)
                .disabled(model.status != nil || model.tunnel.hasConfiguration)
                .accessibilityLabel("Server URL")
            Text("Your browser will open to approve this Mac for your network.").font(.callout).foregroundStyle(.secondary)
            DisclosureGroup("Advanced enrollment", isExpanded: $advanced) {
                SecureField("Enrollment key (optional)", text: $model.enrollmentKey).textFieldStyle(.roundedBorder).padding(.top, 8)
                Text("A one-use enrollment key supplied by your administrator.").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
    private func nodeCard(_ status: ClientStatus) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("This Mac", systemImage: "laptopcomputer").font(.headline)
            if let node = status.node {
                Text(node.hostname).font(.title3.weight(.semibold))
                AddressRow(title: "Mesh IPv4", value: node.ipv4)
                AddressRow(title: "Mesh IPv6", value: node.ipv6)
            } else { Text(status.identity == nil ? "Not enrolled" : "No currently installed node addresses").foregroundStyle(.secondary) }
            Divider()
            LabeledContent("Server", value: status.server)
            HStack { Text("Data plane"); Spacer(); Text(human(status.dataPlane)); RelativeTime(date: status.observedAt) }
            HStack { Text("Sync"); Spacer(); Text(human(status.sync)); if let at = status.lastSyncAt { RelativeTime(date: at) } }
            if let syncError = status.syncError { LabeledContent("Sync issue", value: human(syncError)).foregroundStyle(.orange) }
            if let netmap = status.netmap {
                HStack { Text("Netmap lease"); Spacer(); RelativeTime(date: netmap.expiresAt) }
                LabeledContent("Netmap", value: "Revision \(netmap.reference.revision) · serial \(netmap.reference.serial)")
                if status.applied?.digest != netmap.reference.digest || status.applied?.serial != netmap.reference.serial {
                    Label("This netmap is not applied", systemImage: "exclamationmark.circle").foregroundStyle(.orange)
                }
                if !netmap.missingArtifacts.isEmpty { Text("\(netmap.missingArtifacts.count) rule-set artifacts unavailable").foregroundStyle(.orange) }
            } else { LabeledContent("Netmap lease", value: "No live lease") }
            if let identity = status.identity { LabeledContent("Key generation", value: String(identity.keyGeneration)) }
            if let error = status.lastError {
                NoticeCard(title: "Node · \(human(error))", message: status.lastErrorDetail, error: true)
            }
            if let error = status.networkError { NoticeCard(title: "Network · \(human(error.problem))", message: error.detail, error: true) }
            if let retry = status.retryAt { HStack { Text("Next retry"); Spacer(); RelativeTime(date: retry) } }
            if let at = status.lastNetworkChange { HStack { Text("Network changed"); Spacer(); RelativeTime(date: at) } }
        }.font(.callout).card()
    }
    private func peerList(_ status: ClientStatus) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("Peers").font(.headline); Text("\(status.peers.count)").foregroundStyle(.secondary); Spacer() }
            HStack { Image(systemName: "magnifyingglass").foregroundStyle(.secondary); TextField("Search name or Mesh address", text: $model.search).textFieldStyle(.plain) }.padding(10).background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 8))
            if model.peers.isEmpty {
                ContentUnavailableView(model.search.isEmpty ? (status.node == nil ? "Peers unavailable" : "No visible peers") : "No matching peers", systemImage: "network", description: Text(model.search.isEmpty ? (status.node == nil ? "No netmap is currently installed." : "The installed netmap contains no other nodes.") : "Try another name or address."))
            } else {
                VStack(spacing: 0) {
                    ForEach(model.peers, id: \.nodeId) { peer in
                        Button { selectedPeer = peer.nodeId } label: { PeerRow(model: model, peer: peer) }.buttonStyle(.plain)
                            .contextMenu { PeerCopyActions(peer: peer) }
                        if peer.nodeId != model.peers.last?.nodeId { Divider().padding(.leading, 46) }
                    }
                }
            }
        }.card()
    }
}

struct ConnectionHeader: View {
    let model: AppModel
    var compact: Bool
    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "network").font(compact ? .title2 : .largeTitle).foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 5) {
                Text(compact ? "Meshway" : model.title).font(compact ? .headline : .title2.weight(.semibold))
                HStack(spacing: 6) {
                    Circle().fill(model.status?.phase == .running ? Color.green : Color.secondary).frame(width: 6, height: 6)
                    Text(model.phase).font(.callout)
                }
                Text("macOS VPN · \(model.vpnLabel)").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if model.busy { ProgressView().controlSize(.small) }
            Toggle("Connect", isOn: Binding(get: { model.status?.desiredUp ?? false }, set: { enabled in Task { await model.connect(enabled) } }))
                .labelsHidden().toggleStyle(.switch)
                .disabled(model.busy || model.sampleMode || (model.status == nil && model.server.isEmpty))
                .help("Change the Node's desired connection state")
        }
    }
}
struct ApprovalCard: View {
    let model: AppModel
    let status: ClientStatus
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Approve this Mac in your browser", systemImage: "person.badge.key").font(.headline)
            Text("Waiting for approval. Meshway will continue automatically after your administrator approves this request.").foregroundStyle(.secondary)
            if let link = status.enrollmentUrl, let url = model.approvalURL(link) {
                Button("Open approval page", systemImage: "arrow.up.right.square") { NSWorkspace.shared.open(url) }.disabled(model.sampleMode)
            } else { Text("No approval URL was provided. Ask the Server operator to configure its Console origin.").foregroundStyle(.orange) }
        }.card()
    }
}
struct SampleBanner: View {
    var body: some View { Label("SAMPLE DATA · Debug preview · actions disabled", systemImage: "testtube.2").font(.caption.weight(.semibold)).foregroundStyle(.orange).help("Preview only. No Node or VPN connection is opened, and system-changing actions are disabled.") }
}
struct NoticeCard: View {
    let title: String
    let message: String?
    let error: Bool
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: error ? "exclamationmark.circle" : "info.circle").font(.callout.weight(.semibold))
            if let message { Text(message).font(.callout).textSelection(.enabled) }
        }.frame(maxWidth: .infinity, alignment: .leading).padding(14)
            .foregroundStyle(error ? Color.orange : Color.secondary)
            .background((error ? Color.orange : Color.accentColor).opacity(0.07), in: RoundedRectangle(cornerRadius: 10))
    }
}
struct RelativeTime: View {
    let date: Date
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            Text(date, format: .relative(presentation: .named)).foregroundStyle(.secondary)
                .help(date.formatted(date: .complete, time: .complete))
        }
    }
}
struct AddressRow: View {
    let title: String
    let value: String?
    var body: some View {
        HStack {
            Text(title).foregroundStyle(.secondary)
            Spacer()
            if let value {
                Text(value).font(.callout.monospaced()).textSelection(.enabled)
                Button { copy(value) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless).help("Copy \(title)")
            } else { Text("Not assigned").foregroundStyle(.tertiary) }
        }
    }
}
extension View {
    func card() -> some View { padding(20).frame(maxWidth: .infinity, alignment: .leading).background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 14)).overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.primary.opacity(0.06))) }
}
func copy(_ value: String) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(value, forType: .string) }
func human<T>(_ value: T) -> String {
    String(describing: value).replacingOccurrences(of: "([a-z])([A-Z])", with: "$1 $2", options: .regularExpression).capitalized
}
func milliseconds(_ value: UInt64) -> String { String(format: "%.2f ms", Double(value) / 1000) }
